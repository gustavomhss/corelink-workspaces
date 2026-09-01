#!/usr/bin/env bash
# workflow-permissions-selftest.sh — teeth for workflow-permissions.sh.
#
# The fixture is a copy of the real workflow corpus. The clean copy must pass;
# every defect class claimed by the guard must make that same guard go red and
# identify the offending workflow. The tests are offline and never modify the
# repository's workflows.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$REPO_ROOT/scripts/workflow-permissions.sh"
REAL_WF="$REPO_ROOT/.github/workflows"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
CASE_N=0
FAILED=0

fresh() {
  local d="$WORK/case-$CASE_N" f
  rm -rf "$d"
  mkdir -p "$d"
  for f in "$REAL_WF"/*.yml "$REAL_WF"/*.yaml; do
    if [ -e "$f" ]; then cp "$f" "$d"/; fi
  done
  printf '%s' "$d"
}

expect_fail() {
  local desc="$1" needle="$2" dir="$3" out rc=0
  out="$(bash "$GATE" --workflow-dir "$dir" 2>&1)" || rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "  ✗ $desc — the guard PASSED. It is blind to this defect."
    FAILED=$((FAILED + 1)); return
  fi
  if ! printf '%s' "$out" | grep -qF -- "$needle"; then
    echo "  ✗ $desc — went red but never named '$needle'. Output was:"
    printf '%s\n' "$out" | sed 's/^/      /'
    FAILED=$((FAILED + 1)); return
  fi
  echo "  ✓ $desc"
}

expect_pass() {
  local desc="$1" dir="$2" out
  if out="$(bash "$GATE" --workflow-dir "$dir" 2>&1)"; then
    echo "  ✓ $desc"
    return
  fi
  echo "  ✗ $desc — the guard rejected a supported explicit policy. Output was:"
  printf '%s\n' "$out" | sed 's/^/      /'
  FAILED=$((FAILED + 1))
}

# Remove the top-level block without duplicating the guard's implementation.
remove_permissions() {
  local file="$1"
  sed '/^permissions:$/,/^$/d' "$file" > "$file.new"
  mv "$file.new" "$file"
}

echo "workflow-permissions teeth:"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
if bash "$GATE" --workflow-dir "$d" >/dev/null 2>&1; then
  echo "  ✓ an unmodified copy of the real .github/workflows PASSES"
else
  echo "  ✗ an unmodified copy of the real .github/workflows FAILED"
  FAILED=$((FAILED + 1))
fi

# The historical fixture hook was an inherited environment variable.  A
# workflow could set that variable and redirect the production gate to a clean
# directory.  Keep a negative control proving that the environment is ignored:
# the explicit bad fixture must still be inspected and named.
CASE_N=$((CASE_N + 1)); clean="$(fresh)"
CASE_N=$((CASE_N + 1)); bad="$(fresh)"
remove_permissions "$bad/ci.yml"
out=""; rc=0
out="$(WORKFLOW_PERMS_WF_DIR="$clean" bash "$GATE" --workflow-dir "$bad" 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qF -- 'ci.yml'; then
  echo "  ✓ an inherited fixture variable cannot redirect the guard"
else
  echo "  ✗ an inherited fixture variable redirected or obscured the explicit corpus"
  FAILED=$((FAILED + 1))
fi

CASE_N=$((CASE_N + 1)); d="$WORK/case-$CASE_N"
mkdir -p "$d"
expect_fail "an empty workflow corpus fails closed" "no non-exempt workflow files" "$d"

CASE_N=$((CASE_N + 1)); d="$WORK/missing-$CASE_N"
expect_fail "a missing workflow directory fails closed" "missing or not a directory" "$d"

for wf in ci.yml conformance-canary.yml e2e-evidence.yml live-journeys.yml; do
  CASE_N=$((CASE_N + 1)); d="$(fresh)"
  remove_permissions "$d/$wf"
  expect_fail "a missing permissions block in $wf is named" "$wf" "$d"
done

# Both extensions are part of the invariant; a permissions-less workflow must
# not be hidden by the yml-only glob that caused the original blind spot.
CASE_N=$((CASE_N + 1)); d="$(fresh)"
printf '%s\n' 'name: evil' 'on: workflow_dispatch' > "$d/evil.yaml"
expect_fail "a permissions-less .yaml workflow is named" "evil.yaml" "$d"

# Dotfiles are still workflow YAML when they live directly in this directory.
CASE_N=$((CASE_N + 1)); d="$(fresh)"
printf '%s\n' 'name: hidden evil' 'on: workflow_dispatch' > "$d/.evil.yml"
expect_fail "a permissions-less hidden workflow is named" ".evil.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed 's/^permissions:$/permissions: write-all/' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_fail "permissions: write-all is refused" "ci.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed 's/^permissions:$/permissions: "write-all"/' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_fail "quoted permissions: write-all is refused" "ci.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed "s/^permissions:$/permissions: 'write-all'/" "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_fail "single-quoted permissions: write-all is refused" "ci.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed 's/^permissions:$/permissions: write-all # still overbroad/' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_fail "permissions: write-all with a YAML comment is refused" "ci.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed 's/^permissions:$/permissions: >-/' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_fail "a folded permissions scalar fails closed" "ci.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
awk '{ print; if ($0 == "permissions:") print "permissions:" }' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_fail "duplicate top-level permissions keys fail closed" "ci.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed 's/^permissions:$/permissions: "read-all"/' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_pass "the explicit quoted read-all scalar remains supported" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
sed 's/^permissions:$/permissions: {}/' "$d/ci.yml" > "$d/ci.yml.new"
mv "$d/ci.yml.new" "$d/ci.yml"
expect_pass "an explicitly empty permission mapping remains supported" "$d"

echo
if [ "$FAILED" -gt 0 ]; then
  echo "workflow-permissions selftest: $FAILED case(s) FAILED — the guard does not have the teeth it claims." >&2
  exit 1
fi
echo "workflow-permissions selftest: all $CASE_N cases pass (clean tree green; every claimed defect class red AND named)"
