#!/usr/bin/env bash
# pr-title-check-selftest.sh — red-on-revert corpus for pr-title-check.sh.
#
# Each case invokes the production checker in a fresh process. Consequently an
# accidental `exit 0`, a broadened type list, or a weakened separator/scope/
# breaking-marker rule turns at least one negative control red.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$REPO_ROOT/scripts/pr-title-check.sh"
passed=0
failed=0

expect_accept() {
  local title="$1" out rc=0
  out="$(PR_TITLE="$title" bash "$CHECK" 2>&1)" || rc=$?
  if [ "$rc" -eq 0 ]; then
    printf '  ✓ accepts %q\n' "$title"
    passed=$((passed + 1))
  else
    printf '  ✗ rejected valid title %q\n%s\n' "$title" "$out" >&2
    failed=$((failed + 1))
  fi
}

expect_reject() {
  local title="$1" out rc=0
  out="$(PR_TITLE="$title" bash "$CHECK" 2>&1)" || rc=$?
  if [ "$rc" -ne 0 ]; then
    printf '  ✓ rejects %q\n' "$title"
    passed=$((passed + 1))
  else
    printf '  ✗ accepted invalid title %q\n%s\n' "$title" "$out" >&2
    failed=$((failed + 1))
  fi
}

echo "PR title check selftest:"

# Enumerate every permitted type in the bare, scoped, and breaking forms.
for type in feat fix chore docs ci refactor test; do
  expect_accept "$type: describe the change"
  expect_accept "$type(clw-cli): describe the change"
  expect_accept "$type(clw-cli)!: describe the breaking change"
done
expect_accept 'fix(core/link): support a slash in the scope'
expect_accept 'fix(9patch): permit a digit at scope start'
expect_accept 'docs: permit # and quotes "as data" in the subject'

# Type vocabulary, shape, scope, marker, delimiter, and subject boundaries.
expect_reject ''
expect_reject 'WIP: investigate later'
expect_reject 'build: add a lane'
expect_reject 'perf: add coverage'
expect_reject 'Feat: wrong case'
expect_reject 'feat'
expect_reject 'feat:add delimiter space'
expect_reject 'feat:  leading subject whitespace'
expect_reject 'feat: trailing subject whitespace '
expect_reject 'feat(): empty scope'
expect_reject 'feat( ): whitespace scope'
expect_reject 'feat(clw cli): whitespace scope'
expect_reject 'feat(clw(cli)): nested scope'
expect_reject 'feat($HOME): shell variable scope'
expect_reject 'feat(`command`): command-substitution scope'
expect_reject 'feat(CLw): uppercase scope'
expect_reject 'feat(scope:part): colon scope'
expect_reject 'feat(scope)!!: duplicate breaking marker'
expect_reject 'feat!(scope): misplaced breaking marker'
expect_reject 'feat(scope)! : space before delimiter'
expect_reject 'feat (scope): space before scope'
expect_reject $'fix(scope): embedded\ttab'
expect_reject $'fix(scope): embedded\nnewline'

if [ "$failed" -ne 0 ]; then
  echo "PR title check selftest: $failed case(s) FAILED; $passed passed" >&2
  exit 1
fi
echo "PR title check selftest: all $passed cases passed (valid forms green; invalid forms red)"
