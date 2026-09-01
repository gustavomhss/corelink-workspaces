#!/usr/bin/env bash
# mutation-verdict-selftest.sh — teeth for scripts/mutation-verdict.sh.
#
# WHY THIS EXISTS AS A SELF-TEST AND NOT AS "the next PR will exercise it"
# The mutation lane only runs when the diff touches Rust AND yields mutants. The PR
# that introduced this classification touched no Rust, so its own mutation check went
# green having never executed the new code. The obvious follow-up — "PR #247 will be
# its first real run" — was tested and is false: with the ledger in place #247 lists
# ZERO mutants, so its lane skips the run step entirely and the classification is
# still never reached. Neither a scratch PR nor waiting produces durable coverage, so
# the logic is tested directly, on every PR, the way this repo already tests its other
# gate decisions (pre-merge-gate-check-selftest.sh, gate-lane-selftest.sh,
# deadman-selftest.sh).
#
# WHAT EACH CASE MUST PIN
# Not "something fired" — WHICH verdict fired, and on non-degenerate inputs. A case
# whose survivor has duration 0 passes under ANY positive threshold, so the diagnostic
# ratio is bounded from both sides with complete Build+Test outcomes just either side
# of the boundary. Crucially, every low-cost outcome still emits SURVIVORS: timing is
# only a POSSIBLY_UNCOMPILED hint and never proof that authorises an exclusion.
#
# The verdict tokens and the field order are a CONTRACT with the workflow, which
# matches on the literal words `POSSIBLY_UNCOMPILED` and `SURVIVORS` and prints the
# second number to the author. Renaming a token or transposing the counts is invisible to a
# test that compares only the verdict, so both are asserted explicitly at the end.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
# shellcheck source=scripts/mutation-verdict.sh
source scripts/mutation-verdict.sh

