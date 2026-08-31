#!/usr/bin/env bash
# deadman-selftest.sh — teeth for scripts/deadman-registry.sh.
#
# A gate that cannot fail is not a gate, and "it printed OK" is not evidence it
# looked at anything. This plants EVERY defect class the registry gate claims to
# catch and asserts that (a) the gate exits non-zero and (b) NAMES the offending
# thing — "something is wrong" sends nobody anywhere.
#
# The fixture is a COPY OF THE REAL .github/workflows, not an invented tree, so
# each case is also a mutation-probe against the actual corpus: deleting one real
# registry row must go red. A fixture-only proof would leave open that the gate
# never engages what ships.
#
# It also asserts the UNMODIFIED copy PASSES — otherwise a gate that simply fires
# on everything would score full marks here.
#
# Offline. No network, no gh. Run by scripts/gates.sh in every lane.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$REPO_ROOT/scripts/deadman-registry.sh"
REAL_WF="$REPO_ROOT/.github/workflows"
DEADMAN="canary-deadman.yml"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
CASE_N=0
FAILED=0

# Portable in-place sed (BSD sed needs an argument to -i; GNU does not).
edit() { sed "$1" "$2" > "$2.new" && mv "$2.new" "$2"; }

fresh() {
  local d="$WORK/case-$CASE_N"
  rm -rf "$d"; mkdir -p "$d"
  # Copy BOTH extensions, one file at a time. The scanner globs `*.yml` and `*.yaml`,
  # so a fixture that copies only `.yml` makes case 0's "an unmodified copy of the real
  # tree PASSES" a claim about a tree that is not the real one.
  #
  # Not `cp src… "$d"/` with two globs: `cp` takes the LAST word as the destination, so
  # with exactly one `.yml` and one `.yaml` present that becomes `cp a.yml b.yaml` —
  # succeeding, and OVERWRITING b.yaml in the real .github/workflows on every gate run.
  # My first attempt at this fix shipped that form with the destination missing
  # entirely; `2>/dev/null` hid the error and the `||` fallback silently copied only
  # `.yml`, so the nit was reported closed while still open. An explicit `if` rather
  # than `[ -e … ] && cp …` so a final non-matching glob cannot make the loop's exit
  # status 1 under `set -e`.
  local f
  for f in "$REAL_WF"/*.yml "$REAL_WF"/*.yaml; do
    if [ -e "$f" ]; then cp "$f" "$d"/; fi
  done
  printf '%s' "$d"
}

# expect_fail <description> <needle-that-must-appear> <fixture-dir>
expect_fail() {
  local desc="$1" needle="$2" dir="$3" out rc=0
  out="$(DEADMAN_WF_DIR="$dir" bash "$GATE" check 2>&1)" || rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "  ✗ $desc — the gate PASSED. It is blind to this defect."
    FAILED=$((FAILED + 1)); return
  fi
  if ! printf '%s' "$out" | grep -qF -- "$needle"; then
    echo "  ✗ $desc — went red but never named '$needle'. Output was:"
    printf '%s\n' "$out" | sed 's/^/      /'
    FAILED=$((FAILED + 1)); return
  fi
  echo "  ✓ $desc"
}

echo "deadman-registry teeth:"

# ── 0. the clean copy of the REAL tree must PASS ─────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
if out="$(DEADMAN_WF_DIR="$d" bash "$GATE" check 2>&1)"; then
  echo "  ✓ an unmodified copy of the real .github/workflows PASSES"
else
  echo "  ✗ an unmodified copy of the real .github/workflows FAILED — the gate fires on everything:"
  printf '%s\n' "$out" | sed 's/^/      /'
  FAILED=$((FAILED + 1))
fi

# ── 1. a scheduled lane nobody registered (mutation-probe on the real corpus) ─
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit '/^ *fuzz\.yml  */d' "$d/$DEADMAN"
expect_fail "a SCHEDULED lane missing from the registry is named" "fuzz.yml" "$d"

# ── 2. a registered lane that is not scheduled ───────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^ *schedule:$/  # schedule removed by selftest/' "$d/fuzz.yml"
expect_fail "a REGISTERED lane with no schedule is named" "fuzz.yml" "$d"

# ── 3. the registry extracting EMPTY must fail, not pass vacuously ───────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit '/deadman:lanes:begin/,/deadman:lanes:end/{/deadman:lanes:/!d;}' "$d/$DEADMAN"
expect_fail "an EMPTY lane registry fails instead of watching zero lanes" "extracted EMPTY" "$d"

# ── 4. a missing delimiter is a parse break, not an empty set ────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit '/deadman:lanes:begin/d' "$d/$DEADMAN"
expect_fail "a deleted begin marker is named" "deadman:lanes:begin" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit '/deadman:known-red:end/d' "$d/$DEADMAN"
expect_fail "a deleted KNOWN_RED end marker is named" "deadman:known-red:end" "$d"

# ── 5. malformed rows ────────────────────────────────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^ *fuzz\.yml  *26.*$/    fuzz.yml/' "$d/$DEADMAN"
expect_fail "a row with no window is named" "malformed registry row" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^ *fuzz\.yml  *26.*$/    fuzz.yml daily/' "$d/$DEADMAN"
expect_fail "a non-integer window is named" "non-integer window" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^ *fuzz\.yml  *26.*$/    fuzz.yml 0/' "$d/$DEADMAN"
expect_fail "a zero window (never satisfiable) is named" "can never be satisfied" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^ *fuzz\.yml  *26.*$/    fuzz.yml 26\n    fuzz.yml 48/' "$d/$DEADMAN"
expect_fail "a duplicated lane is named" "more than once" "$d"

# ── 6. a registered workflow that does not exist ─────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
rm -f "$d/fuzz.yml"
expect_fail "a registry row for a workflow file that is gone is named" "does not exist" "$d"

# ── 7. the deadman registering ITSELF ────────────────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit "s|^ *fuzz\\.yml  *26.*\$|    fuzz.yml 26\\n    $DEADMAN 26|" "$d/$DEADMAN"
expect_fail "the deadman listing itself is refused (WP-36 scope note)" "WP-36" "$d"

# ── 8. KNOWN_RED excusing a lane nobody watches ──────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's|^ *# deadman:known-red:end$|    coverage.yml because reasons\n    # deadman:known-red:end|' "$d/$DEADMAN"
expect_fail "a KNOWN_RED entry outside the registry is named" "coverage.yml" "$d"

# ── 9. trigger shapes this scanner cannot read are REFUSED, not scored ───────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^on:$/on: {schedule: [{cron: "0 1 * * *"}]}/' "$d/coverage.yml"
expect_fail "an INLINE \`on:\` is refused rather than scored unscheduled" "declares its triggers INLINE" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit '/^on:$/d' "$d/coverage.yml"
expect_fail "a workflow with no readable \`on:\` key is named" "no top-level" "$d"

# ── 9b. key spellings YAML folds together, which a bare `schedule:` pattern missed ──
# Cold review got BOTH of these past the scanner SILENTLY — scored unscheduled, not
# refused — on a fixture with 7 scheduled workflows, while the gate reported "6 == 6"
# and passed. That is the third outcome the file's header claimed was unreachable.
# `yaml.safe_load` returns the trigger key `schedule` for each of them.
#
# These are `fail` cases in the sense that the GATE must go red: the fixture registers
# six lanes and now has a seventh scheduled one that is not registered, so the
# bidirectional check must name it. If the spelling were missed, the gate would say
# 6 == 6 and pass.
CASE_N=$((CASE_N + 1)); d="$(fresh)"
edit 's/^  schedule:$/  "schedule":/' "$d/stress.yml"
cp "$d/stress.yml" "$d/ghost-quoted.yml"
edit 's/^  schedule:$/  "schedule":/' "$d/ghost-quoted.yml"
expect_fail "a QUOTED \`\"schedule\":\` key is seen, not scored unscheduled" "ghost-quoted.yml" "$d"

CASE_N=$((CASE_N + 1)); d="$(fresh)"
cp "$d/stress.yml" "$d/ghost-spaced.yml"
edit 's/^  schedule:$/  schedule :/' "$d/ghost-spaced.yml"
expect_fail "a SPACE-before-colon \`schedule :\` key is seen" "ghost-spaced.yml" "$d"

# ── 10. zero scheduled workflows would make the left-hand side vacuous ───────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
for f in "$d"/*.yml; do edit 's/^ *schedule:$/  # schedule removed by selftest/' "$f"; done
expect_fail "ZERO scheduled workflows fails instead of trivially matching" "ZERO scheduled workflows" "$d"

# ── 11. the shell-keyword-assignment trap ────────────────────────────────────
CASE_N=$((CASE_N + 1)); d="$(fresh)"
printf '# planted by selftest\n#   then=$(date -u +%%s)\n' >> "$d/coverage.yml"
expect_fail "a shell KEYWORD used as an assignment target is named" "shell KEYWORD" "$d"

echo
if [ "$FAILED" -gt 0 ]; then
  echo "deadman-registry selftest: $FAILED case(s) FAILED — the gate does not have the teeth it claims." >&2
  exit 1
fi
echo "deadman-registry selftest: all $CASE_N cases pass (clean tree green; every claimed defect class red AND named)"
