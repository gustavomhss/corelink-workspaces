#!/usr/bin/env bash
# release-guards-selftest.sh — offline proof for release.yml's pure decisions.

set -euo pipefail
cd "$(dirname "$0")/.."

GUARDS="$PWD/scripts/release-guards.sh"
# shellcheck source=scripts/release-guards.sh
. "$GUARDS"

pass=0
fail=0
RAN=()
EXPECTED=(
  "A1 real semver tag"
  "A2 rc.N tag"
  "A2 rcN tag"
  "invalid loose v* tag"
  "invalid suffix after rc"
  "A3 real release requires minisign.pub"
  "A4 rc tolerates absent trust root"
  "present trust root passes"
  "invalid kind fails even with a present trust root"
  "A5 failed real self-verification is fatal"
  "successful real self-verification passes"
  "rc tolerates failed self-verification"
  "malformed self-verification status fails closed"
  "invalid kind fails even after successful verification"
  "A6 commit on main is an ancestor"
  "A6 orphan is rejected"
  "A6 unmerged branch commit is rejected"
  "A7 unreachable origin/main fails closed"
  "A8 minified unmetered quota"
  "A8 pretty-printed unmetered quota"
  "A8 minified metered quota"
  "A8 pretty-printed metered quota"
  "malformed quota body is invalid"
  "missing quota field is invalid"
  "A9 reverted strict branch makes A3 pass incorrectly"
  "A9 reverted strict branch makes A5 pass incorrectly"
  "A9 reverted strict branch makes A6 pass incorrectly"
)
if [ "${CLW_RELEASE_GUARDS_SKIP_NEGATIVE_CONTROLS:-0}" != "1" ]; then
  EXPECTED+=(
    "negative-control child baseline passes"
    "negative control catches a trivial same-name A7 probe"
    "negative control catches a trivial same-name A8 probe"
    "negative control catches trivial same-name A9 probes"
  )
fi
TMPROOT="$(mktemp -d "${TMPDIR:-/tmp}/clw-release-guards.XXXXXX")"
trap 'rm -rf "$TMPROOT"' EXIT

ok() {
  printf '  \033[32mok\033[0m    %s\n' "$1"
  pass=$((pass + 1))
}

bad() {
  printf '  \033[31mFAIL\033[0m  %s\n' "$1" >&2
  fail=$((fail + 1))
}

expect_output() {
  local name="$1" expected="$2"
  shift 2
  local got rc=0
  RAN+=("$name")
  got="$("$@" 2>"$TMPROOT/stderr")" || rc=$?
  if [ "$rc" -eq 0 ] && [ "$got" = "$expected" ]; then
    ok "$name"
  else
    bad "$name — expected output '$expected' / exit 0, got '$got' / exit $rc"
  fi
}

expect_status() {
  local name="$1" expected="$2"
  shift 2
  local rc=0
  RAN+=("$name")
  "$@" >"$TMPROOT/stdout" 2>"$TMPROOT/stderr" || rc=$?
  if [ "$rc" -eq "$expected" ]; then
    ok "$name"
  else
    bad "$name — expected exit $expected, got $rc; stderr: $(tr '\n' ' ' <"$TMPROOT/stderr")"
  fi
}

expect_status_stderr() {
  local name="$1" expected="$2" needle="$3"
  shift 3
  local rc=0
  RAN+=("$name")
  "$@" >"$TMPROOT/stdout" 2>"$TMPROOT/stderr" || rc=$?
  if [ "$rc" -eq "$expected" ] && grep -qF -- "$needle" "$TMPROOT/stderr"; then
    ok "$name"
  else
    bad "$name — expected exit $expected and stderr containing '$needle'; got exit $rc / $(tr '\n' ' ' <"$TMPROOT/stderr")"
  fi
}

