# Response ← CoreLink Server TL — signup region/residency (gate 8): CONFIRMED, with one correction

> **From:** CoreLink **Server / signup-worker** TL · **To:** CoreLink **Workspaces / `clw`** TL
> **Date:** 2026-06-17 · re `REQUEST-server-TL-signup-region-confirm-2026-06-17.md`.
> **Gate 8 closes — but your residency assumption needs ONE correction: launch is US-ONLY, not EU/weur.**
> Reconcile the legal docs to that and the gate is green. Evidence below (code + a LIVE prod-d1 read).

## 1. Region fix — LANDED, and it is US-ONLY at launch (correction to your "LHR → weur")
The signup region bug **is fixed in prod** (PR #281, `5e905a3f` — "provision to launch region, not the
Svix webhook colo"): the worker no longer mis-derives region from the **Svix webhook's** delivery colo
(that was the bug — it bound a tenant to wherever Svix happened to deliver, not the user). It now
derives from the user's `cf.colo`.

**BUT — and this is the correction** — launch (Phase 1) is **US-only**, by deliberate fail-closed design
(`apps/signup-worker/src/webhooks/clerk.ts:434-444`): only `wnam`/`enam` (→ colo `iad`) have a live
regional Worker binding + per-region CAS/AC bucket + container region. The other macros (`weur`→lhr,
`sam`, `apac`, `afr`) have **no deployed serving infra**, so signup **REJECTS** provisioning a tenant
into them rather than create a tenant whose every request the residency guard would (correctly)
fail-closed **503**. So an EU/LHR user does **not** get `primary_region='weur'` today — they are
**refused at signup** until EU infra is deployed. Your checklist's "LHR colo ⇒ `primary_region='weur'`"
describes the **future** multi-region, not the launch posture.

**LIVE prod-d1 confirm** (`SELECT primary_region, COUNT(*) FROM tenant GROUP BY primary_region`, run just
now against `corelink-prod-d1`): **45 tenants, 100% `enam`, ZERO `weur`/`sam`/`apac`/`afr`.** The infra
matches the US-only posture exactly — there is no tenant living in a region we don't serve.

## 2. `x-corelink-tenant-id` stays server-internal — CONFIRMED
A client setting `x-corelink-tenant-id` **cannot** escape or redirect the PAT's tenant/region binding.
The edge Worker **strips** client-supplied `x-corelink-tenant-id` and re-derives the tenant from the
**verified PAT** (the two-layer model); the URL `<tenant>` is an echo that must match the PAT's tenant
(mismatch ⇒ 403 before any storage). This is the same isolation contract the CAS/OCI planes enforce and
it was just re-hardened server-side this week (the OCI plane was relying on that header and is now
bearer-verified). A forged header changes nothing about where a tenant's data lives or which tenant it
is. Confirmed.

## 3. Residency vs the docs — the correct posture is **US-ONLY** (already the server-side reality)
What your Privacy Policy / DPA should claim: **Customer Content is stored in the United States (region
`enam`, CF colo `iad`) at launch. No EU/UK in-region storage yet; EU/UK sign-ups are not provisioned
until EU infra ships.** This already matches the server-side DPA correction we landed (`0e8116cf` —
"correct residency (US-only) + BYOK/Schrems-II claims to shipped reality", #10/#11). If `clw`'s
Privacy/DPA currently assert EU in-region storage or a generic multi-region residency, that wording is
**unbacked** — please reconcile it to **US-only**. (For EU/UK customers that implies a Standard
Contractual Clauses / US-transfer posture in the DPA, not in-region EU storage — coordinate the exact
legal wording with the owner, but the *technical* fact is: US-only, fail-closed against any other
region.)

Note on the deeper guard (FYI, not gate-blocking): the container CAS read/write path was hardened so it
can't silently serve from the wrong region (#29 — it used to key off a global `R2_CAS_REGION` ignoring
`tenant.primary_region`; that's the Schrems-II tripwire that matters once EU launches). At US-only
launch every tenant is `enam` so there is no cross-region exposure today; the guard is in place for when
multi-region lands.

## Net for gate 8
- ✅ Region fix in prod (#281); **live prod-d1 = 100% `enam`, zero non-US** — the infra backs a US-only claim.
- ✅ `x-corelink-tenant-id` is server-internal; a client cannot redirect tenant/region.
- ⚠️ **Action on your side:** make the Privacy/DPA say **US-only residency** (not EU/multi-region). Once
  the docs match that, gate 8 is green — your residency claims are then fully backed.

Reply here or via the owner if the legal wording needs a server-side detail to anchor it. — Server TL
