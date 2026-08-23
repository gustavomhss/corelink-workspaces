#!/usr/bin/env bash
# pre-merge-gate-check-selftest.sh — proves scripts/pre-merge-gate-check.sh
# actually refuses in every direction it claims to, not just that it "runs".
# A merge-safety script that has never been shown to refuse is not evidence of
# anything, so this stubs `gh` (PATH-shadowed, no network) and asserts both the
# exit code and the presence of the right message for five scenarios:
#   1. CONFLICTING          -> exit non-zero, prints the rebase instruction
#   2. UNKNOWN               -> exit non-zero
#   3. zero checks           -> exit non-zero
#   4. an always-present gate missing from the check list -> exit non-zero
#   5. healthy (mergeable + all gates present + all green) -> exit 0
#
# Run directly: bash scripts/pre-merge-gate-check-selftest.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE_SCRIPT="$REPO_ROOT/scripts/pre-merge-gate-check.sh"
STUB_DIR="$(mktemp -d)"
trap 'rm -rf "$STUB_DIR"' EXIT

fail_count=0

# healthy check list: the three always-present gates (Defense 2) plus one
# ordinary check, all passing.
HEALTHY_CHECKS='[
  {"name":"gates","bucket":"pass","link":""},
  {"name":"dco","bucket":"pass","link":""},
  {"name":"llvm-cov (report-only)","bucket":"pass","link":""},
  {"name":"shellcheck scripts/*.sh","bucket":"pass","link":""}
]'

# Writes a stub `gh` into $STUB_DIR that answers `pr view` / `pr checks`
# according to the three env vars this function sets, then runs the gate
# script against fake PR number 1 with only $STUB_DIR on PATH (so it cannot
# fall through to a real `gh` and hit the network).
run_case() {
  local mergeable="$1" mergestatus="$2" prstate="$3" checks_json="$4"
  cat > "$STUB_DIR/gh" <<STUB
#!/usr/bin/env bash
case "\$1 \$2" in
  "pr view")
    echo '$mergeable $mergestatus $prstate'
    ;;
  "pr checks")
    cat <<'JSON'
$checks_json
JSON
    ;;
  *)
    echo "unstubbed gh invocation: \$*" >&2
    exit 99
    ;;
esac
STUB
  chmod +x "$STUB_DIR/gh"
  # Prepend (not replace) PATH: the stub `gh` shadows the real one (found
  # first), while `bash` itself (and python3, used internally) stay resolvable.
  PATH="$STUB_DIR:$PATH" bash "$GATE_SCRIPT" 1
}

assert_refuses() {
  local desc="$1" want_substr="$2"; shift 2
  local out rc=0
  out="$(run_case "$@" 2>&1)" || rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "  ✗ FAIL [$desc]: expected non-zero exit, got 0"
    echo "$out" | sed 's/^/      /'
    fail_count=$((fail_count + 1))
    return
  fi
  if [ -n "$want_substr" ] && ! printf '%s' "$out" | grep -qF "$want_substr"; then
    echo "  ✗ FAIL [$desc]: exit=$rc (expected non-zero, got non-zero — OK) but"
    echo "      output missing expected substring: $want_substr"
    echo "$out" | sed 's/^/      /'
    fail_count=$((fail_count + 1))
    return
  fi
  echo "  ✓ PASS [$desc]: refused (exit=$rc)"
}

assert_allows() {
  local desc="$1"; shift
  local out rc=0
  out="$(run_case "$@" 2>&1)" || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "  ✗ FAIL [$desc]: expected exit 0, got $rc"
    echo "$out" | sed 's/^/      /'
    fail_count=$((fail_count + 1))
    return
  fi
  if ! printf '%s' "$out" | grep -qF "All gates green"; then
    echo "  ✗ FAIL [$desc]: exit 0 but missing the green-light message"
    echo "$out" | sed 's/^/      /'
    fail_count=$((fail_count + 1))
    return
  fi
  echo "  ✓ PASS [$desc]: allowed (exit=0)"
}

echo "pre-merge-gate-check self-test"
echo "==============================="

# 1. CONFLICTING
assert_refuses "CONFLICTING" "git rebase origin/main" \
  "CONFLICTING" "DIRTY" "OPEN" "[]"

# 2. UNKNOWN
assert_refuses "UNKNOWN" "not finished computing mergeability" \
  "UNKNOWN" "UNKNOWN" "OPEN" "[]"

# 3. zero checks (mergeable, but no checks at all)
assert_refuses "zero checks" "reports NO checks at all" \
  "MERGEABLE" "CLEAN" "OPEN" "[]"

# 4. an always-present gate missing (dco absent from an otherwise-green list)
MISSING_DCO_CHECKS='[
  {"name":"gates","bucket":"pass","link":""},
  {"name":"llvm-cov (report-only)","bucket":"pass","link":""}
]'
assert_refuses "always-present gate missing (dco)" "always-present gates never ran" \
  "MERGEABLE" "CLEAN" "OPEN" "$MISSING_DCO_CHECKS"

# 5. healthy: mergeable + all three always-present gates present + all green
assert_allows "healthy" \
  "MERGEABLE" "CLEAN" "OPEN" "$HEALTHY_CHECKS"

echo "==============================="
if [ "$fail_count" -ne 0 ]; then
  echo "$fail_count self-test case(s) FAILED — pre-merge-gate-check.sh is not trustworthy as-is." >&2
  exit 1
fi
echo "all 5 self-test cases passed"
