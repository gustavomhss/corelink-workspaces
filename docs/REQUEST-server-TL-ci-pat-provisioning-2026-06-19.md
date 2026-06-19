# Request → CoreLink Server TL — CI conformance PAT (signup-worker not provisioning)

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Server** TL
> **Date:** 2026-06-19 · **Forward via:** owner (Gustavo)
> **Priority:** blocks the `clw` **v0.1.1** release (see Impact).

## What I need (one of two — your call)
A **valid, dedicated CI conformance PAT** (test/dogfood tenant, `enam`) for the `clw` release
pipeline's live-conformance gate. Either:
- **(A)** fix the prod signup-worker provisioning so `scripts/dogfood-signup.sh` completes, **or**
- **(B)** hand me a **seeded CI test PAT + tenant id** directly (like the per-tier seed PATs in
  #303/#308) and I wire it into the GitHub Actions secret myself — no signup-worker needed.

Either unblocks me. (B) is probably faster if the worker needs real work.

## Evidence — signup-worker is NOT provisioning (prod)
Ran the proven dogfood chain **twice** against prod for a fresh CI user; the Clerk user is created,
but the signup-worker never writes `tenant_id` + `pat_plaintext` to Clerk `publicMetadata` within 90s:
```
[1/4] user created (REAL/permanent): user_3FMQqSzxXyXifGJAc04aGNdYexN  (email clw-ci@humangr.com)
[2/4] aguardando signup-worker provisionar tenant + PAT (ate 90s)
ERRO: tenant/PAT nao provisionados em 90s   ← both runs (create + retry-on-existing-user)
```
- Path exercised: Clerk admin `users` create → `user.created` webhook → signup-worker → tenant+PAT in
  D1 → `pat_plaintext` in Clerk publicMetadata (mirrors `corelink-server/scripts/e2e-clerk-signup.sh`).
- The existing dogfood PAT (tenant `8a6b4e4e-…`, from the 2026-06-15 manual 8/8 run) is now **expired**
  — a direct `GET /v1/cas/{tenant}/{0…0}` returns **401** (was 404). So I can't reuse it either.

## Likely cause (your domain — please confirm)
- The **prod Worker redeploy to activate #307 (mint)** that's been pending — if the provisioning path
  isn't live in prod, a new signup never provisions, which is exactly this symptom. **OR**
- The `user.created` webhook doesn't fire / isn't handled for **Clerk-admin-API-created** users (vs the
  real signup flow). The 45 live `enam` tenants were real signups; this is an admin-create.
- Please check `wrangler tail` on the signup-worker for `user_3FMQqSzxXyXifGJAc04aGNdYexN`.

## Impact (why it blocks v0.1.1)
The post-launch audit remediation hardened the release gate: **live conformance is now ship-blocking for
real (non-rc) tags** and requires `CLW_CI_TOKEN`. v0.1.1 (which ships the 20-finding audit remediation,
already merged to `main`) **cannot be tagged** until a valid CI PAT exists. v0.1.0 (live) is unaffected.

## What's already done on my side
- All 3 CI secrets (`CLW_CI_ENDPOINT/TENANT/TOKEN`) are wired into the repo — I just need a **valid token
  value** to drop in (I set them without ever printing the value; I'll do the same with whatever you give).
- Endpoint confirmed: `https://corelink-api.humangr.com`. Region: `enam` (US-only, per the signup-region confirm).

— CoreLink Workspaces / `clw` TL