replace_line_once() {
  local file="$1" needle="$2" replacement="$3" scratch="$4" count
  count="$(grep -cF -- "$needle" "$file" || true)"
  if [ "$count" -ne 1 ]; then
    printf 'sabotage setup expected one production line, found %s: %s\n' \
      "$count" "$needle" >&2
    return 1
  fi
  awk -v needle="$needle" -v replacement="$replacement" '
    $0 == needle { print replacement; next }
    { print }
  ' "$file" >"$scratch"
  mv "$scratch" "$file"
}

expect_recursive_negative_control() {
  local name="$1" sabotage="$2" expected_case="${3:-}" child_dir child_guard
  local child_test child_output child_plain rc=0 needle replacement
  RAN+=("$name")
  child_dir="$TMPROOT/negative-control-$sabotage"
  mkdir -p "$child_dir/scripts"
  child_guard="$child_dir/scripts/release-guards.sh"
  child_test="$child_dir/scripts/release-guards-selftest.sh"
  child_output="$child_dir/child-output"
  child_plain="$child_dir/child-output-plain"
  cp "$GUARDS" "$child_guard"
  cp "$PWD/scripts/release-guards-selftest.sh" "$child_test"

  case "$sabotage" in
    baseline) ;;
    a7)
      needle='    _release_guard_fail "release base '\''${base:-empty}'\'' is unreachable; cannot prove tag ancestry (fail-closed)"'
      replacement='    return 0 # NEGATIVE_CONTROL_A7_UNREACHABLE_BASE_FAIL_OPEN'
      if ! replace_line_once "$child_guard" "$needle" "$replacement" "$child_guard.tmp"; then
        bad "$name — A7 sabotage did not apply exactly once"
        return
      fi
      ;;
    a8)
      needle="  if ! verdict=\"\$(printf '%s' \"\$body\" | jq -er '"
      replacement="  if ! verdict=\"\$(printf '%s' \"\$body\" | head -n 1 | jq -er '"
      if ! replace_line_once "$child_guard" "$needle" "$replacement" "$child_guard.tmp"; then
        bad "$name — A8 sabotage did not apply exactly once"
        return
      fi
      ;;
    a9)
      needle='      _release_guard_fail "required release trust root '\''${path:-minisign.pub}'\'' is absent (expected minisign.pub); refusing a real release"'
      replacement='      _release_guard_fail "required release trust root '\''${path:-minisign.pub}'\'' is absent (expected minisign.pub); refusing a real release"; return 1 # NEGATIVE_CONTROL_A9_REVERT_INEFFECTIVE'
      if ! replace_line_once "$child_guard" "$needle" "$replacement" "$child_guard.tmp"; then
        bad "$name — A9 sabotage did not apply exactly once"
        return
      fi
      ;;
    *)
      bad "$name — unknown sabotage '$sabotage'"
      return
      ;;
  esac

  CLW_RELEASE_GUARDS_SKIP_NEGATIVE_CONTROLS=1 bash "$child_test" >"$child_output" 2>&1 || rc=$?
  if [ "$sabotage" = "baseline" ]; then
    if [ "$rc" -eq 0 ]; then
      ok "$name"
    else
      bad "$name — unsabotaged child exited $rc: $(tail -n 4 "$child_output" | tr '\n' ' ')"
    fi
    return
  fi

  # Strip colour before requiring a functional FAIL for the exact committed case.
  # Merely grepping its name would accept an unrelated failure while this case says ok.
  sed $'s/\033\\[[0-9;]*m//g' "$child_output" >"$child_plain"
  if [ "$rc" -ne 0 ] && grep -qF -- "FAIL  $expected_case" "$child_plain"; then
    ok "$name"
  else
    bad "$name — sabotaged child must fail the exact case '$expected_case' (exit $rc)"
  fi
}

echo "release guard self-test"

# A1/A2 — the two accepted rc spellings and the real-release spelling share one
# parser. Extra invalid cases make sure the classifier does not merely search '-rc'.
expect_output "A1 real semver tag" real release_kind v0.1.13
expect_output "A2 rc.N tag" rc release_kind v0.1.13-rc.1
expect_output "A2 rcN tag" rc release_kind v0.1.13-rc1
expect_output "invalid loose v* tag" invalid release_kind v0.1
expect_output "invalid suffix after rc" invalid release_kind v0.1.13-rc.1-extra

