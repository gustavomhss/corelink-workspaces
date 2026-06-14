# HANDOFF — ref-enumeration LIST endpoint (D-7)

> **For:** the corelink-server session owning the AC (ref/pointer) service —
> `crates/corelink-ac/` and its HTTP surface. **From:** the `clw` go-live
> hardening fleet (ROADMAP v2, M1.4 / M6.4 erasure track). **Severity:** HIGH —
> **GDPR Art.17 go-live blocker.** Without this endpoint, `clw ls --all`,
> `clw prune`, and tenant right-to-erasure are *impossible to implement at all*
> (not merely slow). **Why a handoff, not a fix:** `corelink-server` is a
> separate repo `clw` never edits; this is the scoped contract for that session.

---

## TL;DR

`clw` needs a **LIST endpoint that enumerates all AC refs belonging to a
tenant**. Today there is none: a ref is addressed by
`ref_key = BLAKE3(domain || name)`, a **one-way** hash. The client stores **no
name→key index** and the server exposes **no enumeration**, so given a tenant
there is no way to answer "what refs exist?" — which is the root set required for
mark-and-sweep GC and for proving a tenant's data has been fully erased.

Proposed: `GET /v1/refs/:tenant` (or `GET /v1/ac/:tenant`) returning a paginated
list of the tenant's ref entries.

---

## Evidence / why this is blocking

- **`ref_key` is a one-way digest.** `ref_key = BLAKE3(domain || name)`. From a
  `ref_key` you cannot recover `name`, and from a tenant you cannot derive its
  set of `ref_key`s. The mapping only goes name→key, and only the caller who
  *already knows the name* can compute it.
- **The client keeps no index.** `clw` is a stateless pure client; it does not
  persist a `name → ref_key` table. A fresh machine (or a disposable runner) that
  did not create the ref cannot list it. So enumeration *must* come from the
  server — it is the only party holding the full set.
- **Three client features are hard-blocked (ROADMAP M1.4 / M6.4):**
  - `clw ls --all` — list every ref a tenant owns. Impossible without LIST.
  - `clw prune` / CAS-GC — mark-and-sweep needs the **live root set** (every ref
    → its manifest → its reachable CAS blobs). The roots *are* the tenant's refs;
    you cannot compute reachability without enumerating them. (Pairs with **D-8**
    CAS DELETE/LIST — both are required; ref-LIST alone is insufficient for GC.)
  - **GDPR Art.17 erasure** — to *prove* a tenant's data is gone you must
    enumerate what they have. `clw rm` (D-1) deletes a single known ref by name;
    it cannot find the refs whose names are forgotten. This is the residency /
    right-to-erasure compliance path the product contractually sells.

---

## Proposed contract

### Endpoint shape

```
GET /v1/refs/:tenant?cursor=<opaque>&limit=<n>&domain=<optional>
```

(Or `GET /v1/ac/:tenant` if that better matches the existing AC route prefix —
**owner's call**; the rest of the contract is identical. Pick the one consistent
with the current `/v1/ac/...` ref routes and name it in the reply.)

### Auth — two-layer, tenant-in-path

- **`Authorization: Bearer <tenant-PAT>`** — the same scoped tenant PAT every
  `clw` request carries. The PAT must be **read-authorized for the AC/ref scope**
  of the tenant in the path.
- **Tenant-in-path** (`:tenant`) is the client-facing tenant selector, exactly
  like every other `clw` route. The server resolves/authorizes the PAT against
  this path tenant.
- **`x-corelink-tenant-id` is server-INTERNAL** and MUST NOT be honored from a
  client on this route — `clw` never sets it (see D-9 for the full two-layer auth
  model). A request that sets it from outside must be ignored/rejected.
- A PAT scoped to tenant A requesting `/v1/refs/B` MUST be rejected (403),
  preventing cross-tenant enumeration.

### Pagination

