> ⚠️ **SUPERSEDED (2026-06-16) by `docs/REQUESTS-to-other-TLs-2026-06-16.md`.** The P1 ask below
> (ratify D-1/D-7/D-8) and both P2 CONFIRMs are now **CLOSED** — endpoints deployed (server #269),
> live-conformance 8/8 green, binary e2e against live prod, CAS+AC scope proven. Read the dated
> refresh for the current open asks; this file is kept for history only.

# Cross-TL requests — relay-ready asks from the `clw` (corelink-workspaces) TL

> **Purpose:** the `clw` client side is complete + audited; the remaining go-live work is
> cross-repo. These are the **precise, prioritized requests** to relay to the other TLs.
> Full specs: `docs/HANDOFF-corelink-server-track.md` + the per-item `docs/HANDOFF-*.md`.
>
> **Key leverage:** the server TL does **not** need to implement first — **ratifying the API
> contract** (shapes below) lets `clw` build + mock-test the client side immediately, so the
> moment the server endpoint lands it integrates with zero rework.

---

## TO: corelink-server TL

### P1 — Ratify 3 API contracts (unblocks a whole `clw` milestone: `rm`/`prune`/`ls --all` + GDPR erasure)

Please **ratify (or adjust) these shapes** — `clw` will build the client against the ratified version.

1. **D-1 — `DELETE /v1/ac/{tenant}/{key}`** (delete a ref).
   Proposed: Bearer PAT; tenant-in-path authoritative (cross-tenant → 403); **idempotent** (absent key → `204` *or* `404`, please pick one — `clw` tolerates both but the conformance suite will assert the chosen one). Unblocks **`clw rm`**.
2. **D-7 — ref enumeration: `GET /v1/ac/{tenant}`** (or `/v1/refs/{tenant}`).
   Returns the tenant's ref keys/records, **paginated** (cursor or page). Needed because `ref_key = BLAKE3(domain‖name)` is one-way and the client keeps no name→key index. Unblocks **`clw ls --all`** + the live-root-set for **`prune`** + GDPR erasure.
3. **D-8 — CAS delete + list: `DELETE /v1/cas/{tenant}/{hash}` + `GET /v1/cas/{tenant}`** (list blobs, paginated).
   Unblocks **`clw prune`** (mark-and-sweep GC over the tenant's CAS) and the **GDPR Art.17** path (actually delete a tenant's uploaded data — today impossible; a launch/compliance blocker for a product that sells regional residency).

> Once P1 is ratified, `clw` adds `AcTransport::delete` / `CasTransport::{delete,list}` (frozen-trait additions, lead-owned) + the `rm`/`prune`/`ls --all` commands + the erasure GC, all mock-tested, gated live by the conformance suite when the endpoints deploy.

### P2 — Two CONFIRMs (small answers, unblock scoping)

4. **Region routing (a/b/c)** — does the single global host `corelink-api.humangr.com` **region-route internally** (Cloudflare) so `clw` needs **no** change? Or must `clw` select a regional host (→ `clw` needs a `region→endpoint` map **and** a `primary_region` lookup API = a new dep). One answer settles M7.1.
5. **Clerk PAT scope** — does a fresh signup PAT grant **both** CAS **and** AC scope? (the live D1 `pat` table has a single-`scope` enum — it may not.) Gates the family end-to-end.

### P3 — Campaign #1 (CI acceleration / sccache) — scheduled, NOT blocking the cache-storage launch
6. **D-4 `R2KvStore`** (persistent KV, per-tenant prefix, no hash-verify) → **D-5 native cargo route** (Option-A header-trust, mirror `bazel_v2`) → **D-6 per-tenant write scaling** (shard-DO **B** or bypass-DO **C**). Do **D-6a first**: reconcile `wrangler.toml max_instances=5` vs live `20`. `clw`/sccache is **docs-only** (sccache speaks its own protocol to the route; `clw` is not in that data path).
7. **D-9 — per-job runner PAT minting** (scope/TTL/trigger) — also needed by the runners TL below.

### P4 — Hygiene / security
8. **D-2** — back up `SIGNUP_TOKEN_KEY` (currently write-only deployed secret; unrecoverable if lost).
9. **`clw auth rotate`** — a token-rotation endpoint contract (when scoped) closes the one shipped client stub.

---

## TO: CoreLink runners TL

Freeze the **runner ↔ clw contract** (no artifact exists yet):
- Does the runner **shell out to the `clw` binary** or **link `clw` as a library**?
- Where does a **disposable** runner get its config + PAT? (it has no `~/.clw/config.toml`; a per-job injected secret — coordinate with server **D-9**.)
- What **ref-naming** convention does the runner snapshot back to (ownership / collision with user refs)?
- Auth: the runner authenticates with a **scoped tenant PAT** (`Authorization: Bearer` + tenant-in-path) — `x-corelink-tenant-id` is **server-internal**, never set by clw/runner.

## TO: hugit / githugr TL (if in scope)

Define the seam (design-first): does **githugr** trigger a `clw snapshot`/`run` on push? Does **hugit** consume `clw` refs for portable workspaces? No contract exists today.

---

## What `clw` does the moment each lands
| You relay + they ratify | `clw` immediately builds (client-side, mock-tested, integration-ready) |
|---|---|
| D-1 | `clw rm` |
| D-7 | `clw ls --all` + ref-enumeration for prune/erasure |
| D-8 | `clw prune` (CAS GC) + **GDPR Art.17 erasure** |
| Region answer (a/b/c) | M7.1 — either nothing (a/c) or a `region→endpoint` resolver (b) |
| Clerk scope confirm | the family e2e smoke can go green |
| Runner contract | the runner↔clw seam + e2e |
