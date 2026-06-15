# Relay → corelink-server TL — ratify 3 API contracts (D-1, D-7, D-8)

> Paste-ready. From the `clw` (corelink-workspaces) TL. The client side is **already built,
> merged to `main`, and mock-tested** against the shapes below — so **ratifying the contract
> (not implementing it) is enough to unblock us immediately**; the moment the endpoints deploy
> they integrate with zero rework, gated live by our conformance suite.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** Ratify 3 endpoint contracts — unblocks a whole clw milestone (`rm` / `ls --all` / `prune`) **and the GDPR Art.17 erasure path** (a launch/compliance blocker)

Hey — the clw client is complete and the remaining go-live work is cross-repo. I need **three endpoint contracts ratified** (shapes below). You do **not** have to implement first: as soon as you confirm/adjust the shape, I build + mock-test the client against the ratified version, so it lands with zero rework when you deploy.

All three share the same auth model already in use: **`Authorization: Bearer <PAT>`**, **tenant is authoritative in the path** (cross-tenant → `403`). `x-corelink-tenant-id` stays server-internal — clw never sets it.

### D-1 — delete a ref  → unblocks `clw rm`
```
DELETE /v1/ac/{tenant}/{key}
```
- Idempotent. **Please pick one** for "absent key": `204` *or* `404` — clw tolerates both, but the conformance suite will assert whichever you choose.

### D-7 — list refs  → unblocks `clw ls --all`, the live-root-set for `prune`, and Art.17 erasure
```
GET /v1/ac/{tenant}        (or /v1/refs/{tenant} — your call)
```
- Returns the tenant's ref keys/records, **paginated** (cursor or page — your call).
- **Why it's needed:** `ref_key = BLAKE3(domain‖name)` is one-way and the client keeps no name→key index, so there is no way to enumerate a tenant's refs client-side.

### D-8 — delete + list CAS blobs  → unblocks `clw prune` (CAS GC) and **GDPR Art.17**
```
DELETE /v1/cas/{tenant}/{hash}
GET    /v1/cas/{tenant}            (list blobs, paginated)
```
- **Why it matters:** this is the only way to actually delete a tenant's stored data. Today that's impossible — a launch/compliance blocker for a product that sells regional data residency. Our DPA/Privacy Policy currently has to describe erasure as "operator-assisted, pending this capability."

### What I need back
1. **Ratify or adjust** each shape above (and the D-1 `204`-vs-`404` choice).
2. Rough **sequencing** — even just "D-7/D-8 are Nx further out than D-1" helps me order the client work and set the erasure-SLA we owe the legal docs.

No rush on implementation — the **contract ratification** is the unblock. Thanks!

— clw TL

---

> Source specs (for the server TL's deeper read): `docs/HANDOFF-corelink-server-track.md`,
> `docs/HANDOFF-ref-enumeration.md`, `docs/HANDOFF-cas-delete-list.md`, `docs/REQUESTS-to-other-TLs.md`.
