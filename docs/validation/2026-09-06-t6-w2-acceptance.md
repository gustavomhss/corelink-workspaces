# T6-W2 acceptance — clw 0.1.12

**Decision: local release gates pass where executable; signed publication remains
blocked by release authority.**

This record is for the PR #290 head after the version and evidence correction.
It records reproducible local results and does not claim a signature, a hosted
release run, or a published artifact.

## Source and version

The PR head before this correction was:

```text
repository: HuGR-Labs/corelink-workspaces
base_head:  6e098a9b09d6796a55ebc0d8fa7f7fcddfd605a9
```

The intended release is `0.1.12`: the PR title, this acceptance record's
scope, and the changelog release heading all identify that version. The
workspace manifest and lockfile now report `0.1.12` for all 13 `clw-*`
packages, including `clw-win-acl`.

Reproduce the version and lock checks with:

```text
cargo metadata --locked --no-deps --format-version 1
13 clw-* packages; one version: 0.1.12

rg -n -A2 '^name = "h2"' Cargo.lock
h2 0.4.17
```

The evaluated source commit is:

```text
source_commit: fe73060 (stack-pressure correction)
source_tree:   4d6de103fa247a240e202e76f88408e0aa612f1c
```

The lane results below were run against that exact source commit.

## Local release gates

The following checks passed in the isolated worktree
`/private/tmp/corelink-workspaces-pr290-local-gate`:

```text
actionlint .github/workflows/*.yml
yamllint -d relaxed .github/workflows/release.yml
shellcheck -S warning scripts/*.sh
bash -n scripts/*.sh
bash scripts/release-signing-guards-selftest.sh
bash scripts/release-guards-selftest.sh
bash scripts/minisign-trust-root-selftest.sh
bash scripts/workflow-permissions-selftest.sh
bash scripts/deadman-selftest.sh
bash scripts/check-release-notes.sh
bash scripts/check-minisign-trust-root.sh
bash scripts/workflow-permissions.sh
bash scripts/release-signing-guards.sh action-pins .github/workflows/release.yml
cargo fmt --all -- --check
git diff --check
cargo check --workspace --locked
cargo test -p clw-run --locked -- --nocapture
cargo test -p clw-cli --bin clw --locked
```

The release workflow contains exactly 14 approved immutable action pins, and
its expression braces are balanced. Default `yamllint` still reports the
repository's existing line-length/style findings; the relaxed configuration
parses the workflow successfully. This is a repository baseline issue rather
than a new versioning error.

The focused CLI test passed:

```text
cargo test -p clw-cli --bin clw run::tests --locked -- --nocapture
5 passed, 0 failed
```

The prior evidence filter `cargo test -p clw-run required_hit` is stale: no
current test name contains that filter. The first full local run exposed a
stack overflow in `tests::changed_input_yields_different_key` on the default
test-thread stack. The cause was the large `compute_key` future retaining the
streaming digest/tool-resolution state machines across nested `await`s. Commit
`fe73060` boxes those future boundaries (and the per-input file digest
boundaries), reducing stack pressure without changing the key contract.
After that correction the focused and full `clw-run` gates pass without
`RUST_MIN_STACK`:

```text
cargo test -p clw-run --locked -- --nocapture
51 passed, 0 failed

cargo test -p clw-cli --bin clw --locked
216 passed, 0 failed
```

The workspace-wide run also exercised the same code and all compiled suites;
one timing assertion in the erase benchmark was sensitive to concurrent host
load during the first run. The benchmark passed five isolated repetitions
(6.54–6.88 s each), and the complete `clw-cli` binary suite passed above. No
timing threshold or assertion was weakened.

## Earlier baseline lanes

The following diagnostics were run against the pre-fix source commit
`56eaf82060fbac7b25fe22242af153e70acb7c33`. They are retained as historical
baseline evidence and are not claims about the corrected source commit above:

```text
stack/test:
  FAIL, cargo test -p clw-run --locked -- --nocapture
  stack overflow in tests::changed_input_yields_different_key; exit 101

coverage:
  FAIL, cargo llvm-cov --workspace --exclude clw-e2e-evidence
  lines 70.27% (26,597 total); configured floor 88%; exit 42

mutation:
  PASS, cargo-mutants
  27 mutants considered; 0 missed, 0 timeouts, 0 surviving; exit 0
  Rust diff was empty, so no mutation fix was approved or composed.

build matrix:
  PASS, cargo build --release --locked -p clw-cli
  x86_64-apple-darwin: Mach-O x86_64, clw 0.1.12,
    sha256 0fd549d8b2ec4c21ac8d4cdd04a1b66c4eb2523db0be2e46e23af18b6a67b1d4
  aarch64-apple-darwin: Mach-O arm64,
    sha256 030a481dd2f0b73898d0d5345e521b31815a1a87bcf574a1528015238820e066
  The arm64 artifact was built and inspected but cannot execute on this host.
```

Coverage reports were generated in the isolated lane worktree and are not
release artifacts. The stack failure from this baseline was closed by
`fe73060`; the corrected focused and full gates are recorded above. Coverage,
mutation, and build-matrix lanes were not rerun after that small source fix.

## Evidence boundary

The earlier evidence named commits, trees, a five-target staging directory,
and live authenticated output that are not present or reproducible at this PR
head. Those claims were removed from this record. There is no committed
`SHA256SUMS`, `.minisig`, or signed release asset, and no local
`/private/tmp/clw-0.1.12-artifact` staging package was available during this
run.

No signature or publication was fabricated. A read-only release lookup found
the public mirror at `v0.1.11` and no published `v0.1.12`.

## Remote-only requirements

The following gates require owner-controlled infrastructure and were not
represented as local passes:

- Windows Authenticode and macOS signing/notarization require self-hosted
  runners, certificates, credentials, and signing services.
- Minisign signing and release publication require the private key/password
  and release tokens.
- Five-target packaging, immutable asset digests, and hosted provenance need
  the release runners and their artifact staging.
- Live authenticated conformance needs the remote service endpoint and token.
- SLSA provenance is a hosted public-repository release step.

Mutation testing, coverage, and Windows execution were not run as part of this
local release-gate correction; they require separate toolchains/workflows and
would not establish signed publication authority. No tag, merge, or publish was
attempted.
