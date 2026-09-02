#!/usr/bin/env bash
# scoped-package-set-selftest.sh — offline, anti-vacuity tests for the real expander.
set -euo pipefail
cd "$(dirname "$0")/.."

# shellcheck source=scripts/scoped-package-set.sh
. scripts/scoped-package-set.sh

tmp="$(mktemp -d "${TMPDIR:-/tmp}/clw-scoped-package-set.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"

pass=0; fail=0
ok() { printf '  \033[32mok\033[0m    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$1" >&2; fail=$((fail + 1)); }

# Capture a real locked metadata document once.  Each case then substitutes only the
# cargo executable, so it exercises the actual function and jq program without a Rust
# build or network access.
cargo metadata --format-version 1 --all-features --locked > "$tmp/good.json"

cat > "$tmp/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "$*" = 'metadata --format-version 1 --all-features --locked' ] || {
  echo "unexpected cargo invocation: $*" >&2
  exit 72
}
printf '%s\n' "$*" > "$CLW_SCOPED_CARGO_ARGS"
if [ "${CLW_SCOPED_CARGO_FAIL:-0}" = 1 ]; then exit 73; fi
cat "$CLW_SCOPED_METADATA"
EOF
chmod +x "$tmp/bin/cargo"

run_good() {
  CLW_SCOPED_METADATA="$tmp/good.json" CLW_SCOPED_CARGO_ARGS="$tmp/cargo-args" \
    PATH="$tmp/bin:$PATH" scoped_package_set "$1"
}

# jq 1.6 requires parentheses around a reduce expression before binding its result
# with `as`. The happy-path invocation below parses the real production filter with
# the jq selected by PATH; retain this source-shape assertion so newer jq versions
# cannot mask a regression back to the jq-1.6-incompatible bare form.
# shellcheck disable=SC2016 # The grep patterns intentionally contain literal jq variables.
if grep -qFx '    | (reduce range(0; ($members | length)) as $_ ($changed_ids | unique;' \
  scripts/scoped-package-set.sh \
  && grep -qFx '       )) as $closure' scripts/scoped-package-set.sh; then
  ok 'reverse closure binding remains jq-1.6-compatible'
else
  bad 'reverse closure binding is not jq-1.6-compatible'
fi

expected='clw-cli
clw-conformance
clw-hydrate
clw-integration-tests
clw-manifest
clw-run
clw-snapshot'
if got="$(run_good clw-manifest)" && [ "$got" = "$expected" ]; then
  ok 'clw-manifest includes its exact reverse workspace closure and historical sinks'
else
  bad "clw-manifest set mismatch: ${got:-expander failed}"
fi

if [ "$(<"$tmp/cargo-args")" = 'metadata --format-version 1 --all-features --locked' ]; then
  ok 'metadata uses --format-version 1 --all-features --locked'
else
  bad 'metadata flags drifted'
fi

if got="$(run_good 'clw-manifest clw-manifest')" && [ "$got" = "$expected" ]; then
  ok 'duplicate changed directories remain de-duplicated'
else
  bad 'duplicate changed directories changed the selected package set'
fi

refuse() { # NAME SCOPE METADATA [CARGO_FAIL]
  local name="$1" scope="$2" metadata="$3" cargo_fail="${4:-0}" out rc=0
  if out="$(CLW_SCOPED_METADATA="$metadata" CLW_SCOPED_CARGO_ARGS="$tmp/cargo-args" \
      CLW_SCOPED_CARGO_FAIL="$cargo_fail" PATH="$tmp/bin:$PATH" scoped_package_set "$scope" 2>&1)"; then
    bad "$name accepted: $out"
  else
    rc=$?
    [ "$rc" -ne 0 ] && ok "$name fails closed"
  fi
}

printf '{not JSON\n' > "$tmp/not-json.json"
refuse 'malformed metadata JSON' clw-manifest "$tmp/not-json.json"
refuse 'missing crate-to-workspace mapping' clw-not-a-member "$tmp/good.json"