# A3/A4 — the same absent path is strict for real and tolerant for rc.
MISSING="$TMPROOT/missing/minisign.pub"
expect_status_stderr "A3 real release requires minisign.pub" 1 minisign.pub \
  require_trust_root real "$MISSING"
expect_status "A4 rc tolerates absent trust root" 0 \
  require_trust_root rc "$MISSING"
mkdir -p "$TMPROOT/present"
: >"$TMPROOT/present/minisign.pub"
expect_status "present trust root passes" 0 \
  require_trust_root real "$TMPROOT/present/minisign.pub"
expect_status "invalid kind fails even with a present trust root" 1 \
  require_trust_root invalid "$TMPROOT/present/minisign.pub"

# A5 — pin both sides of the self-verification result and the rc tolerance seam.
expect_status "A5 failed real self-verification is fatal" 1 \
  require_selfverify_ok real 1
expect_status "successful real self-verification passes" 0 \
  require_selfverify_ok real 0
expect_status "rc tolerates failed self-verification" 0 \
  require_selfverify_ok rc 1
expect_status "malformed self-verification status fails closed" 1 \
  require_selfverify_ok real not-a-status
expect_status "invalid kind fails even after successful verification" 1 \
  require_selfverify_ok invalid 0

# A6/A7 — construct the complete ancestry graph offline. `feature_sha` descends
# from main but is not merged into it; `orphan_sha` shares no ancestry at all.
REPO="$TMPROOT/repo"
git init -q -b main "$REPO"
git -C "$REPO" config user.name "release-guards-selftest"
git -C "$REPO" config user.email "release-guards-selftest@example.invalid"
git -C "$REPO" commit -q --allow-empty -m main
main_sha="$(git -C "$REPO" rev-parse HEAD)"
empty_tree="$(git -C "$REPO" mktree </dev/null)"
feature_sha="$(printf 'unmerged feature\n' | git -C "$REPO" commit-tree "$empty_tree" -p "$main_sha")"
orphan_sha="$(printf 'orphan\n' | git -C "$REPO" commit-tree "$empty_tree")"

expect_status "A6 commit on main is an ancestor" 0 \
  bash -c 'cd "$1" && . "$2" && assert_tag_is_ancestor "$3" refs/heads/main' \
  _ "$REPO" "$GUARDS" "$main_sha"
expect_status_stderr "A6 orphan is rejected" 1 "not an ancestor" \
  bash -c 'cd "$1" && . "$2" && assert_tag_is_ancestor "$3" refs/heads/main' \
  _ "$REPO" "$GUARDS" "$orphan_sha"
expect_status_stderr "A6 unmerged branch commit is rejected" 1 "not an ancestor" \
  bash -c 'cd "$1" && . "$2" && assert_tag_is_ancestor "$3" refs/heads/main' \
  _ "$REPO" "$GUARDS" "$feature_sha"
expect_status_stderr "A7 unreachable origin/main fails closed" 1 "unreachable" \
  bash -c 'cd "$1" && . "$2" && assert_tag_is_ancestor "$3" origin/main' \
  _ "$REPO" "$GUARDS" "$main_sha"

# A8 — whitespace does not decide quota policy. Pin both boolean verdicts and
# malformed/missing-field degradation, not merely one happy body.
expect_output "A8 minified unmetered quota" unmetered parse_quota_body '{"unmetered":true}'
expect_output "A8 pretty-printed unmetered quota" unmetered parse_quota_body $'{\n  "unmetered": true\n}'
expect_output "A8 minified metered quota" metered parse_quota_body '{"unmetered":false}'
expect_output "A8 pretty-printed metered quota" metered parse_quota_body $'{\n  "unmetered": false\n}'
expect_output "malformed quota body is invalid" invalid parse_quota_body '{'
expect_output "missing quota field is invalid" invalid parse_quota_body '{}'

