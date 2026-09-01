#!/usr/bin/env bash
# pre-merge-gate-check.sh <PR-number>
# =============================================================================
# MANDATORY pre-merge gate for corelink-workspaces. Run this BEFORE every
# `gh pr merge`. The head is sampled around the checks read; the printed merge
# command uses GitHub CLI's compare-and-swap flag to close the remaining window.
#
#   bash scripts/pre-merge-gate-check.sh <PR>
#
# Exits 0 ONLY when the PR is mergeable AND its real gates ran AND every
# non-skipping check is green. Exits 1 (and prints why) otherwise — in which
# case DO NOT MERGE.
#
# Why this exists: `pull_request` workflows build and run on the MERGE REF.
# When a PR conflicts with its base, GitHub cannot construct that ref, so it
# creates NO check runs at all for those workflows — not queued, not failed,
# ABSENT. `pull_request_target` workflows (if any) run on the BASE branch, so
# those still fire regardless. That asymmetry means `gh pr checks` on a
# conflicting PR can report "nothing failing" — which reads as green to both a
# human and a script — while none of the load-bearing gates ran at all.
#
# This is not a rare state here: PR #237 in THIS repo sat CONFLICTING with
# `gh pr checks` showing zero failures; after a rebase it produced 3 real
# checks. The exact same failure shape bit corelink-server the same day: PR
# #967 printed "✅ All gates green" while dco/gitleaks/changelog/secrets-matrix
# had never run — six `pull_request_target` metadata jobs (labels, size
# bucket, welcome comment, dependabot sentinel) all passed trivially and read
# as a green light.
#
# ⛔ Branch protection here (as in corelink-server) treats `required checks`
# loosely, so THIS SCRIPT is the last line of defense. It must never fail OPEN.
#
# Three defenses below, in order of how badly a blind merge needs them:
#   1. mergeable must be MERGEABLE (not CONFLICTING/UNKNOWN)
#   2. the always-present gates must actually appear in the check list
#   3. "no checks at all" is a FAILURE, not "nothing to gate"
# =============================================================================
set -euo pipefail

PR="${1:?usage: bash scripts/pre-merge-gate-check.sh <PR-number>}"

# ── Defense 1: mergeability ───────────────────────────────────────────────────
# GitHub computes `mergeable` asynchronously, so UNKNOWN means "ask again", not
# "fine". Both non-MERGEABLE states are refused: on a conflict the check list is
# actively misleading (see the header), and on UNKNOWN we cannot yet tell.
state="$(gh pr view "$PR" --json mergeable,mergeStateStatus,state,headRefOid \
  -q '"\(.mergeable) \(.mergeStateStatus) \(.state) \(.headRefOid)"' 2>/dev/null \
  || echo "ERROR ERROR ERROR ERROR")"
read -r mergeable mergestatus prstate head_before <<<"$state"

if [ "$prstate" != "OPEN" ]; then
  echo "  ⛔ DO NOT MERGE PR #$PR — the PR is $prstate, not OPEN."
  exit 1
fi

if [ "$mergeable" != "MERGEABLE" ]; then
  echo "  ⛔ DO NOT MERGE PR #$PR — mergeable=$mergeable ($mergestatus)."
  echo
  if [ "$mergeable" = "CONFLICTING" ]; then
    echo "     A CONFLICTING PR gets NO \`pull_request\` checks at all: those run on"
    echo "     the merge ref, which GitHub cannot build. Any green you see below is"
    echo "     \`pull_request_target\` metadata jobs only — it proves NOTHING."
    echo
    echo "     Fix:  git rebase origin/main && git push --force-with-lease"
    echo "     Then re-run this script and wait for the real checks to appear."
  else
    echo "     GitHub has not finished computing mergeability. Re-run in a moment;"
    echo "     do NOT read this as a green light."
  fi
  exit 1
fi

# ── Defense 3 (ordering: checked before the per-check loop) ───────────────────
# "No checks reported" used to `exit 0` with "nothing to gate". This repo runs
# `gates` + `dco` (ci.yml) and `llvm-cov (floor 88%)` (coverage.yml) on EVERY
# PR unconditionally — no checks means those workflows did not fire, which is a
# reason to STOP, not to merge.
json="$(gh pr checks "$PR" --json name,bucket,link 2>/dev/null || true)"
head_after="$(gh pr view "$PR" --json headRefOid -q .headRefOid 2>/dev/null || echo ERROR)"
if [ "$head_before" = "ERROR" ] || [ "$head_after" = "ERROR" ] \
  || [ "$head_before" != "$head_after" ]; then
  echo "  ⛔ DO NOT MERGE PR #$PR — head changed while checks were being read."
  echo "     before=$head_before"
  echo "     after =$head_after"
  echo "     Re-run this script against the new head and wait for its checks."
  exit 1
