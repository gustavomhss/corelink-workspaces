# Action requested → CoreLink Server TL — reply needed (clw go-live)

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Server / API** TL
> **Date:** 2026-06-17 · **Send via:** owner (gustavo@humangr.com)
> **Status:** **Reply requested.** This is the consolidated list of items where `clw`'s go-live is
> waiting on a **server-side answer**. Everything the server-TL previously committed (D-9 runner mint
> #305/#307, `auth rotate` endpoint #306, per-tier seed #303/#308, enam conformance credential,
> rustup-nuke fix #312) is **delivered and acknowledged with thanks** — nothing below re-opens those.

There is exactly **one blocking ask** (item 1) and **one planning ask** (item 2). A short reply on
each closes the clw↔server seam from our side.

---

## 1. 🔴 BLOCKING — confirm the signup-worker region/residency posture (clw exit-gate 8)

clw's erasure half of gate 8 is **live and conformance-green** (`clw erase`). The **residency/region
half is entirely server-side** and clw cannot verify it from the client. clw's published
Privacy Policy / DPA assert a region/residency posture for Customer Content; if the signup-worker
routing is wrong, those claims are unbacked — a gate-8 failure.

**Please confirm (or correct so I can reconcile the docs):**
- (a) The signup-worker region fix is **live in prod** — tenants are provisioned/stored in the correct
  region, and a non-NA colo (e.g. LHR) resolves to the intended primary region (the launch checklist
  notes a `wrangler d1 execute` check expecting `primary_region='weur'` for an EU colo — confirm the
  live value).
- (b) `x-corelink-tenant-id` stays **server-internal** — a client setting it cannot escape or redirect
  the PAT's tenant/region binding (two-layer model, `docs/HANDOFF-runner-pat-minting.md`).
- (c) What clw's Privacy/DPA claim about **where Customer Content lives** matches what the infra does —
  or tell me the correct posture and I will fold it into the legal docs.

*(Full detail: `docs/REQUEST-server-TL-signup-region-confirm-2026-06-17.md`.)*

## 2. 🟡 PLANNING (no action yet) — the D-12 authenticated download route

The owner is deciding clw's distribution surface (a Homebrew tap and/or an authenticated R2 route are
the candidates). **No action needed until the owner picks the surface** — this is a heads-up so you have
context. **If** the choice is the authenticated R2-behind-CoreLink-auth route (D-12), the question we'll
need answered then is: what is the **auth contract** for that download route (how does an authenticated
client fetch `releases/clw/<version>/<asset>` — same Bearer-PAT, a separate download token, or a signed
URL)? Hold this until the owner confirms the surface; I'll re-ping with the concrete ask if D-12 is chosen.

---

**Not in scope here (deliberately):** the family-e2e runner-invocation path / live runner-mint
(`#307` redeploy) — that coordination is parked on the clw side until clw is fully release-ready, per
owner direction; I will open it as its own relay when ready, not now.

**Requested:** a short reply on item 1 (and ack of item 2 as noted-for-later). Reply via the owner.

— CoreLink Workspaces / `clw` TL
