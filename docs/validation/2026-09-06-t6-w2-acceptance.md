# T6-W2 acceptance — clw 0.1.12

**Decision: BLOCKED-AUTHORITY for signed publication; local protocol ACCEPT.**

This record qualifies the local `clw` candidate and keeps the required-hit
success proof separate from the runners action's refusal path.

## Immutable source pin

The candidate was checked from:

```text
repository: HuGR-Labs/corelink-workspaces
commit:     aca24f728ca44eac26118653a631033297eec51f
tree:       5fb882026864b4d958412ccc08c65ca124f22d75
version:    0.1.12
```

`cargo metadata --locked --no-deps` reports one unique version for every
`clw-*` package: `0.1.12`. The locked dependency graph resolves `h2 v0.4.17`.
No floating `latest` or mutable artifact URL was used for the local proof.

## Focused checks

All commands ran in an isolated worktree at
`/private/tmp/corelink-t6w2-clw-accept`.

```text
cargo test -p clw-run required_hit --locked -- --nocapture
4 passed, 0 failed

cargo test -p clw-cli --bin clw run::tests --locked -- --nocapture
5 passed, 0 failed

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

The `clw-run` tests cover a miss, malformed record, CAS digest substitution,
and replay of a cached non-zero result. Every negative case verifies that the
marker command was not spawned.

## Authenticated HIT → restore proof

The proof used the existing local `~/.clw/config.toml` identity without
printing its token. `clw auth whoami --json` returned the configured endpoint
and tenant with `token: "<set>"`.

Against that endpoint, the same input and command were run twice. The first
run returned `rc=0` and `[clw] cache miss`; its output hash was
`946b0d5eb2c6689559fe40e2b0e497f3ceeacd19e055922949c086424d940572`.
The child created a marker. The marker was then moved aside without changing
the command or input key. The second run used `--require-hit`, returned
`rc=0` and `[clw] cache hit`, restored the identical output hash, and left the
marker **absent**. This is the no-child assertion.

Persisted proof directory:

```text
/private/tmp/clw-t6w2-live.Ig1sVO
```

Observed stderr hashes:

```text
miss: 052173251915cb7f15d1a3aff29aaaa5426726a2681865c7a6e78fb3a1618fe0
hit:  a9951c128629197a772e38289c7cbf058f717be5bd8867edd1115f944cc9a679
```

The live proof is evidence for the candidate behavior at the pinned source
commit. It is not a signed release asset or a hosted release-pipeline result.

## Refusal path kept separate

On the isolated runners worktree at
`/private/tmp/corelink-t6w2-runner-accept`,
`bash actions/corelink-memoize/test/policy.sh` passed:

```text
optional-cold: rc=0
optional-internal-fallback: rc=0
required-absence: rc=78
required-no-json-api: rc=78
invalid-policy: rc=78
T6-W2 policy fixtures: all green
```

The refusal fixtures assert the wrapped command marker is absent. They test
the installed `clw 0.1.5` compatibility boundary in the action and must not be
reported as a successful cache HIT. The new `clw 0.1.12` required-hit proof is
the separate authenticated protocol above.

## Local release package

The complete five-target staging package is at:

```text
/private/tmp/clw-0.1.12-artifact
```

Its exact source is recorded in `SOURCE_PROVENANCE.txt`:

```text
source_commit: 2a323ff5484c22f46a188056c7efbaf683eb89ee
source_tree:   26568cf1f0a3c14d8f4d35b3f890778d9b10639a
```

`BUILD_MATRIX.md`, `CONSUMER_PINS.md`, and `SHA256SUMS` contain the five
target digests. `sha256sum -c SHA256SUMS` passed for all binaries and package
metadata. The native x86_64 macOS binary reports `clw 0.1.12`; all other
targets pass format and digest checks. No `.minisig` file is present because
the official private key is not locally available.

## Release authority boundary

Read-only GitHub checks at acceptance time show the public mirror's latest
release is still `v0.1.11`; there is no published `v0.1.12`, signed
`SHA256SUMS`, or release binary digest to pin. The release workflow requires
owner-provisioned signing and cross-repository publication credentials
(`MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD`, `CLW_RELEASES_TOKEN`, and the
platform signing credentials). Those credentials were not available to this
local acceptance work.

Therefore the candidate is behaviorally accepted, but the release artifact is
`BLOCKED-AUTHORITY` until the owner provisions the signing/publish authority
and the real tag-triggered run produces the immutable asset manifest. No tag,
release, formula bump, or remote publication was attempted.
