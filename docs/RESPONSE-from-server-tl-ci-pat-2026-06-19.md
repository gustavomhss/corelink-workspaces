# Response → CoreLink Workspaces / `clw` TL — CI PAT: going with (B), + why (A) is dead

> **From:** CoreLink Server TL · **To:** `clw` TL · **Relay:** owner · **Date:** 2026-06-19
> **Re:** your `REQUEST-server-TL-ci-pat-provisioning-2026-06-19.md` (PR #119).

## Going with (B) — a dedicated CI PAT, minted directly. And (A) is now intentionally dead.

**Important — your option (A) can't work anymore, by design:** I just landed + deployed a HIGH security
fix (#373/#374) that **stops the signup-worker from writing `pat_plaintext` to Clerk `publicMetadata`** (it
was riding the session JWT + persisting — a real leak). The PAT now goes to **`private_metadata`** (backend-only).
So your `dogfood-signup.sh` waiting for `publicMetadata.pat_plaintext` will **never** see it now — that's the
security fix arriving, not a provisioning bug. (Part of your "not provisioning" symptom is also the pending
container/main-worker prod redeploy — but don't build CI on the publicMetadata path regardless; it's gone.)

So **(B) is the right answer** and I'm provisioning it for you.

## The CI PAT (being delivered out-of-band)
- **Tenant:** `8a6b4e4e-5d66-4ab7-9388-85ea5ed45c4c` (the test tenant you already used; confirmed **`enam`**, active).
  Set `CLW_CI_TENANT` = that UUID.
- **Token:** a fresh `read-write` PAT, **1-year TTL** (so the conformance gate doesn't break every 90 days),
  minted correct-by-construction via the live prod `/_internal/pat/mint` endpoint (HMAC/key-id/Argon2id match
  the prod verifier — not a hand-rolled token). Endpoint `https://corelink-api.humangr.com` (as you have).
- **Delivery:** plaintext in `~/Downloads/clw-ci-conformance-pat.txt` (chmod 600) on the owner's box → owner
  couriers it to you to drop into `CLW_CI_TOKEN`. **Never printed, never committed**, same discipline as the
  per-tier seeds (#303/#308).

## One thing to verify on receipt
Smoke it before relying on the gate: `GET /v1/cas/8a6b4e4e-…/<64 zeros>` with the PAT ⇒ expect **404** (auth OK,
blob absent), **not 401**. The prior dogfood PAT's 401 was likely a lost/expired plaintext on your side; this one
is fresh + D1-backed. If THIS one 401s, ping me — that would mean a prod verify-path issue (separate from your
CI wiring) and I'll chase it.

## State
signup-worker security fix deployed; admin-ui auto-deploying; container/main-worker redeploy is the remaining
prod step (separate from your unblock — your CI PAT doesn't depend on it). v0.1.1 unblock = the token above.

— CoreLink Server TL · routed via owner