command -v jq >/dev/null 2>&1 || { echo "FAIL: jq is absent — every case below would take the malformed-report path and pass vacuously."; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# The roll-call. A hand-maintained COUNT rots silently: review added a trivial case,
# deleted a real one, and the total still matched, so the floor reported success while
# coverage had been swapped away. Names cannot be traded that way — a deleted case
# leaves its name unaccounted for, and adding one is a visible edit here. Same rule as
# the repo's other gates: derive the check from what was actually checked.
EXPECTED=(
  only-low-cost only-real mixed none
  just-under-threshold just-over-threshold
  small-crate-real small-crate-low-cost multi-package-fast-real
  no-baseline no-baseline-just-under no-baseline-real-survivor zero-baseline
  renamed-duration-field renamed-summary-field
  malformed-report absent-report empty-report partial-phase-record
  unrecognised-phase non-success-phase negative-duration
  string-scenario-not-baseline pr247-shape contract-line
  workflow-vocabulary workflow-wiring
  executed-entry-point numeric-normalisation
)
RAN=()
fail=0

# outcome <summary> <scenario-json> <build-duration> <test-duration>
# This is cargo-mutants 27.0.0's observed shape, verified against PR #247's artifact:
# exactly Build then Test, both successful and carrying numeric durations.
outcome() {
  [ "$#" -eq 4 ] || { echo "outcome requires exactly four arguments" >&2; return 1; }
  local summary="$1" scenario="$2" build_duration="$3" test_duration="$4"
  printf '{"summary":"%s","scenario":%s,"phase_results":[{"phase":"Build","duration":%s,"process_status":"Success"},{"phase":"Test","duration":%s,"process_status":"Success"}]}' \
    "$summary" "$scenario" "$build_duration" "$test_duration"
}

fixture() { printf '{"outcomes":[%s]}' "$2" > "$TMP/$1.json"; printf '%s\n' "$TMP/$1.json"; }

# A mutant's real `scenario` is an OBJECT — `{"Mutant":{...}}` — never null. Fixtures
# used null, which made `== "Baseline"` indistinguishable from `!= null` or
# `(type)=="string"`: all three left the suite green. Using the real shape pins the
# selector to the value it actually reads.
MUT='{"Mutant":{"package":"clw-x","file":"src/lib.rs","line":1}}'

# assert <name> <expected> <got> <because>
assert() {
  RAN+=("$1")
  [ "$3" = "$2" ] && return 0
  fail=$(( fail + 1 ))
  printf 'FAIL: %s\n  expected: %s\n  got:      %s\n  because:  %s\n' "$1" "$2" "$3" "$4"
}

# check <name> <expected-verdict> <because> <outcomes-json-body>
check() {
  local got; got="$(mutation_verdict "$(fixture "$1" "$4")")"
  assert "$1" "$2" "${got#* * }" "$3"
}

BASELINE="$(outcome Success '"Baseline"' 110 27)"   # 137s total; 5% of it = 6.85s

# ── NEGATIVE CONTROL: the comparator in assert() ─────────────────────────────
# Every case below asserts a PASS, and the roll-call counts names that RAN — and
# `RAN+=` happens before the comparison. Both are satisfied by an `assert` that has
# stopped comparing: delete the equality test in assert() and this suite is green,
# WITH the exclusive if/else reintroduced, WITH the counts transposed, and WITH the
# ratio widened to absurdity. Review demonstrated it.
# The roll-call cannot see it, because every name still runs; it answers "were the
# cases executed", never "were they decided".
#
# The counters are read as DELTAS, so this block keeps working if a case is ever
# inserted above it. Absolute values would go red here claiming "assert() is not
# comparing", which would be a false diagnosis of a real regression elsewhere.
#
# So run one case whose expectation NO input can produce and require that it FAIL.
# It runs in a SUBSHELL: its FAIL text is expected output rather than a finding, and
# `fail`/`RAN` mutate only inside it, so the real cases and their roll-call below
# are untouched. The subshell hands its counters back on stdout. A harness that
# cannot say NO cannot be trusted when it says YES.
nc_fail="$fail"; nc_ran="${#RAN[@]}"
nc="$( { check comparator-negative-control "THIS-VERDICT-CANNOT-EXIST" \
           "negative control — mutation_verdict emits only POSSIBLY_UNCOMPILED and SURVIVORS, so this expectation is unsatisfiable by construction" \
           "$BASELINE,$(outcome MissedMutant "$MUT" 20 27)" >/dev/null 2>&1
         printf '%s %s' "$(( fail - nc_fail ))" "$(( ${#RAN[@]} - nc_ran ))"; } )"
if [ "$nc" != "1 1" ]; then
  echo "FAIL: negative control — an IMPOSSIBLE expectation did not fail."
  echo "      wanted 'fail=1 ran=1', got '${nc}'."
  echo "      assert() is not comparing, so every case below passes vacuously and the"
  echo "      roll-call still accounts for every name. Nothing else in this file can"
  echo "      detect that, so the suite stops here."
  exit 1
fi

# ── the four shapes of an exit-2 report ──────────────────────────────────────
check only-low-cost "POSSIBLY_UNCOMPILED SURVIVORS" \
  "the sole low-cost survivor is flagged for build-log inspection but remains actionable because timing does not prove cfg-out" \
  "$BASELINE,$(outcome MissedMutant "$MUT" 0 0)"

check only-real "SURVIVORS" \
  "the survivor cost 20s+27s — it compiled and ran, so the suite genuinely failed to kill it" \
  "$BASELINE,$(outcome MissedMutant "$MUT" 20 27)"

# The diagnostic must be additive: the presence of one low-cost outcome cannot hide
# SURVIVORS or tell any author not to write a test.
check mixed "POSSIBLY_UNCOMPILED SURVIVORS" \
  "the low-cost hint and the actionable survivor verdict must both remain visible" \
  "$BASELINE,$(outcome MissedMutant "$MUT" 0 0),$(outcome MissedMutant "$MUT" 20 27)"

check none "SURVIVORS" \
  "exit 2 with no MissedMutant parsed means the report did not say what we think — send the author to the tests, not to the ledger" \
  "$BASELINE"

