# Threat model — `clw` (CoreLink Workspaces CLI)

> Client-side model reconciled against commit
> `be96b8394c66d1dc36ce7ca2f186780f0ea48308` (0.1.12), 2026-09-18.
> Companion to [SECURITY.md](../SECURITY.md) and
> [ARCHITECTURE.md](ARCHITECTURE.md). This is a documentation correction, not
> runtime remediation, an operational certification or a new release claim.

The June audit and follow-up are historical evidence, not current runtime
guarantees. This reconciliation supersedes the present-tense conclusions of the
[prior model](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/docs/THREAT-MODEL.md).
Dated audit artifacts are not rewritten here.
The current campaign's canonical obligations are in
[tracker #325](https://github.com/HuGR-Labs/corelink-workspaces/issues/325).
A mitigation below is not a declaration that every related finding has closed.

`clw` is a client, not a server or a compute-isolation service. Untrusted inputs
include remote records and chunks, selected local paths and metadata, and the
output of commands the user explicitly launches. Tests may start local HTTP
fixtures; that does not make a test server a shipped service.


> **Integrated changes at this source:** [PR #329](https://github.com/HuGR-Labs/corelink-workspaces/pull/329)
> supplies Unix hydrate temporary ownership and pre-publication permissions;
> [PR #334](https://github.com/HuGR-Labs/corelink-workspaces/pull/334) supplies
> checked per-file size validation. Both are already in the pinned main commit.
> Earlier test records retain their original source identities; this documentation
> update is not a new runtime-test or release result.

## Assets

| Asset | Location and boundary | Required protection |
|---|---|---|
| Static or redeemed bearer token | Resolved configuration/environment; CAS/AC Authorization header. A runner bootstrap ticket and its lease/fabric binding are distinct from the bearer token. | Prevent unintended disclosure. The authority of a stolen token depends on server-enforced scope; do not assume every token grants all tenant operations. |
| Selected workspace content | Source tree, remote chunks/manifest, local cache and hydrate destination. | Content integrity, intended access and recoverability. These are separate properties. |
| Refs and memo records | Tenant AC and any related server-side catalogs. | Correct association of names/results with content, explicit lifecycle and concurrency rules. |
| Local config, cache and temporary files | Default or configured paths on the host. | Permission policy during creation, writing and publication, plus owned cleanup. A protected cache does not protect an unrelated hydrate destination. |

The credential resolver and command-gated bootstrap are in
[`config.rs`](../crates/clw-cli/src/config.rs) and
[`main.rs`](../crates/clw-cli/src/main.rs). Content and transport contracts are in
[`clw-types`](../crates/clw-types/src/lib.rs). Source inspection does not establish
which credentials or production policies are currently provisioned.

## Trust boundaries

**Transport destination and server authority.**
[`HttpClient::with_concurrency`](../crates/clw-client/src/lib.rs) uses parsed
`is_safe_https`/`is_loopback_http` checks, rejects endpoints outside that policy
and disables redirects. The loopback exception is for local development/tests;
it is not a prefix match against a raw hostname string. TLS validation does not
prove a configured host belongs to CoreLink. The operator chooses the destination;
server-side authentication, tenant authorization and storage isolation must be
verified at the service boundary, not inferred from a tenant string in a URL.

**Remote bytes and local effects.** Content-address verification detects bytes
that do not match the expected digest. It does not authenticate who selected that
digest, prove metadata consistency, prove safe link composition or make a
namespace representable on every filesystem. Path and semantic checks remain
necessary after integrity verification. Ref key domain separation is not an ACL.

**Host files and command execution.** The application manages config/cache and
writes selected destinations. `clw run` spawns locally and removes known secret
environment names, but preserves other ambient state. A child running as the same
user can still access files allowed to that user, including a reachable config.
This is not a sandbox and environment scrubbing is not filesystem isolation.
Do not infer tenant credential safety from a test that observes only the child's
environment.

**Publication and lifecycle.** A file rename is not a tree transaction, a ref
record is not indefinite retention, and a set of per-request transport deadlines
is not a guaranteed deadline for the entire command or a blocked filesystem call.
Server-side GC, other CAS consumers and writers are separate actors.

## Threats, implemented measures and remaining obligations

| Concern | Implemented measure at the reviewed source | Remaining boundary / canonical obligation |
|---|---|---|
| Unsafe materialization paths | `Manifest::from_bytes`, `validate_rel_path` and `validate_symlink_target_contained`; separate files and links passes. | Entry paths and link targets have different rules: contained relative targets may contain `..`. Lexical acceptance and files-before-links do not prove safe combined resolution. [#296](https://github.com/HuGR-Labs/corelink-workspaces/issues/296) remains the confinement obligation. |
| Local publication confidentiality | Config/cache permission handling exists; integrated #297 creates Unix hydrate temporaries owner-only, applies final mode before sync/rename and cleans only acquired temporaries. | Intentionally broader final modes can apply before rename. Windows config/cache effective access and failure policy are [#317](https://github.com/HuGR-Labs/corelink-workspaces/issues/317), not a Unix-mode inference. |
| Incorrect or forged content bytes | CAS transport contract requires BLAKE3 verification; hydrate checks root/chunk integrity and, with integrated #306, checked per-file size equality before materialization; local cache checks read content. | Expected digest, semantic metadata validity and availability are distinct. Broader parser acceptance remains #308; these size checks do not establish safe link composition or a representable namespace. |
| Loss through destination aliases | Literal entry ordering and uniqueness are validated. | Literal uniqueness does not prove filesystem uniqueness. [#303](https://github.com/HuGR-Labs/corelink-workspaces/issues/303) covers loss on unrepresentable destination names. |
| Resource use | Hydrate checks entry, aggregate chunk-byte, link and chunk counts; output capture and HTTP bodies also have caps. | See the precise scope below. Snapshot growth [#291](https://github.com/HuGR-Labs/corelink-workspaces/issues/291), Windows copy expansion/order [#298](https://github.com/HuGR-Labs/corelink-workspaces/issues/298)/[#299](https://github.com/HuGR-Labs/corelink-workspaces/issues/299), cache maintenance/temporaries [#304](https://github.com/HuGR-Labs/corelink-workspaces/issues/304)/[#314](https://github.com/HuGR-Labs/corelink-workspaces/issues/314) have separate boundaries. |
| Late effects after failure | Each file writer has a finish/abort and cleanup protocol. | Sibling-writer quiescence after the outer error is [#300](https://github.com/HuGR-Labs/corelink-workspaces/issues/300); cleanup must not race effects whose completion has not been observed. |
| Stale or incorrect memo reuse | Run-key domain `clw/run/v3/` includes declared inputs, environment, command/tool and cwd; producer writes on exit 0. | Tool selection [#301](https://github.com/HuGR-Labs/corelink-workspaces/issues/301), OS environment encoding [#302](https://github.com/HuGR-Labs/corelink-workspaces/issues/302), received-record eligibility [#307](https://github.com/HuGR-Labs/corelink-workspaces/issues/307). Undeclared and concurrently changing inputs are not made immutable by computing a key. |
| Misleading execution/delivery result | CLI forwards captured output and maps internal failures. | Error-phase [#305](https://github.com/HuGR-Labs/corelink-workspaces/issues/305) and delivery [#310](https://github.com/HuGR-Labs/corelink-workspaces/issues/310) remain distinct. Exit integers alone cannot prove absence of side effects. |
| Loss of named or shared content | New content is normally uploaded before publishing its ref; prune marks known refs and rejects unreadable recognized roots. | Ref replacement [#292](https://github.com/HuGR-Labs/corelink-workspaces/issues/292), missing-root repair [#294](https://github.com/HuGR-Labs/corelink-workspaces/issues/294), other root families [#293](https://github.com/HuGR-Labs/corelink-workspaces/issues/293) and concurrent GC [#295](https://github.com/HuGR-Labs/corelink-workspaces/issues/295). A partial root view must not authorize deletion. |
| Unsafe implementation | Workspace `unsafe_code = "forbid"` and the separately reviewed Windows ACL exception; configuration/source guards accompany compiler checks. | Linting applies to the code compiled for the selected target. Guard configuration or cross-compilation is not native runtime evidence or proof that every dependency/platform is memory-safe. See [SECURITY.md](../SECURITY.md) and the current manifests/workflows. |

This table is defensive scope documentation. It does not publish attack fixtures
or claim new reproduction of any issue.

## Resource bounds: what is actually bounded

At the reviewed source, `enforce_resource_bounds` in
[`clw-hydrate`](../crates/clw-hydrate/src/lib.rs) checks at most 1,000,000 manifest
entries, 1 TiB summed regular-file `ChunkRef.size` bytes, 100,000 symlink entries
and 64,000,000 total chunk references. A per-file check limits chunk references to
16,000,000. These policies run after the root bytes have been fetched, verified
and parsed, and before destination materialization. They are not checks before
all allocation. The symlink-count cap is not a general cycle-resolution proof.

The summed regular-file budget is neither a physical disk quota nor accounting
for every Windows fallback copy. The parent disk can still run out of space.
The single CAS body cap is 5 MiB; it is not the maximum total workspace size.
The root is a flat manifest blob, not recursive per-directory Merkle metadata.
Resource and copy gaps above remain named rather than contradicted by a blanket
claim that exhaustion or traversal is already solved.

## Destructive operations and retention

[`rm`](../crates/clw-cli/src/subcmds/rm.rs),
[`prune`](../crates/clw-cli/src/subcmds/prune.rs) and
[`erase`](../crates/clw-cli/src/subcmds/erase.rs) are implemented commands, not
future milestones. Their consent rails do not by themselves prove completeness:

- `rm` removes a selected AC reference, not the referenced CAS tree.
- `prune` protects recognized current workspace roots/chunks and RunRecord streams.
  It does not traverse `parent`; shared catalogs and concurrent publication still
  require their tracked fixes. Preserve the incomplete-root and empty-live-set
  safety rails rather than treating them as evidence of universal root coverage.
- `erase` drains ref deletion before blob deletion and re-enumerates for at most
  three passes, reporting partial results. This is not, by itself, a writer fence,
  account-offboarding protocol or proof about every backup/log/catalog. Its actual
  server scope is the verification obligation in
  [#316](https://github.com/HuGR-Labs/corelink-workspaces/issues/316).

Lineage or possession of a digest does not reserve its bytes against collection.
The retention decision in [#315](https://github.com/HuGR-Labs/corelink-workspaces/issues/315)
must identify a supported root and the applicable collection envelope.
Do not label a legal-compliance or residency promise satisfied or violated solely
from this client model or from the presence/absence of a CLI command.

## Non-goals and evidence requirements

A local root adversary and isolation of arbitrary child code from its own user
require controls outside this client. This does **not** waive protection against
untrusted remote manifests or intended access control among non-root local users.
Capture of a mutable working tree, undeclared inputs and transitive toolchain
identity are not automatically converted into hermetic execution by memoization.
Server secret backup, physical residency, service deployment and release signing
are outside the evidence provided by these client source reads.

Each claimed fix needs its own relevant regression and healthy controls on an
identified source/build. Windows access needs Windows evidence, and private Unix
publication needs effective-access evidence rather than only final mode bits.
Source inspection, previously recorded results, tests run for another PR and a
proposed patch are different evidence classes. Preserve historical reports;
update this model when their corresponding changes and acceptance really land.
The pinned [source-to-claim index](ARCHITECTURE.md#source-to-claim-review-index)
records the main implementation anchors for this reconciliation.
