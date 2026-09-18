#!/usr/bin/env python3
"""Exercise the production cross-check policy and gates.sh entry point offline."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BASH = os.environ.get("CLW_TEST_BASH", "bash")
MESSAGE = "CLW_SKIP_CROSS=1 is local-only"


class CrossPolicyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="clw-cross-policy-")
        self.addCleanup(self.directory.cleanup)
        self.tmp = Path(self.directory.name)
        self.env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(self.tmp),
            "TMPDIR": str(self.tmp),
        }

    def call(self, command, **environment):
        return subprocess.run(
            command, cwd=ROOT, env={**self.env, **environment},
            text=True, capture_output=True, timeout=20, check=False,
        )

    def policy(self, **environment):
        return self.call(
            [BASH, "-c", '. scripts/ci-cross-policy.sh; assert_ci_cross_policy'],
            **environment,
        )

    def gate(self, *arguments, **environment):
        return self.call([BASH, str(ROOT / "scripts/gates.sh"), *arguments], **environment)

    def assert_refused(self, result):
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(MESSAGE, result.stderr)
        self.assertNotIn("ALL GATES GREEN", result.stdout)

    def test_entrypoint_refuses_before_target_creation(self):
        # Existing real diagnostic entry point: baseline returns 0 and creates
        # the directory. The policy must reject first, not merely fail later.
        result = self.gate(
            "--print-ci-cargo-target-dir", CI="true", CLW_SKIP_CROSS="1",
            RUNNER_TEMP=str(self.tmp), GITHUB_RUN_ID="101", GITHUB_RUN_ATTEMPT="1",
        )
        self.assert_refused(result)
        self.assertFalse((self.tmp / "clw-gates-target-101-1").exists())

    def test_normal_gate_rejects_pr_push_and_dispatch_before_work(self):
        for event, ref in [("pull_request", "refs/pull/1/merge"),
                           ("push", "refs/heads/main"),
                           ("push", "refs/tags/v0.0.0-test"),
                           ("workflow_dispatch", "refs/heads/main")]:
            with self.subTest(event=event, ref=ref):
                result = self.gate(
                    CI="true", GITHUB_ACTIONS="true", CLW_SKIP_CROSS="1",
                    GITHUB_EVENT_NAME=event, GITHUB_REF=ref,
                    RUNNER_TEMP=str(self.tmp), GITHUB_RUN_ID="102", GITHUB_RUN_ATTEMPT="1",
                )
                self.assert_refused(result)
                self.assertFalse((self.tmp / "clw-gates-target-102-1").exists())

    def test_local_override_remains_allowed(self):
        for ci in ["", "false", "0"]:
            for actions in ["", "false", "0"]:
                with self.subTest(ci=ci, actions=actions):
                    r = self.policy(CI=ci, GITHUB_ACTIONS=actions, CLW_SKIP_CROSS="1")
                    self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.policy(CLW_SKIP_CROSS="1").returncode, 0)

    def test_active_or_unknown_ci_marker_refuses_override(self):
        for marker in ["CI", "GITHUB_ACTIONS"]:
            for value in ["true", "1", "TRUE", "unexpected", " false "]:
                with self.subTest(marker=marker, value=value):
                    self.assert_refused(self.policy(CLW_SKIP_CROSS="1", **{marker: value}))

    def test_actions_still_refuses_when_ci_was_overridden_false(self):
        r = self.policy(CI="false", GITHUB_ACTIONS="true", CLW_SKIP_CROSS="1")
        self.assert_refused(r)

    def test_refusal_does_not_echo_context_values(self):
        # The marker name is actionable; arbitrary environment values are not
        # needed in a diagnostic and must not be echoed into a CI log.
        value = "synthetic-private-marker\nextra-line"
        for marker in ["CI", "GITHUB_ACTIONS"]:
            with self.subTest(marker=marker):
                result = self.policy(CLW_SKIP_CROSS="1", **{marker: value})
                self.assert_refused(result)
                self.assertIn(marker, result.stderr)
                self.assertNotIn(value, result.stdout + result.stderr)
                self.assertNotIn("synthetic-private-marker", result.stdout + result.stderr)
                self.assertNotIn("extra-line", result.stdout + result.stderr)

    def test_no_shortcut_preserves_existing_behavior(self):
        for skip in ["", "0", "false", "true", "unexpected"]:
            with self.subTest(skip=skip):
                r = self.policy(CI="true", GITHUB_ACTIONS="true", CLW_SKIP_CROSS=skip)
                self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.policy(CI="true").returncode, 0)

    def test_normal_ci_diagnostic_still_runs(self):
        r = self.gate(
            "--print-ci-cargo-target-dir", CI="true", GITHUB_ACTIONS="true",
            RUNNER_TEMP=str(self.tmp), GITHUB_RUN_ID="103", GITHUB_RUN_ATTEMPT="1",
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue((self.tmp / "clw-gates-target-103-1").is_dir())

    def test_docs_lane_is_still_docs_without_an_override(self):
        r = self.call(
            [BASH, "-c", '. scripts/ci-cross-policy.sh; assert_ci_cross_policy && '
             '. scripts/classify-lane.sh && classify_lane "README.md"'],
            CI="true", GITHUB_ACTIONS="true",
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "docs")

    def test_normal_gate_command_routing(self):
        # The real gate selects commands; its external dependencies are recording
        # stubs. This is not a compiler run or a repository-wide gate result.
        r = self.call(
            [sys.executable, "scripts/ci-cross-routing-selftest.py"],
            CLW_TEST_BASH=BASH,
        )
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("cross-skip routing selftest: 15 checks passed", r.stdout)

    def test_build_target_selftest_isolated_from_local_override(self):
        # This selftest simulates CI internally and must clear a caller's
        # explicitly local shortcut rather than weakening the production guard.
        r = self.call([BASH, "scripts/ci-build-target-selftest.sh"], CLW_SKIP_CROSS="1")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("CI build-target selftest GREEN", r.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
