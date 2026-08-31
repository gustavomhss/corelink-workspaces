#!/usr/bin/env bash
# deadman-registry.sh — the ONLY reader of the lane registry in
# .github/workflows/canary-deadman.yml, and the gate that keeps that registry
# equal to the set of scheduled workflows.
#
#   bash scripts/deadman-registry.sh lanes      # print "<workflow> <max_age_hours>"
#   bash scripts/deadman-registry.sh known-red  # print "<workflow> <reason>"
#   bash scripts/deadman-registry.sh check      # OFFLINE registry gate (scripts/gates.sh)
#   bash scripts/deadman-registry.sh watch      # ONLINE surveillance (canary-deadman.yml)
#
# ── WHAT `check` GUARANTEES, AND WHAT IT DOES NOT ────────────────────────────
# It guarantees that the registry and the set of workflows this scanner can SEE a
# schedule in are exactly equal, both directions. It sees a schedule by matching
# `^[[:space:]]*schedule:` — a block-style key. YAML has other spellings, and
# rather than lose a spelling race the scanner REFUSES what it cannot read: a
# workflow whose `on:` value is inline (flow style `on: {…}` / `on: [push]` /
# `on: push`) is a hard FAILURE naming the file, not a silent "no schedule here".
# So the reachable outcomes are: parsed, or refused — for the key spellings this
# scanner normalises, which are an optional quote around `schedule` and whitespace
# before the colon. It is a SCANNER, not a YAML parser: a spelling outside that set
# would be scored unscheduled rather than refused. Two such spellings got past the
# bare `schedule:` pattern in review, which is why the set is enumerated here instead
# of the earlier claim that no third outcome existed.
#
# It does NOT verify that the cron in the workflow matches the window in the
# registry — a lane could move from daily to weekly and keep a 26h window, which
# this would score as a healthy registration and the deadman would then false-alarm
# on. The window is a human judgement about acceptable silence, not a copy of the
# cron, so it is reviewed in a diff (the registry comments carry the cron for that
# review) rather than derived.
#
# ── ANTI-VACUITY ─────────────────────────────────────────────────────────────
# Every way of NOT getting a list is a named failure, never a zero and never a
# skip: the workflow file missing, a marker missing, duplicated markers, a
# malformed row, a non-integer or non-positive window, a registered workflow file
# that does not exist, a duplicate lane, and — the one always forgotten — the lane
# list coming back EMPTY. An empty registry is not "vacuously satisfied"; it is
# this parser having broken, and a deadman watching zero lanes would print OK for
# a repo whose entire surveillance had stopped.
#
# The KNOWN_RED ledger is the deliberate exception: it may legitimately be empty
# (that is its desired state), so emptiness there is fine — but its markers must
# still both be present, so a deleted ledger is a failure rather than a silent
# "nothing is excused".
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WF_DIR="${DEADMAN_WF_DIR:-$REPO_ROOT/.github/workflows}"
DEADMAN_WF="canary-deadman.yml"
WF="$WF_DIR/$DEADMAN_WF"

fatal() {
  echo "::error::DEADMAN-REGISTRY: $*" >&2
  echo "DEADMAN-REGISTRY FAILED: $*" >&2
  exit 1
}

# Extract the lines strictly between a begin and an end marker. Both markers must
# appear EXACTLY once, in order; anything else is a parse break, not an empty set.
extract_block() {
  local begin="$1" end="$2" nb ne
  [ -f "$WF" ] || fatal "$WF does not exist — the registry cannot be read, so nothing is being watched."
  nb="$(grep -c -F -- "$begin" "$WF" || true)"
  ne="$(grep -c -F -- "$end" "$WF" || true)"
  [ "$nb" = "1" ] || fatal "marker '$begin' appears $nb times in $DEADMAN_WF (expected exactly 1) — the registry block is not delimited, so its scope would be guesswork."
  [ "$ne" = "1" ] || fatal "marker '$end' appears $ne times in $DEADMAN_WF (expected exactly 1) — the registry block is not delimited, so its scope would be guesswork."
  awk -v b="$begin" -v e="$end" '
    index($0, b) { seen=1; f=1; next }
    index($0, e) { if (!f) { print "MARKER-ORDER-BROKEN" > "/dev/stderr"; exit 3 } f=0; next }
    f { print }
    END { if (!seen) exit 3 }
  ' "$WF" || fatal "the '$begin' / '$end' markers in $DEADMAN_WF are out of order — refusing to guess the block's extent."
}

