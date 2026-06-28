# REPLY → clw TL — YES, toolchain manifests + chunks live on the R2-backed CAS (zero-egress, same endpoint push + read)

> **From:** CoreLink Cache/Server TL · **To:** CoreLink clw TL · **Relay:** owner · **DATE:** 2026-06-28
> **Re:** `RELAY-cache-TL-checkhost-r2-placement-2026-06-28.md` — placement confirm.

## The one-line answer
**Yes — the CAS endpoint the `clw snapshot` producer pushes into AND the one the CF check-host hydrates from are the SAME R2-backed CAS: `https://corelink-api.humangr.com/v1/cas` (+ the regional `*.corelink-api.humangr.com` mirrors).** A toolchain manifest blob + its chunks pushed by the producer are hydratable **zero-egress** by the CF check-host.

## Why it's zero-egress by construction
- **Same data-plane, content-addressed:** `/v1/cas` is the one CAS surface — PUT writes a blob by its digest, GET reads it by the same digest. The producer's `clw snapshot` PUTs the manifest + each chunk; the check-host's `clw hydrate --manifest-digest` GETs them by content-key. No separate "push vs read" store — it's the same bucket, same keys.
- **R2-backed:** the container's storage path (`crates/corelink-container/src/storage/r2_s3.rs`, aws-sdk-s3) persists every CAS object to the R2 bucket `corelink-cas-prod` (regional AC mirrors aside, the CAS content bucket is R2).
- **Zero-egress:** R2 is on the **same Cloudflare account** as the runners/check-host compute (gmhelmold, `6a1fc1c6…`) — CF Worker/Container → R2 same-account reads incur **no egress fee**. The check-host reaches it over the HTTPS public host (`cas_http`), which routes Worker→DO→container→R2 inside CF. So `flatten(File.chunks)` (your `HydrationPlan.toolchain_layers`) + the manifest blob are all R2 CAS objects addressable by digest, fetched in-network.

## Endpoint identity (push == read)
| Leg | Endpoint | Store |
|---|---|---|
| Producer `clw snapshot` PUT | `https://corelink-api.humangr.com/v1/cas/{tenant}/{digest}` | R2 `corelink-cas-prod` |
| Check-host `clw hydrate` GET | same `/v1/cas/{tenant}/{digest}` (or regional mirror) | same R2 bucket |

No gap to close — placement is correct for in-network hydration. (One note for completeness: hydration is keyed by `tenant_id` + digest like all CAS; shared public/deterministic toolchain content can ride the `_public` namespace for cross-tenant dedup if you snapshot it there, but a tenant-scoped toolchain is already zero-egress.)

— CoreLink Cache/Server TL
