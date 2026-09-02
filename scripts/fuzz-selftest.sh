#!/usr/bin/env bash
# Structural tripwires for the detached fuzz workspace. This runs without
# nightly or compiling the workspace, so CI catches a target/corpus drift before
# spending a deterministic libFuzzer run slot.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

targets=(
  fuzz_manifest_from_bytes
  fuzz_validate_rel_path
  fuzz_validate_symlink_target_contained
  fuzz_digest_from_hex
  fuzz_ref_record_from_bytes
  fuzz_validate_tenant
)

actual_targets="$(awk '
  /^\[\[bin\]\]$/ { in_bin = 1; next }
  in_bin && /^name = / {
    gsub(/^name = "|"$/, "")
    print
    in_bin = 0
  }
' fuzz/Cargo.toml | sort)"
expected_targets="$(printf '%s\n' "${targets[@]}" | sort)"
if [ "$actual_targets" != "$expected_targets" ]; then
  echo "fuzz target manifest inventory drifted" >&2
  diff -u <(printf '%s\n' "$expected_targets") <(printf '%s\n' "$actual_targets") >&2 || true
  exit 1
fi

workflow_targets="$(
  sed -n '/^[[:space:]]*matrix:/,/^[[:space:]]*steps:/p' .github/workflows/fuzz.yml |
    sed -n 's/^[[:space:]]*- \(fuzz_[a-z0-9_]*\)$/\1/p' |
    sort
)"
if [ "$workflow_targets" != "$expected_targets" ]; then
  echo "fuzz workflow matrix inventory drifted" >&2
  diff -u <(printf '%s\n' "$expected_targets") <(printf '%s\n' "$workflow_targets") >&2 || true
  exit 1
fi

for target in "${targets[@]}"; do
  test -f "fuzz/fuzz_targets/${target}.rs" || {
    echo "missing fuzz source for ${target}" >&2
    exit 1
  }
  test -s "fuzz/corpus/${target}/seed" || {
    echo "missing or empty seed corpus for ${target}" >&2
    exit 1
  }
  git ls-files --error-unmatch "fuzz/corpus/${target}/seed" >/dev/null
done

test -x scripts/fuzz-coverage-check.sh || {
  echo "missing executable fuzz coverage-growth check" >&2
  exit 1
}
bash scripts/fuzz-workflow-selftest.sh
bash scripts/fuzz-coverage-check-selftest.sh

# The old strict validator is intentionally not a target: production invokes
# the containment-aware version after a link path has passed validate_rel_path.
if rg -q 'validate_symlink_target\(' fuzz/fuzz_targets; then
  echo "a fuzz target still calls dead validate_symlink_target" >&2
  exit 1
fi

# These checks pair each new direct target call with a non-test production call,
# so a renamed/dead API cannot leave a green but vacuous fuzz target behind.
rg -Fq 'validate_symlink_target_contained(link_rel, target)' \
  fuzz/fuzz_targets/fuzz_validate_symlink_target_contained.rs
rg -Fq 'validate_symlink_target_contained(path, target)?' crates/clw-types/src/lib.rs
rg -Fq 'RefRecord::from_bytes(data)' fuzz/fuzz_targets/fuzz_ref_record_from_bytes.rs
rg -Fq 'RefRecord::from_bytes(&ref_bytes)?' crates/clw-hydrate/src/lib.rs
rg -Fq 'validate_tenant(tenant)' fuzz/fuzz_targets/fuzz_validate_tenant.rs
rg -Fq 'validate_tenant(&tenant)?' crates/clw-cli/src/config.rs

echo "fuzz selftest GREEN: ${#targets[@]} live targets with tracked seed corpora"
