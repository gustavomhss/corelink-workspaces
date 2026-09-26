#!/usr/bin/env python3
"""Verify the hosted issue 613 Action/CLI matrix and write a bounded receipt."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REPOS = {
    "workspaces": ("HuGR-dev/corelink-workspaces", "1259579816", ROOT),
    "runners": (
        "HuGR-dev/corelink-runners",
        "1266754321",
        Path(os.environ.get("RUNNERS_DIRECTORY", ROOT.parent / "runners")),
    ),
}
MAX_RECEIPT_REQUESTS = 512


def git(path: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(path), *args], text=True).strip()


def canonical_repository(remote: str) -> str | None:
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
        failures.append("workflow is not running in canonical HuGR-dev workspaces")
    workflow_sha = os.environ.get("GH_SHA", "")
    if not re.fullmatch(r"[0-9a-f]{40}", workflow_sha):
        failures.append("workflow commit is not an exact 40-hex SHA")
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
        actual[key] = {
            "repository": name,
            "repository_id": repo_id,
            "requested_sha": requested,
            "actual_sha": sha,
            "remote": remote,
        }
    if failures:
        raise ValueError("; ".join(failures))
    return actual


def direct_cli_cases(temp: Path) -> tuple[list[dict[str, object]], list[str]]:
    """Drive the built CLI directly so receipt bytes are independently observable."""
    failures: list[str] = []
    results: list[dict[str, object]] = []
    binary = shutil.which("clw")
    if not binary:
        raise ValueError("compiled clw executable is missing from PATH")
    base = dict(os.environ)
    base["CLW_ENDPOINT"] = os.environ["CLW_ENDPOINT"]
    base["CLW_TENANT"] = "issue-613-test"
    base["CLW_TOKEN"] = "issue-613-local-only"

    def count(marker: Path) -> int:
        try:
            return len(marker.read_bytes().splitlines())
        except OSError:
            return 0

    def invoke(label: str, command: list[str], marker: Path, *, endpoint: str | None = None,
               cache: str | None = None, limit: int = 45) -> dict[str, object]:
        state_path = temp / f"{label}.state"
        env = dict(base)
        env["CLW_CACHE_DIR"] = cache or str(temp / f"cache-{label}")
        env["CLW_RUN_STATE_FILE"] = str(state_path)
        if endpoint:
            env["CLW_ENDPOINT"] = endpoint
        argv = [binary, "run", "--input", "Cargo.toml", "--", *command]
        try:
            proc = subprocess.run(
                argv, cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                timeout=limit, check=False,
            )
            return_code = proc.returncode if proc.returncode >= 0 else 128 + -proc.returncode
        except subprocess.TimeoutExpired:
            return_code = "TIMEOUT"
        try:
            state_bytes = state_path.read_bytes()
            state_text = state_bytes.decode("ascii")
        except OSError:
            state_bytes = b""
            state_text = None
        classification = {
            "EXECUTED": "child_verdict",
            "NOT_STARTED": "preexec_failure",
            "DISPATCHING": "execution_uncertain",
        }.get(state_text, "unknown")
        return {
            "label": label,
            "argv": ["clw", *argv[1:]],
            "exit": return_code,
            "classification": classification,
            "receipt_bytes_hex": state_bytes.hex(),
            "receipt_text": state_text,
            "effect_count": count(marker),
        }

    def check(item: dict[str, object], exit_code: int, state: str, effects: int) -> None:
        results.append(item)
        if item["exit"] != exit_code or item["receipt_text"] != state + "\n" \
                or item["effect_count"] != effects:
            failures.append(
                f"{item['label']}: expected exit={exit_code}, receipt={state}, effects={effects}; "
                f"observed exit={item['exit']}, receipt={item['receipt_text']!r}, "
                f"effects={item['effect_count']}"
            )

    # Same command and isolated cache key: a clean success is a real hit on repeat.
    marker0 = temp / "effect-success"
    cmd0 = ["/bin/bash", "-c", f"printf x >> {marker0}; exit 0"]
    shared_cache = str(temp / "cache-success-repeat")
    check(invoke("cli-exit-0-first", cmd0, marker0, cache=shared_cache), 0, "EXECUTED", 1)
    check(invoke("cli-exit-0-repeat", cmd0, marker0, cache=shared_cache), 0, "NOT_STARTED", 1)

    for code in (7, 125):
        marker = temp / f"effect-exit-{code}"
        command = ["/bin/bash", "-c", f"printf x >> {marker}; exit {code}"]
        shared_cache = str(temp / f"cache-exit-{code}")
        check(invoke(f"cli-exit-{code}-first", command, marker, cache=shared_cache), code, "EXECUTED", 1)
        check(invoke(f"cli-exit-{code}-repeat", command, marker, cache=shared_cache), code, "EXECUTED", 2)

    marker = temp / "effect-dead-endpoint"
    cmd = ["/bin/bash", "-c", f"printf x >> {marker}; exit 0"]
    check(invoke("cli-dead-endpoint-preexec", cmd, marker,
                 endpoint="http://127.0.0.1:1", limit=100), 125, "NOT_STARTED", 0)

    # The kernel cannot exec this file because its shebang interpreter is absent.
    missing_interpreter = temp / "absent-i613-interpreter"
    executable = temp / "spawn-failure"
    executable.write_text(f"#!{missing_interpreter}\nexit 0\n", encoding="ascii")
    executable.chmod(0o755)
    marker = temp / "effect-spawn-failure"
    check(invoke("cli-spawn-os-failure", [str(executable)], marker), 125, "NOT_STARTED", 0)

    # Marker persistence proves dispatch before the child terminates the CLI parent.
    marker = temp / "effect-postdispatch-parent-signal"
    command = ["/bin/bash", "-c", f"printf x >> {marker}; kill -TERM \"$PPID\"; exec sleep 1"]
    check(invoke("cli-postdispatch-parent-signal", command, marker, limit=15),
          143, "DISPATCHING", 1)

    marker = temp / "effect-child-signal"
    command = ["/bin/bash", "-c", f"printf x >> {marker}; kill -TERM $$"]
    check(invoke("cli-child-signal", command, marker), 143, "EXECUTED", 1)

    marker = temp / "effect-output-cap"
    command = ["/bin/bash", "-c", f"printf x >> {marker}; head -c 6291456 /dev/zero"]
    check(invoke("cli-output-cap", command, marker, limit=25), 137, "DISPATCHING", 1)

    # A short background child inherits stdout; the CLI must finish inside its drain grace.
    marker = temp / "effect-held-pipe"
    command = ["/bin/bash", "-c", f"printf x >> {marker}; (sleep 4) & exit 0"]
    check(invoke("cli-held-pipe-grace", command, marker, limit=15), 0, "EXECUTED", 1)
    return results, failures


def runtime() -> tuple[dict[str, object], list[str]]:
    repos = sources()
    runner_temp = Path(os.environ["RUNNER_TEMP"])
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="issue-613-direct-", dir=runner_temp) as directory:
        direct_temp = Path(directory)
        direct, direct_failures = direct_cli_cases(direct_temp)
        failures.extend(direct_failures)

    action_effects = {}
    for code in (0, 7, 125):
        first = int(os.environ.get(f"ACTION_{code}_FIRST_COUNT", "-1"))
        repeat = int(os.environ.get(f"ACTION_{code}_REPEAT_COUNT", "-1"))
        wanted_first, wanted_repeat = (1, 1) if code == 0 else (1, 2)
        first_outcome = os.environ.get(f"ACTION_{code}_FIRST", "missing")
        repeat_outcome = os.environ.get(f"ACTION_{code}_REPEAT", "missing")
        expected_outcome = "success" if code == 0 else "failure"
        action_effects[str(code)] = {
            "command": f"printf x >> EFFECT_LOG; exit {code}",
            "first_count": first,
            "repeat_count": repeat,
            "expected_counts": [wanted_first, wanted_repeat],
            "step_outcomes": [first_outcome, repeat_outcome],
            "expected_outcome": expected_outcome,
        }
        if (first, repeat) != (wanted_first, wanted_repeat):
            failures.append(f"Action exit {code} child effects were not {wanted_first}/{wanted_repeat}")
        if first_outcome != expected_outcome or repeat_outcome != expected_outcome:
            failures.append(f"Action exit {code} did not preserve both child step outcomes")

    fallback_path = runner_temp / "issue-613-action-fallback-effects"
    try:
        fallback_count = len(fallback_path.read_bytes().splitlines())
    except OSError:
        fallback_count = 0
    fallback = {
        "endpoint": "http://127.0.0.1:1",
        "command": "printf x >> RUNNER_TEMP/issue-613-action-fallback-effects; exit 0",
        "effect_count": fallback_count,
        "step_outcome": os.environ.get("ACTION_PREEXEC", "missing"),
    }
    if fallback_count != 1 or fallback["step_outcome"] != "success":
        failures.append("Action dead endpoint pre-execution failure did not fall back exactly once")

    action_edge_cases = {}
    for key, label in (
        ("POSTDISPATCH", "postdispatch"),
        ("SIGNAL", "signal"),
        ("OUTPUT_CAP", "output-cap"),
    ):
        marker = runner_temp / f"issue-613-action-{label}-effects"
        try:
            effects = len(marker.read_bytes().splitlines())
        except OSError:
            effects = 0
        outcome = os.environ.get(f"ACTION_{key}", "missing")
        action_edge_cases[label] = {
            "effect_count": effects,
            "expected_effect_count": 1,
            "step_outcome": outcome,
            "expected_step_outcome": "failure",
        }
        if effects != 1 or outcome != "failure":
            failures.append(f"Action {label} did not preserve one child effect and failure")

    try:
        held_count = int(os.environ.get("ACTION_HELD_COUNT", "-1"))
        held_start = int(os.environ.get("ACTION_HELD_START_NS", "-1"))
        held_end = int(os.environ.get("ACTION_HELD_END_NS", "-1"))
    except ValueError:
        held_count = held_start = held_end = -1
    held_duration_seconds = (held_end - held_start) / 1_000_000_000 if held_end >= held_start else None
    held_outcome = os.environ.get("ACTION_HELD_PIPE", "missing")
    action_edge_cases["held-pipe-grace"] = {
        "effect_count": held_count,
        "expected_effect_count": 1,
        "step_outcome": held_outcome,
        "expected_step_outcome": "success",
        "duration_seconds": held_duration_seconds,
        "maximum_duration_seconds": 15,
    }
    if held_count != 1 or held_outcome != "success" or held_duration_seconds is None \
            or held_duration_seconds > 15:
        failures.append("Action held-pipe path did not finish successfully with one effect within 15 seconds")

    request_path = Path(os.environ["I613_REQUEST_LOG"])
    request_lines = request_path.read_text(encoding="ascii").splitlines() if request_path.exists() else []
    request_lines = request_lines[:MAX_RECEIPT_REQUESTS]
    if not request_lines:
        failures.append("loopback service received no CAS/AC requests")
    if any(not re.fullmatch(
        r"(?:GET|PUT) /v1/(?:ac|cas)/issue-613-test/[a-f0-9]{64} (?:200|201|404)", line
    ) for line in request_lines):
        failures.append("loopback service request log contains an unexpected request")

    receipt: dict[str, object] = {
        "schema": "corelink.issue-613.receipt.v2",
        "repositories": repos,
        "workflow_commit_sha": os.environ.get("GH_SHA", ""),
        "workflow_run_url": (
            f"{os.environ.get('GH_SERVER_URL', 'https://github.com')}/"
            f"{os.environ.get('GH_REPOSITORY', '')}/actions/runs/{os.environ.get('GH_RUN_ID', '')}"
        ),
        "workflow_run_id": os.environ.get("GH_RUN_ID", ""),
        "hosted_runner": "ubuntu-24.04",
        "permissions": {"contents": "read"},
        "credentials": "GITHUB_TOKEN for checkout only; synthetic loopback test bearer",
        "service_boundary": "127.0.0.1 only; bounded in-memory CAS/AC double; no provider endpoints",
        "action_cases": action_effects,
        "action_preexec_fallback": fallback,
        "action_postdispatch_signal_output_pipe": action_edge_cases,
        "direct_cli_cases": direct,
        "local_service_request_count": len(request_lines),
        "local_service_requests": request_lines,
        "receipt_request_log_truncated": len(request_lines) == MAX_RECEIPT_REQUESTS,
    }
    return receipt, failures


def static() -> list[str]:
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
        r"(?m)^      runners_sha:\s*$",
        r"(?m)^        required:\s*true\s*$",
        r"(?m)^          ref:\s*\$\{\{\s*inputs\.workspaces_sha\s*\}\}\s*$",
        r"(?m)^          ref:\s*\$\{\{\s*inputs\.runners_sha\s*\}\}\s*$",
        r"(?m)^          repository:\s*HuGR-dev/corelink-workspaces\s*$",
        r"(?m)^          repository:\s*HuGR-dev/corelink-runners\s*$",
        r"(?m)^          persist-credentials:\s*false\s*$",
        r"(?m)^          python3 scripts/verify_i613_crossrepo_matrix\.py --runtime --receipt",
    )
    for pattern in required:
        if not re.search(pattern, source):
            failures.append(f"workflow is missing required contract: {pattern}")

    trigger_lines = source.splitlines()
    try:
        trigger_index = next(i for i, line in enumerate(trigger_lines) if line == "on:")
        next_root = next(
            i for i in range(trigger_index + 1, len(trigger_lines))
            if trigger_lines[i] and not trigger_lines[i][0].isspace()
            and not trigger_lines[i].lstrip().startswith("#")
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
    if re.search(r"HuGR-Labs/corelink-workspaces|9e85d73d9c0bd83c265678f204161fe3d6adb973", source):
        failures.append("workflow contains a stale repository identity or runner SHA")
    if re.search(r"(?i)corelink-dogfood|runs-on:\s*['\"]?self-hosted|\$\{\{\s*secrets\.", source):
        failures.append("workflow must not use self-hosted runners, dogfood, or repository secrets")
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
            path = args.receipt or Path(os.environ["RUNNER_TEMP"]) / "issue-613-receipt.json"
            path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            print(f"issue 613 receipt: {path}")
            if failures:
                print("issue 613 matrix: FAIL: " + "; ".join(failures), file=sys.stderr)
                return 1
            print("issue 613 Action + CLI matrix: PASS")
        if not args.static and not args.runtime:
            parser.error("choose --static or --runtime")
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError) as error:
        print(f"issue 613 contract: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
