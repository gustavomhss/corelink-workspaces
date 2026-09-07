#!/usr/bin/env bash
# Mutation tests for real-tag secret and five-asset signature gates.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUARD="$ROOT/scripts/release-signing-guards.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/clw-release-signing.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

failures=0
expect_fail() {
  local label="$1" needle="$2"
  shift 2
  local output rc=0
  if output="$("$@" 2>&1)"; then
    rc=0
  else
    rc=$?
  fi
  if [[ "$rc" -ne 0 ]] && grep -qF -- "$needle" <<<"$output"; then
    echo "  ✓ $label"
  else
    echo "  ✗ $label" >&2
    printf '%s\n' "$output" >&2
    failures=$((failures + 1))
  fi
}

expect_pass() {
  local label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "  ✓ $label"
  else
    echo "  ✗ $label" >&2
    failures=$((failures + 1))
  fi
}

echo "release signing guards: secret fail-closed cases"
expect_fail "real tag rejects absent macOS credentials" MACOS_CERT_P12_BASE64 \
  env -u MACOS_CERT_P12_BASE64 -u MACOS_CERT_PASSWORD -u MACOS_SIGN_IDENTITY \
    -u MACOS_NOTARY_KEY_BASE64 -u MACOS_NOTARY_KEY_ID -u MACOS_NOTARY_ISSUER_ID \
    bash "$GUARD" secrets real MACOS_CERT_P12_BASE64 MACOS_CERT_PASSWORD MACOS_SIGN_IDENTITY MACOS_NOTARY_KEY_BASE64 MACOS_NOTARY_KEY_ID MACOS_NOTARY_ISSUER_ID
expect_pass "RC permits explicit macOS dry-run skip" \
  env -u MACOS_CERT_P12_BASE64 -u MACOS_CERT_PASSWORD -u MACOS_SIGN_IDENTITY \
    -u MACOS_NOTARY_KEY_BASE64 -u MACOS_NOTARY_KEY_ID -u MACOS_NOTARY_ISSUER_ID \
    bash "$GUARD" secrets prerelease MACOS_CERT_P12_BASE64 MACOS_CERT_PASSWORD MACOS_SIGN_IDENTITY MACOS_NOTARY_KEY_BASE64 MACOS_NOTARY_KEY_ID MACOS_NOTARY_ISSUER_ID
expect_fail "real tag rejects absent Windows credentials" AZURE_TENANT_ID \
  env -u AZURE_TENANT_ID -u AZURE_CLIENT_ID -u AZURE_CLIENT_SECRET -u AZURE_SUBSCRIPTION_ID \
    -u AZURE_CODE_SIGNING_ACCOUNT -u AZURE_CERT_PROFILE -u AZURE_ENDPOINT \
    bash "$GUARD" secrets real AZURE_TENANT_ID AZURE_CLIENT_ID AZURE_CLIENT_SECRET AZURE_SUBSCRIPTION_ID AZURE_CODE_SIGNING_ACCOUNT AZURE_CERT_PROFILE AZURE_ENDPOINT
expect_fail "real tag rejects absent mirror token" CLW_RELEASES_TOKEN \
  env -u CLW_RELEASES_TOKEN bash "$GUARD" secrets real CLW_RELEASES_TOKEN

echo "release signing guards: five-asset signature gate"
DIST="$WORK/release"
mkdir -p "$DIST"
minisign -G -W -f -p "$WORK/minisign.pub" -s "$WORK/minisign.key" >/dev/null 2>&1
version=9.9.9
linux_x86="clw-${version}-x86_64-unknown-linux-gnu"
linux_arm="clw-${version}-aarch64-unknown-linux-gnu"
mac_x86="clw-${version}-x86_64-apple-darwin"
mac_arm="clw-${version}-aarch64-apple-darwin"
windows="clw-${version}-x86_64-pc-windows-gnu.exe"
for asset in "$linux_x86" "$linux_arm" "$mac_x86" "$mac_arm" "$windows"; do
  printf 'asset %s\n' "$asset" > "$DIST/$asset"
done
for asset in "$linux_x86" "$linux_arm"; do
  minisign -S -s "$WORK/minisign.key" -m "$DIST/$asset" >/dev/null 2>&1
done
printf 'zip\n' > "$DIST/$mac_x86.zip"
printf 'zip\n' > "$DIST/$mac_arm.zip"
printf '{"asset":"%s","status":"Accepted","codesign_verified":true}\n' "$mac_x86" > "$DIST/$mac_x86.notarization.json"
printf '{"asset":"%s","status":"Accepted","codesign_verified":true}\n' "$mac_arm" > "$DIST/$mac_arm.notarization.json"
printf '{"asset":"%s","status":"Signed","verified":true}\n' "$windows" > "$DIST/$windows.authenticode.json"
expect_pass "complete five-asset proofs pass" bash "$GUARD" assets real "$DIST" "$WORK/minisign.pub" "$version"
rm -f "$DIST/$mac_arm.notarization.json"
expect_fail "missing macOS attestation blocks publish gate" "notarization" \
  bash "$GUARD" assets real "$DIST" "$WORK/minisign.pub" "$version"
printf 'digest fixture\n' > "$DIST/SHA256SUMS"
minisign -S -s "$WORK/minisign.key" -m "$DIST/SHA256SUMS" >/dev/null 2>&1
expect_pass "signed checksum gate passes" \
  bash "$GUARD" checksums real "$DIST" "$WORK/minisign.pub"
rm -f "$DIST/SHA256SUMS.minisig"
expect_fail "missing checksum signature blocks publish" "SHA256SUMS is unsigned" \
  bash "$GUARD" checksums real "$DIST" "$WORK/minisign.pub"

echo "release signing guards: action pin mutation"
PIN_FIXTURE="$WORK/release.yml"
cp "$ROOT/.github/workflows/release.yml" "$PIN_FIXTURE"
python3 - "$PIN_FIXTURE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "actions/checkout@11d5960a326750d5838078e36cf38b85af677262"
if needle not in text:
    raise SystemExit("checkout pin fixture needle missing")
path.write_text(text.replace(needle, "actions/checkout@v4", 1), encoding="utf-8")
PY
expect_fail "mutable action ref blocks release" "unapproved or mutable" \
  bash "$GUARD" action-pins "$PIN_FIXTURE"

if ((failures)); then
  echo "release signing guards selftest: ${failures} case(s) FAILED" >&2
  exit 1
fi
echo "release signing guards selftest: clean"
