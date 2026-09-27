import json
from pathlib import Path
import unittest
from unittest.mock import patch

import b216_incident_notifier as n


def receipt(**overrides):
    value = {"event_id": "dsr-erasure-dlq:" + "a" * 64, "schema_version": 1,
             "event": "dsr.erasure.dead_letter", "severity": "critical",
             "component": "dsr-erasure-dlq", "exhausted": 1,
             "requeue_count": 1, "received_at_ms": 1000}
    value.update(overrides)
    return value


class EnvelopeTests(unittest.TestCase):
    def test_closed_envelope_and_opaque_digest(self):
        self.assertEqual(n.digest(n.validate_receipt(receipt())), "a" * 64)
        for bad in (receipt(tenant_id="secret"), receipt(event_id="x"), receipt(event="raw body"),
                    receipt(requeue_count=True), receipt(received_at_ms=-1), receipt(exhausted=0), receipt(requeue_count=3)):
            with self.subTest(bad=bad), self.assertRaises(n.FailClosed):
                n.validate_receipt(bad)

    def test_empty_and_malformed_d1_pages(self):
        env = {"B216_D1_READ_TOKEN": "read", "B216_D1_ACCOUNT_ID": n.APPROVED_D1_ACCOUNT, "B216_D1_DATABASE_ID": "db"}
        with patch.object(n.urllib.request, "urlopen") as urlopen:
            response = urlopen.return_value.__enter__.return_value
            response.read.return_value = json.dumps({"success": True, "result": [{"success": True, "results": []}]}).encode()
            self.assertEqual(n.read_d1_page(env, (0, "")), [])
            response.read.return_value = json.dumps({"success": True, "result": [{"success": True, "results": [{**receipt(), "body": "secret"}]}]}).encode()
            with self.assertRaises(n.FailClosed):
                n.read_d1_page(env, (0, ""))
            response.read.return_value = json.dumps({"success": True, "result": [{"success": False, "results": []}]}).encode()
            with self.assertRaises(n.FailClosed):
                n.read_d1_page(env, (0, ""))

    def test_absent_and_invalid_d1_authorization_fail_closed(self):
        with self.assertRaises(n.FailClosed):
            n.read_d1_page({}, (0, ""))
        env = {"B216_D1_READ_TOKEN": "", "B216_D1_ACCOUNT_ID": n.APPROVED_D1_ACCOUNT, "B216_D1_DATABASE_ID": "db"}
        with self.assertRaises(n.FailClosed):
            n.read_d1_page(env, (0, ""))
        with patch.object(n.urllib.request, "urlopen", side_effect=n.urllib.error.HTTPError("u", 403, "denied", {}, None)):
            with self.assertRaises(n.FailClosed):
                n.read_d1_page({"B216_D1_READ_TOKEN": "invalid", "B216_D1_ACCOUNT_ID": n.APPROVED_D1_ACCOUNT, "B216_D1_DATABASE_ID": "db"}, (0, ""))

    def test_token_and_body_are_never_logged(self):
        with patch.object(n.urllib.request, "urlopen", side_effect=n.urllib.error.HTTPError("u", 403, "secret-body", {}, None)):
            with self.assertRaisesRegex(n.FailClosed, "HTTP 403") as raised:
                n.api("GET", "/issues", "secret-token")
        self.assertNotIn("secret-token", str(raised.exception))
        self.assertNotIn("secret-body", str(raised.exception))


