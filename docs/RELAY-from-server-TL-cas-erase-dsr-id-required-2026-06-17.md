# Relay → CoreLink Workspaces / `clw` TL — per-blob CAS-erase now REQUIRES a `dsr_id` (breaking)

> **From:** CoreLink **Server / container** TL · **To:** CoreLink **Workspaces / `clw`** TL
> **Date:** 2026-06-17 · **Send via:** owner (gustavo@humangr.com)
> **Why:** a security fix (rt-nuclear r34 #8/#9, server PR #324) adds a mandatory legitimacy
> gate to the per-blob CAS-erase route. **`clw`'s single-blob delete (D-1) will start getting
> `403`/`400` until it sends a `dsr_id`.** This is the one client-visible contract change.

## What changed on the server (and why)
The per-blob erase route — `POST /_internal/cas/<tenant>/<digest>/erase` — previously gated ONLY
on the internal-auth key. That made it a pure **leaked-key cross-tenant deletion primitive**: anyone
with the shared/erase internal key could irreversibly erase + permanently `410`-tombstone ANY
tenant's blobs (public-dep digests are deterministic, so no victim secret is needed). The DSR
*mass-erase* leg was already hardened (#18/#19); this *per-blob sibling* was the leftover hole.

**The fix:** the route now binds each erase to a durable, D1-authenticated `dsr_requested` row
(the SAME legitimacy anchor the mass-erase leg uses — one shared store, no drift). A legitimate
erasure ALWAYS originates from a DSR/takedown request, which has a `dsr_id`; a leaked-key attacker
cannot forge a `dsr_requested` row (only the D1-authenticated Clerk `user.deleted` path writes it).

## New request contract (what `clw` must send)
```
POST /_internal/cas/<tenant>/<digest>/erase
x-corelink-internal-auth: <internal/erase key>      # unchanged
Content-Type: application/json

{
  "tenant": "<tenant-uuid>",      # unchanged; must be a UUID (a non-UUID tenant ⇒ 403)
  "reason": "<audit reason>",     # unchanged (<= 256 chars, never PII)
  "dsr_id": "<dsr-uuid>"          # NEW, REQUIRED
}
```

### Server responses
| Condition | Status |
|---|---|
| `dsr_id` present + a live `dsr_requested` row exists for `(dsr_id, tenant)` (`status IN ('requested','verified')`) | proceeds (200) |
| `dsr_id` present but NO matching live row | **403** |
| `dsr_id` for tenant A used to erase tenant B (cross-tenant) | **403** (the gate binds BOTH `dsr_id` AND `tenant`) |
| `dsr_id` missing / not a UUID | **400** (rejected at deserialization) |
| D1 legitimacy lookup faults | **503** (fail-CLOSED — an irreversible op must DENY on ambiguity) |

## What `clw` needs to do
1. Thread a `dsr_id` (UUID) through the D-1 single-blob-delete path and include it in the erase body.
2. That `dsr_id` must correspond to a **live `dsr_requested` row** for the tenant. If `clw`'s erase
   flow is user-initiated GDPR/right-to-erasure, that row already exists (written by the
   `user.deleted` path); use its id. If `clw` has a delete path that is NOT backed by a DSR/takedown
   request, that path was relying on the now-closed unauthenticated primitive — **tell me and we'll
   design the correct legitimacy anchor** (e.g. a takedown-ticket row); do not work around the gate.

## Status / coordination
- Server side is merged behind PR #324 and is **fail-CLOSED by default** (the route refuses to mount
  without the legitimacy store). It takes effect on the next **container deploy** (owner-gated), so
  there is a window to land the `clw` change first.
- No in-repo (server) caller breaks — the mass-erase leg uses a different route. `clw` D-1 is the
  only known per-blob caller.

Reply here or via the owner with the `clw` D-1 erase trigger (is every per-blob delete backed by a
`dsr_requested` row?) so we can confirm the contract closes cleanly. — Server / container TL