# ── the 0.05 ratio, bounded from BOTH sides ──────────────────────────────────
# These two straddle the 6.85s boundary by a whisker, so moving the ratio in either
# direction reddens one of them. Durations of 0 cannot do this — they satisfy every
# positive threshold, which is exactly why the cases above do not pin the constant.
check just-under-threshold "POSSIBLY_UNCOMPILED SURVIVORS" \
  "6.0s is under 5% of a 137s baseline, so it raises a hint without excusing the survivor" \
  "$BASELINE,$(outcome MissedMutant "$MUT" 3.0 3.0)"

check just-over-threshold "SURVIVORS" \
  "7.5s is over the same 6.85s boundary and does not raise the diagnostic hint" \
  "$BASELINE,$(outcome MissedMutant "$MUT" 3.5 4.0)"

# Same absolute duration, different diagnostic hints, decided by the baseline. Both
# remain SURVIVORS regardless.
SMALL="$(outcome Success '"Baseline"' 1 1)"
check small-crate-real "SURVIVORS" \
  "0.9s exceeds 5% of a 2s baseline, so the old absolute threshold would raise a noisy hint" \
  "$SMALL,$(outcome MissedMutant "$MUT" 0.5 0.4)"

check small-crate-low-cost "POSSIBLY_UNCOMPILED SURVIVORS" \
  "0.05s is under 5% at this scale, so it is worth inspecting but is not excused" \
  "$SMALL,$(outcome MissedMutant "$MUT" 0.02 0.03)"

# The adversarial shape that disproves the old inference: cargo-mutants can baseline
# every package in a multi-package diff and then run a mutant only in its own small
# package. Complete successful 1s+1s phases against a 100s+100s baseline are not
# evidence of cfg-out. The hint may fire, but SURVIVORS must remain unconditional.
MULTI_PACKAGE="$(outcome Success '"Baseline"' 100 100)"
check multi-package-fast-real "POSSIBLY_UNCOMPILED SURVIVORS" \
  "a fast real survivor in a small package must never be converted into an exclusion recommendation by an aggregate baseline" \
  "$MULTI_PACKAGE,$(outcome MissedMutant "$MUT" 1 1)"

# ── no usable Baseline means no relative timing evidence ────────────────────
# The old one-second fallback revived the machine-dependent threshold the ratio
# replaced. Fast, slow and zero-duration survivors all stay actionable here.
check no-baseline "SURVIVORS" \
  "without a Baseline there is no relative timing evidence, so even a zero-duration survivor must not be excused" \
  "$(outcome MissedMutant "$MUT" 0 0)"

check no-baseline-just-under "SURVIVORS" \
  "a fast survivor without a Baseline must not be sent to the ledger by an absolute machine-dependent threshold" \
  "$(outcome MissedMutant "$MUT" 0.3 0.2)"

check no-baseline-real-survivor "SURVIVORS" \
  "a slower survivor without a Baseline takes the same fail-safe path" \
  "$(outcome MissedMutant "$MUT" 20 27)"

check zero-baseline "SURVIVORS" \
  "a non-positive Baseline is unusable evidence and must degrade toward writing the test" \
  "$(outcome Success '"Baseline"' 0 0),$(outcome MissedMutant "$MUT" 0 0)"

# ── incomplete or unrecognised evidence stays actionable ────────────────────
# A schema/report we cannot read completely must send the author to the tests, never
# into even a diagnostic hint. These fixtures use the real scenario and phase shape up to the one
# field each case deliberately damages.
printf '{"outcomes":[{"summary":"MissedMutant","scenario":{"Mutant":{}},"phase_results":[{"phase":"Build","secs":0,"process_status":"Success"},{"phase":"Test","secs":0,"process_status":"Success"}]}]}' > "$TMP/rd.json"
got="$(mutation_verdict "$TMP/rd.json")"
assert renamed-duration-field "SURVIVORS" "${got#* * }" \
  "a schema rename must degrade toward 'write the test', never toward 'do not write it'"

printf '{"outcomes":[{"verdict":"MissedMutant","scenario":{"Mutant":{}},"phase_results":[{"phase":"Build","duration":0,"process_status":"Success"},{"phase":"Test","duration":0,"process_status":"Success"}]}]}' > "$TMP/rs.json"
got="$(mutation_verdict "$TMP/rs.json")"
assert renamed-summary-field "SURVIVORS" "${got#* * }" \
  "with .summary renamed nothing parses as a MissedMutant, so the ordinary message must win"