# Strip trailing `# …` comments and surrounding whitespace; drop blank lines.
strip_comments() {
  sed -e 's/#.*$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' | grep -v '^$' || true
}

lanes() {
  local raw line wf hours seen=""
  # `|| exit 1`: see the note in known_red() — a fatal inside a command
  # substitution must have its status re-asserted, never left to `set -e`.
  raw="$(extract_block '# deadman:lanes:begin' '# deadman:lanes:end' | strip_comments)" || exit 1
  [ -n "$raw" ] || fatal "the lane registry extracted EMPTY. Either every scheduled lane really was deregistered, or this parser's reading of $DEADMAN_WF broke — and a deadman watching zero lanes prints OK for a repo whose surveillance has entirely stopped. Failing instead."
  while IFS= read -r line; do
    # shellcheck disable=SC2086  # deliberate word-split: a row is exactly two fields
    set -- $line
    [ "$#" -eq 2 ] || fatal "malformed registry row '$line' in $DEADMAN_WF — expected exactly '<workflow-file> <max_age_hours>', got $# field(s)."
    wf="$1"; hours="$2"
    case "$wf" in
      *.yml|*.yaml) : ;;
      *) fatal "registry row '$line' names '$wf', which is not a .yml/.yaml workflow file." ;;
    esac
    [ "$wf" != "$DEADMAN_WF" ] || fatal "the registry lists $DEADMAN_WF itself. A dead-man's-switch cannot be an element of the set it watches: if it stops running, its own entry stops being evaluated. Its liveness is out of scope here — see WP-36."
    [ -f "$WF_DIR/$wf" ] || fatal "registry row '$line' names $wf, which does not exist in $WF_DIR — the deadman would be watching a workflow that is not there."
    case "$hours" in
      ''|*[!0-9]*) fatal "registry row '$line' has a non-integer window '$hours'." ;;
    esac
    [ "$hours" -gt 0 ] || fatal "registry row '$line' has a window of $hours hours; a window of 0 can never be satisfied."
    case " $seen " in *" $wf "*) fatal "registry lists $wf more than once — two windows for one lane is ambiguous." ;; esac
    seen="$seen $wf"
    printf '%s %s\n' "$wf" "$hours"
  done <<< "$raw"
}

known_red() {
  # May legitimately be EMPTY (that is the desired state) — but the markers must
  # exist, so a DELETED ledger is a parse failure rather than a silent "nothing is
  # excused". extract_block is captured on its own line: piping it straight into a
  # `… || true` filter would let its fatal exit be swallowed by that `|| true`.
  # The `|| exit 1` is NOT belt-and-braces, it is load-bearing, and the selftest is
  # what proved it: inside a command substitution, a failing assignment does not
  # reliably abort under `set -e`, so execution fell through to the `… || true`
  # filter below and this function returned 0 with an empty ledger — a deleted
  # KNOWN_RED marker printed its error and the gate went GREEN anyway. Exactly the
  # fail-open this file's header swears cannot happen. Never let a `|| true` share
  # a subshell with a fatal you rely on.
  local raw out
  raw="$(extract_block '# deadman:known-red:begin' '# deadman:known-red:end')" || exit 1
  out="$(printf '%s\n' "$raw" \
    | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
    | grep -v '^$' | grep -v '^#' || true)"
  [ -z "$out" ] || printf '%s\n' "$out"
}

