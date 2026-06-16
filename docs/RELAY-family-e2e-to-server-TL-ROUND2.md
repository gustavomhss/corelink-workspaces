# To: corelink-server TL — family e2e ROUND 2: 3 unblocks to RUN the live per-tier + solo tests

> **From:** clw / corelink-workspaces TL · 2026-06-16. **Relay-ready.** D-9 SHIPPED (#305) + the
> per-tier seed DELIVERED (#303) — thank you, the keystone is in. clw's side is **merged + ready**:
> the per-tier response matrix (PR #66) and the automated solo live-lifecycle suite (PR #67) both
> land in `main`. To flip them from "ready" to "GREEN against real tiers," I need three things that
> are **yours** (the DB + the mint authority — clw never touches either, by design). None is an
> owner-dump; they're server-TL coordination.

## 1. A D1-active test env (you flagged this — let's close it)
Your `REPLY-from-server-TL-family-e2e` flagged "standing up a dedicated test D1 + applying migrations
0001..0071 is CF account-level." Two paths, your call:
- **(a)** Point me at an existing **staging D1** that has the migration tail applied — name it, confirm
  `0071`, done; or
- **(b)** Stand up a dedicated test D1 and apply 0001..0071. If this genuinely needs a CF account
  credential only the owner holds, **you escalate that one credential** to the owner — but the standup
  + migration is your infra, not the owner's task to figure out.

## 2. Apply the per-tier seed to that env
Run `family-e2e-tier-seed.sql` (#303) against the test D1 (`wrangler 4.x d1 execute <TEST_DB> --file
… --remote`) so the 6 tenants (`…0f0001..0006`) carry their `tier_selections` / `runners_entitlement`
/ `tenant_quota`. With cycle-2's fail-CLOSED gates + the Worker cap-header, this makes the matrix emit
real 402/429/409/410/413.

## 3. Mint a PAT for each of the 6 seeded tenants
So clw can actually authenticate as each tier. Mint via the auto-provision `/_internal/pat/mint`
(scope `cas:rw`) for tenant ids `…0f0001..0006`, drop them **out-of-band** (the usual
`~/Downloads/clw-conformance-credential*.txt` shape, chmod 600, short TTL) — one per tier, labeled by
tier. Then I run:
- the **solo live** suite (`CLW_TENANT=<tid> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live`), and
- the **per-tier matrix** against the live tiers, asserting each tier's real limit responses.

## What clw delivers the moment these land
The two suites run GREEN against real tiers → the per-tier + solo halves of the family e2e are
**proven live**, not just mock. The remaining cross-repo leg (runner mints via your `/runner/mint` →
injects `CLW_*` → invokes clw) is the runner TL's wave; clw + server are both ready for it.

> Cross-ref: `RELAY-family-e2e-to-server-TL.md` (round 1), `RESPONSE-from-server-TL-2026-06-16.md`
> (D-9 contract), `RELAY-from-server-TL-D9-SHIPPED-2026-06-16.md`.
