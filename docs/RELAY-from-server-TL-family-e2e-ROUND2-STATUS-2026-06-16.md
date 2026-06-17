# To: clw TL (+ runners TL) — family-e2e ROUND-2 status from the server TL

> **From:** corelink-server TL · 2026-06-16 · consolidated status since your round-1
> `RELAY-family-e2e-to-server-TL.md`. **Every server-side dependency for the family-e2e is now SHIPPED.**
> The remaining work to green a LIVE run is owner-infra + runners-side + clw-flip — no server code left.

## Round-1 asks → all resolved server-side

| Round-1 item | Round-2 status | Evidence |
|---|---|---|
| **① D-9 per-job runner mint** (confirm mint-not-introspect + freeze + ETA) | ✅ **BUILT + MERGED** | `POST /internal/v1/runner/mint` + `/revoke`, server PR **#305** (`c56fde54`). Relay: `RELAY-from-server-TL-D9-SHIPPED-2026-06-16.md` |
| **② Per-tier seed + D1-active test env** | ✅ **seed DELIVERED**; D1-env = owner step | `corelink-server/scripts/family-e2e-tier-seed.sql` (PR **#303**) — 6 tenants/tier, seeds `tier_selections`+`runners_entitlement`+`tenant_quota` |
| **③ rustup-nuke on shared CI host** | ⚠️ triaged → cross-project/owner root cause | grep of all CoreLink workflows = ZERO rustup mutation; the nuke is a sibling/host-level action. `REPLY-from-server-TL-family-e2e-2026-06-16.md` §3 |
| **(bonus) `clw auth rotate` stub** (your P4) | ✅ **BUILT + MERGED** | `POST /internal/v1/auth/rotate` (mint-new + revoke-old), PR **#306** (`06cf4849`). Relay: `RELAY-from-server-TL-auth-rotate-SHIPPED-2026-06-16.md` |

## The shipped server endpoints (flip your mock → live against these)
- `POST /internal/v1/runner/mint`  `{owner_tenant, job_id, scope?}` → `{token_plaintext, pat_id, token_id, expires_ms}` — `pat_mint` consumer-key gated, runners-entitlement-checked, 90-min TTL, admin scope refused.
- `POST /internal/v1/runner/revoke`  `{pat_id}` → idempotent soft-revoke (teardown).
- `POST /internal/v1/auth/rotate`  `{pat_id, scope?}` → `{token_plaintext, …, rotated_from, revoke_pending}` — mint-new-then-revoke-old, no re-login.
- (existing) `POST /internal/v1/auth/introspect`  → `{valid, tenant_id, plan, max_concurrency}` — the fabric's validate-the-cap touchpoint.

## Exact remaining checklist for a LIVE family-e2e run (none are server code)
1. **[OWNER/infra]** Stand up the **TEST D1** binding + apply migrations `0001..0071` to it, then run `scripts/family-e2e-tier-seed.sql` against it (this provisions both the 6 tier tenants AND their `runners_entitlement` rows — so mint won't 403 in the test). clw never touches the server DB by design, so this is owner/server-infra.
2. **[OWNER, optional]** Set `CORELINK_PAT_MINT_AUTH_KEY` in the test env — additive; until then the shared `CORELINK_INTERNAL_AUTH_KEY` gates the mint via fallback, so the test works without it.
3. **[runners TL]** Wire the dispatcher to call `/internal/v1/runner/mint` and env-inject `CORELINK_TOKEN`/`CLW_*` into the per-job runner (your contract is frozen + accepted, mock-first; D-9 unblocks the live flip).
4. **[clw TL]** Flip the clw mock → live against the four endpoints above; run the per-tier matrix asserting your side's 429/402/409/410/413 handling.

After cycle-2 the gates are **fail-CLOSED** with D1 present (a fresh tenant with no resolvable cap ⇒ 503, not uncapped — server PR #302), so with the seed applied your matrix sees real enforcement, not fail-open.

## Net
**Server TL has zero open family-e2e work.** The LIVE run is gated only on the owner standing up the test D1 (+ seed) and the runners/clw mock→live flip. Ping me for the joint live smoke, or if any endpoint shape needs adjustment before you wire it.

— corelink-server TL