fi
if [ -z "$json" ] || [ "$json" = "[]" ]; then
  echo "  ⛔ DO NOT MERGE PR #$PR — the PR reports NO checks at all."
  echo "     That is not 'nothing to gate': every PR here runs gates + dco + the"
  echo "     coverage floor unconditionally, so zero checks means the workflows"
  echo "     never fired."
  exit 1
fi

GATE_JSON="$json" python3 - "$PR" <<'PY'
import os, sys, json
pr = sys.argv[1]
data = json.loads(os.environ["GATE_JSON"])
# Bucket handling is ALLOWLIST-based, not denylist-based, and that is the whole
# point. Testing only `b == "fail"` / `b == "pending"` would let every other
# bucket fall through as green — so a **cancelled** check (the shape an
# overloaded/killed runner takes) would read as a pass. A cancelled gate has not
# run; it has proven nothing.
#
# Anything that is not an explicit PASS or an explicit SKIP is now BLOCKING,
# including a bucket name GitHub has not invented yet. Unknown => blocking is
# the fail-CLOSED direction, which is the only acceptable one here.
PASS_BUCKETS = {"pass"}
SKIP_BUCKETS = {"skipping"}
order = {"fail": 0, "cancel": 0, "pending": 1, "skipping": 2, "pass": 3}
mark = {"pass": "✓", "fail": "✗", "cancel": "✗", "pending": "…", "skipping": "-"}
fails, pends = [], []
for c in sorted(data, key=lambda x: order.get(x.get("bucket"), 0)):
    b = c.get("bucket")
    print(f"  {mark.get(b,'✗')} {str(b):9} {c.get('name')}")
    if b in PASS_BUCKETS or b in SKIP_BUCKETS:
        continue
    if b == "pending":
        pends.append(c)
    else:
        # fail, cancel, or anything unrecognised.
        fails.append(c)
print()

# ── Defense 2: the real gates must be PRESENT, not merely not-failing ─────────
# Chosen by reading .github/workflows/*.yml for a bare `on: pull_request` with
# NO `paths:` filter — the only two such workflows in this repo are:
#   - ci.yml       (jobs: `gates`, `dco`)          on: pull_request  (no paths)
#   - coverage.yml (job: `llvm-cov (floor 88%)`)    on: pull_request  (no paths)
# Every other pull_request-triggered workflow (shellcheck.yml, windows.yml) is
# `paths:`-filtered, so it legitimately does not run on every PR and cannot be
# used as an always-present signal. If any of the three below is missing, the
# pull_request workflows did not fire and the rest of this list is metadata
# jobs that gate nothing. Substring match keeps this robust against job-name
# edits ("dco" -> "dco-check", "gates" -> "gates (fmt+clippy+test)").
#
# Keep this list SMALL and path-filter-free. Adding a path-filtered gate here
# would make the script fail on PRs that legitimately skip it — turning a
# fail-open into a fail-noisy, which gets the whole check disabled by the next
# person in a hurry.
REQUIRED_PRESENT = ["dco", "gates", "llvm-cov"]
names = " ".join(c.get("name", "").lower() for c in data)
missing = [g for g in REQUIRED_PRESENT if g not in names]
if missing:
    print(f"  ⛔ DO NOT MERGE PR #{pr} — the always-present gates never ran: "
          f"{', '.join(missing)}.")
    print("     ci.yml (gates, dco) and coverage.yml (llvm-cov) trigger on a bare")
    print("     `on: pull_request` with no `paths:` filter, so their ABSENCE means")
    print("     the pull_request workflows did not fire for this head sha. The")
    print("     checks listed above (if any) gate nothing.")
    print("     Usual cause: the PR conflicts, or the head sha was force-pushed")
    print("     while runs were being created. Rebase, push, and re-run.")
    sys.exit(1)

if fails or pends:
    print(f"  ⛔ DO NOT MERGE PR #{pr} — {len(fails)} not-green, {len(pends)} pending.")
    for c in fails:
        b = c.get("bucket")
        why = "" if b == "fail" else f"  [bucket={b} — not a pass; it did not run to a verdict]"
        print(f"     ✗ {c.get('name')}  {c.get('link','')}{why}")
    print("     Fix or re-run until green. Use --admin ONLY for a documented,")
    print("     non-blocking infra/flake reason you state explicitly.")
    sys.exit(1)
print(f"  ✅ All gates green — OK to merge PR #{pr}.")
PY

echo "     Approved head: $head_after"
echo "     Merge with: gh pr merge $PR --squash --delete-branch --match-head-commit $head_after"
