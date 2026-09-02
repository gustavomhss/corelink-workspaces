#!/usr/bin/env bash
# Run one detached cargo-fuzz target and prove its persistent corpus reaches
# more libFuzzer edge coverage than the tracked seed by itself.
set -euo pipefail

target="${1:-}"
corpus_restored="${2:-}"
case "$target" in
  fuzz_manifest_from_bytes|fuzz_validate_rel_path|fuzz_validate_symlink_target_contained|fuzz_digest_from_hex|fuzz_ref_record_from_bytes|fuzz_validate_tenant)
    ;;
  *)
    echo "usage: $0 <known-fuzz-target> <true|false>" >&2
    exit 2
    ;;
esac
case "$corpus_restored" in
  true|false)
    ;;
  *)
    echo "usage: $0 <known-fuzz-target> <true|false>" >&2
    exit 2
    ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
seed="fuzz/corpus/${target}/seed"
measurement_runs=0
fuzz_runs=1000000
test -s "$seed" || {
  echo "missing tracked seed corpus: ${seed}" >&2
  exit 1
}

scratch="$(mktemp -d "${TMPDIR:-/tmp}/clw-fuzz-cov.XXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/cold"
cp "$seed" "$scratch/cold/seed"

if command -v sha256sum >/dev/null 2>&1; then
  lock_before="$(sha256sum fuzz/Cargo.lock | awk '{print $1}')"
else
  lock_before="$(shasum -a 256 fuzz/Cargo.lock | awk '{print $1}')"
fi

run_fuzz() {
  local corpus="$1"
  local runs="$2"
  local log="$3"
  set +e
  cargo +nightly fuzz run "$target" "$corpus" -- \
    -runs="$runs" -seed=1 -rss_limit_mb=4096 2>&1 | tee "$log"
  local status="${PIPESTATUS[0]}"
  set -e
  return "$status"
}

max_cov() {
  awk '
    match($0, /cov: [0-9]+/) {
      value = substr($0, RSTART + 5, RLENGTH - 5) + 0
      if (value > maximum) maximum = value
      found = 1
    }
    END { if (!found) exit 1; print maximum }
  ' "$1"
}

fuzz_exit=0
proof_exit=0
cold_cov=""
restored_cov=""
run_fuzz "$scratch/cold" "$measurement_runs" "$scratch/cold.log" || fuzz_exit=$?
if [ "$fuzz_exit" -eq 0 ] && [ "$corpus_restored" = true ]; then
  # -runs=0 evaluates every existing input but generates no mutations. Using
  # the same budget for both corpora makes this a proof about restored inputs,
  # not a trivial comparison between different amounts of fuzzing.
  run_fuzz "fuzz/corpus/${target}" "$measurement_runs" "$scratch/restored.log" || fuzz_exit=$?
fi
if [ "$fuzz_exit" -eq 0 ] && [ "$corpus_restored" = true ]; then
  if ! cold_cov="$(max_cov "$scratch/cold.log")"; then
    echo "::error::libFuzzer emitted no cov: count for the cold ${target} corpus" >&2
    proof_exit=1
  elif ! restored_cov="$(max_cov "$scratch/restored.log")"; then
    echo "::error::libFuzzer emitted no cov: count for the restored ${target} corpus" >&2
    proof_exit=1
  elif [ "$restored_cov" -le "$cold_cov" ]; then
    echo "::error::${target} restored corpus adds no coverage: cold=${cold_cov}, restored=${restored_cov}" >&2
    proof_exit=1
  fi
fi
if [ "$fuzz_exit" -eq 0 ] && [ "$proof_exit" -eq 0 ]; then
  # Grow both a bootstrapped and a restored corpus after measuring it. A fresh
  # immutable cache key saves successful discoveries for the next scheduled run.
  run_fuzz "fuzz/corpus/${target}" "$fuzz_runs" "$scratch/fuzz.log" || fuzz_exit=$?
fi

if command -v sha256sum >/dev/null 2>&1; then
  lock_after="$(sha256sum fuzz/Cargo.lock | awk '{print $1}')"
else
  lock_after="$(shasum -a 256 fuzz/Cargo.lock | awk '{print $1}')"
fi
if [ "$lock_before" != "$lock_after" ]; then
  echo "::error::cargo-fuzz changed fuzz/Cargo.lock despite the locked metadata preflight" >&2
  git diff -- fuzz/Cargo.lock >&2 || true
  exit 1
fi
if [ "$fuzz_exit" -ne 0 ]; then
  exit "$fuzz_exit"
fi
if [ "$proof_exit" -ne 0 ]; then
  exit "$proof_exit"
fi

if [ "$corpus_restored" = true ]; then
  echo "${target} restored-corpus proof GREEN: cold=${cold_cov}, restored=${restored_cov} (equal measurement budget)"
else
  echo "${target} corpus cache MISS: bootstrapped and fuzzed from the tracked seed; the next restored run must prove coverage growth"
fi
