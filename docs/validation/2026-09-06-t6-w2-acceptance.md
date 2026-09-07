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

The final correction commit and its tree are recorded by the commit containing
this file; verify them with `git rev-parse HEAD` and `git rev-parse HEAD^{tree}`.

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
current test name contains that filter. Running the current `clw-run` suite,
including `tests::miss_executes_and_stores_then_hit_replays`, aborts with a
stack overflow in the local test process before producing a test result. This
is recorded as an unresolved local test-environment limitation; it is not
reported as a pass.

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