- **Cursor-based, opaque, forward-only.** Response carries `next_cursor` (or
  `null`/absent on the last page). Client passes it back as `?cursor=`.
- `limit` is a hint with a server-enforced **max page size** (suggest 1000).
- **Stability:** a ref created/deleted mid-pagination need not appear/disappear
  consistently, but the cursor MUST remain valid (no 4xx) and MUST make forward
  progress — `clw prune` tolerates a slightly-stale snapshot (it re-reads roots
  under a conservative sweep), but it cannot tolerate a cursor that 500s.

### Response schema

```json
{
  "tenant": "<tenant>",
  "refs": [
    {
      "ref_key": "<hex BLAKE3 digest>",
      "domain": "<domain string, if the server stores it>",
      "name": "<plaintext name, if and only if the server stored it>",
      "manifest_hash": "<CAS hash the ref points at>",
      "updated_at": "<RFC3339>"
    }
  ],
  "next_cursor": "<opaque>|null"
}
```

- `ref_key` is **always** present (it is what the client addresses).
- `name` / `domain` are **best-effort**: include them only if the server
  persisted the pre-image at write time. If the server stored *only* the hash,
  return `ref_key` + `manifest_hash` — that is still enough for GC (the manifest
  is the GC root) though not for a human-readable `ls --all`. **Please state in
  the reply whether the pre-image name is recoverable** — it determines whether
  `ls --all` can show names or only opaque keys.
- `manifest_hash` is the **GC root pointer** — `clw prune` follows it into the
  CAS reachability walk. It is the load-bearing field; please ensure it is
  present for every ref.

### Idempotency / semantics

- **Pure read, side-effect-free, safe to retry.** No idempotency key needed.
- Empty tenant → `200` with `"refs": []` (NOT 404 — "no refs" is a valid state,
  distinct from "no such tenant").
- Unknown/unauthorized tenant → `403`/`404` per existing AC convention (state
  which; do not leak existence across tenants).

### clw-side consumer

- `clw ls --all` → page through `GET /v1/refs/:tenant`, render `name` (or
  `ref_key` if no pre-image) + `manifest_hash`.
- `clw prune` → enumerate refs (roots) → for each, fetch its manifest → union of
  reachable CAS hashes = **live set** → sweep CAS (D-8 LIST) → CAS-DELETE (D-8)
  everything not in the live set.
- `clw` erasure flow → enumerate → `clw rm` (D-1) each ref → `clw prune` to GC
  the now-orphaned blobs → assert LIST returns empty.

---

## Confirm with owner

1. **Route name:** `/v1/refs/:tenant` vs `/v1/ac/:tenant` — which fits the
   existing AC route prefix?
2. **Pre-image recoverability:** does the server persist `name`/`domain` (so
   `ls --all` can show human names), or only `ref_key`? Determines the response
   schema and the `ls --all` UX.
3. **Pagination contract:** confirm cursor-based + the max `limit`. Is a stable
   snapshot feasible, or is best-effort-forward-progress the guarantee?
4. **Empty-vs-unknown tenant:** confirm `200 []` for empty and the status for
   unknown/unauthorized (don't leak cross-tenant existence).
5. **Auth scope:** does the existing tenant PAT scope already cover ref-LIST, or
   is a distinct read scope needed? (Ties to the **D-1 single-`scope` enum
   drift** — the PAT must grant AC-read; confirm with Clerk/signup.)
6. **Ordering** with **D-8** (CAS DELETE/LIST): both are required for GC; schedule
   together. Ref-LIST yields roots; CAS-LIST yields the sweep candidates.

## Cross-references

- ROADMAP dependency ledger: `docs/ROADMAP-go-live.md` (D-7), §M1.4, §M6.4.
- Sibling deps: `docs/HANDOFF-cas-delete-list.md` (D-8),
  `docs/HANDOFF-runner-pat-minting.md` (D-9, the two-layer auth model).
- `clw rm` / AC delete: D-1 (`DELETE /v1/ac/...`).