printf 'not json at all {{{' > "$TMP/bad.json"
got="$(mutation_verdict "$TMP/bad.json")"
assert malformed-report "SURVIVORS" "${got#* * }" \
  "an instrument that cannot read its own report must not be the thing that excuses a survivor"

got="$(mutation_verdict "$TMP/does-not-exist.json")"
assert absent-report "SURVIVORS" "${got#* * }" \
  "an absent report provides no timing evidence, so do not raise the hint"

# MED-1: the `''` arm of the normalisation guards was deletable with everything
# green, and it is the MORE reachable arm — a zero-length outcomes.json (truncated
# write, disk full, interrupted run) makes jq exit 0 emitting NOTHING, so both
# counters are empty strings. The script's own comment names that mechanism; nothing
# tested it. Without the arm the function prints "  " and the lane, again, exits 2
# with no annotation. I had tested the `*[!0-9]*` arm and left this one, in the very
# commit that fixed three other "the header names a hazard the suite cannot see".
: > "$TMP/empty.json"
got="$(mutation_verdict "$TMP/empty.json")"
assert empty-report "0 0 SURVIVORS" "$got" \
  "a zero-length report makes jq succeed emitting nothing, so both counters are empty; unguarded they reach the integer comparisons and the printed line loses its fields"

# One readable duration is not a complete run. The old `| numbers | add` accepted
# Build=0.5s, silently dropped the unreadable 27s Test duration and raised a false hint.
printf '{"outcomes":[{"scenario":"Baseline","summary":"Success","phase_results":[{"phase":"Build","duration":110,"process_status":"Success"},{"phase":"Test","duration":27,"process_status":"Success"}]},{"scenario":{"Mutant":{}},"summary":"MissedMutant","phase_results":[{"phase":"Build","duration":0.5,"process_status":"Success"},{"phase":"Test","secs":27,"process_status":"Success"}]}]}' > "$TMP/partial.json"
got="$(mutation_verdict "$TMP/partial.json")"
assert partial-phase-record "1 0 SURVIVORS" "$got" \
  "one unreadable phase must invalidate the timing evidence rather than make a real survivor look artificially cheap"

printf '{"outcomes":[{"scenario":"Baseline","summary":"Success","phase_results":[{"phase":"Build","duration":110,"process_status":"Success"},{"phase":"Test","duration":27,"process_status":"Success"}]},{"scenario":{"Mutant":{}},"summary":"MissedMutant","phase_results":[{"phase":"Build","duration":0.2,"process_status":"Success"},{"phase":"Check","duration":0.3,"process_status":"Success"}]}]}' > "$TMP/unrecognised.json"
got="$(mutation_verdict "$TMP/unrecognised.json")"
assert unrecognised-phase "1 0 SURVIVORS" "$got" \
  "only the observed Build+Test shape is timing evidence; a new or renamed phase must fail safe"

printf '{"outcomes":[{"scenario":"Baseline","summary":"Success","phase_results":[{"phase":"Build","duration":110,"process_status":"Success"},{"phase":"Test","duration":27,"process_status":"Success"}]},{"scenario":{"Mutant":{}},"summary":"MissedMutant","phase_results":[{"phase":"Build","duration":0.2,"process_status":"Failure"},{"phase":"Test","duration":0.3,"process_status":"Success"}]}]}' > "$TMP/non-success.json"
got="$(mutation_verdict "$TMP/non-success.json")"
assert non-success-phase "1 0 SURVIVORS" "$got" \
  "a failed phase is not evidence of a complete cheap run and must not excuse the survivor"

printf '{"outcomes":[{"scenario":"Baseline","summary":"Success","phase_results":[{"phase":"Build","duration":110,"process_status":"Success"},{"phase":"Test","duration":27,"process_status":"Success"}]},{"scenario":{"Mutant":{}},"summary":"MissedMutant","phase_results":[{"phase":"Build","duration":-1,"process_status":"Success"},{"phase":"Test","duration":0.3,"process_status":"Success"}]}]}' > "$TMP/negative.json"
got="$(mutation_verdict "$TMP/negative.json")"
assert negative-duration "1 0 SURVIVORS" "$got" \
  "a negative duration is invalid evidence, not proof that the mutant was never compiled"

