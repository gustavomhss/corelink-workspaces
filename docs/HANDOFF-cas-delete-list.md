# HANDOFF — CAS DELETE + CAS LIST for blob GC (D-8)

> **For:** the corelink-server session owning the CAS (content-addressed blob)
> service and its `CasTransport` HTTP surface (R2-backed). **From:** the `clw`
> go-live hardening fleet (ROADMAP v2, M6.4 Right-to-Erasure track).
> **Severity:** HIGH — **GDPR Art.17 go-live blocker.** `clw prune` and tenant
> right-to-erasure cannot delete or even enumerate CAS blobs because
> `CasTransport` has **neither a DELETE nor a LIST** today. **Why a handoff, not
> a fix:** `corelink-server` is a separate repo `clw` never edits.

---

## TL;DR

The CAS service can `PUT` (upload) and `GET`/`HEAD` (fetch/exists) a blob by
content hash, but it **cannot delete one** and **cannot list a tenant's blobs**.
That makes garbage collection and right-to-erasure structurally impossible:

- `clw rm` (D-1) deletes only the AC **ref pointer** — the underlying CAS blobs
  are **orphaned, never reclaimed** (ROADMAP M1.4 explicitly documents this leak).
- A tenant therefore **cannot actually delete their data**, only un-name it.
- There is no way to enumerate what blobs a tenant holds (for a sweep, an audit,
  or a residency proof).

Proposed: `DELETE /v1/cas/:tenant/:hash` (idempotent, tenant-scoped) + a
`GET /v1/cas/:tenant` LIST. Together with ref-LIST (**D-7**) these let `clw`
run a mark-and-sweep GC.

---

## Evidence / why this is blocking

- **`CasTransport` is upload-only + read.** It exposes PUT and GET/HEAD; no
  delete, no enumerate. (ROADMAP M1.2 confirms prod CAS PUT returns 201 for both
  new and repeat blobs — the write side is well-defined; the *reclaim* side
  simply does not exist.)
- **`clw rm` orphans blobs by design.** ROADMAP M1.4: "`rm` alone orphans CAS
  blobs (documented until prune exists)." Pruning needs CAS DELETE; the live-set
  computation needs CAS LIST (to know the sweep candidates) and ref-LIST (D-7, to
  know the roots).
- **GDPR Art.17 (ROADMAP M6.4) is go-live blocking.** "A tenant must be able to
  actually delete their data… Requires: client `prune`/CAS-GC + server ref-LIST
  (D-7) + server CAS DELETE/LIST (D-8). Until these land, the residency promise
  is non-compliant." The product *sells* regional residency, so non-deletability
  is a contractual defect, not a nice-to-have.

---

## Proposed contract

### CAS DELETE

```
DELETE /v1/cas/:tenant/:hash
```

- **Auth:** `Authorization: Bearer <tenant-PAT>` with CAS-write/delete scope for
  `:tenant`; **tenant-in-path** is the selector. `x-corelink-tenant-id` is
  server-INTERNAL and never honored from a client (see D-9). A PAT for tenant A
  deleting under `/v1/cas/B/...` → `403`.
- **Tenant-scoped:** deletion only affects the calling tenant's namespace/prefix.
  Because CAS is content-addressed and **per-tenant prefixed** in R2, the same
  blob bytes under another tenant's prefix are untouched. (Confirm the storage
  layout actually isolates tenants by prefix so a delete cannot cross tenants.)
