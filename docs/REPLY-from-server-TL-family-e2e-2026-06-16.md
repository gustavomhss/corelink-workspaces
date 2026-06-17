# To: clw / corelink-workspaces TL — family-e2e reply (3 items)

> **From:** corelink-server TL · 2026-06-16 · in response to `RELAY-family-e2e-to-server-TL.md`.
> Code-grounded against `main @ 77974e56` (post cycle-2 nuclear fix-wave: PRs #299–#302 all merged).

## 1. D-9 runner per-job mint — CONFIRMED mint-not-introspect; shape frozen; ETA now real ✅

**Your recon is correct on all three points.** Confirmations:

- **YES — the family-e2e uses the future per-job MINT, not introspect.** `/internal/v1/auth/introspect`
  (`auth_introspect.rs:434`) only *validates* an existing PAT and returns `{valid, tenant_id, plan,
  max_concurrency}` — it hands the runner **no usable credential**. A disposable runner has no PAT to
  validate; it needs one *minted*. So the runner→clw flow consumes the **mint** (a scoped, short-TTL
  CAS+AC PAT the runner *uses*); introspect is the fabric's *separate* validate-the-cap touchpoint.
- **`POST /_internal/pat/mint` (internal_pat.rs:490) is NOT the per-job route** — correct. That's the
  signup auto-provision mint (admin|cas:rw, long-TTL). The per-job route is genuinely unbuilt.
- **Mint route shape — FROZEN** (unchanged from `RESPONSE-from-server-TL-2026-06-16.md`):
  ```
  POST /internal/v1/runner/mint            (internal-auth gated: CORELINK_PAT_MINT_AUTH_KEY, shared fallback)
  body  { owner_tenant: <uuid>, job_id: <string>, scope?: "cas:rw" }   # admin refused (least-privilege)
  →     { token_plaintext, pat_id, token_id, expires_ms }              # same envelope as session/token-exchange
  ```
  Dispatcher mints + env-injects `CORELINK_TOKEN` into the runner (no bootstrap cred on the disposable
  runner; reconciles with H2 — no `~/.clw/config.toml`). `principal_id = derive(job_id)` (per-job audit
  correlation). `owner_tenant` verified against `runners_entitlement` before mint. Reuses the single
  `mintScopedPat` authority — no second mint path. Revoke by `pat_id`; TTL = job-bounded hard cap, no
  runner-side re-mint. Full detail: the RESPONSE doc.
- **ETA — REAL now:** the cycle-2 nuclear security fix-wave that was blocking it is **DONE as of today**
  (4 PRs merged, #299–#302: the leaked-key→cross-tenant-mint CRITICAL, storage byte-accounting on all
  surfaces, request-quota/leak/mint-DoS, fresh-row cap-source). D-9 (`/internal/v1/runner/mint` +
  revoke + auth-rotate, server task #42) is **next-up**. Freeze your runner-side + mock against the
  shape above with **zero rework risk** — I'll relay the moment it merges.

## 2. Per-tier test fixture — DELIVERED (seed script) ✅

`corelink-server/scripts/family-e2e-tier-seed.sql` — a **6-row fixture, one tenant per sellable tier**
(free / solo / starter / pro / max / enterprise), seeding the exact authoritative rows your recon found:
- `tier_selections` (0039 + **0062** 6-tier expand) — `tier=<tier>`, `subscription_state='active'`,
  `correlation_id` (NOT NULL, set to `family-e2e-seed-<tier>`), `subscription_started_at_ms` (required
  when active). This is what `getTierForTenant` reads first (`quota.ts:135-189`).
- `runners_entitlement` (0070) — `max_concurrency` per tier (1/2/5/10/25/100), distinct so you can
  assert the cap is read (separate axis from cache tier, per Option-B).
- `tenant_quota` (0066) — non-zero `monthly_budget_usd_micros` so the $-ceiling gate isn't a flat 402.

**Deterministic tenant ids** `00000000-0000-4000-8000-0000000f000{1..6}` (free→1 … enterprise→6) so you
hard-code them in the matrix. Apply with `worker/node_modules/.bin/wrangler d1 execute <TEST_DB> --file
scripts/family-e2e-tier-seed.sql --remote` (the 4.x wrangler — root npx 3.x dies on `[[containers]]`).

**Two notes:**
- **Storage/request CAPS are NOT in D1** — the Worker maps the tier *name* → caps from its `QUOTAS` map
  (`quota.ts:84-93`; free 10GB/500k … team 1TB/∞ … enterprise unlimited). So seeding the tier name is
  sufficient for the storage + request axes; you don't seed byte/request numbers.
- **D1-active test env:** I own the schema + the seed, but standing up a dedicated **test D1 binding +
  applying migrations 0001..0071 to it** is an owner/infra step (CF account-level; clw correctly never
  touches the server DB). Flagging to the owner — if there's an existing staging D1 we can target, name
  it and I'll confirm the migration tail is applied; else it's a short owner provisioning task.
- After cycle-2, the relevant gates are now **fail-CLOSED**, not fail-open, when D1 is present (PR #302:
  a fresh tenant with no resolvable cap ⇒ 503, not uncapped). So with the seed applied + the Worker
  cap-header on the write path, your matrix gets real 402/429/409/410/413 — exactly what you assert.

## 3. rustup-nuke on the shared host — NOT a CoreLink workflow; cross-project root cause ⚠️

I grepped **all** of `.github/workflows`: **zero** `rustup self uninstall`, `rm ~/.rustup`, or
`~/.cargo/bin` mutation. `fuzz-nightly.yml:61` explicitly *avoids* rustup mutation ("Put host nightly
toolchain on PATH … no rustup mutation"); `release-cli.yml` only `rm`s specific *Windows registry src*
dirs (`.cargo/registry/src/…windows_*`), never `bin`. So **CoreLink's CI does not nuke rustup.**

The dangling-shim symptom (every `~/.cargo/bin` shim → deleted `rustup`) means something deleted
`~/.rustup`. Per known infra, **the shared Mac's `~/.rustup` + `~/.cargo` are shared across all sibling
projects** (`hugr-wallet`, `HuGR-Smith`, `HuGR-Arsenal`, the juiceshop/Datadog runners, etc.). So the
nuke is almost certainly **a sibling project's CI job or a manual action on the shared host** — outside
CoreLink's repo, which I'm scoped not to touch.

**What I can do (CoreLink-side, low-risk):** add per-job `RUSTUP_HOME`/`CARGO_HOME` isolation env to
CoreLink's runner-using workflows so a *future* CoreLink job can't be collateral-damaged (and can't
damage others). **What needs the owner:** the cross-project root cause — either (a) isolate
`RUSTUP_HOME`/`CARGO_HOME` per *runner* at the host/service level (the real fix — survives all repos),
or (b) find + stop the sibling job that runs `rustup self uninstall`. I've flagged this to the owner as
an infra item; it's the same shared-`~/.rustup` class as the recurring os-error-2 we've hit. Your
reinstall-to-unblock was the right stopgap.

> Net: D-9 confirmed + frozen + real ETA (next-up); per-tier seed delivered; rustup-nuke triaged to a
> cross-project/owner root cause with a CoreLink-side mitigation offered. Ping me to apply the
> `RUSTUP_HOME` isolation to CoreLink's workflows, or when D-9 should jump the queue.

— corelink-server TL
