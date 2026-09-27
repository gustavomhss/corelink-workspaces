#!/usr/bin/env python3
"""Fail-closed private B-216 D1 receipt notifier (code proof only)."""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

REPO = "HuGR-dev/corelink-workspaces"
CONTROL_TITLE = "[B-216 notifier control] durable cursor"
INCIDENT_PREFIX = "[B-216 incident]"
OVERDUE_MS = 15 * 60 * 1000
APPROVED_D1_ACCOUNT = "51284495e71acdb5a7677e7383ab026b"
RECEIPT_FIELDS = {"event_id", "schema_version", "event", "severity", "component", "exhausted", "requeue_count", "received_at_ms"}
EVENT_RE = re.compile(r"^dsr-erasure-dlq:[0-9a-f]{64}$")
ACK_RE = re.compile(r"^/ack ([0-9a-f]{64})$")


class FailClosed(RuntimeError):
    pass


def validate_receipt(row):
    if not isinstance(row, dict) or set(row) != RECEIPT_FIELDS:
        raise FailClosed("malformed receipt envelope")
    if not isinstance(row["event_id"], str) or not EVENT_RE.fullmatch(row["event_id"]):
        raise FailClosed("invalid opaque event id")
    if type(row["schema_version"]) is not int or row["schema_version"] != 1:
        raise FailClosed("invalid schema version")
    if row["event"] != "dsr.erasure.dead_letter" or row["severity"] != "critical" or row["component"] != "dsr-erasure-dlq":
        raise FailClosed("invalid receipt classification")
    if type(row["exhausted"]) is not int or row["exhausted"] != 1 or type(row["requeue_count"]) is not int or row["requeue_count"] not in (0, 1) or type(row["received_at_ms"]) is not int or row["received_at_ms"] < 0:
        raise FailClosed("invalid receipt values")
    return row


def digest(row):
    return row["event_id"].split(":", 1)[1]


