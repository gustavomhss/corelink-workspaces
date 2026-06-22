#!/usr/bin/env bash
# family-e2e.sh — drive the live family-e2e suite across the 6 tier tenants.
#
# clw-side driver for milestone M10 in docs/family-e2e-STATUS.md. Given a creds
# file holding the live endpoint + per-tier (tenant, PAT) — the exact shape
# requested from the server TL in docs/RELAY-server-TL-family-e2e-D1-CHASE-*.md —
# it runs the live conformance + solo-lifecycle suite (crate clw-conformance,
# `--features live`) once PER tier tenant, in REQUIRE mode (skips become hard
# failures), and prints a single PASS/FAIL/SKIP table.
#
# It NEVER fabricates coverage: a tier absent from the creds file is reported
# SKIP, not PASS. The live suites only ever touch the sandbox tenant + the
# unique `ws-<suffix>` names they create (best-effort self-cleanup; never a
# tenant-wide erase/prune).
#
# ---------------------------------------------------------------------------
# Creds file format (comments with '#', blank lines ignored):
#
#     ENDPOINT=https://corelink-api.humangr.com
#     # tier        tenant_id    pat
#     free          <tenant>     <pat>
#     solo          <tenant>     <pat>
#     starter       <tenant>     <pat>
#     pro           <tenant>     <pat>
#     max           <tenant>     <pat>
#     enterprise    <tenant>     <pat>
#
# The PAT is read ONLY from this file and exported into the child `cargo test`
# env; it is never echoed. Name the file `*.creds` (e.g. `family-e2e.creds`) —
# the repo .gitignore ignores `*.creds` so it can never be committed by accident.
#
# Usage:
#   scripts/family-e2e.sh <creds-file>            # run the live matrix
#   scripts/family-e2e.sh <creds-file> --dry-run  # parse + print the plan only
#
# Toolchain note (self-hosted Mac): if `cargo` is missing because the rustup
# proxy was nuked, put the pinned toolchain on PATH first, e.g.
#   export PATH="$HOME/.rustup/toolchains/1.91.1-x86_64-apple-darwin/bin:$PATH"
# ---------------------------------------------------------------------------
set -euo pipefail

TIERS=(free solo starter pro max enterprise)

die() { echo "error: $*" >&2; exit 2; }

[ $# -ge 1 ] || die "usage: $0 <creds-file> [--dry-run]"
CREDS="$1"; shift || true
DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1
[ -f "$CREDS" ] || die "creds file not found: $CREDS"

# --- parse ----------------------------------------------------------------
ENDPOINT=""
declare -a HAVE_TIER HAVE_TENANT HAVE_PAT
while IFS= read -r line || [ -n "$line" ]; do
  # strip a trailing comment + surrounding whitespace; skip blanks/comments.
  # `tr -d '\r'` first so a Windows-authored creds file (CRLF) can't leave a
  # trailing CR on the PAT field (BSD sed's [[:space:]] doesn't reliably match CR).
  line="$(echo "$line" | tr -d '\r')"
  line="${line%%#*}"
  line="$(echo "$line" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
  [ -z "$line" ] && continue
  if [[ "$line" == ENDPOINT=* ]]; then
    ENDPOINT="${line#ENDPOINT=}"
    continue
  fi
  # data line: <tier> <tenant> <pat>
  read -r f1 f2 f3 _rest <<<"$line"
  ok=0; for t in "${TIERS[@]}"; do [ "$f1" = "$t" ] && ok=1 && break; done
  [ "$ok" = 1 ] || { echo "warn: ignoring unrecognized line (tier '$f1')" >&2; continue; }
  [ -n "$f2" ] && [ -n "$f3" ] || die "tier '$f1' line missing tenant or pat"
  HAVE_TIER+=("$f1"); HAVE_TENANT+=("$f2"); HAVE_PAT+=("$f3")
done < "$CREDS"

[ -n "$ENDPOINT" ] || die "no ENDPOINT= line in $CREDS"
[ "${#HAVE_TIER[@]}" -ge 1 ] || die "no tier lines parsed from $CREDS"

echo "family-e2e driver — endpoint: $ENDPOINT"
echo "tiers provided: ${HAVE_TIER[*]}"
present_count=${#HAVE_TIER[@]}
missing=()
for t in "${TIERS[@]}"; do
  found=0; for h in "${HAVE_TIER[@]}"; do [ "$h" = "$t" ] && found=1 && break; done
  [ "$found" = 0 ] && missing+=("$t")
done
[ "${#missing[@]}" -gt 0 ] && echo "tiers MISSING (will report SKIP): ${missing[*]}"

if [ "$DRY_RUN" = 1 ]; then
  echo
  echo "[dry-run] would run, per provided tier:"
  echo "  CLW_ENDPOINT=$ENDPOINT CLW_TENANT=<tenant> CLW_TOKEN=<pat> \\"
  echo "    CLW_CONFORMANCE_REQUIRE=1 cargo test -p clw-conformance --features live"
  echo "[dry-run] no tests executed, no creds used."
  exit 0
fi

command -v cargo >/dev/null 2>&1 || die "cargo not on PATH (see toolchain note in this script's header)"

# --- run per tier ----------------------------------------------------------
declare -a RESULT
fail_total=0
for i in "${!HAVE_TIER[@]}"; do
  tier="${HAVE_TIER[$i]}"; tenant="${HAVE_TENANT[$i]}"; pat="${HAVE_PAT[$i]}"
  echo
  echo "=== tier: $tier (tenant: $tenant) ==="
  if CLW_ENDPOINT="$ENDPOINT" CLW_TENANT="$tenant" CLW_TOKEN="$pat" \
       CLW_CONFORMANCE_REQUIRE=1 \
       cargo test -p clw-conformance --features live -- --test-threads=1; then
    RESULT+=("$tier PASS")
  else
    RESULT+=("$tier FAIL")
    fail_total=$((fail_total + 1))
  fi
done

# --- report ----------------------------------------------------------------
echo
echo "================ family-e2e live matrix ================"
for line in "${RESULT[@]}"; do printf '  %-12s %s\n' "${line% *}" "${line#* }"; done
for t in "${missing[@]}"; do printf '  %-12s %s\n' "$t" "SKIP (no creds)"; done
echo "======================================================="
echo "ran $present_count tier(s); $fail_total failed; ${#missing[@]} skipped (no creds)"
[ "$fail_total" -eq 0 ] || exit 1
