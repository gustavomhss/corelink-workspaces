# REQUEST → server TL: a programmatic signup path for clw E2E persona provisioning

**Date:** 2026-07-19 · **From:** clw TL · **Priority:** MEDIUM (unblocks the story-driven E2E suite)
**Blocks:** the "simulate a real user, indistinguishable from in-house validation" E2E suite for clw.

## Why
We're rebuilding clw's E2E suite as **story-driven user simulations** — each test is a persona
(a real-looking customer) driving the real `clw` binary through a realistic journey. The owner's bar:
**the system must think it's a genuine user, not in-house validation.** For that, each persona should be
provisioned through the **real signup front door** (Clerk → `apps/signup-worker` → `corelink-signup`
atomic D1 provision → first PAT), NOT the `mint-dogfood-pat.sh` back door.

Note (scoping, so we ask for the right thing): clw only talks to the **data plane**
(`corelink-api` = CAS/AC/auth). That plane can't distinguish a minted tenant from a signed-up one — so
the *reason* to want real signup is (a) the owner wants maximum fidelity and (b) it lets us additionally
cover the **signup → first-use pilot** flow end-to-end, which nothing currently tests as one path.

## What we need (any ONE of these works)
A **repeatable, scriptable** way to provision a real test tenant via the actual signup orchestrator and
get back the resulting identity. Concretely, one of:

1. **Clerk test-mode path** — a Clerk *test-instance* backend API key + the signup-worker webhook signing
   secret (or a staging equivalent), so our harness can: create a Clerk test user → fire the real signed
   `POST /v1/signup` webhook → provision. (This exercises the real orchestrator.)
2. **A dedicated E2E signup endpoint** — behind an admin/test credential, runs the **real**
   `SignupOrchestrator` (not fakes) and returns `{ tenant_id, primary_region, first_pat }`.
3. **Expose what `e2e-pilot-onboarding` uses** — if that harness already provisions a real/pilot tenant
   end-to-end, share that path/credential so clw's suite can call it.

### Return contract we need
- `tenant_id` (uuid) + `primary_region` (enam is fine) + the **first PAT** value (the `ShownOnceToken`
  GET result, before it's invalidated) with `read-write` scope.

### Operational asks
- **Repeatable:** a fresh persona per run (or a small pool we can recycle) — the suite runs on demand.
- **Cleanable:** a way to tear down / `erase` the test tenants afterward (or confirmation that GDPR
  `erase` on the tenant is sufficient) so we don't accrete dead tenants.
- **Env:** prod or a prod-like env; `enam` region is fine (we're aware of the `primary_region` hardcode —
  `docs/HANDOFF-signup-region-hardcode.md` — not blocking for enam personas).
- **Cost:** confirm these test tenants won't trip billing/Stripe (or land as `pending_billing_link`
  harmlessly) — we don't want to create real billable customers.

## What clw already does / doesn't need from you
- clw drives the data plane itself once it has `{tenant_id, PAT}` — you don't need to do anything on the
  CAS/AC side.
- If a real-signup path is heavy to expose, tell us the **minimum** you can give (even a manual one-time
  pool of N pre-signed-up persona tenants + PATs) and we'll design the suite around it.

## Contact
Reply to the clw TL via the owner (courier). Doc lives at
`corelink-workspaces/docs/REQUEST-server-TL-e2e-signup-provisioning.md` (main).