# LOW-5: with `"scenario": null` fixtures, `== "Baseline"` was indistinguishable from
# `(type)=="string"`, and the $MUT object shape alone does NOT close that — review
# measured it still green. A future schema with a second string scenario emitted
# before the baseline would then be taken as the baseline. Here "Check" costs 1000s:
# read correctly the threshold is 5% of 137s and a 20s survivor is a real one; taken
# as the baseline the threshold becomes 50s and that same survivor gets a false hint.
check string-scenario-not-baseline "SURVIVORS" \
  "only the outcome whose scenario is exactly \"Baseline\" may set the threshold; any other string-valued scenario appearing first must not" \
  "$(outcome Success '"Check"' 500 500),$BASELINE,$(outcome MissedMutant "$MUT" 10 10)"

# Actual PR #247 scale and population: the archived cargo-mutants 27.0.0 report has
# a 110.977s+27.159s Baseline, sixteen low-cost survivors around 0.15-0.36s per phase,
# and one survivor at 4.337s+36.969s. Logs later established why the sixteen were
# cheap; outcomes.json alone only supports a 17/16 diagnostic, never an exclusion.
pr247_body="$(outcome Success '"Baseline"' 110.977 27.159)"
for _ in {1..16}; do
  pr247_body+=",$(outcome MissedMutant "$MUT" 0.2 0.3)"
done
pr247_body+=",$(outcome MissedMutant "$MUT" 4.337 36.969)"
got="$(mutation_verdict "$(fixture pr247-shape "$pr247_body")")"
assert pr247-shape "17 16 POSSIBLY_UNCOMPILED SURVIVORS" "$got" \
  "the real PR #247 population raises sixteen hints while all seventeen remain actionable until logs are inspected"

# ── the contract with the workflow ───────────────────────────────────────────
# Every case above compares only the verdict, so transposing the two counts in the
# printf leaves the whole suite green — while the lane tells the author the WRONG
# number of never-compiled mutants. Assert the full line once.
got="$(mutation_verdict "$(fixture contract "$BASELINE,$(outcome MissedMutant "$MUT" 0 0),$(outcome MissedMutant "$MUT" 20 27)")")"
assert contract-line "2 1 POSSIBLY_UNCOMPILED SURVIVORS" "$got" \
  "field order is part of the documented contract: the lane prints \$possible_uncompiled to the author, so transposed counts are wrong"

# The token spellings are a contract too — the workflow matches the literal words.
# Rename one consistently across script and test and every case here stays green
# while the lane exits 2 with no ::error:: annotation at all.
WF=".github/workflows/mutation.yml"
missing=""
if [ -f "$WF" ]; then
  for tok in POSSIBLY_UNCOMPILED SURVIVORS; do
    grep -q "\*\" ${tok} \"\*" "$WF" || missing="${missing:+$missing }$tok"
  done
else
  missing="(workflow absent)"
fi
assert workflow-vocabulary "" "$missing" \
  "$WF must still match the exact tokens this script emits; a consistent rename would silently mute the lane's only author-facing message"

# MED-A: the vocabulary check greps for the TOKENS, and those survive an inline
# reimplementation — so it pinned the words while the lane could stop calling this
# script entirely. Three edits left the suite green: transposing the two `read`
# fields (the lane then tells the author the wrong count, which is round 1's MED-1
# reopened on the consumer side), dropping the report argument, and replacing the
# call with hardcoded values. Assert the wiring itself.
# Comments are stripped first, and the match is the WHOLE wiring line rather than
# two independent greps. Both matter: LOW-E's fix put a pointer comment immediately
# above this line, so a future editor making that pointer concrete ("invoke it as
# `bash scripts/mutation-verdict.sh …` and bind with `read -r missed possible_uncompiled …`")
# would satisfy two prose-sniffing greps while the call itself was gone — review
# demonstrated exactly that, 21 green with the call replaced by an inline
# reimplementation. Matching the whole line also closes today's gap where the call
# and the read need not be related to each other at all.
# shellcheck disable=SC2016  # the literal text is the point: this is the line we
# require to be present in the workflow, not something to expand here.
WIRE='read -r missed possible_uncompiled verdict_rest <<<"$(bash scripts/mutation-verdict.sh mutants.out/outcomes.json)"'
wiring=""
if [ -f "$WF" ]; then
  sed 's/[[:space:]]*#.*//' "$WF" | grep -qF "$WIRE" || wiring="wiring-line"
