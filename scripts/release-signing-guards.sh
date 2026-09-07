#!/usr/bin/env bash
# Offline-testable fail-closed guards for the release workflow.
set -euo pipefail

usage() {
  echo "usage: $0 secrets <real|prerelease> NAME..." >&2
  echo "       $0 assets <real|prerelease> RELEASE_DIR PUBKEY VERSION" >&2
  echo "       $0 checksums <real|prerelease> RELEASE_DIR PUBKEY" >&2
  echo "       $0 action-pins WORKFLOW" >&2
  exit 2
}

[[ $# -ge 1 ]] || usage
command_name="$1"
if [[ "$command_name" == action-pins ]]; then
  [[ $# -eq 2 ]] || usage
  mode=real
else
  [[ $# -ge 2 ]] || usage
  mode="$2"
  [[ "$mode" == real || "$mode" == prerelease ]] || usage
fi

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    echo "::error::need sha256sum or shasum -a 256 to verify release proof" >&2
    return 1
  fi
}

check_secrets() {
  local missing=() name
  for name in "${@:3}"; do
    [[ -n "${!name:-}" ]] || missing+=("$name")
  done
  if ((${#missing[@]} == 0)); then
    echo "release secrets: all required values are present"
    return 0
  fi
  if [[ "$mode" == prerelease ]]; then
    echo "::warning::release secrets absent on prerelease; dry-run skip: ${missing[*]}"
    return 0
  fi
  echo "::error::release secrets absent on a real tag; refusing release: ${missing[*]}" >&2
  return 1
}

check_json_proof() {
  local proof="$1" expected_asset="$2" expected_status="$3" bound_file="$4" digest_field="$5"
  local expected_digest
  expected_digest="$(sha256_file "$bound_file")"
  python3 - "$proof" "$expected_asset" "$expected_status" "$digest_field" "$expected_digest" "${WINDOWS_EXPECTED_SIGNER_SUBJECT:-}" "${WINDOWS_EXPECTED_SIGNER_ISSUER:-}" <<'PY'
import json
import sys

path, expected_asset, expected_status, digest_field, expected_digest, expected_signer, expected_issuer = sys.argv[1:]
try:
    with open(path, encoding="utf-8") as handle:
        value = json.load(handle)
except (OSError, ValueError) as exc:
    raise SystemExit(f"invalid release signature proof {path}: {exc}")
if value.get("asset") != expected_asset:
    raise SystemExit(f"signature proof asset mismatch in {path}")
if value.get("status") != expected_status:
    raise SystemExit(f"signature proof status mismatch in {path}")
if value.get(digest_field, "").lower() != expected_digest:
    raise SystemExit(f"signature proof digest mismatch in {path}")
if expected_status == "Accepted":
    if value.get("codesign_verified") is not True:
        raise SystemExit(f"macOS proof is not codesign-verified in {path}")
else:
    if value.get("chain_status") != "Valid" or value.get("signer_subject") == "" or value.get("signer_issuer") == "":
        raise SystemExit(f"Windows proof is not chain-verified in {path}")
    if expected_signer and value.get("signer_subject") != expected_signer:
        raise SystemExit(f"Windows signer subject mismatch in {path}")
    if expected_issuer and value.get("signer_issuer") != expected_issuer:
        raise SystemExit(f"Windows signer issuer mismatch in {path}")
PY
}

check_assets() {
  local release_dir="$3" pubkey="$4" version="$5"
  if [[ "$mode" == prerelease ]]; then
    echo "::warning::asset signature gate skipped for prerelease dry-run"
    return 0
  fi
  [[ -d "$release_dir" ]] || { echo "::error::release directory missing: $release_dir" >&2; return 1; }
  [[ -s "$pubkey" ]] || { echo "::error::minisign trust root missing: $pubkey" >&2; return 1; }
  command -v minisign >/dev/null 2>&1 || { echo "::error::minisign is required for the asset signature gate" >&2; return 1; }

  local linux_x86="clw-${version}-x86_64-unknown-linux-gnu"
  local linux_arm="clw-${version}-aarch64-unknown-linux-gnu"
  local mac_x86="clw-${version}-x86_64-apple-darwin"
  local mac_arm="clw-${version}-aarch64-apple-darwin"
  local windows="clw-${version}-x86_64-pc-windows-gnu.exe"
  local asset
  for asset in "$linux_x86" "$linux_arm" "$mac_x86" "$mac_arm" "$windows"; do
    [[ -s "$release_dir/$asset" ]] || { echo "::error::required release asset missing: $asset" >&2; return 1; }
  done

  for asset in "$linux_x86" "$linux_arm"; do
    [[ -s "$release_dir/$asset.minisig" ]] || { echo "::error::unsigned Linux asset: $asset" >&2; return 1; }
    minisign -V -p "$pubkey" -m "$release_dir/$asset" >/dev/null
  done
  for asset in "$mac_x86" "$mac_arm"; do
    [[ -s "$release_dir/$asset.zip" ]] || { echo "::error::notarized macOS archive missing: $asset" >&2; return 1; }
    check_json_proof "$release_dir/$asset.notarization.json" "$asset" Accepted "$release_dir/$asset.zip" zip_sha256
  done
  [[ -n "${WINDOWS_EXPECTED_SIGNER_SUBJECT:-}" ]] || { echo "::error::expected Windows signer subject is not configured" >&2; return 1; }
  [[ -n "${WINDOWS_EXPECTED_SIGNER_ISSUER:-}" ]] || { echo "::error::expected Windows signer issuer is not configured" >&2; return 1; }
  check_json_proof "$release_dir/$windows.authenticode.json" "$windows" Valid "$release_dir/$windows" sha256
  echo "asset signature gate: five target assets have verified signature/attestation evidence"
}

check_checksums() {
  local release_dir="$3" pubkey="$4"
  if [[ "$mode" == prerelease ]]; then
    echo "::warning::checksum signature gate skipped for prerelease dry-run"
    return 0
  fi
  [[ -s "$release_dir/SHA256SUMS" ]] || { echo "::error::SHA256SUMS missing" >&2; return 1; }
  [[ -s "$release_dir/SHA256SUMS.minisig" ]] || { echo "::error::SHA256SUMS is unsigned" >&2; return 1; }
  minisign -V -p "$pubkey" -m "$release_dir/SHA256SUMS" >/dev/null
  echo "checksum signature gate: SHA256SUMS verified"
}

check_action_pins() {
  local workflow="$2"
  python3 - "$workflow" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8").splitlines()
approved = {
    "actions/checkout": "11d5960a326750d5838078e36cf38b85af677262",
    "taiki-e/install-action": "e67fa11c4b9316fa714ddf0abed07a0c3143b95b",
    "actions/upload-artifact": "ea165f8d65b6e75b540449e92b4886f43607fa02",
    "actions/download-artifact": "d3f86a106a0bac45b974a628896c90dbdf5c8093",
    "actions/attest-build-provenance": "ef244123eb79f2f7a7e75d99086184180e6d0018",
}
uses = []
for number, line in enumerate(text, 1):
    match = re.match(r"^\s*(?:-\s+)?uses:\s*([^\s#]+)", line)
    if match:
        uses.append((number, match.group(1)))
if len(uses) != 14:
    raise SystemExit(f"release action inventory changed: expected 14 uses, found {len(uses)}")
for number, ref in uses:
    action, sep, commit = ref.rpartition("@")
    if not sep or action not in approved or commit != approved[action]:
        raise SystemExit(f"unapproved or mutable release action at line {number}: {ref}")
print(f"release action pins: clean ({len(uses)} approved immutable refs)")
PY
}

case "$command_name" in
  secrets) check_secrets "$@" ;;
  assets) check_assets "$@" ;;
  checksums) check_checksums "$@" ;;
  action-pins) check_action_pins "$@" ;;
  *) usage ;;
esac