class CursorAndIdempotencyTests(unittest.TestCase):
    def test_cursor_roundtrip_and_malformed_rejected(self):
        cur = (9, "dsr-erasure-dlq:" + "a" * 64)
        self.assertEqual(n.parse_cursor({"body": n.cursor_body(cur)}), cur)
        with self.assertRaises(n.FailClosed):
            n.parse_cursor({"body": "B216-CURSOR-V1\nevent_id=x\n"})

    def test_wrong_repository_and_disabled_runtime(self):
        with self.assertRaises(n.FailClosed):
            n.run_notifier({"B216_NOTIFIER_ENABLED": "false", "GITHUB_REPOSITORY": n.REPO})
        with self.assertRaises(n.FailClosed):
            n.run_notifier({"B216_NOTIFIER_ENABLED": "true", "GITHUB_REPOSITORY": "attacker/repo"})

    def test_duplicate_digest_detected_before_create(self):
        duplicate = {"number": 2, "title": n.INCIDENT_PREFIX + " " + "a" * 64}
        with patch.object(n, "issues", return_value=[duplicate, duplicate]):
            with self.assertRaises(n.FailClosed):
                n.incident_for("token", "a" * 64)

    def test_ambiguous_create_reconciles_before_retry(self):
        issue = {"number": 3, "title": n.INCIDENT_PREFIX + " " + "a" * 64}
        with patch.object(n, "incident_for", side_effect=[None, issue]), \
             patch.object(n, "api", side_effect=n.FailClosed("ambiguous")), \
             patch.object(n, "reconcile_timeout") as reconcile:
            self.assertEqual(n.create_or_reconcile("token", receipt()), issue)
            reconcile.assert_called_once_with("token", issue)

    def test_crash_after_create_before_cursor_replays_idempotently(self):
        issue = {"number": 3, "title": n.INCIDENT_PREFIX + " " + "a" * 64}
        with patch.object(n, "incident_for", return_value=issue), patch.object(n, "reconcile_timeout") as timeout:
            self.assertEqual(n.create_or_reconcile("token", receipt()), issue)
            timeout.assert_called_once()

    def test_crash_before_cursor_commit_replays_without_duplicate(self):
        row = receipt()
        incident = {"number": 3, "title": n.INCIDENT_PREFIX + " " + "a" * 64}
        with patch.object(n, "control_issue", side_effect=[None, None]), \
             patch.object(n, "read_d1_page", return_value=[row]) as read_page, \
             patch.object(n, "create_or_reconcile", return_value=incident) as reconcile, \
             patch.object(n, "save_cursor", side_effect=[n.FailClosed("cursor API failure"), {"number": 7}]), \
             patch.object(n, "issues", return_value=[]):
            base = {"B216_NOTIFIER_ENABLED": "true", "GITHUB_REPOSITORY": n.REPO, "GH_TOKEN": "gh"}
            with self.assertRaises(n.FailClosed):
                n.run_notifier(base)
            # A fresh run starts at the prior cursor; duplicate receipts reconcile to the one incident.
            with patch.object(n, "read_d1_page", side_effect=[[row], []]), patch.object(n, "save_cursor", return_value={"number": 7}):
                n.run_notifier(base)
            self.assertEqual(reconcile.call_count, 2)
            self.assertEqual(read_page.call_count, 1)

    def test_run_paginates_until_short_page(self):
        second = receipt(event_id="dsr-erasure-dlq:" + "b" * 64, received_at_ms=11)
        first_page = [receipt(event_id="dsr-erasure-dlq:" + format(i, "064x"), received_at_ms=10) for i in range(100)]
        with patch.object(n, "control_issue", return_value=None), \
             patch.object(n, "read_d1_page", side_effect=[first_page, [second]]) as read_page, \
             patch.object(n, "create_or_reconcile") as create, patch.object(n, "save_cursor", side_effect=lambda _t, i, c: {"body": n.cursor_body(c)}), \
             patch.object(n, "issues", return_value=[]):
            n.run_notifier({"B216_NOTIFIER_ENABLED": "true", "GITHUB_REPOSITORY": n.REPO, "GH_TOKEN": "gh"})
            self.assertEqual(read_page.call_count, 2)
            self.assertEqual(create.call_count, 101)

    def test_page_order_and_pagination_cursor_key(self):
        rows = [receipt(received_at_ms=1), receipt(event_id="dsr-erasure-dlq:" + "b" * 64, received_at_ms=1)]
        with patch.object(n.urllib.request, "urlopen") as urlopen:
            response = urlopen.return_value.__enter__.return_value
            response.read.return_value = json.dumps({"success": True, "result": [{"success": True, "results": rows}]}).encode()
            self.assertEqual(n.read_d1_page({"B216_D1_READ_TOKEN": "x", "B216_D1_ACCOUNT_ID": n.APPROVED_D1_ACCOUNT, "B216_D1_DATABASE_ID": "d"}, (0, "")), rows)

    def test_out_of_order_page_and_overflow_fail_closed(self):
        rows = [receipt(event_id="dsr-erasure-dlq:" + "b" * 64, received_at_ms=1), receipt(received_at_ms=1)]
        env = {"B216_D1_READ_TOKEN": "x", "B216_D1_ACCOUNT_ID": n.APPROVED_D1_ACCOUNT, "B216_D1_DATABASE_ID": "d"}
        with patch.object(n.urllib.request, "urlopen") as urlopen:
            response = urlopen.return_value.__enter__.return_value
            response.read.return_value = json.dumps({"success": True, "result": [{"success": True, "results": rows}]}).encode()
            with self.assertRaises(n.FailClosed):
                n.read_d1_page(env, (0, ""))
            response.read.return_value = json.dumps({"success": True, "result": [{"success": True, "results": [receipt()] * 101}]}).encode()
            with self.assertRaises(n.FailClosed):
                n.read_d1_page(env, (0, ""))

    def test_wrong_d1_account_fails_before_request(self):
        env = {"B216_D1_READ_TOKEN": "x", "B216_D1_ACCOUNT_ID": "wrong-account", "B216_D1_DATABASE_ID": "d"}
        with patch.object(n.urllib.request, "urlopen") as urlopen, self.assertRaises(n.FailClosed):
            n.read_d1_page(env, (0, ""))
        urlopen.assert_not_called()