jq 'del(.resolve)' "$tmp/good.json" > "$tmp/no-resolve.json"
refuse 'missing resolve graph' clw-manifest "$tmp/no-resolve.json"

# Cargo exposes the direct edge IDs in both `dependencies` and `deps[].pkg`.
# Reject disagreement instead of accepting a silently truncated reverse graph.
jq '(.packages[] | select(.name == "clw-conformance") | .id) as $id
    | (.resolve.nodes[] | select(.id == $id) | .dependencies) = []' \
  "$tmp/good.json" > "$tmp/inconsistent-edges.json"
refuse 'inconsistent resolve edge representations' clw-types "$tmp/inconsistent-edges.json"

# Agreement between Cargo's two resolved-edge views is not enough if both were
# truncated. Cross-check them against the workspace path dependencies declared
# by the package metadata as a third, independently shaped representation.
jq '(.packages[] | select(.name == "clw-conformance") | .id) as $id
    | (.resolve.nodes[] | select(.id == $id) | .dependencies) = []
    | (.resolve.nodes[] | select(.id == $id) | .deps) = []' \
  "$tmp/good.json" > "$tmp/consistently-truncated-edges.json"
refuse 'consistently truncated resolved workspace edges' clw-types \
  "$tmp/consistently-truncated-edges.json"

# Give a second workspace member the same directory as clw-manifest.  A name-only
# implementation would miss this ambiguity; the expander must reject it.
jq '(.packages[] | select(.name == "clw-cli") | .manifest_path) |= sub("clw-cli/Cargo.toml$"; "clw-manifest/Cargo.toml")' \
  "$tmp/good.json" > "$tmp/ambiguous-map.json"
refuse 'ambiguous crate-directory mapping' clw-manifest "$tmp/ambiguous-map.json"
refuse 'cargo metadata command failure' clw-manifest "$tmp/good.json" 1

mkdir -p "$tmp/no-jq-bin"
cp "$tmp/bin/cargo" "$tmp/no-jq-bin/cargo"
cat > "$tmp/no-jq-bin/jq" <<'EOF'
#!/usr/bin/env bash
exit 74
EOF
chmod +x "$tmp/no-jq-bin/jq"
if CLW_SCOPED_METADATA="$tmp/good.json" CLW_SCOPED_CARGO_ARGS="$tmp/cargo-args" \
  PATH="$tmp/no-jq-bin:$PATH" scoped_package_set clw-manifest >/dev/null 2>&1; then
  bad 'jq failure was accepted'
else
  ok 'jq failure fails closed'
fi

# Anti-vacuity: this case proves the test is not merely checking that the changed
# package and two old sinks are present.  clw-conformance is a reverse dependent of
# clw-manifest only through the workspace graph, so deleting reverse traversal makes
# the exact-set assertion above fail.
if printf '%s\n' "$expected" | grep -qx 'clw-conformance'; then
  ok 'fixture requires a non-sink transitive reverse dependent'
else
  bad 'anti-vacuity fixture lost its transitive reverse dependent'
fi

# The real metadata document needs the registry graph. It must never move above
# the Markdown-only fast exit and turn a docs PR into a network/toolchain lane.
docs_exit_count="$(grep -cFx '      exit 0 ;;' scripts/gates.sh || true)"
docs_exit_line="$(grep -nFx '      exit 0 ;;' scripts/gates.sh | cut -d: -f1)"
selftest_count="$(grep -cFx 'bash scripts/scoped-package-set-selftest.sh' scripts/gates.sh || true)"
selftest_line="$(grep -nFx 'bash scripts/scoped-package-set-selftest.sh' scripts/gates.sh | cut -d: -f1)"
if [ "$docs_exit_count" -eq 1 ] && [ "$selftest_count" -eq 1 ] \
  && [ -n "$docs_exit_line" ] && [ -n "$selftest_line" ] \
  && [ "$selftest_line" -gt "$docs_exit_line" ]; then
  ok 'real metadata selftest stays after the docs-only fast exit'
else
  bad 'real metadata selftest moved into the docs-only lane'
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
echo 'scoped-package-set selftest: reverse closure and fail-closed behavior verified.'
