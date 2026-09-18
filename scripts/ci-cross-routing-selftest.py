#!/usr/bin/env python3
"""Test real gates.sh routing using explicit recording stubs; no Rust or network."""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
bash = shutil.which(os.environ.get("CLW_TEST_BASH", "bash"))
if bash is None:
    raise SystemExit("cross-skip selftest: bash is required")
helper = root / "scripts/ci-cross-policy.sh"
gates = root / "scripts/gates.sh"
classifier = root / "scripts/classify-lane.sh"
checks = 0

with tempfile.TemporaryDirectory(prefix="clw-cross-skip-selftest-") as directory:
    fixture = Path(directory)
    home = fixture / "home"
    home.mkdir()
    # Do not inherit credentials, CI inputs, shell hooks or the caller's skip.
    clean = {"PATH": os.defpath, "HOME": str(home), "LC_ALL": "C", "TMPDIR": str(fixture)}

    def run(argv, overrides=None):
        env = dict(clean)
        env.update(overrides or {})
        # On timeout, stop only the process group owned by this test case.
        with subprocess.Popen(argv, env=env, cwd=fixture, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True,
                              start_new_session=(os.name == "posix")) as child:
            try:
                stdout, stderr = child.communicate(timeout=15)
            except subprocess.TimeoutExpired:
                if os.name == "posix":
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                else:
                    child.kill()
                child.communicate()
                raise
            return subprocess.CompletedProcess(argv, child.returncode, stdout, stderr)

    def require(condition, label, result):
        global checks
        if not condition:
            raise SystemExit(
                f"cross-skip selftest FAILED: {label}\n"
                f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
            )
        checks += 1

    # Exercise command selection through the entire ORIGINAL gates.sh, not a
    # transcription of its final cross branch. Compiler, Git responses and
    # subordinate gate programs below are deliberately recording stubs. This
    # verifies routing/failure propagation, NEVER a compiler or full-gate pass.
    route_tree = fixture / "routing-tree"
    (route_tree / "scripts").mkdir(parents=True)
    for source in [gates, helper, classifier]:
        shutil.copy2(source, route_tree / "scripts" / source.name)
    # Package graph expansion is not under test here. Keep this dependency
    # explicit and bounded; its real self-test is still selected by gates.sh.
    (route_tree / "scripts/scoped-package-set.sh").write_text(
        'scoped_package_set() {\n'
        '  [ "$1" = "clw-run " ] || return 65\n'
        '  printf "%s\\n" clw-cli clw-run\n'
        '}\n', encoding="utf-8")
    tools = fixture / "recording-tools"
    tools.mkdir()
    recorder = tools / "record.sh"
    # A native Bash recorder keeps the routing test independent of startup
    # costs/import hooks in the host's Python installation. Arguments here
    # are controlled command tokens, never user paths or secret values.
    recorder.write_text("#!" + bash + "\n" + r'''
set -euo pipefail
name=${0##*/}
for arg in "$name" "$@"; do
  case "$arg" in *$'\t'*|*$'\n'*) exit 96 ;; esac
done
{ printf '%s' "$name"; printf '\t%s' "$@"; printf '\n'; } >> "$ROUTING_LOG"
case "$name" in
  bash)
    case "${1:-}" in
      scripts/check-release-notes.sh|scripts/workflow-permissions.sh|scripts/check-unsafe-posture.sh|\
      scripts/deadman-registry.sh|scripts/deadman-selftest.sh|scripts/workflow-permissions-selftest.sh|\
      scripts/fuzz-selftest.sh|scripts/pr-title-workflow-selftest.sh|scripts/ci-build-target-selftest.sh|\
      scripts/cross-skip-policy-selftest.sh|scripts/check-minisign-trust-root.sh|\
      scripts/minisign-trust-root-selftest.sh|scripts/gate-lane-selftest.sh|\
      scripts/mutation-verdict-selftest.sh|scripts/scoped-package-set-selftest.sh|\
      scripts/selftest-unsafe-posture.sh) ;;
      *) exit 96 ;;
    esac
    ;;
  python3)
    [ "$#" -eq 1 ] && [ "$1" = scripts/ci-cross-policy-selftest.py ] || exit 96
    if [ "${ROUTING_FAIL:-}" = selftest ]; then exit 71; fi
    ;;
  git)
    case "${1:-}" in
      fetch) ;;
      merge-base) printf '%s\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ;;
      diff)
        [ "$#" -eq 3 ] && [ "$2" = --name-only ] && [ "$3" = origin/main...HEAD ] || exit 96
        printf '%s\n' "$ROUTING_DIFF"
        ;;
      *) exit 96 ;;
    esac
    ;;
  cargo)
    case "${1:-}" in fmt|clippy|test|deny|audit) ;; *) exit 96 ;; esac
    if [ "$1" = test ] && [ "${ROUTING_FAIL:-}" = host-test ]; then exit 72; fi
    for arg in "$@"; do
      if [ "$arg" = --target ] && [ "${ROUTING_FAIL:-}" = cross ]; then exit 73; fi
    done
    ;;
  rustup) [ "$*" = 'target add x86_64-pc-windows-gnu' ] || exit 96 ;;
  *) exit 96 ;;
esac
''', encoding="utf-8")
    recorder.chmod(0o755)
    for tool in ["bash", "python3", "git", "cargo", "cargo-audit", "rustup"]:
        (tools / tool).symlink_to(recorder)

    ci_env = {"CI": "true", "GITHUB_ACTIONS": "true", "GITHUB_RUN_ID": "319",
              "GITHUB_RUN_ATTEMPT": "3", "GITHUB_BASE_REF": "main"}
    cross = ["cargo", "clippy", "--workspace", "--all-targets", "--all-features",
             "--target", "x86_64-pc-windows-gnu", "--", "-D", "warnings"]
    setup = ["rustup", "target", "add", "x86_64-pc-windows-gnu"]
    # label, environment, real classifier input, expected lane, exit, cross
    scenarios = [
        ("local default", {}, "", "full", 0, True),
        ("local exact skip", {"CLW_SKIP_CROSS": "1"}, "", "full", 0, False),
        ("local explicit false", {"CI": "false", "GITHUB_ACTIONS": "0", "CLW_SKIP_CROSS": "1"}, "", "full", 0, False),
        ("main push", {**ci_env, "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/main"}, "", "full", 0, True),
        ("tag push", {**ci_env, "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/tags/v0.0.0"}, "", "full", 0, True),
        ("dispatch", {**ci_env, "GITHUB_EVENT_NAME": "workflow_dispatch"}, "", "full", 0, True),
        ("PR scripts", {**ci_env, "GITHUB_EVENT_NAME": "pull_request"}, "scripts/gates.sh", "full", 0, True),
        ("PR crate", {**ci_env, "GITHUB_EVENT_NAME": "pull_request"}, "crates/clw-run/src/lib.rs", "scoped", 0, True),
        ("PR documents", {**ci_env, "GITHUB_EVENT_NAME": "pull_request"}, "README.md\ndocs/GUIDE.md", "docs", 0, False),
        ("nonexact skip in CI", {**ci_env, "CLW_SKIP_CROSS": "true"}, "", "full", 0, True),
        ("prohibited CI skip", {**ci_env, "CLW_SKIP_CROSS": "1"}, "", "refused", 1, False),
        ("Actions independent", {"CI": "false", "GITHUB_ACTIONS": "true", "CLW_SKIP_CROSS": "1"}, "", "refused", 1, False),
        ("cross failure propagates", {**ci_env, "ROUTING_FAIL": "cross"}, "", "full", 73, True),
        ("host failure stops cross", {**ci_env, "ROUTING_FAIL": "host-test"}, "", "full", 72, False),
        ("selftest failure stops compilation", {**ci_env, "ROUTING_FAIL": "selftest"}, "", "selftest-failed", 71, False),
    ]
    for index, (label, context, changed, lane, exit_code, needs_cross) in enumerate(scenarios):
        case_dir = fixture / f"routing-{index}"
        case_dir.mkdir()
        log = case_dir / "commands.tsv"
        context = {**context, "PATH": str(tools) + os.pathsep + os.defpath,
                   "ROUTING_LOG": str(log), "ROUTING_DIFF": changed,
                   "RUNNER_TEMP": str(case_dir / "runner"),
                   "CARGO_TARGET_DIR": str(case_dir / "target")}
        try:
            result = run([bash, str(route_tree / "scripts/gates.sh")], context)
        except subprocess.TimeoutExpired as exc:
            print("ROUTING_TIMEOUT", label, "STDOUT", repr(exc.stdout), "STDERR", repr(exc.stderr),
                  "CALLS", log.read_text()[-8000:] if log.exists() else "none", file=sys.stderr)
            raise
        calls = [line.split("\t") for line in log.read_text().splitlines()] if log.exists() else []
        cargo_calls = [call for call in calls if call[0] == "cargo"]
        rustup_calls = [call for call in calls if call[0] == "rustup"]
        target_calls = [call for call in cargo_calls if "--target" in call]
        required_selftest = ["python3", "scripts/ci-cross-policy-selftest.py"]
        good = (result.returncode == exit_code and
                target_calls == ([cross] if needs_cross else []) and
                rustup_calls == ([setup] if needs_cross else []))
        if lane == "refused":
            good = good and not calls and "GATE FAILED: CLW_SKIP_CROSS=1" in result.stderr
        else:
            good = good and calls.count(required_selftest) == 1
            if cargo_calls:
                good = good and calls.index(required_selftest) < calls.index(cargo_calls[0])
        if lane in {"docs", "selftest-failed"}:
            good = good and not cargo_calls
        if lane == "full" and exit_code == 0:
            good = good and ["cargo", "test", "--workspace"] in calls and ["cargo", "deny", "check"] in calls
        if lane == "scoped":
            good = good and ["cargo", "test", "-p", "clw-cli", "-p", "clw-run"] in calls
            good = good and not any(call[:2] == ["cargo", "deny"] for call in calls)
        if exit_code == 0:
            good = good and "ALL GATES GREEN" in result.stdout
        else:
            good = good and "ALL GATES GREEN" not in result.stdout
        require(good, "command routing (dependencies stubbed): " + label, result)
        print(f"  routing passed: {label} (recording stubs, exit={result.returncode})")


print(f"cross-skip routing selftest: {checks} checks passed (command stubs; not full gate)")