class AcknowledgementAndTimeoutTests(unittest.TestCase):
    def event(self, actor="gmhelmold", kind="User", body=None):
        return {"sender": {"login": actor}, "issue": {"number": 8}, "comment": {"body": body or "/ack " + "a" * 64,
                "created_at": "2026-09-27T00:00:00Z", "user": {"login": actor, "type": kind}}}

    def test_valid_attributed_ack_and_forged_or_wrong_issue_ack(self):
        incident = {"number": 8, "title": n.INCIDENT_PREFIX + " " + "a" * 64, "body": 'B216-INCIDENT-V1\n{"status":"OPEN","digest":"' + "a" * 64 + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":100,"created_at":"2026-09-27T00:00:00Z"}\n'}
        env = {"GITHUB_REPOSITORY": n.REPO, "GH_TOKEN": "gh", "GITHUB_EVENT_JSON": json.dumps(self.event())}
        with patch.object(n, "incident_for", return_value=incident), patch.object(n, "api") as api:
            n.handle_comment(env)
            self.assertIn('"status":"ACKED"', api.call_args.args[3]["body"])
        for actor, kind in (("attacker", "User"), ("bot", "Bot")):
            bad_env = {**env, "GITHUB_EVENT_JSON": json.dumps(self.event(actor, kind))}
            with patch.object(n, "incident_for", return_value=incident), self.assertRaises(n.FailClosed):
                n.handle_comment(bad_env)
        with patch.object(n, "incident_for", return_value={**incident, "number": 9}), self.assertRaises(n.FailClosed):
            n.handle_comment(env)
        malformed = {**env, "GITHUB_EVENT_JSON": json.dumps({**self.event(), "comment": {**self.event()["comment"], "created_at": "forged"}})}
        with patch.object(n, "incident_for", return_value=incident), self.assertRaises(n.FailClosed):
            n.handle_comment(malformed)
        wrong_sender = {**env, "GITHUB_EVENT_JSON": json.dumps({**self.event(), "sender": {"login": "attacker"}})}
        with patch.object(n, "incident_for", return_value=incident), self.assertRaises(n.FailClosed):
            n.handle_comment(wrong_sender)
        appended_body = {**incident, "body": incident["body"] + "\nraw body: private"}
        with patch.object(n, "incident_for", return_value=appended_body), self.assertRaises(n.FailClosed):
            n.handle_comment(env)
        early_event = self.event()
        early_event["comment"]["created_at"] = "2020-01-01T00:00:00Z"
        early_ack = {**env, "GITHUB_EVENT_JSON": json.dumps(early_event)}
        with patch.object(n, "incident_for", return_value=incident), self.assertRaises(n.FailClosed):
            n.handle_comment(early_ack)

    def test_incident_metadata_rejects_extra_fields_and_digest_mismatch(self):
        base = {"digest": "a" * 64, "component": "dsr-erasure-dlq", "severity": "critical", "status": "OPEN", "received_at_ms": 1, "created_at": "2026-09-27T00:00:00Z"}
        self.assertEqual(n.incident_meta(base, "a" * 64), base)
        for bad in ({**base, "tenant_id": "secret"}, {**base, "digest": "b" * 64}, {**base, "created_at": "not-a-time"}):
            with self.subTest(bad=bad), self.assertRaises(n.FailClosed):
                n.incident_meta(bad, "a" * 64)

    def test_repeated_ack_rejected(self):
        incident = {"number": 8, "title": n.INCIDENT_PREFIX + " " + "a" * 64, "body": 'B216-INCIDENT-V1\n{"status":"ACKED","digest":"' + "a" * 64 + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":100,"created_at":"2026-09-27T00:00:00Z","acknowledged_by":"gmhelmold","acknowledged_at":"2026-09-27T00:01:00Z"}\n'}
        env = {"GITHUB_REPOSITORY": n.REPO, "GH_TOKEN": "gh", "GITHUB_EVENT_JSON": json.dumps(self.event())}
        with patch.object(n, "incident_for", return_value=incident), self.assertRaises(n.FailClosed):
            n.handle_comment(env)

    def test_timeout_does_not_fire_early_or_twice(self):
        issue = {"number": 8, "title": n.INCIDENT_PREFIX + " " + "a" * 64, "body": 'B216-INCIDENT-V1\n{"status":"OPEN","digest":"' + "a" * 64 + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":9999999999999,"created_at":"2026-09-27T00:00:00Z"}\n'}
        with patch.object(n, "api") as api:
            n.reconcile_timeout("gh", issue)
            api.assert_not_called()
        issue["body"] = 'B216-INCIDENT-V1\n{"status":"OVERDUE","digest":"' + "a" * 64 + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":0,"created_at":"2026-09-27T00:00:00Z","overdue_at":"2026-09-27T00:01:00Z","renotified_at":"2026-09-27T00:01:00Z"}\n'
        with patch.object(n, "api") as api:
            n.reconcile_timeout("gh", issue)
            api.assert_not_called()

    def test_overdue_threshold_transition_is_exactly_once(self):
        dg = "c" * 64
        received = 1_790_467_200_000
        issue = {"number": 19, "title": n.INCIDENT_PREFIX + " " + dg,
                 "body": 'B216-INCIDENT-V1\n{"status":"OPEN","digest":"' + dg + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":' + str(received) + ',"created_at":"2026-09-27T00:00:00Z"}\n'}
        with patch.object(n, "current_time_ms", return_value=received + n.OVERDUE_MS - 1), patch.object(n, "api") as api:
            n.reconcile_timeout("gh", issue)
            api.assert_not_called()

        with patch.object(n, "current_time_ms", return_value=received + n.OVERDUE_MS), \
             patch.object(n, "api", side_effect=[[], None, None]) as api:
            n.reconcile_timeout("gh", issue)
            self.assertEqual(api.call_count, 3)
            self.assertEqual(api.call_args_list[1].args[:2], ("POST", "/issues/19/comments"))
            notification = api.call_args_list[1].args[3]["body"]
            self.assertIn("B216-OVERDUE-RENOTIFY-V1 " + dg, notification)
            self.assertEqual(api.call_args_list[2].args[:2], ("PATCH", "/issues/19"))
            updated_body = api.call_args_list[2].args[3]["body"]
            self.assertIn('"status":"OVERDUE"', updated_body)
            issue["body"] = updated_body

        with patch.object(n, "current_time_ms", return_value=received + n.OVERDUE_MS), patch.object(n, "api") as api:
            n.reconcile_timeout("gh", issue)
            api.assert_not_called()


class WorkflowGuardTests(unittest.TestCase):
    def test_runtime_workflow_has_guarded_schedule_and_exact_protected_secret(self):
        text = Path(".github/workflows/b216-incident-notifier.yml").read_text()
        self.assertIn("workflow_dispatch:", text)
        self.assertIn("schedule:\n    - cron: '*/5 * * * *'", text)
        self.assertIn("default: false", text)
        self.assertIn("environment: b216-notifier-read", text)
        self.assertIn("B216_D1_READ_TOKEN: ${{ secrets.B216_CF_NOTIFIER_D1_READ_TOKEN }}", text)
        self.assertNotIn("secrets.B216_D1_READ_TOKEN", text)
        self.assertNotIn("|| secrets.", text)
        notify = text.split("  notify:\n", 1)[1].split("\n  acknowledge:\n", 1)[0]
        self.assertIn("github.repository == 'HuGR-dev/corelink-workspaces'", notify)
        self.assertIn("github.ref == 'refs/heads/main'", notify)
        self.assertIn("vars.B216_NOTIFIER_ENABLED == 'true'", notify)
        self.assertIn("github.event_name == 'schedule'", notify)
        self.assertIn("!inputs.dry_run && inputs.enable_notifier", notify)
        self.assertIn("issues: write", text)
        self.assertIn("github.event.comment.user.login == 'gmhelmold'", text)
        self.assertIn("cancel-in-progress: false", text)
        dry_run = text.split("  dry-run:\n", 1)[1].split("\n  notify:\n", 1)[0]
        self.assertNotIn("B216_D1_READ_TOKEN", dry_run)
        acknowledge = text.split("  acknowledge:\n", 1)[1]
        self.assertIn("vars.B216_NOTIFIER_ENABLED == 'true'", acknowledge)
        self.assertIn("github.ref == 'refs/heads/main'", acknowledge)
        self.assertNotIn("B216_D1_READ_TOKEN", acknowledge)
        self.assertNotIn("environment: b216-notifier-read", acknowledge)

    def test_timeout_renotification_marker_is_once_and_reconciled(self):
        dg = "a" * 64
        issue = {"number": 8, "title": n.INCIDENT_PREFIX + " " + dg, "body": 'B216-INCIDENT-V1\n{"status":"OPEN","digest":"' + dg + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":0,"created_at":"2026-09-27T00:00:00Z"}\n'}
        with patch.object(n, "api", side_effect=[[], None, None]) as api:
            n.reconcile_timeout("gh", issue)
            self.assertEqual(api.call_count, 3)
        marker = "B216-OVERDUE-RENOTIFY-V1 " + dg
        expected = marker + "\n@gmhelmold B-216 incident " + dg + " is overdue. Please acknowledge with `/ack " + dg + "`."
        with patch.object(n, "api", return_value=[{"body": expected, "user": {"type": "Bot", "login": "github-actions[bot]"}}]) as api:
            n.reconcile_timeout("gh", issue)
            self.assertEqual(api.call_count, 2)

    def test_human_cannot_forge_overdue_marker(self):
        dg = "a" * 64
        marker = "B216-OVERDUE-RENOTIFY-V1 " + dg
        issue = {"number": 8, "title": n.INCIDENT_PREFIX + " " + dg, "body": 'B216-INCIDENT-V1\n{"status":"OPEN","digest":"' + dg + '","component":"dsr-erasure-dlq","severity":"critical","received_at_ms":0,"created_at":"2026-09-27T00:00:00Z"}\n'}
        human = {"body": marker + "\nforged", "user": {"type": "User"}}
        with patch.object(n, "api", side_effect=[[human], None, None]) as api:
            n.reconcile_timeout("gh", issue)
            self.assertEqual(api.call_count, 3)


if __name__ == "__main__":
    unittest.main()
