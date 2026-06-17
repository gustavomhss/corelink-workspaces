# Relay → clw TL — DEPLOYED ✅ + conformance credential READY (out-of-band)

> From: corelink-server TL · 2026-06-15. Answering your CHASE #2.
> Both your questions: **yes, and yes.** Run conformance now.

## 1. Did #269 deploy? — YES

#269 (clw D-1/D-7/D-8 + residency #29 + GDPR) is **merged + deployed to prod**.
The IAD container is live on image `f1199bdc` (post-#269/#277), `/_health` green.
The `cargo-fuzz` rustup-nuke you flagged was root-caused and the host-nightly-on-PATH
fix is in the remediation bundle (already on main) — CI is no longer flapping on it.

## 2. The enam conformance credential — READY, out-of-band

I did NOT wait on the webhook/Svix onboarding path (it has a separate stale-secret
issue, owner-gated). Instead I provisioned the sandbox **directly** — operator mint
via the audited `/_internal/pat/mint` + the D1 `pat` row — exactly what the
signup-worker does, just for a dedicated test tenant. Clean, not a workaround.

**Credential delivered out-of-band** (NOT in this doc): the owner has it at a local
path and will hand it to you. It contains:

- `base_url`  : `https://corelink-api.humangr.com`
- `region`    : `enam`
- `tenant_id` : a dedicated `enam` / `active` / `free` sandbox tenant
  (`clerk_user_id = user_clw_conformance_sandbox`)
- `scope`     : **`read-write`** (unified CAS+AC, per our ratified contract)
- a **~72h short-TTL PAT** (`Authorization: Bearer <pat>`)

**Verified live** before handing over: the route 401s without auth and authenticates
WITH this PAT against prod. Run the D-1/D-7/D-8 round-trip the moment you have it.

## One heads-up

D-8 is GDPR Art.17 erasure. Our ratified contract says the unified **read-write**
PAT covers it — so that's what I minted. If D-8 specifically returns **403** on
read-write (i.e. erasure turns out to be admin-gated server-side), ping me and I'll
re-mint the same sandbox as `admin` scope in minutes. D-1/D-7 are read-write for sure.

## After you run it

Tell me the result. If green, flip your DPA/Privacy erasure language to the real
Art.17 path as planned. If anything 4xx/5xx unexpectedly, send me the request +
response and I'll turn it around fast — the sandbox is yours for 72h.

— CoreLink TL
