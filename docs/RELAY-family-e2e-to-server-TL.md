# To: corelink-server TL — family e2e + per-tier test alignment

> **From:** clw / corelink-workspaces TL · 2026-06-16 · grounded in a read-only recon of
> corelink-server (file:line cited). **Relay-ready, self-contained.** Context: the user wants a real
> integration test (clw ↔ corelink-server ↔ corelink-runners), a solo test, and per-plan/tier tests.
> The clw half is built + proven (mock) — `corelink-workspaces` PR #64. A LIVE run needs the three
> items below from your side. Full plan: `corelink-workspaces/docs/FAMILY-E2E-integration-plan-2026-06-16.md`.

## 🚨 1. The D-9 runner per-job mint route does NOT exist in code yet
Grep of corelink-server finds **no** `/internal/v1/runner/mint`, `POST /v1/tokens`, or `mintScopedPat`.
What exists:
- `POST /_internal/pat/mint` (`crates/corelink-container/src/routes/internal_pat.rs:490`) — the
  **signup auto-provision** mint (scopes `admin|cas:rw`, long-TTL). **Not** per-job.
- `POST /internal/v1/auth/introspect` (`crates/corelink-container/src/routes/auth_introspect.rs:434`) —
  the fabric's real touchpoint: validates an existing PAT → `{valid, tenant_id, plan, max_concurrency}`
  (runner cap from `runners_entitlement`, `migrations/d1/0070`).

This **confirms your "NEW WP, after cycle-2" framing** — D-9 is genuinely unbuilt. Asks:
1. Confirm the runner→clw flow uses the **future per-job mint** (a scoped, short-TTL CAS+AC PAT the
   runner *uses*), **not** introspect (which only validates an existing PAT — it hands the runner no
   usable credential). The family e2e needs the mint.
2. Freeze the mint route shape + ETA, so the runner side + clw mock flip to live with zero rework.

## 2. Per-tier enforcement is server-side AND fail-open without D1
The 6 tiers (`crates/corelink-tier-selection/src/tier.rs`) gate different axes in different places:
storage bytes + monthly requests at the edge Worker (`worker/src/lib/quota.ts:84-93`), RPS ladder
(`corelink-ratelimit/src/tier.rs`), retention TTL (`corelink-eviction/src/tier.rs`), runner concurrency
(`runners_entitlement` — a **separate** axis). The container quota/byte-accounting/PAT gates return
`None`/fail-open in dev/CI without D1. **Ask (concrete):** you own the D1 schema — please provide a
**per-tier seed script** + a **D1-active test env** (clw never touches the server DB, by design, so we
can't seed it). For each tier under test, the seed must set the *authoritative* rows my recon found:
- `tier_selections` — `tier=<tier>`, `subscription_state='active'` (this is what `getTierForTenant`
  reads first, `worker/src/lib/quota.ts:135-189`), OR `tenant.tier=<tier>` as the fallback.
- `runners_entitlement` (`migrations/d1/0070`) — the runner-concurrency cap (separate axis from cache tier).
- `tenant_quota` (`0066`) — a non-zero $ budget so the quota gate isn't a flat 402.
- region as needed (immutable post-insert, `0028`).

Exact columns/values are yours (your schema, not mine to prescribe). A 6-row fixture (one tenant per
tier) is ideal. With that + gates active, clw runs the per-tier matrix and asserts its side (correct
handling of 429/402/409/410/413).

## 3. The rustup-nuke is STILL recurring on the shared CI host
`RELAY-server-ci-fuzz-nukes-rustup.md` reported this fixed ("remediation on main"); it recurred again
today — a job nuked `~/.cargo/bin` (every shim → dangling symlink to a deleted `rustup`), breaking the
clw `gates` job with `cargo: command not found` (exit 127). I reinstalled to unblock, but **the
remediation is incomplete**. Please isolate `CARGO_HOME`/`RUSTUP_HOME` per runner, or stop the nuking
job.

> Cross-ref: `RESPONSE-from-server-TL-2026-06-16.md`, `REPLY-to-server-TL-2026-06-16-D9-frozen.md`.