- **Idempotent:** deleting an absent hash is **success, not error**. Suggested:
  `204 No Content` whether the blob existed or not. (Rationale: mark-and-sweep
  may race a concurrent re-upload or a prior partial sweep; the client must be
  able to retry a DELETE without distinguishing "already gone" from "just
  deleted." An idempotent 204 makes the sweep safely re-runnable.)
- **404 vs 2xx:** prefer **2xx for both** present and absent (idempotent delete).
  If you must distinguish, return `204` on actual delete and `404` on absent —
  but then **state it**, because the client treats `404` on DELETE as benign
  (already-erased) and will not retry. **Do NOT** return `404` for an
  authorization failure (that must be `403`) — conflating them would let `clw`
  mistake "not allowed" for "already gone" and skip erasure.
- **Validation:** reject a malformed `:hash` (`400`); the hash format is the
  existing CAS hash (BLAKE3 hex). Deleting must not require the client to prove
  the bytes (it's addressing by hash, not by content).

### CAS LIST

```
GET /v1/cas/:tenant?cursor=<opaque>&limit=<n>
```

```json
{
  "tenant": "<tenant>",
  "blobs": [
    { "hash": "<hex>", "size": <bytes>, "uploaded_at": "<RFC3339>" }
  ],
  "next_cursor": "<opaque>|null"
}
```

- **Auth/pagination:** identical model to D-7 — Bearer tenant-PAT, tenant-in-path,
  cursor-based forward-only pagination, server-enforced max `limit` (suggest
  1000). Pure read, side-effect-free, retry-safe.
- **`size`** is needed for the disk-space/quota UX (ROADMAP M5.6) and useful for a
  residency audit; include it if R2 exposes it cheaply.
- Empty tenant → `200 { "blobs": [] }`, never 404.
- This LIST maps onto an R2 `list` over the tenant prefix — please confirm the
  prefix layout makes that a single bounded enumeration.

### The GC algorithm clw runs on top

Mark-and-sweep over the tenant's live root set (conservative — never deletes a
reachable blob, may temporarily retain an unreachable one):

```
1. roots   = enumerate AC refs           (D-7  GET /v1/refs/:tenant)
2. live    = ∅
   for each ref in roots:
       manifest = GET its manifest_hash   (existing CAS GET)
       live    ∪= every CAS hash reachable from that manifest
                  (manifest entries + any sub-manifest chunks)
3. present = enumerate CAS blobs          (D-8  GET /v1/cas/:tenant)
4. garbage = present \ live
5. for each hash in garbage:
       DELETE /v1/cas/:tenant/:hash       (D-8, idempotent)
```

- **Safety ordering:** compute `live` *fully* before any DELETE. A blob is
  deleted only if it is in `present` and NOT in `live`. New uploads racing the
  sweep are safe: a freshly-PUT blob not yet referenced by any ref is either
  already in `present` (and protected only if reachable) — to avoid deleting an
  in-flight upload, `clw prune` applies a **grace window** (skip blobs with
  `uploaded_at` newer than the sweep start). `uploaded_at` in the LIST response
  is what makes this grace window possible — please include it.
- **Erasure (Art.17):** `clw rm` every ref (D-1) → run the sweep above → assert
  CAS LIST returns empty (or only graced-but-now-orphaned blobs, swept on the
  next pass). LIST is also the **proof of erasure** artifact.

---

## Confirm with owner

1. **Idempotent-delete status code:** `204` for both present/absent (preferred),
   or `204`/`404` split? `clw` needs the contract pinned to know whether to retry.
2. **Tenant isolation in storage:** confirm CAS blobs are per-tenant prefixed in
   R2 so a DELETE cannot reclaim bytes another tenant still references (content
   addressing means identical bytes may exist under multiple tenants).
3. **LIST feasibility:** is `GET /v1/cas/:tenant` a bounded R2 prefix-list? Is
   `size` + `uploaded_at` cheaply available (needed for quota UX + the GC grace
   window)?
4. **Auth scope:** does the tenant PAT scope grant CAS-delete + CAS-list, or are
   new scopes needed? (Ties to the **D-1 single-`scope` enum drift** — the PAT
   must grant CAS-write/delete; confirm with Clerk/signup.)
5. **Reference counting alternative:** if the server already refcounts blobs
   server-side, client-side mark-and-sweep may be unnecessary — but the client
   still needs LIST for erasure-proof. Confirm which model wins.
6. **Schedule with D-7:** both are required for GC/erasure; land them together so
   M6.4 can close as a unit.

## Cross-references

- ROADMAP dependency ledger: `docs/ROADMAP-go-live.md` (D-8), §M1.4, §M6.4.
- Sibling deps: `docs/HANDOFF-ref-enumeration.md` (D-7, the GC roots),
  `docs/HANDOFF-runner-pat-minting.md` (D-9, the two-layer auth model).
- `clw rm` / AC delete: D-1 (`DELETE /v1/ac/...`).
