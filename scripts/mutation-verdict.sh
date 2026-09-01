#!/usr/bin/env bash
# mutation-verdict.sh — decide WHICH message a cargo-mutants exit-2 deserves.
#
# The single home of that decision: .github/workflows/mutation.yml EXECUTES this file
# (`bash scripts/mutation-verdict.sh <report>`) and mutation-verdict-selftest.sh SOURCES
# it, so the lane and its test cannot drift into two copies.
#
# The lane's half of that is ASSERTED, not merely stated: `workflow-wiring` fails if
# the workflow stops calling this script on one uncommented line binding its fields in
# order, and `executed-entry-point` runs it as a subprocess the way the lane does.
# Before those existed the header made the claim while an inline reimplementation, a
# transposed `read`, and deleting the `BASH_SOURCE`/`$0` block below each left the
# self-test fully green. The self-test's half — that it still SOURCES this file rather
# than carrying its own copy — has no dedicated check; it holds because a stale inlined
# copy diverges from the real script and `executed-entry-point` then fails. That is one
# fixture doing it incidentally, not an assertion, and it is stated that way on purpose.
#
# This file holds no policy of its own beyond the classification; the workflow owns
# the exit code.
#
# THE PROBLEM IT SOLVES
# cargo-mutants reports every survivor as MissedMutant. PR #247 exposed a useful
# diagnostic inside that population: sixteen outcomes completed at almost no cost
# against the run's baseline, and their build logs later confirmed that the changed
# Windows-only source was not part of the Linux build. Timing makes those outcomes
# worth inspecting, but it does NOT prove why they were fast.
#
# In particular, cargo-mutants can baseline all packages touched by a diff while a
# mutant runs only its own package. A real survivor in a small package can therefore
# cost less than 5% of an aggregate baseline. Treating that ratio as proof of cfg-out
# sends an author to the exclusion ledger instead of to the missing test — failure in
# the dangerous direction. This script consequently emits POSSIBLY_UNCOMPILED only
# as a diagnostic hint and ALWAYS retains SURVIVORS for every MissedMutant. A ledger
# entry requires separate, auditable build evidence.
#
# WHY A RATIO AND NOT A CONSTANT
# The first version used "< 1s". That is machine- and repo-dependent: it is safe in
# THIS repo only because the observed durations are bimodal. On a smaller crate, or a
# faster machine, a real survivor can land under a second. `.outcomes[]` carries the
# Baseline scenario, so 5% of it is a machine-relative way to select outcomes for
# inspection. It is a triage signal, not a semantic verdict.
#
# The one baseline quoted in this file is PR #247's CI run on the corelink runner:
# 110s build + 27s test, so the threshold there is 6.85s. That is the only baseline
# figure here that can be reproduced from something in the repo (`.cargo/mutants.toml`
# records the same run). An earlier version of this comment also quoted a second,
# local baseline of 37.69s to argue the ratio branch really evaluates; that number is
# not reproducible from this tree and has been removed rather than left standing —
# a file whose whole thesis is falsifiability must not carry a figure nobody can
# check. That the ratio branch really evaluates is pinned instead by the self-test's
# `just-under-threshold` / `just-over-threshold` pair, which straddle 6.85s and fail
# if the ratio moves in either direction.
#
# WHY THE WHOLE TIMING SHAPE IS REQUIRED
# Duration is only a useful diagnostic relative to a complete, successful run. cargo-mutants
# 27.0.0 emits exactly two phase results here, in order: Build and Test, each with a
# non-negative numeric duration and process_status=Success. The classifier requires
# that shape for BOTH the Baseline and each candidate. Accepting merely one numeric
# field is unsafe: a report with Build=0.5s and an unreadable 27s Test duration looks
# artificially cheap and raises a false diagnostic. Likewise, without a
# positive Baseline there is no machine-relative evidence at all; the old absolute
# one-second fallback reintroduced the exact machine-dependent threshold this ratio
# replaced. Missing, partial, failed or unrecognised timing therefore degrades toward
# SURVIVORS — "write the test" — and never authorises exclusion.
#
# CONTRACT
#   mutation_verdict <outcomes.json>
#     prints: "<missed> <possible_uncompiled> <VERDICT...>" on one line
#     VERDICT is SURVIVORS, optionally preceded by POSSIBLY_UNCOMPILED when complete
#     timings fall below the diagnostic ratio. It is never empty: an unreadable,
#     absent or malformed report yields SURVIVORS, so a broken instrument sends the
#     author to the tests rather than to the ledger.
#   Returns 0 always — the classification is the product; the caller decides the exit.
#
# THE RATIO IS TESTED IN BOTH DIRECTIONS
# A suite built only from zero-duration survivors pins no positive ratio. The
# self-test therefore puts complete Build+Test outcomes just below and just above 5%
# of the same Baseline. It separately proves that an absent or unusable Baseline and
# an incomplete phase record take the fail-safe SURVIVORS path.
# The verdict TOKENS and the field ORDER are likewise contract, not detail: the
# workflow matches the literal words and prints the second number to the author, and
# a consistent rename across this file and its test would leave every case green while
# muting the lane's only annotation. Both are asserted there.