# The set of workflows this scanner can see a schedule in, excluding the deadman.
scheduled_lanes() {
  local f base found=""
  for f in "$WF_DIR"/*.yml "$WF_DIR"/*.yaml; do
    [ -e "$f" ] || continue   # an unmatched glob stays literal
    base="$(basename "$f")"
    # Refuse what this scanner cannot read, rather than scoring it "no schedule".
    if grep -qE "^['\"]?on['\"]?:[[:space:]]*[^[:space:]]" "$f"; then
      fatal "$base declares its triggers INLINE (\`on: <value>\` on one line). This scanner only reads block-style \`on:\` with an indented \`schedule:\` key, so it cannot tell whether $base is scheduled. Convert it to block style, or teach this scanner the shape — do not leave it unreadable."
    fi
    grep -qE "^['\"]?on['\"]?:[[:space:]]*$" "$f" \
      || fatal "$base has no top-level \`on:\` key this scanner can find — every workflow has triggers, so this is a parse break, not a workflow without them."
    [ "$base" != "$DEADMAN_WF" ] || continue
    # Tolerate the spellings YAML treats as the SAME mapping key: an optional quote
    # around the key, and whitespace before the colon. Cold review got both past the
    # bare `schedule:` pattern SILENTLY — not refused, not detected — which is the one
    # outcome the header above swears is unreachable. `yaml.safe_load` returns the
    # trigger key `schedule` for `"schedule":` and for `schedule :` alike.
    if grep -qE '^[[:space:]]*['"'"'"]?schedule['"'"'"]?[[:space:]]*:' "$f"; then
      found="$found$base"$'\n'
      printf '%s\n' "$base"
    fi
  done
  [ -n "$found" ] || fatal "found ZERO scheduled workflows in $WF_DIR. Either every scheduled lane was deleted, or this scan broke — and an empty left-hand side would make the registry trivially complete no matter what it contains. Failing instead."
}

# `then=$(date …)` at the head of an `if` block reads as the `then` KEYWORD to
# every human and every editor's syntax highlighter; it is a variable assignment
# that happens to be legal, and the next person to edit around it introduces a
# real bug. The same trap exists for every other shell keyword. Assert none of
# them is ever the left-hand side of an assignment in CI shell.
# (The pattern below deliberately never contains the literal `<keyword>=` so this
# guard does not flag itself.)
keyword_assignment_guard() {
  local hits kw_re
  kw_re='(^|[^[:alnum:]_])(then|else|elif|fi|do|done|if|case|esac|while|until|for|in|function|select)='
  hits="$(grep -nIE "$kw_re" "$WF_DIR"/*.yml "$WF_DIR"/*.yaml "$REPO_ROOT"/scripts/*.sh 2>/dev/null \
    | grep -v 'deadman-registry\.sh' | grep -v 'deadman-selftest\.sh' || true)"
  if [ -n "$hits" ]; then
    echo "$hits" >&2
    fatal "a shell KEYWORD is used as an assignment target above. It is legal and it is a trap: it reads as the keyword. Rename the variable."
  fi
}

check() {
  local registered scheduled missing extra kr wf
  registered="$(lanes | awk '{print $1}' | sort)"
  scheduled="$(scheduled_lanes | sort)"

  # Leg 1: a scheduled lane nobody registered -> nothing watches it.
  missing="$(comm -23 <(printf '%s\n' "$scheduled") <(printf '%s\n' "$registered"))"
  if [ -n "$missing" ]; then
    echo "$missing" >&2
    fatal "the workflow(s) above are SCHEDULED but absent from the lane registry in $DEADMAN_WF — they can stop running and nothing would go red. Add a '<workflow> <max_age_hours>' row between the deadman:lanes markers."
  fi

  # Leg 2: a registered lane that is not scheduled -> the registry overstates its
  # reach. This leg feels redundant and is not: a registry that claims to watch a
  # lane GitHub never runs would fail the deadman forever on freshness, and a
  # registry nobody trusts is ignored in both directions.
  extra="$(comm -13 <(printf '%s\n' "$scheduled") <(printf '%s\n' "$registered"))"
  if [ -n "$extra" ]; then
    echo "$extra" >&2
    fatal "the workflow(s) above are REGISTERED but have no schedule — the registry overstates what is being watched, and the deadman would go red on a lane that is not supposed to run on its own. Give the workflow a schedule, or remove its registry row."
  fi

  # Leg 3: the KNOWN_RED ledger may only excuse lanes that are actually watched.
  kr="$(known_red)"
  if [ -n "$kr" ]; then
    while IFS= read -r wf; do
      wf="${wf%% *}"
      case " $(printf '%s' "$registered" | tr '\n' ' ') " in
        *" $wf "*) : ;;
        *) fatal "KNOWN_RED excuses '$wf', which is not in the lane registry — excusing a lane nobody watches excuses nothing." ;;
      esac
    done <<< "$kr"
  fi

  keyword_assignment_guard

  echo "deadman registry: $(printf '%s\n' "$registered" | wc -l | tr -d ' ') lane(s) registered == $(printf '%s\n' "$scheduled" | wc -l | tr -d ' ') scheduled workflow(s); KNOWN_RED entries: $(printf '%s' "$kr" | grep -c '^' || true)"
}

# ── the online half ──────────────────────────────────────────────────────────

to_epoch() {
  date -u -d "$1" +%s 2>/dev/null && return 0
  date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s 2>/dev/null && return 0
  return 1
}

emit() {
  echo "$1"
  [ -z "${GITHUB_STEP_SUMMARY:-}" ] || echo "$1" >> "$GITHUB_STEP_SUMMARY"
}

# Informational ONLY — never fails the deadman. How long the repo has gone without
# a tag is a release-cadence signal a human reads, not a health invariant: a repo
# that is deliberately not releasing is not broken. When it cannot be determined
# it says UNKNOWN rather than inventing a number.
days_since_last_tag() {
  local tag ts now
  if ! tag="$(git -C "$REPO_ROOT" for-each-ref --sort=-creatordate --count=1 \
      --format='%(refname:short) %(creatordate:unix)' refs/tags 2>/dev/null)" || [ -z "$tag" ]; then
    echo "::warning::days-since-last-tag: UNKNOWN — no tags are readable here (a shallow checkout has none; this job checks out with fetch-depth: 0 precisely so they are)."
    emit "days-since-last-tag: UNKNOWN (no readable tags)"
    return 0
  fi
  ts="${tag##* }"
  now="$(date -u +%s)"
  emit "days-since-last-tag: $(( (now - ts) / 86400 )) (newest tag: ${tag%% *}) — informational, never fails this lane"
}

watch() {
  local failures=0 excused wf hours line runs latest created concl url age_h now run_epoch registry
  local ov_lane="${DEADMAN_OVERRIDE_LANE:-}" ov_hours="${DEADMAN_OVERRIDE_HOURS:-}"
  # Captured into a variable BEFORE the loop on purpose: a failing command
  # substitution inside a here-string is not reliably caught by `set -e`, so
  # `done <<< "$(lanes)"` could iterate zero times and report OK if the parser broke.
  registry="$(lanes)"

  if [ -n "$ov_lane" ] || [ -n "$ov_hours" ]; then
    [ -n "$ov_lane" ] && [ -n "$ov_hours" ] \
      || fatal "override_lane and override_hours must be given together (got lane='$ov_lane' hours='$ov_hours')."
    case "$ov_hours" in ''|*[!0-9]*) fatal "override_hours '$ov_hours' is not an integer." ;; esac
    emit "⚠️  WINDOW OVERRIDE IN EFFECT: $ov_lane -> ${ov_hours}h. This run is a TEST of the deadman, NOT a health verdict for that lane."
  fi

  excused=" $(known_red | awk '{print $1}' | tr '\n' ' ') "
  now="$(date -u +%s)"

  while IFS= read -r line; do
    wf="${line%% *}"; hours="${line##* }"
    if [ "$wf" = "$ov_lane" ]; then hours="$ov_hours"; fi

    # A gh failure is a NAMED failure. It must never read as "no runs found":
    # "the API refused us" and "this lane is dead" are opposite conclusions.
    # --event schedule is load-bearing. Every lane here carries workflow_dispatch as
    # well, so without it ONE manual `gh workflow run mutation.yml` buys 8 days of
    # false freshness on a lane whose cron is dead — and the verdict below says "the
    # SCHEDULE has stopped reporting", which would then be a stronger claim than the
    # data supports. This WP is about dead crons; ask about crons.
    if ! runs="$(gh run list --workflow "$wf" --event schedule -L 30 --json createdAt,conclusion,status,url 2>&1)"; then
      emit "❌ $wf — could not LIST runs (gh failed). This is the instrument breaking, not a verdict: $runs"
      failures=$((failures + 1))
      continue
    fi
    # An in-progress run is not evidence of a completed report, so it is filtered
    # out here: a lane that starts and never finishes is still dark.
    if ! latest="$(printf '%s' "$runs" | jq -c 'map(select(.status=="completed")) | sort_by(.createdAt) | last // empty' 2>&1)"; then
      emit "❌ $wf — could not PARSE the run list: $latest"
      failures=$((failures + 1))
      continue
    fi

    # Condition 1: does a completed run exist at all?
    if [ -z "$latest" ]; then
      emit "❌ $wf — NO completed run found in the last 30 runs. The lane is not running; whatever it watches is DARK."
      failures=$((failures + 1))
      continue
    fi
    # createdAt, not updatedAt: the clock starts when the run STARTED. A long run is
    # therefore scored slightly older than it is. Harmless at 26h/192h windows against
    # sub-hour lanes, and deliberately conservative — it can only make the deadman
    # complain early, never late. Noted so a future window tightening does not inherit
    # the bias without seeing it.
    created="$(printf '%s' "$latest" | jq -r '.createdAt')"
    concl="$(printf '%s' "$latest" | jq -r '.conclusion')"
    url="$(printf '%s' "$latest" | jq -r '.url')"
    if ! run_epoch="$(to_epoch "$created")"; then
      emit "❌ $wf — could not parse the run timestamp '$created'. Instrument broken, not a verdict."
      failures=$((failures + 1))
      continue
    fi
    age_h=$(( (now - run_epoch) / 3600 ))

    # Condition 2: is it recent? Never excused — an excused-red lane that STOPS
    # running is exactly the blindness this workflow exists to catch.
    if [ "$age_h" -gt "$hours" ]; then
      emit "❌ $wf — newest completed run is ${age_h}h old (window ${hours}h). The schedule has stopped reporting: a SILENT blind spot. $url"
      failures=$((failures + 1))
      continue
    fi

    # Condition 3: did it conclude successfully?
    if [ "$concl" = "success" ]; then
      case "$excused" in
        *" $wf "*)
          emit "❌ $wf — listed in KNOWN_RED but its newest run (${age_h}h ago) SUCCEEDED. The ledger is STALE; delete the entry. $url"
          failures=$((failures + 1))
          ;;
        *) emit "✅ $wf — ${age_h}h old (window ${hours}h), concluded success." ;;
      esac
      continue
    fi
    case "$excused" in
      *" $wf "*)
        emit "⚠️  $wf — ${age_h}h old, concluded '$concl'. EXCUSED by the KNOWN_RED ledger. $url"
        ;;
      *)
        emit "❌ $wf — newest completed run (${age_h}h ago) concluded '$concl', not success. $url"
        failures=$((failures + 1))
        ;;
    esac
  done <<< "$registry"

  days_since_last_tag

  emit ""
  emit "Scope: this watches that THE WATCHING RUNS. It is NOT evidence the system is alive — a lane can be fresh, green, and asserting nothing. The deadman's OWN liveness is out of scope (WP-36)."

  if [ "$failures" -gt 0 ]; then
    echo "::error::DEAD-MAN'S-SWITCH: $failures lane(s) failed — see the annotations above."
    exit 1
  fi
  emit "OK — every registered lane exists, is inside its window, and concluded as expected."
}

case "${1:-}" in
  lanes) lanes ;;
  known-red) known_red ;;
  check) check ;;
  watch) check; watch ;;
  *) echo "usage: $0 {lanes|known-red|check|watch}" >&2; exit 2 ;;
esac
