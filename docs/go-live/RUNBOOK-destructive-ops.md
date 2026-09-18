# Runbook — scoped storage deletion (`prune`, `erase`)

> Audience: authorized operators, support and on-call. Source baseline:
> `be96b8394c66d1dc36ce7ca2f186780f0ea48308` (0.1.12), reviewed 2026-09-18.
> This procedure describes the client, not a new production deployment or a
> legal-compliance attestation. It does not change contractual deletion duties.

The prior introduction treated server PR #269 as not yet deployed. That was a
historical dependency, not a current availability check. The client implements
listing and deletion and treats unavailable capabilities as errors. Verify the
selected service and identity explicitly; a workflow, old relay or `doctor`
(which is offline) cannot establish live availability. The original runbook
remains available in Git history; dated audit and relay records are preserved.

## 0. Authorization and stopping conditions

Always inspect the resolved endpoint, tenant, credential scope, affected data
surfaces and current writers before any deletion. Obtain authorization for the
specific operation. Run the dry-run first and read the complete result and exit
status. Consent and a plausible count are not proof of safe reachability.

Known blockers to unrestricted tenant-wide prune are
[#293](https://github.com/HuGR-Labs/corelink-workspaces/issues/293) (other root
families) and [#295](https://github.com/HuGR-Labs/corelink-workspaces/issues/295)
(publication races). Do not sweep a shared/busy tenant by assuming that low-write
hours or one more listing closes either gap. Where root completeness and writer
coordination are not established, stop and retain the data. This is containment,
not closure of those issues or a new GC protocol.

Use dedicated synthetic tenants for acceptance tests. This docs change authorizes
no production deletion. Preserve results and hashes according to the approved
evidence policy; do not store tokens or customer payloads in issue comments.

## 1. `clw prune` — known-root mark and sweep

### Mechanism and limitations

The client enumerates CAS candidates, then marks recognized workspace manifests
and chunks plus RunRecord stdout/stderr. It does not follow `RefRecord.parent`,
and that mark set does not include every adapter/server catalog sharing CAS.
Hash-domain separation does not establish blob ownership.

The client refuses an incomplete recognized live set. It also refuses an empty
live set over nonempty CAS, in dry-run as well as destructive mode. These are
important safeguards, not a proof that all root families were enumerated.
Listing candidates before marking does not protect a publication that becomes
live after the mark. No client-local lock controls all other writers.

### Procedure within an explicitly accepted envelope

1. Confirm the operation's authority and the complete root/writer envelope.
   Stop if either is unknown; do not substitute a `doctor` report for this step.
2. Inspect references and errors with an explicitly selected endpoint/tenant.
   Put global flags before the subcommand:
   ```sh
   clw --endpoint "$APPROVED_ENDPOINT" --tenant "$APPROVED_TENANT" --json ls --all
   clw --endpoint "$APPROVED_ENDPOINT" --tenant "$APPROVED_TENANT" prune
   ```
   Read `run_cache_entries`, `skipped`, `errors` and the command status as well
   as workspace counts. A zero `.count` alone is not an empty AC/store proof.
3. Only after approving the dry-run and independently establishing the required
   envelope, a destructive invocation adds `--yes`. This flag changes consent,
   not the GC consistency protocol. Never suppress an incomplete-live-set error.
4. Record results, including partial progress, and verify the relevant consumers
   can read protected content with cold caches. Another dry-run can help inspect
   leftovers but is not sufficient proof that live data survived.

A failure while sweeping may follow successful deletes. Treat the output as
partial, inspect authoritative state and coordinate writers before a retry.
An idempotent individual DELETE does not make the multi-request procedure atomic.

## 2. `clw erase` — clear enumerated AC/CAS data

### Mechanism and scope

`erase` lists the selected tenant's AC references and CAS blobs. It drains the
ref-delete phase before the blob-delete phase, re-enumerates, and performs at
most three verification passes. It reports partial/incomplete results and exits
nonzero when the selected verification does not establish an empty enumeration.
An accepted asynchronous DELETE response is not evidence that physical deletion
has completed; observe the resulting authoritative state.

This sequence alone does not prove permanent writer exclusion, removal from all
other catalogs, backups/logs or account offboarding. Ref deletion before blob
deletion is not a transaction with concurrent publishers. The actual server
connection and any stronger completion promise remain
[#316](https://github.com/HuGR-Labs/corelink-workspaces/issues/316).
Do not call a clear of these enumerated surfaces a certificate that every legal
or contractual erasure obligation has been fulfilled.

### Procedure

1. Validate the request and tenant mapping under the existing privacy/DPA process.
   Record its identifier, authority, scope and the actual endpoint, not secrets.
2. Identify every surface included in the requested guarantee and the server's
   write-exclusion/completion mechanism where required. Escalate gaps before
   declaring offboarding or permanent erasure; this runbook does not waive them.
3. Preview using the explicit endpoint/tenant:
   ```sh
   clw --endpoint "$APPROVED_ENDPOINT" --tenant "$APPROVED_TENANT" erase
   ```
4. A destructive invocation requires `--yes` plus the exact tenant confirmation
   (interactive or `--confirm-tenant` for noninteractive/JSON use). Mismatch or
   absence of required confirmation refuses the operation. No such command was
   executed to validate this documentation.
5. Capture the exit, completion flag when present, distinct deleted counts,
   remaining counts and any partial-error note. A dry-run exit 0 is not deletion.
6. Verify AC, CAS and any other required server surfaces against the selected
   contract, accounting for asynchronous work and new writers. Close the
   technical/compliance record only to the extent its separate requirements
   have actually been met; `ls` workspace count alone is insufficient.

## 3. Results and recovery

| Outcome | Interpretation |
|---|---|
| Successful dry-run | Preview completed; nothing was intentionally deleted. |
| Successful destructive client result | The client's enumerated scope passed its verification; do not infer broader permanent erasure. |
| Capability unavailable (405/501) | Operation failed; not proof of an empty store or proof that a specific old deployment is still missing. |
| Confirmation refused/mismatched | Deletion was not authorized; retain the failure. |
| Incomplete live set / empty-live-set floor | Prune refused; do not bypass the safeguard. |
| Partial deletion, exhausted verification or transport failure | Preserve progress and inspect state; some earlier effects may have committed. |

`--json` is output formatting, not permission. Check both terminal status and the
actual result variant; not every failure has the same field set. The current
[CLI contract](../CLI-CONTRACT.md) and command handlers are the source of truth.
Do not relabel timeout, retry exhaustion or pending deletion as success.

There is no data rollback provided by `git revert`. Stop new destructive work,
preserve evidence and use an independently authorized recovery procedure when
available. Do not automatically rerun across a different tenant, endpoint or
uncoordinated writer state. Removing the local footprint with `clw uninstall`
does not delete remote data or revoke a server credential.

## 4. Sources and separate obligations

Client sources at the reviewed commit:
[prune](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-cli/src/subcmds/prune.rs),
[erase](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-cli/src/subcmds/erase.rs),
[ls](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-cli/src/subcmds/ls.rs)
and [HTTP listing/deletion](https://github.com/HuGR-Labs/corelink-workspaces/blob/be96b8394c66d1dc36ce7ca2f186780f0ea48308/crates/clw-client/src/lib.rs).

[Architecture](../ARCHITECTURE.md), [threat model](../THREAT-MODEL.md),
[privacy policy](../PRIVACY-POLICY.md) and [DPA](../DPA-subprocessors.md)
have separate roles. The legal documents are not amended by this technical
clarification. #292 owns atomic ref replacement; #293/#295 own GC safety;
#315 owns retention; #316 owns erase scope/guarantees. No runtime fix or live
availability test is claimed by this runbook revision.