set -uo pipefail

# BOTH verdicts when the diff carries both kinds. An exclusive if/else hides a real
# survivor behind any low-cost one, and tells that survivor's author NOT to write the
# test that would kill it. So POSSIBLY_UNCOMPILED is an additive hint and SURVIVORS is
# unconditional. Timing alone never accounts for or excuses a survivor.
mutation_verdict() {
  local report="${1:-}"
  local missed=0 possible_uncompiled=0

  if [ -n "$report" ] && [ -f "$report" ]; then
    missed=$(jq '[.outcomes[]? | select(.summary=="MissedMutant")] | length' \
               "$report" 2>/dev/null || echo 0)
    possible_uncompiled=$(jq '
      def complete_elapsed:
        if ((.phase_results? | type) == "array"
            and ([.phase_results[]?.phase] == ["Build", "Test"])
            and all(.phase_results[]?;
                    ((.duration? | type) == "number" and .duration >= 0
                     and .process_status? == "Success")))
        then ([.phase_results[].duration] | add)
        else null
        end;
      ([.outcomes[]? | select(.scenario? == "Baseline")
        | complete_elapsed | select(. != null and . > 0)] | first) as $base
      | if ($base | type) != "number" then 0
        else [.outcomes[]? | select(.summary=="MissedMutant")
              | complete_elapsed as $elapsed
              | select(($elapsed | type) == "number"
                       and $elapsed < ($base * 0.05))]
             | length
        end' "$report" 2>/dev/null || echo 0)
  fi

  # Normalise before arithmetic. BOTH arms are reachable and both are tested: a
  # zero-length report makes jq exit 0 emitting NOTHING (the `''` arm — `empty-report`),
  # and a file holding two concatenated JSON documents makes it emit "1\n1", which is
  # not a number (the `*[!0-9]*` arm — `numeric-normalisation`). Unguarded, either one
  # reaches the integer comparisons and costs the printed line its fields, so the lane
  # reads an empty verdict, misses both `case` arms, and exits 2 silently.
  case "${missed}" in ''|*[!0-9]*) missed=0 ;; esac
  case "${possible_uncompiled}" in ''|*[!0-9]*) possible_uncompiled=0 ;; esac

  local verdict="SURVIVORS"
  [ "$possible_uncompiled" -gt 0 ] && verdict="POSSIBLY_UNCOMPILED $verdict"

  printf '%s %s %s\n' "$missed" "$possible_uncompiled" "$verdict"
}

# Executed directly (not sourced): classify the argument and print the verdict.
# BASH_SOURCE[0] == $0 exactly when this file is the entry point.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  mutation_verdict "${1:-}"
fi
