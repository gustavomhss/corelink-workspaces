#!/usr/bin/env bash
# ci-build-target-selftest.sh — prove CI target reuse stops at the job boundary.
set -euo pipefail
cd "$(dirname "$0")/.."

fixture="$(mktemp -d "${TMPDIR:-/tmp}/clw-ci-target-selftest.XXXXXX")"
trap 'rm -rf -- "$fixture"' EXIT
gates="$PWD/scripts/gates.sh"

target_for() {
  # The fixture simulates CI, independently of the caller's local shortcut.
  CI=true CLW_SKIP_CROSS=0 RUNNER_TEMP="$fixture" GITHUB_RUN_ID="$1" GITHUB_RUN_ATTEMPT="$2" \
    CARGO_TARGET_DIR="$fixture/forged-persistent-target" "$gates" --print-ci-cargo-target-dir \
    | tail -n 1
}

first="$(target_for 1001 1)"
again="$(target_for 1001 1)"
next="$(target_for 1002 1)"
expected="$fixture/clw-gates-target-1001-1"

[ "$first" = "$expected" ] || { echo "selftest failed: job target escaped RUNNER_TEMP" >&2; exit 1; }
[ "$again" = "$first" ] || { echo "selftest failed: one job cannot reuse its Cargo target" >&2; exit 1; }
[ "$next" != "$first" ] || { echo "selftest failed: distinct CI runs share a Cargo target" >&2; exit 1; }
[ -d "$first" ] || { echo "selftest failed: job-local Cargo target was not created" >&2; exit 1; }

echo "CI build-target selftest GREEN: one job reuses its target; another run gets a distinct temporary target."