# A9 — teeth. Mutate the ONE strict/fail-closed return, after asserting the edit
# applied exactly once. The three strict acceptance cases must then flip to exit 0.
STRICT='return 1 # RELEASE_GUARD_STRICT_FAIL_CLOSED'
if [ "$(grep -cF -- "$STRICT" "$GUARDS")" -ne 1 ]; then
  bad "A9 setup — expected exactly one strict fail-closed return"
else
  MUTANT="$TMPROOT/release-guards-reverted.sh"
  sed 's/return 1 # RELEASE_GUARD_STRICT_FAIL_CLOSED/return 0 # RELEASE_GUARD_STRICT_FAIL_CLOSED/' \
    "$GUARDS" >"$MUTANT"
  expect_status "A9 reverted strict branch makes A3 pass incorrectly" 0 \
    bash -c '. "$1" && require_trust_root real "$2"' _ "$MUTANT" "$MISSING"
  expect_status "A9 reverted strict branch makes A5 pass incorrectly" 0 \
    bash -c '. "$1" && require_selfverify_ok real 1' _ "$MUTANT"
  expect_status "A9 reverted strict branch makes A6 pass incorrectly" 0 \
    bash -c 'cd "$1" && . "$2" && assert_tag_is_ancestor "$3" refs/heads/main' \
    _ "$REPO" "$MUTANT" "$feature_sha"
fi

if [ "${CLW_RELEASE_GUARDS_SKIP_NEGATIVE_CONTROLS:-0}" != "1" ]; then
  echo
  echo "recursive negative controls"
  expect_recursive_negative_control \
    "negative-control child baseline passes" baseline
  expect_recursive_negative_control \
    "negative control catches a trivial same-name A7 probe" a7 \
    "A7 unreachable origin/main fails closed"
  expect_recursive_negative_control \
    "negative control catches a trivial same-name A8 probe" a8 \
    "A8 pretty-printed unmetered quota"
  expect_recursive_negative_control \
    "negative control catches trivial same-name A9 probes" a9 \
    "A9 reverted strict branch makes A3 pass incorrectly"
fi

# Report functional failures before checking completeness. Otherwise a real guard
# regression gets diagnosed as "a case disappeared", hiding the useful failure under
# an anti-vacuity symptom — the exact ordering bug found in the mutation self-test.
echo
printf 'functional results: %d passed, %d failed\n' "$pass" "$fail"

# Exact named-case roll-call. A numeric floor can be held constant by adding a trivial
# case while deleting A7/A8/A9; checking only for missing expected names misses a
# duplicated case or a surprise case that should have been reviewed. Compare the two
# complete multisets instead: missing, unexpected and duplicate names are all red.
printf '%s\n' "${EXPECTED[@]}" | LC_ALL=C sort >"$TMPROOT/expected-cases"
printf '%s\n' "${RAN[@]}" | LC_ALL=C sort >"$TMPROOT/ran-cases"
if ! cmp -s "$TMPROOT/expected-cases" "$TMPROOT/ran-cases"; then
  missing="$(comm -23 "$TMPROOT/expected-cases" "$TMPROOT/ran-cases" | tr '\n' ' ')"
  unexpected="$(comm -13 "$TMPROOT/expected-cases" "$TMPROOT/ran-cases" | tr '\n' ' ')"
  duplicates="$(uniq -d "$TMPROOT/ran-cases" | tr '\n' ' ')"
  echo "FAIL: exact case roll-call differs from the committed contract." >&2
  [ -z "$missing" ] || echo "      missing: $missing" >&2
  [ -z "$unexpected" ] || echo "      unexpected: $unexpected" >&2
  [ -z "$duplicates" ] || echo "      duplicate: $duplicates" >&2
  fail=$((fail + 1))
else
  echo "case roll-call: all ${#EXPECTED[@]} committed cases ran exactly once"
fi

if [ "$fail" -ne 0 ]; then
  echo "release guard self-test: $fail failure(s)" >&2
  exit 1
fi
echo "release guard self-test: all ${#EXPECTED[@]} release decisions behave as specified."
