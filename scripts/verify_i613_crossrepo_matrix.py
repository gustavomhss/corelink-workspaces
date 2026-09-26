#!/usr/bin/env python3
"""Check issue 613's source pins and write its exact-SHA run receipt."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
BASE_RUNNERS_SHA = "9e85d73d9c0bd83c265678f204161fe3d6adb973"
REPOS = {
    "workspaces": ("HuGR-Labs/corelink-workspaces", "1259579816", ROOT),
    "runners": (
        "HuGR-dev/corelink-runners",
        "1266754321",
        Path(os.environ.get("RUNNERS_DIRECTORY", ROOT.parent / "runners")),
    ),
}


def git(path: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(path), *args], text=True).strip()


def canonical_repository(remote: str) -> str | None:
    """Return an exact GitHub owner/repository path from a canonical origin URL."""
    match = re.fullmatch(
        r"(?:https://github\.com/|ssh://git@github\.com/|git@github\.com:)"
        r"([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?",
        remote,
    )
    return match.group(1).lower() if match else None


def sources() -> dict[str, object]:
    failures = []
    actual = {}
    if os.environ.get("GH_REPOSITORY") != REPOS["workspaces"][0]:
        failures.append("workflow is not running in the canonical private workspaces repository")
    workflow_sha = os.environ.get("GH_SHA", "")
    if not re.fullmatch(r"[0-9a-f]{40}", workflow_sha):
        failures.append("workflow definition is not bound to an exact 40-hex commit SHA")
    for key, (name, repo_id, path) in REPOS.items():
        sha = git(path, "rev-parse", "HEAD")
        remote = git(path, "remote", "get-url", "origin")
        requested = os.environ.get("WORKSPACES_SHA" if key == "workspaces" else "RUNNERS_SHA", "")
        configured_id = os.environ.get(
            "WORKSPACES_REPOSITORY_ID" if key == "workspaces" else "RUNNERS_REPOSITORY_ID", ""
        )
        if not re.fullmatch(r"[0-9a-f]{40}", requested) or requested != sha:
            failures.append(f"{key} checkout is not the requested exact 40-hex SHA")
        if configured_id != repo_id:
            failures.append(f"{key} repository ID does not match the canonical repository")
        if canonical_repository(remote) != name.lower():
            failures.append(f"{key} checkout remote has the wrong repository identity")
        actual[key] = {"repository": name, "repository_id": repo_id, "sha": sha}
    if failures:
        raise ValueError("; ".join(failures))
    return actual


def read_file(path: Path) -> str | None:
    try:
        return path.read_text(encoding="ascii")
    except OSError:
        return None


def runtime() -> tuple[dict[str, object], list[str]]:
    repos = sources()
    temp = Path(os.environ.get("RUNNER_TEMP", "/tmp"))
    code = int(os.environ["MATRIX_EXIT"])
    if code not in (0, 7, 125):
        raise ValueError("matrix exit code is outside the declared cases")
    suffix = str(code)
    effects_path = temp / f"issue-613-effects-{suffix}"
    try:
        effect_count = len(effects_path.read_bytes().splitlines())
    except OSError:
        effect_count = 0
    states = {
        phase: read_file(temp / f"issue-613-state-{suffix}-{phase}")
        for phase in ("first", "repeat")
    }
    outcome = {
        phase: os.environ.get(f"OUTCOME_{phase.upper()}", "missing")
        for phase in ("first", "repeat")
    }
    expected_count = 1 if code == 0 else 2
    expected_states = (
        {"first": "EXECUTED\n", "repeat": "NOT_STARTED\n"}
        if code == 0 else {"first": "EXECUTED\n", "repeat": "EXECUTED\n"}
    )
    expected_outcome = "success" if code == 0 else "failure"
    fallback = None
    if code == 0:
        fallback_count = len((temp / "issue-613-effects-fallback").read_bytes().splitlines()) \
            if (temp / "issue-613-effects-fallback").exists() else 0
        fallback = {
            "command": "Action optional fallback: printf x\\n >> EFFECT_LOG; exit 0",
            "endpoint": "http://127.0.0.1:1",
            "execution_state": read_file(temp / "issue-613-state-fallback"),
            "effect_count": fallback_count,
            "step_outcome": os.environ.get("OUTCOME_FALLBACK", "missing"),
        }

    request_path = Path(os.environ.get("I613_REQUEST_LOG", "/nonexistent"))
    requests = (request_path.read_text(encoding="ascii").splitlines()
                if request_path.exists() else [])
    receipt: dict[str, object] = {
        "schema": "corelink.issue-613.receipt.v1",
        "repositories": repos,
        "workflow_commit_sha": os.environ.get("GH_SHA", ""),
        "runners_sha_is_current_baseline": repos["runners"]["sha"] == BASE_RUNNERS_SHA,
        "workflow_run_url": (
            f"{os.environ.get('GH_SERVER_URL', 'https://github.com')}/"
            f"{os.environ.get('GH_REPOSITORY', '')}/actions/runs/{os.environ.get('GH_RUN_ID', '')}"
        ),
        "workflow_run_id": os.environ.get("GH_RUN_ID", ""),
        "hosted_runner": "ubuntu-24.04",
        "permissions": {"contents": "read"},
        "credentials": "GITHUB_TOKEN for checkout only; synthetic loopback test bearer",
        "service_boundary": "127.0.0.1 only; in-memory CAS/AC double; no provider endpoints",
        "case": {
            "child_exit": code,
            "action_input_run": "printf 'x\\n' >> \"$EFFECT_LOG\"; exit \"$CHILD_EXIT\"",
            "cli_argv_template": "clw run --input Cargo.toml --env CHILD_EXIT -- bash -c CL_RUN",
            "expected_effect_count": expected_count,
            "observed_effect_count": effect_count,
            "expected_execution_states": expected_states,
            "observed_execution_states": states,
            "expected_step_outcome": expected_outcome,
            "observed_step_outcomes": outcome,
        },
        "pre_execution_fallback": fallback,
        "local_service_requests": requests,
    }
    failures = []
    if effect_count != expected_count:
        failures.append(f"child exit {code} ran {effect_count} times; expected {expected_count}")
    if states != expected_states:
        failures.append("CLI execution-state receipt did not match the expected dispatch result")
    if any(value != expected_outcome for value in outcome.values()):
        failures.append("Action step outcome did not preserve the child verdict")
    if not requests or any(not re.fullmatch(
        r"(?:GET|PUT) /v1/(?:ac|cas)/issue-613-test/[a-f0-9]{64} (?:200|201|404)", line
    ) for line in requests):
        failures.append("loopback service request log is empty or contains an unexpected request")
    if fallback is not None and (
        fallback["effect_count"] != 1 or fallback["execution_state"] != "NOT_STARTED\n"
        or fallback["step_outcome"] != "success"
    ):
        failures.append("proven pre-execution failure did not fall back exactly once")
    return receipt, failures


def static() -> list[str]:
    """Check the workflow's bounded, secretless, exact-SHA contract locally."""
    path = ROOT / ".github/workflows/issue-613-crossrepo-matrix.yml"
    source = path.read_text(encoding="utf-8")
    failures = []
    required = (
        r"(?m)^on:\s*$",
        r"(?m)^  workflow_dispatch:\s*$",
        r"(?m)^permissions:\s*$",
        r"(?m)^  contents:\s*read\s*$",
        r"(?m)^    if:\s*github\.ref\s*==\s*'refs/heads/main'\s*$",
        r"(?m)^    runs-on:\s*ubuntu-24\.04\s*$",
        r"(?m)^    timeout-minutes:\s*([1-9][0-9]*)\s*$",
        r"(?m)^      WORKSPACES_REPOSITORY_ID:\s*['\"]?1259579816['\"]?\s*$",
        r"(?m)^      RUNNERS_REPOSITORY_ID:\s*['\"]?1266754321['\"]?\s*$",
        r"(?m)^      workspaces_sha:\s*$",
        r"(?m)^        required:\s*true\s*$",
        r"(?m)^          ref:\s*\$\{\{\s*inputs\.workspaces_sha\s*\}\}\s*$",
        r"(?m)^          ref:\s*\$\{\{\s*inputs\.runners_sha\s*\}\}\s*$",
        r"(?m)^          repository:\s*HuGR-Labs/corelink-workspaces\s*$",
        r"(?m)^          repository:\s*HuGR-dev/corelink-runners\s*$",
        r"(?m)^          persist-credentials:\s*false\s*$",
        r"(?m)^          python3 scripts/verify_i613_crossrepo_matrix\.py \\\s*$",
        r"(?m)^            --runtime(?:\s|$)",
    )
    for pattern in required:
        if not re.search(pattern, source):
            failures.append(f"workflow is missing required contract: {pattern}")

    trigger_lines = source.splitlines()
    try:
        trigger_index = next(i for i, line in enumerate(trigger_lines) if line == "on:")
        next_root = next(
            i for i in range(trigger_index + 1, len(trigger_lines))
            if trigger_lines[i] and not trigger_lines[i][0].isspace() and not trigger_lines[i].lstrip().startswith("#")
        )
        trigger_keys = [
            line.strip().rstrip(":") for line in trigger_lines[trigger_index + 1:next_root]
            if line.startswith("  ") and not line.startswith("    ") and line.strip()
        ]
        if trigger_keys != ["workflow_dispatch"]:
            failures.append("workflow must have workflow_dispatch as its only trigger")
    except (StopIteration, ValueError):
        failures.append("workflow trigger block is malformed")

    pins = [
        action for action in re.findall(r"(?m)^\s+uses:\s*([^\s#]+)", source)
        if not action.startswith("./")
    ]
    if not pins or any(not re.fullmatch(r"[^@]+@[0-9a-f]{40}", action) for action in pins):
        failures.append("every external Action must be pinned to a full commit SHA")

    timeout = re.search(r"(?m)^    timeout-minutes:\s*([1-9][0-9]*)\s*$", source)
    if timeout and int(timeout.group(1)) > 60:
        failures.append("workflow job timeout must not exceed 60 minutes")
    if len(re.findall(r"(?m)^\s+contents:\s*read\s*$", source)) < 2:
        failures.append("both workflow and job permissions must be contents: read")

    forbidden = (
        (r"(?i)corelink-dogfood", "workflow must not use corelink-dogfood"),
        (r"(?i)runs-on:\s*['\"]?self-hosted", "workflow must not use a self-hosted runner"),
        (r"\$\{\{\s*secrets\.", "workflow must not read repository secrets"),
        (r"(?i)\b(?:aws|azure|gcp|cloudflare)_?(?:access|secret|token|key)\b", "workflow must not configure provider credentials"),
        (r"(?i)\b(?:deploy|production|staging)\b", "workflow must not configure a deployment or production path"),
    )
    for pattern, message in forbidden:
        if re.search(pattern, source):
            failures.append(message)
    for required_file in (ROOT / "scripts/i613_local_ac_server.py", ROOT / "scripts/verify_i613_crossrepo_matrix.py"):
        if not required_file.is_file():
            failures.append(f"workflow helper is missing: {required_file.relative_to(ROOT)}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--static", action="store_true")
    parser.add_argument("--runtime", action="store_true")
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    try:
        if args.static:
            failures = static()
            if failures:
                print("issue 613 static contract: FAIL: " + "; ".join(failures), file=sys.stderr)
                return 1
            print("issue 613 static contract: PASS")
        if args.runtime:
            receipt, failures = runtime()
            path = args.receipt or Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "issue-613-receipt.json"
            path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            print(f"issue 613 receipt: {path}")
            if failures:
                print("issue 613 matrix: FAIL: " + "; ".join(failures), file=sys.stderr)
                return 1
            print("issue 613 Action + CLI matrix: PASS")
        if not args.static and not args.runtime:
            parser.error("choose --static or --runtime")
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"issue 613 contract: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
