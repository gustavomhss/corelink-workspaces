# CoreLink Workspaces — Architecture

> **#302 source update:** memoization paragraphs below additionally describe the
> `clw/run/v4/` transition and rejection of declared values not representable as
> UTF-8. Earlier source dates remain historical; this is not release evidence.

## Thesis and reviewed scope

The durable object is a snapshot of the **selected files**, not the machine or a
running process. `clw` publishes their content and a manifest to CoreLink, restores
them into a new or empty destination, and separately memoizes command output.
Dependencies, toolchains and build artifacts are captured only when their files
are in the selected tree. This is not a VM image, a process checkpoint or an
atomic capture of a tree being modified by other processes.

This description is reconciled against source commit
`be96b8394c66d1dc36ce7ca2f186780f0ea48308` (workspace version 0.1.12), on
2026-09-18. It describes that source, not a newly verified deployment or release.
The [audit tracker #325](https://github.com/HuGR-Labs/corelink-workspaces/issues/325)
records outstanding obligations; an open or proposed PR does not change this
baseline. This documentation change neither repairs nor closes those obligations.
The [threat model](THREAT-MODEL.md) separates required properties from protections
already implemented.


> **Integrated changes at this source:** [PR #329](https://github.com/HuGR-Labs/corelink-workspaces/pull/329)
> supplies Unix hydrate temporary ownership and pre-publication permissions;
> [PR #334](https://github.com/HuGR-Labs/corelink-workspaces/pull/334) supplies
> checked per-file size validation. Both are already in the pinned main commit.
> Earlier test records retain their original source identities; this documentation
> update is not a new runtime-test or release result.

## Components and dependency kinds

The workspace members are listed in [Cargo.toml](../Cargo.toml). The main
responsibilities are:

| Component | Responsibility |
|---|---|
| `clw-types` | Digests, flat manifest and ref types, validation, transport traits and constants. |
| `clw-manifest` / `clw-chunk` | Ignore-aware tree walking and manifest comparison / FastCDC chunking. |
| `clw-client` / `clw-cache` | HTTP CAS/AC transport / local content cache. |
| `clw-snapshot` / `clw-hydrate` | Publication / reconstruction of the selected tree. |
| `clw-run` | Declared-input key derivation and local subprocess-output memoization. |
| `clw-cli` / `clw-win-acl` | Command and configuration wiring / the Windows ACL implementation. |
| `clw-integration-tests`, `clw-conformance`, `clw-e2e-evidence` | Integration, opt-in live checks and evidence tooling; not a server implementation. |

This is not a single linear crate stack. The following table records the direct
internal **normal dependencies of the four orchestration crates**, rather than
claiming to be the whole dependency graph:

| Crate | Internal normal dependencies | Other relevant dependency kind |
|---|---|---|
| [`clw-snapshot`](../crates/clw-snapshot/Cargo.toml) | `clw-types`, `clw-chunk`, `clw-manifest`, `clw-cache` | Uses transport traits; no direct normal dependency on `clw-client`. |
| [`clw-hydrate`](../crates/clw-hydrate/Cargo.toml) | `clw-types`, `clw-cache`, `clw-client` | `clw-snapshot` is a **dev-dependency** for the round-trip example. |
| [`clw-run`](../crates/clw-run/Cargo.toml) | `clw-types`, `clw-manifest`, `clw-client` | No direct normal dependency on `clw-cache`. |
| [`clw-cli`](../crates/clw-cli/Cargo.toml) | `clw-types`, `clw-snapshot`, `clw-hydrate`, `clw-run`, `clw-manifest`, `clw-cache`, `clw-client` | `clw-win-acl` is Windows-target-specific; `clw-e2e-evidence` is a dev-dependency. |

Normal, target-specific and dev edges must remain distinguishable when reviewing
changes. A test dependency does not establish a production dependency, and a
trait-based caller can use a transport implementation without importing its crate.

## Stored objects and authority

CAS stores blob bytes addressed by BLAKE3. FastCDC uses 256 KiB minimum, 1 MiB
average and 4 MiB maximum chunk settings; the generic CAS body cap is 5 MiB.
The constants and wire types live in [`clw-types`](../crates/clw-types/src/lib.rs).

A root is **one flat JSON manifest**, with `version`, `total_size` and `entries`.
File entries reference ordered chunks; symlink entries carry targets; `Dir`
entries represent empty directories. Nonempty directories are implied by paths.
There is no recursive per-directory manifest tree in this format. The producer
emits compact deterministic JSON with strictly ordered entry paths. Reader
acceptance is a separate contract: it must not silently normalize received bytes
and claim that the normalized object has the original root digest. Detailed
format acceptance is tracked in [#308](https://github.com/HuGR-Labs/corelink-workspaces/issues/308).

AC stores opaque records: named `RefRecord`s and command `RunRecord`s. Ref key
hash domains distinguish user and runner names, but **domain separation is not
an authorization boundary** and does not establish ownership of shared CAS blobs.
The remote service must enforce the credential's tenant and permissions.
A root supplied by digest avoids name resolution; it does not create retention.
`RefRecord.parent` records lineage and is not traversed by the reviewed prune
marking path.

## Dataflows

### Snapshot

The implementation in [`clw-snapshot`](../crates/clw-snapshot/src/lib.rs) walks
with [`clw-manifest`](../crates/clw-manifest/src/lib.rs): ignore rules apply,
`.git` components are excluded, links are not followed, and unrepresentable
external link targets are reported as skipped. Files are read over time; hashing
them does not freeze their source directory.

```text
walk selected tree -> pre-read named ref -> process files and chunks
  upload path: cache chunk -> check remote presence -> PUT missing chunk
  occupied/non-force path: compute content without uploading it
-> serialize and size-check root manifest
-> same root? return unchanged
-> occupied/non-force and different content? refuse
-> otherwise ensure root blob exists -> publish named ref
```

For a new publication, chunks and the root are handled before the ref is written.
This ordering is **not** a transaction with concurrent collection. Forced
replacement currently uses separate DELETE and PUT requests; it is not an atomic
compare-and-swap. Same-root success can precede repair of a missing root blob.
These are distinct tracked defects:
[#292](https://github.com/HuGR-Labs/corelink-workspaces/issues/292),
[#295](https://github.com/HuGR-Labs/corelink-workspaces/issues/295) and
[#294](https://github.com/HuGR-Labs/corelink-workspaces/issues/294), respectively.
Neither a presence check nor `--force` is a general integrity scrub of
corrupt-but-present remote bytes.

The small-file growth bound is tracked in
[#291](https://github.com/HuGR-Labs/corelink-workspaces/issues/291): a post-read
check must not be described as preventing the allocation that already occurred.
The root size check follows chunk processing, so it is not a preflight that avoids
all upload work for an oversized root.

### Hydrate

Both entry points converge on the same root validation and materializer:

```text
name -> AC RefRecord -> root     OR     caller-supplied root digest
-> fetch root bytes -> verify expected BLAKE3 -> parse manifest
-> check each file size with overflow detection, aggregate sizes/counts and entry/link policy
-> create destination/directories
-> fetch chunks cache-first and reconstruct regular files
-> materialize links: Unix links / Windows target copies
-> best-effort local cache maintenance -> report
```

The root digest verifies **which bytes** were fetched, not whether all their
semantics are acceptable. The reviewed preflight uses checked addition for each
file's chunk sizes and rejects overflow or disagreement with `File.size`.
It retains the aggregate `total_size` equality and resource caps before destination
creation or chunk materialization. This is the integrated
[#306](https://github.com/HuGR-Labs/corelink-workspaces/issues/306) correction,
not a new manifest parser or a guarantee about destination representability.

The writer uses a temporary and an explicit `Finish` message before publication;
EOF without `Finish` aborts that writer. Publication is **per file**, not atomic
for the directory tree. A failure may leave already completed files. The outer
ordinary-error path's sibling-writer completion is tracked in
[#300](https://github.com/HuGR-Labs/corelink-workspaces/issues/300); do not infer
quiescence or safe concurrent cleanup merely from a returned error.

Files-before-links is an ordering measure, not proof of complete confinement.
Combined link semantics and destination name aliases remain
[#296](https://github.com/HuGR-Labs/corelink-workspaces/issues/296) and
[#303](https://github.com/HuGR-Labs/corelink-workspaces/issues/303).
The integrated #297 fix creates Unix temporaries with mode 0600, arms cleanup only
after exclusive creation, and applies the requested safe final mode through the
owned descriptor before sync/rename. Failed acquisition cannot delete an occupant.
Intentionally broader final permissions take effect before the rename; this does
not grant whole-tree or Windows ACL guarantees.
Windows copies intentionally do not preserve link identity; their expansion
accounting and dependency order are tracked in
[#298](https://github.com/HuGR-Labs/corelink-workspaces/issues/298) and
[#299](https://github.com/HuGR-Labs/corelink-workspaces/issues/299).

### Run (memoized command)

The reviewed key domain is `clw/run/v4/`. The key includes declared input paths
and digests (plus supported modes), declared environment presence/values, the
command vector, tool identity and cwd. See
[`clw-run`](../crates/clw-run/src/lib.rs) for its encoding and fallback behavior.

```text
compute declared-input key -> initial AC lookup
  parsed record and fetchable streams -> replay recorded output/exit
  absent, unparseable or unusable record -> execute one local child
-> capture bounded stdout/stderr
-> child exit 0: best-effort store streams and RunRecord
-> return outcome; CLI forwards output and exits
```

An **initial AC transport error propagates**; it is not the same branch as a
parsed record whose streams cannot be fetched. The producer stores successful
commands only, but the reviewed consumer does not separately reject a parsed
nonzero `RunRecord`; [#307](https://github.com/HuGR-Labs/corelink-workspaces/issues/307)
owns that mismatch. Replay eligibility and the GC's reference extraction are
separate responsibilities.

The record has no output-file inventory: a HIT does not restore deleted build
artifacts. The command runs on this host with ambient environment minus the known
secret names; it is not sent to a compute scheduler or placed in a sandbox by
`clw`. The caller must declare relevant inputs and provide any required isolation.
HOME and same-user filesystem access are not removed by environment scrubbing.
Tool selection, declared OS environment encoding, error-phase classification and
stream delivery have separate obligations
[#301](https://github.com/HuGR-Labs/corelink-workspaces/issues/301),
[#302](https://github.com/HuGR-Labs/corelink-workspaces/issues/302),
[#305](https://github.com/HuGR-Labs/corelink-workspaces/issues/305) and
[#310](https://github.com/HuGR-Labs/corelink-workspaces/issues/310).
The integer exits 125/137 can also be legitimate child results; they alone do not
establish execution history or authorize automatic retry.


### Declared environment identity update (#302)

The v4 run-key domain separates the new reader from v3 records that conflated
non-Unicode OS values with absent values. Unsupported declared values now fail
before AC lookup or spawn without including their bytes in diagnostics. Supported
UTF-8, absent and empty inputs remain distinct. RunRecord serialization and its
GC reachability decoder are unchanged; no old entry is deleted by this migration.
Executable selection (#301) and concurrently changing inputs remain separate.


## Lifecycle operations and limits

| Surface | Implemented mechanism | Limit that must remain visible |
|---|---|---|
| [`rm`](../crates/clw-cli/src/subcmds/rm.rs) | Consent-gated deletion of one named/raw AC ref. | Does not delete that root's CAS content or establish an atomic replacement. |
| [`prune`](../crates/clw-cli/src/subcmds/prune.rs) | Mark recognized workspace roots/chunks and RunRecord streams, then optionally sweep candidates; incomplete recognized roots cause refusal. | Does not follow `parent`. Other shared root families and concurrent publication require [#293](https://github.com/HuGR-Labs/corelink-workspaces/issues/293)/[#295](https://github.com/HuGR-Labs/corelink-workspaces/issues/295); do not advertise unrestricted safe tenant-wide GC. |
| [`erase`](../crates/clw-cli/src/subcmds/erase.rs) | Consent gates, drained ref-delete phase, then blob-delete phase, with re-enumeration for at most three passes and partial-progress reporting. | This client sequence alone does not prove permanent offboarding, deletion of every server catalog/backup or exclusion of new writers; [#316](https://github.com/HuGR-Labs/corelink-workspaces/issues/316). |
| [`LocalCache`](../crates/clw-cache/src/lib.rs) | Local verified content and best-effort eviction. | Not authoritative remote retention or a hard disk quota; short-lived maintenance and owned temporary cleanup are [#304](https://github.com/HuGR-Labs/corelink-workspaces/issues/304)/[#314](https://github.com/HuGR-Labs/corelink-workspaces/issues/314). |

These limitations do not approve new features. Optional checkpoint retention is
a decision in [#315](https://github.com/HuGR-Labs/corelink-workspaces/issues/315),
not an implicit promise of automatic history preservation.

## Security notes

The production HTTP client sends a bearer token to the configured tenant URL,
checks its parsed HTTPS/loopback policy and does not follow redirects. A valid
HTTPS URL is not an allowlist of CoreLink-owned hosts: configuring a destination
is an authorization decision. The transport trait's integrity requirement and
hydrate's manifest/chunk checks do not grant filesystem authority to the payload.
See [`HttpClient::with_concurrency`](../crates/clw-client/src/lib.rs) and
[`CasTransport`](../crates/clw-types/src/lib.rs).

Static credential resolution and command-gated runner credential bootstrap live
in [`config.rs`](../crates/clw-cli/src/config.rs) and
[`main.rs`](../crates/clw-cli/src/main.rs). A bootstrap ticket is distinct from
the resulting bearer token. Config/cache permissions, arbitrary hydrate paths
and the child process's filesystem access are different boundaries. Windows
privacy requires its own effective-access evidence
([#317](https://github.com/HuGR-Labs/corelink-workspaces/issues/317)).

This repository implements the client and test tooling, not server storage,
authorization, scheduler isolation or operational secret backup. Compilation,
configured workflows, historical conformance and an open PR are not evidence that
a current service or distributed binary provides a claimed guarantee.

## Source-to-claim review index

All source links below pin the reviewed commit. This index is source-inspection
evidence, not a claim that the referenced tests were executed in this docs change.

| Claim | Reviewed implementation |
|---|---|
| Selected tree, ignores, links | [walk](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-manifest/src/lib.rs#L61-L175) |
| Flat root and producer/reader distinction | [types and parsing](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-types/src/lib.rs#L107-L215) |
| No-op, root publication, forced ref update | [snapshot](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-snapshot/src/lib.rs#L625-L788) |
| Preflight, parallel files, links and cache maintenance | [hydrate](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-hydrate/src/lib.rs#L290-L555) |
| Per-file Finish/EOF, rename and metadata ordering | [file writer](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-hydrate/src/lib.rs#L618-L732) |
| Resource policy and limits | [hydrate caps](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-hydrate/src/lib.rs#L30-L161) |
| Run domain, secret list and stored record | [run types](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-run/src/lib.rs#L39-L125) |
| Key inputs, lookup, replay and local spawn | [run](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-run/src/lib.rs#L580-L815) |
| Recognized GC roots, no parent traversal | [compute_live_set](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-cli/src/subcmds/prune.rs#L311-L385) |
| Delete phase ordering, bounded verification | [erase_with_verification](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-cli/src/subcmds/erase.rs#L347-L438) |
| Endpoint policy, no redirects | [HTTP construction](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-client/src/lib.rs#L240-L296) |
