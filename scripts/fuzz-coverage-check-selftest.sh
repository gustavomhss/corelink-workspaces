#!/usr/bin/env bash
# Deterministic harness for the cold/restored comparison. It replaces cargo with
# a tiny libFuzzer-output oracle so the gate's anti-vacuity logic is testable
# without nightly, compilation, or a million real executions.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

fixture="$(mktemp -d "${TMPDIR:-/tmp}/clw-fuzz-proof-selftest.XXXXXX")"
trap 'rm -rf -- "$fixture"' EXIT

cargo() {
  local corpus="${5:-}"
  local runs="${7:-}"
  local coverage
  case "$runs" in
    -runs=0)
      if [[ "$corpus" == fuzz/corpus/* ]]; then
        coverage="${CLW_FUZZ_MOCK_RESTORED_COV:?}"
      else
        coverage=10
      fi
      ;;
    -runs=1000000)
      coverage=30
      printf 'fuzz executed\n' >"${CLW_FUZZ_MOCK_MARKER:?}"
      ;;
    *)
      echo "unexpected mocked cargo arguments: $*" >&2
      return 99
      ;;
  esac
  printf '#2 DONE cov: %s\n' "$coverage"
}
export -f cargo

target=fuzz_validate_tenant

CLW_FUZZ_MOCK_RESTORED_COV=20
CLW_FUZZ_MOCK_MARKER="$fixture/restored-growth"
export CLW_FUZZ_MOCK_RESTORED_COV CLW_FUZZ_MOCK_MARKER
growth_output="$(bash scripts/fuzz-coverage-check.sh "$target" true)"
grep -Fq 'restored-corpus proof GREEN: cold=10, restored=20 (equal measurement budget)' <<<"$growth_output"
test -s "$CLW_FUZZ_MOCK_MARKER"

CLW_FUZZ_MOCK_RESTORED_COV=10
CLW_FUZZ_MOCK_MARKER="$fixture/equal-corpus"
export CLW_FUZZ_MOCK_RESTORED_COV CLW_FUZZ_MOCK_MARKER
equal_output=""
if equal_output="$(bash scripts/fuzz-coverage-check.sh "$target" true 2>&1)"; then
  echo "equal cold/restored coverage unexpectedly passed" >&2
  exit 1
fi
grep -Fq 'restored corpus adds no coverage: cold=10, restored=10' <<<"$equal_output"
test ! -e "$CLW_FUZZ_MOCK_MARKER"

CLW_FUZZ_MOCK_RESTORED_COV=10
CLW_FUZZ_MOCK_MARKER="$fixture/bootstrap"
export CLW_FUZZ_MOCK_RESTORED_COV CLW_FUZZ_MOCK_MARKER
bootstrap_output="$(bash scripts/fuzz-coverage-check.sh "$target" false)"
grep -Fq 'corpus cache MISS: bootstrapped and fuzzed from the tracked seed' <<<"$bootstrap_output"
test -s "$CLW_FUZZ_MOCK_MARKER"

echo "fuzz coverage proof selftest GREEN: equal budgets, cache-hit proof, and cache-miss bootstrap"