def api(method, path, token, payload=None):
    if not token:
        raise FailClosed("missing GitHub authorization")
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request("https://api.github.com/repos/" + REPO + path, data=data, method=method,
                                 headers={"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=20) as response:
            body = response.read()
            return json.loads(body) if body else None
    except urllib.error.HTTPError as exc:
        raise FailClosed("GitHub API failure: HTTP " + str(exc.code)) from None
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError):
        raise FailClosed("GitHub API transport or response failure") from None


def issues(token, state="all"):
    found = []
    page = 1
    while page <= 100:
        batch = api("GET", "/issues?state=" + state + "&per_page=100&page=" + str(page), token)
        if not isinstance(batch, list):
            raise FailClosed("malformed issue listing")
        found.extend(i for i in batch if "pull_request" not in i)
        if len(batch) < 100:
            return found
        page += 1
    raise FailClosed("issue listing overflow")


def control_issue(token):
    matches = [i for i in issues(token) if i.get("title") == CONTROL_TITLE]
    if len(matches) > 1:
        raise FailClosed("duplicate control issue")
    return matches[0] if matches else None


def parse_cursor(issue):
    if issue is None:
        return (0, "")
    body = issue.get("body") or ""
    m = re.fullmatch(r"B216-CURSOR-V1\nreceived_at_ms=(0|[1-9][0-9]*)\nevent_id=(|dsr-erasure-dlq:[0-9a-f]{64})\n", body)
    if not m:
        raise FailClosed("malformed control cursor")
    return int(m.group(1)), m.group(2)


def cursor_body(cursor):
    return "B216-CURSOR-V1\nreceived_at_ms=%d\nevent_id=%s\n" % cursor


def incident_body(row):
    return ("B216-INCIDENT-V1\n" + json.dumps({"digest": digest(row), "component": row["component"], "severity": row["severity"], "status": "OPEN", "received_at_ms": row["received_at_ms"], "created_at": now_iso()}, sort_keys=True, separators=(",", ":")) + "\n\n@gmhelmold\n")


def now_iso():
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def current_time_ms():
    return int(dt.datetime.now(dt.timezone.utc).timestamp() * 1000)


def incident_meta(value, expected_digest):
    if not isinstance(value, dict):
        raise FailClosed("malformed incident metadata")
    allowed = {"digest", "component", "severity", "status", "received_at_ms", "created_at", "overdue_at", "renotified_at", "acknowledged_at", "acknowledged_by"}
    if set(value) - allowed:
        raise FailClosed("incident metadata is not closed")
    if value.get("digest") != expected_digest or value.get("component") != "dsr-erasure-dlq" or value.get("severity") != "critical":
        raise FailClosed("incident identity mismatch")
    if type(value.get("received_at_ms")) is not int or value["received_at_ms"] < 0:
        raise FailClosed("invalid incident receipt timestamp")
    if "created_at" not in value:
        raise FailClosed("incident creation timestamp missing")
    if value.get("status") not in ("OPEN", "OVERDUE", "ACKED"):
        raise FailClosed("invalid incident status")
    for key in ("created_at", "overdue_at", "renotified_at", "acknowledged_at"):
        if key in value:
            parse_timestamp(value[key])
    if value["status"] == "OPEN" and ("overdue_at" in value or "renotified_at" in value or "acknowledged_at" in value or "acknowledged_by" in value):
        raise FailClosed("OPEN incident has final-state attribution")
    if value["status"] == "OVERDUE" and ("overdue_at" not in value or "renotified_at" not in value or "acknowledged_at" in value or "acknowledged_by" in value):
        raise FailClosed("invalid overdue metadata")
    if value["status"] == "ACKED" and (value.get("acknowledged_by") != "gmhelmold" or "acknowledged_at" not in value):
        raise FailClosed("invalid acknowledgement metadata")
    return value


def parse_timestamp(value):
    if not isinstance(value, str) or not value:
        raise FailClosed("invalid timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise FailClosed("invalid timestamp") from None
    if parsed.tzinfo is None:
        raise FailClosed("timestamp has no timezone")
    return parsed


def incident_for(token, dg):
    title = INCIDENT_PREFIX + " " + dg
    found = [i for i in issues(token) if i.get("title") == title]
    if len(found) > 1:
        raise FailClosed("duplicate digest incidents")
    return found[0] if found else None


def create_or_reconcile(token, row):
    dg = digest(row)
    existing = incident_for(token, dg)
    if existing:
        reconcile_timeout(token, existing)
        return existing
    try:
        created = api("POST", "/issues", token, {"title": INCIDENT_PREFIX + " " + dg, "body": incident_body(row), "assignees": ["gmhelmold"]})
        if isinstance(created, dict) and created.get("title") == INCIDENT_PREFIX + " " + dg and type(created.get("number")) is int:
            return created
        raise FailClosed("malformed issue-create confirmation")
    except FailClosed:
        # The response may have been lost after GitHub committed the issue.
        reconciled = incident_for(token, dg)
        if reconciled:
            reconcile_timeout(token, reconciled)
            return reconciled
        raise


def save_cursor(token, issue, cursor):
    body = cursor_body(cursor)
    if issue is None:
        try:
            created = api("POST", "/issues", token, {"title": CONTROL_TITLE, "body": body})
            if isinstance(created, dict) and created.get("title") == CONTROL_TITLE and type(created.get("number")) is int:
                return created
            raise FailClosed("malformed cursor-create confirmation")
        except FailClosed:
            reread = control_issue(token)
            if reread and parse_cursor(reread) == cursor:
                return reread
            raise
    api("PATCH", "/issues/" + str(issue["number"]), token, {"body": body})
    return issue


def reconcile_timeout(token, incident):
    body = incident.get("body") or ""
    match = re.fullmatch(r"B216-INCIDENT-V1\n(\{[^\n]+\})\n(?:\n@gmhelmold\n)?", body)
    if not match:
        raise FailClosed("malformed incident envelope")
    meta = json.loads(match.group(1))
    meta = incident_meta(meta, incident.get("title", "").removeprefix(INCIDENT_PREFIX + " "))
    if meta.get("status") != "OPEN":
        return
    received = meta.get("received_at_ms")
    if type(received) is not int or current_time_ms() < received + OVERDUE_MS:
        return
    marker = "B216-OVERDUE-RENOTIFY-V1 " + meta["digest"]
    comments = incident_comments(token, incident["number"])
    if not has_overdue_marker(comments, meta["digest"]):
        try:
            api("POST", "/issues/" + str(incident["number"]) + "/comments", token,
                {"body": marker + "\n@gmhelmold B-216 incident " + meta["digest"] + " is overdue. Please acknowledge with `/ack " + meta["digest"] + "`."})
        except FailClosed:
            comments = incident_comments(token, incident["number"])
            if not has_overdue_marker(comments, meta["digest"]):
                raise
    meta.update(status="OVERDUE", overdue_at=meta.get("overdue_at", now_iso()), renotified_at=meta.get("renotified_at", now_iso()))
    api("PATCH", "/issues/" + str(incident["number"]), token,
        {"body": "B216-INCIDENT-V1\n" + json.dumps(meta, sort_keys=True, separators=(",", ":")) + "\n"})


def incident_comments(token, number):
    found = []
    for page in range(1, 101):
        batch = api("GET", "/issues/%s/comments?per_page=100&page=%d" % (number, page), token)
        if not isinstance(batch, list):
            raise FailClosed("malformed incident comments")
        found.extend(batch)
        if len(batch) < 100:
            return found
    raise FailClosed("incident comments overflow")


def has_overdue_marker(comments, dg):
    marker = "B216-OVERDUE-RENOTIFY-V1 " + dg
    expected = marker + "\n@gmhelmold B-216 incident " + dg + " is overdue. Please acknowledge with `/ack " + dg + "`."
    return any(c.get("body") == expected and (c.get("user") or {}).get("type") == "Bot" and (c.get("user") or {}).get("login") == "github-actions[bot]" for c in comments)


def read_d1_page(env, cursor, limit=100):
    token = env.get("B216_D1_READ_TOKEN")
    account = env.get("B216_D1_ACCOUNT_ID")
    database = env.get("B216_D1_DATABASE_ID")
    if not token or not account or not database:
        raise FailClosed("missing account-scoped D1 Read configuration")
    if account != APPROVED_D1_ACCOUNT:
        raise FailClosed("unapproved D1 account")
    sql = ("SELECT event_id,schema_version,event,severity,component,exhausted,requeue_count,received_at_ms "
           "FROM dsr_alert_receipts WHERE received_at_ms>? OR (received_at_ms=? AND event_id>?) "
           "ORDER BY received_at_ms,event_id LIMIT ?")
    url = "https://api.cloudflare.com/client/v4/accounts/%s/d1/database/%s/query" % (urllib.parse.quote(account, safe=""), urllib.parse.quote(database, safe=""))
    payload = json.dumps({"sql": sql, "params": [cursor[0], cursor[0], cursor[1], limit]}).encode()
    req = urllib.request.Request(url, data=payload, method="POST", headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=20) as response:
            decoded = json.loads(response.read())
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, json.JSONDecodeError):
        raise FailClosed("D1 read failed") from None
    if not isinstance(decoded, dict) or decoded.get("success") is not True or not isinstance(decoded.get("result"), list) or len(decoded["result"]) != 1:
        raise FailClosed("malformed D1 response")
    query_result = decoded["result"][0]
    if not isinstance(query_result, dict) or query_result.get("success") is not True:
        raise FailClosed("D1 query result was unsuccessful")
    rows = query_result.get("results")
    if not isinstance(rows, list) or len(rows) > limit:
        raise FailClosed("malformed or overflowing D1 page")
    checked = [validate_receipt(r) for r in rows]
    keys = [(r["received_at_ms"], r["event_id"]) for r in checked]
    if keys != sorted(set(keys)) or any(k <= cursor for k in keys):
        raise FailClosed("unstable D1 page order")
    return checked


def run_notifier(env):
    if env.get("B216_NOTIFIER_ENABLED") != "true":
        raise FailClosed("runtime disabled")
    if env.get("GITHUB_REPOSITORY") != REPO:
        raise FailClosed("wrong destination repository")
    gh_token = env.get("GH_TOKEN")
    if not gh_token:
        raise FailClosed("missing same-repo issue authorization")
    control = control_issue(gh_token)
    cursor = parse_cursor(control)
    while True:
        page = read_d1_page(env, cursor)
        if not page:
            break
        for row in page:
            create_or_reconcile(gh_token, row)
        cursor = (page[-1]["received_at_ms"], page[-1]["event_id"])
        control = save_cursor(gh_token, control, cursor)
        if len(page) < 100:
            break
    for incident in issues(gh_token):
        if re.fullmatch(re.escape(INCIDENT_PREFIX) + r" [0-9a-f]{64}", incident.get("title", "")):
            reconcile_timeout(gh_token, incident)


def handle_comment(env):
    if env.get("GITHUB_REPOSITORY") != REPO:
        raise FailClosed("wrong destination repository")
    event = json.loads(env.get("GITHUB_EVENT_JSON", "{}"))
    issue = event.get("issue") or {}
    comment = event.get("comment") or {}
    match = ACK_RE.fullmatch(comment.get("body", ""))
    if not match:
        return
    actor = (comment.get("user") or {}).get("login")
    if actor != "gmhelmold" or (comment.get("user") or {}).get("type") != "User" or (event.get("sender") or {}).get("login") != actor:
        raise FailClosed("unattributed acknowledgement")
    dg = match.group(1)
    token = env.get("GH_TOKEN")
    target = incident_for(token, dg)
    if not target or target.get("number") != issue.get("number"):
        raise FailClosed("ack target mismatch")
    parse_timestamp(comment.get("created_at"))
    body = target.get("body") or ""
    meta_match = re.fullmatch(r"B216-INCIDENT-V1\n(\{[^\n]+\})\n(?:\n@gmhelmold\n)?", body)
    if not meta_match:
        raise FailClosed("malformed incident metadata")
    meta = incident_meta(json.loads(meta_match.group(1)), dg)
    if meta.get("status") not in ("OPEN", "OVERDUE"):
        raise FailClosed("ack is early or state is final")
    acknowledged_at = parse_timestamp(comment["created_at"])
    if acknowledged_at < parse_timestamp(meta["created_at"]):
        raise FailClosed("ack predates incident creation")
    received_at = dt.datetime.fromtimestamp(meta["received_at_ms"] / 1000, tz=dt.timezone.utc)
    if acknowledged_at < received_at:
        raise FailClosed("ack predates receipt")
    meta.update(status="ACKED", acknowledged_at=comment.get("created_at"), acknowledged_by="gmhelmold")
    api("PATCH", "/issues/" + str(target["number"]), token, {"body": "B216-INCIDENT-V1\n" + json.dumps(meta, sort_keys=True, separators=(",", ":")) + "\n"})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("run", "ack"))
    args = parser.parse_args()
    try:
        (run_notifier if args.command == "run" else handle_comment)(os.environ)
    except (FailClosed, ValueError, KeyError, TypeError, AttributeError) as exc:
        print("notifier stopped: " + str(exc), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
