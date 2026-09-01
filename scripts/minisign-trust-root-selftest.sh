#!/usr/bin/env bash
# minisign-trust-root-selftest.sh — offline teeth for the installer/docs key pin.
#
# Each negative control mutates a copy of the real docs/minisign.pub or install.sh and
# invokes the production guard against that copy. This is intentionally not a second
# implementation of the parser: a guard that is replaced with `return 0` makes these
# cases red-on-revert immediately.
set -euo pipefail
cd "$(dirname "$0")/.."

GUARD="$PWD/scripts/check-minisign-trust-root.sh"
GATES="$PWD/scripts/gates.sh"
TMPROOT="$(mktemp -d "${TMPDIR:-/tmp}/clw-minisign-trust-root.XXXXXX")"
trap 'rm -rf "$TMPROOT"' EXIT

pass=0
fail=0

ok() { printf '  \033[32mok\033[0m    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$1" >&2; fail=$((fail + 1)); }

fixture() {
  local name="$1" root
  root="$TMPROOT/$name"
  mkdir -p "$root/docs" "$root/scripts"
  cp docs/minisign.pub "$root/docs/minisign.pub"
  cp scripts/install.sh "$root/scripts/install.sh"
  printf '%s\n' "$root"
}

assert_allows() {
  local name="$1" root="$2" out rc=0
  out="$(bash "$GUARD" "$root" 2>&1)" || rc=$?
  if [ "$rc" -eq 0 ] && grep -qF 'minisign trust-root guard: clean' <<<"$out"; then
    ok "$name"
  else
    bad "$name — expected green, got exit $rc: $out"
  fi
}

assert_refuses() {
  local name="$1" needle="$2" root="$3" out rc=0
  out="$(bash "$GUARD" "$root" 2>&1)" || rc=$?
  if [ "$rc" -ne 0 ] && grep -qF -- "$needle" <<<"$out"; then
    ok "$name"
  else
    bad "$name — expected non-zero and '$needle', got exit $rc: $out"
  fi
}

assert_gate_wiring() {
  local guard_count guard_line classifier_line
  guard_count="$(grep -cxF 'bash scripts/check-minisign-trust-root.sh' "$GATES" || true)"
  guard_line="$(grep -nF 'bash scripts/check-minisign-trust-root.sh' "$GATES" | cut -d: -f1 || true)"
  classifier_line="$(grep -nF '. scripts/classify-lane.sh' "$GATES" | cut -d: -f1 || true)"
  if [ "$guard_count" -eq 1 ] && [[ "$guard_line" =~ ^[0-9]+$ ]] \
    && [[ "$classifier_line" =~ ^[0-9]+$ ]] && [ "$guard_line" -lt "$classifier_line" ]; then
    ok 'gates runs the guard exactly once before classification'
  else
    bad "guard wiring — expected one call before classify-lane.sh, got count=$guard_count guard=$guard_line classifier=$classifier_line"
  fi
}

echo 'minisign trust-root self-test'
echo '==============================='

root="$(fixture baseline)"
assert_allows 'baseline: real files byte-match' "$root"
assert_gate_wiring

root="$(fixture missing-pubkey)"
rm "$root/docs/minisign.pub"
assert_refuses 'missing docs/minisign.pub fails closed' 'docs/minisign.pub is absent' "$root"

root="$(fixture missing-installer-key)"
sed '/^CLW_MINISIGN_PUBKEY_LINE=/d' "$root/scripts/install.sh" > "$root/scripts/install.sh.tmp"
mv "$root/scripts/install.sh.tmp" "$root/scripts/install.sh"
assert_refuses 'missing installer key fails closed' 'CLW_MINISIGN_PUBKEY_LINE is absent' "$root"

root="$(fixture duplicate-installer-key)"
printf '%s\n' 'CLW_MINISIGN_PUBKEY_LINE="RWRtOQ5KtbhXSxt24bM/T+ndWfjUDUm8c7gr8194pxnyWEUS/CTDIkm0"' >> "$root/scripts/install.sh"
assert_refuses 'duplicate installer key fails closed' 'appears 2 times' "$root"

root="$(fixture duplicate-published-key)"
sed -n '2p' "$root/docs/minisign.pub" > "$root/duplicate-key-line"
sed -n '1p' "$root/duplicate-key-line" >> "$root/docs/minisign.pub"
assert_refuses 'duplicate published key fails closed' 'expected exactly two newline-terminated lines' "$root"

root="$(fixture malformed-pubkey)"
sed -n '1p' "$root/docs/minisign.pub" > "$root/docs/minisign.pub.tmp"
printf '%s\n' 'not-a-minisign-key' >> "$root/docs/minisign.pub.tmp"
mv "$root/docs/minisign.pub.tmp" "$root/docs/minisign.pub"
assert_refuses 'malformed published key fails closed' 'line 2 is not a 56-character base64' "$root"

root="$(fixture malformed-installer-key)"
sed 's/^CLW_MINISIGN_PUBKEY_LINE=.*/CLW_MINISIGN_PUBKEY_LINE=not-quoted/' \
  "$root/scripts/install.sh" > "$root/scripts/install.sh.tmp"
mv "$root/scripts/install.sh.tmp" "$root/scripts/install.sh"
assert_refuses 'malformed installer key fails closed' 'in scripts/install.sh is malformed' "$root"

root="$(fixture mismatch)"
sed '2s/.$/A/' "$root/docs/minisign.pub" > "$root/docs/minisign.pub.tmp"
mv "$root/docs/minisign.pub.tmp" "$root/docs/minisign.pub"
assert_refuses 'different valid key bytes fail closed' 'does not byte-match' "$root"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || { echo 'MINISIGN TRUST-ROOT SELFTEST FAILED' >&2; exit 1; }
echo 'minisign trust-root self-test: all negative controls refused.'