else
  wiring="(workflow absent)"
fi
assert workflow-wiring "" "$wiring" \
  "the header claims the lane and this test cannot drift into two copies; that holds only while the lane still CALLS this script and binds its fields in the documented order, on one line, outside a comment"

# MED-C: everything above SOURCES the file. The lane EXECUTES it, and the header
# names deleting the entry-point block as the hazard — yet deleting it left the suite
# green while the lane would emit no annotation at all. Run it the way the lane does.
got="$(bash scripts/mutation-verdict.sh "$(fixture executed "$BASELINE,$(outcome MissedMutant "$MUT" 0 0),$(outcome MissedMutant "$MUT" 20 27)")")"
assert executed-entry-point "2 1 POSSIBLY_UNCOMPILED SURVIVORS" "$got" \
  "the workflow runs this as a subprocess, so the BASH_SOURCE/\$0 block is load-bearing; without it the lane reads empty output, both case arms miss, and it exits 2 with no ::error:: at all"

# LOW-H: the numeric normalisation is asserted as protective in the script header,
# and the first case written for it was UNREACHABLE — `.outcomes[]?` swallows a
# structurally wrong document and jq still returns 0, so the guards never fired and
# deleting them stayed green. The reachable input is a file holding TWO concatenated
# JSON documents (a truncated or doubly-written report): jq processes each and emits
# "1\n1", which is not a number. With the guards that normalises to zeros and the
# ordinary message. Without them the function emits THREE lines, not a truncated one,
# and the workflow's `read` binds only the first — leaving the verdict empty, so both
# case arms miss and the lane exits 2 with no `::error::` at all. (An earlier version
# of this comment said `read` bound the fields to the WRONG values; review measured
# it and they bind EMPTY. The consequence is the muted lane, not a wrong count.)
printf '{"outcomes":[{"summary":"MissedMutant","phase_results":[{"duration":9}]}]}\n{"outcomes":[{"summary":"MissedMutant","phase_results":[{"duration":9}]}]}\n' > "$TMP/multidoc.json"
got="$(mutation_verdict "$TMP/multidoc.json")"
assert numeric-normalisation "0 0 SURVIVORS" "$got" \
  "multi-document jq output is not a number; unnormalised it breaks the comparisons and the printed line, so the lane parses garbage"

# ── anti-vacuity: the harness itself ─────────────────────────────────────────
# Reported AFTER the failures above, so a genuine regression is diagnosed as itself
# rather than as a missing case — review hit exactly that inversion.
#
# This roll-call and the negative control at the top cover DIFFERENT failures and
# neither substitutes for the other: the roll-call catches a case that stopped
# RUNNING, the negative control catches a comparator that stopped DECIDING.
missing_cases=""
for want in "${EXPECTED[@]}"; do
  case " ${RAN[*]} " in *" $want "*) ;; *) missing_cases="${missing_cases:+$missing_cases }$want" ;; esac
done
if [ -n "$missing_cases" ]; then
  echo "FAIL: these cases never ran: ${missing_cases}"
  echo "      A harness that stops executing cases reports success in exactly the same way"
  echo "      as one that passes them, so an unaccounted-for name is itself a failure."
  fail=$(( fail + 1 ))
fi

if [ "$fail" -ne 0 ]; then
  echo "mutation-verdict selftest: ${fail} FAILED, $(( ${#RAN[@]} - fail )) passed"
  exit 1
fi
echo "mutation-verdict selftest: ${#RAN[@]} cases passed"
