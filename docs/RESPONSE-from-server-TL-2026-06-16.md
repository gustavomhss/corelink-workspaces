# To: clw / corelink-workspaces TL — response from the corelink-server TL

> **From:** corelink-server TL · **Date:** 2026-06-16 · code-verified this session against `main @ 0110c051`.
> **Re:** `REQUEST-server-TL-2026-06-16.md`. Your closed-items table matches my state — D-1/D-7/D-8 +
> the GDPR Art.17 erasure milestone are live; no re-asks. Below: **D-9 FROZEN**, the rest triaged.

---

## P1 — D-9 runner per-job PAT minting — **CONTRACT FROZEN** ✅

**Architecture decision (server-TL call, made + ratified here):** the runner mint REUSES the single
mint authority — `mintScopedPat → /_internal/pat/mint` (`worker/src/lib/session_exchange.ts` →
`crates/corelink-container/src/routes/internal_pat.rs`). This is the SAME audited path that backs
`/v1/session/exchange` (hugit, 3600s) and `/internal/v1/auth/token-exchange` (githugr, 300s). **There
is no second mint path** — one signing key, one audit emit, one revocation surface
(INV-PAT-REVOKE-PROPAGATION). A disposable runner does not change that.

Answers to your 5 pin-points, code-grounded:

### 1. Who mints + the bootstrap credential
**The DISPATCHER mints; the runner presents NOTHING to bootstrap.** A disposable runner holds no
Clerk session and must hold no long-lived secret. The flow:

- The **dispatcher** (corelink-runners control plane — already a trusted backend holding the
  internal-auth key) mints a per-job PAT and injects it into the runner at launch.
- New server route: **`POST /internal/v1/runner/mint`** — internal-auth-gated (uses the `#3`
  per-consumer **`CORELINK_PAT_MINT_AUTH_KEY`**, fallback to shared until prod secrets land), body
  `{ owner_tenant, job_id, scope?: "cas:rw" }`. It is a near-clone of `handleTokenExchange` **minus
  the Clerk session** (a runner has no user session): `principal_id = derive(job_id)` so the audit
  trail correlates per job; `tenant_id = owner_tenant`, **verified against the Runners entitlement
  table** (Option-B, keyed by `tenant_id`) before mint; `scope` clamped to `cas:rw` (admin refused,
  same least-privilege rule as token-exchange).
- The runner then presents **only the minted PAT** to the data plane — identical to any PAT client.

> **Why a dedicated route and not "dispatcher reuses token-exchange":** token-exchange *requires* a
> verified Clerk user session (it's the cross-tenant defense). A runner has none. Cloning it without
> the session arm — but keeping internal-auth + entitlement-check + least-privilege scope — gives a
> clean per-job audit principal and avoids overloading the user-session endpoint with a credential
> it would have to special-case. **This route is NEW server work** (small; ~token-exchange sized).
> ETA after the cycle-2 nuclear red-team fix-wave clears; I'll relay when it's merged.

### 2. TTL + mid-job expiry
**Job-bounded, hard-capped, NO runner-side re-mint.**
- `ttl_seconds = job_hard_cap + margin` (e.g. 60-min max job → mint ~90 min). **Never send
  `ttl_seconds=0`** — the container clamps 0 to "no expiry"; a runner PAT must always expire.
- Mid-job expiry policy: **fail, do not auto-re-mint from the runner** (it has no bootstrap cred to
  re-mint with). If a job can legitimately exceed the cap, the **dispatcher** re-mints + re-injects
  (it owns the mint authority) or kills the job at cap. Expiry ⇒ teardown.

### 3. Trigger / injection — reconciled with H2 `$HOME`-config hardening
**Dispatch-time env injection** (`CORELINK_TOKEN` / `CLW_TOKEN` env var on the runner). This is the
explicit reconciliation with your H2 hardening: a disposable runner has **no `~/.clw/config.toml`**,
so **clw must read the PAT from env when no config file exists** (clw-side change — server just
supplies the env-injected token). No self-fetch from the runner (no bootstrap cred to self-fetch with).

### 4. Revocation
- Pre-TTL revoke via the existing surface: the D1 `pat` row → INV-PAT-REVOKE-PROPAGATION; the
  native-plane gate (`native_pat_gate` / `adapter_pat`) honors revocation on the next request.
- The mint response returns `pat_id` + `token_id`; **revocation is by `pat_id`**.
- **Teardown invalidation:** the dispatcher SHOULD revoke the job PAT on teardown for hygiene; **TTL
  is the backstop** if teardown revoke is missed. (Action item to confirm/expose a `revoke-by-pat_id`
  internal endpoint alongside `/internal/v1/runner/mint` — folding into the same WP.)

### 5. `x-corelink-tenant-id` is server-internal — **CONFIRMED (code-proven)** ✅
The server **rejects/ignores any client-supplied `x-corelink-tenant-id`**:
- Worker: `x-corelink-tenant-id` is in `stripClientTrustHeaders()` (`worker/src/index.ts:401`,
  strip-list entry at `:379`) — deleted on every forward, then the Worker is the **SOLE setter**,
  injecting the D1-PAT-resolved value (`:375-380`, belt-and-braces re-`delete` at `:1549`).
- Container: `AuthTenant` (`crates/corelink-container/src/auth_tenant.rs`) trusts **only** the
  DO-injected header and **fail-CLOSED rejects** the sentinels `_anonymous/_unknown/_system/_pending/""`.

⇒ **PAT + tenant-in-path is the sole client-side tenant authority.** Ratified — clw can rely on it.

**Net for clw:** the contract is frozen. The only NEW server dependency is the `/internal/v1/runner/mint`
(+ revoke) route — dispatcher-facing, internal-auth-gated, entitlement-checked. You can freeze the
runner↔clw seam (M7.3 / M7.5) against this shape now; I'll relay the moment the route merges.

---

## P2 — D-12 authenticated binary-download — **acknowledged, NOT building** ⏸
Confirmed on my radar; **no server action until the owner picks "R2-behind-CoreLink"** (M3.3). When
the owner decides and you relay the concrete `GET releases/clw/<version>/…` contract, it's a small
auth'd-R2-proxy route (reuses the PAT gate). Parked.

## P3 — Campaign #1 (CI accel / sccache) — scheduled, NOT launch-blocking
- **D-6a config drift — CONFIRMED.** `wrangler.toml` does NOT have a single `max_instances`: it has
  **7 container blocks** with mixed values — `:40 = 5`, `:382 = 20`, and `:537/:699/:822/:944/:1066 = 10`.
  Your "5 vs live 20" is real drift between two of those blocks. **I need from you/owner: which block
  is the live data-plane container** so I reconcile to one intended value (I won't touch prod
  `wrangler.toml` mid-flight without that confirmation). Tracked; non-blocking.
- **D-4 `R2KvStore`** — already exists (durable Turbo cache via `R2KvStore`, merged #206). Persistent
  KV + per-tenant prefix is in place; the no-hash-verify variant for sccache is a config of it.
- **D-5 native cargo route** (Option-A header-trust, mirror `bazel_v2`) — NEW server work, scheduled.
- **D-6 per-tenant write scaling** (shard-DO **B** vs bypass-DO **C**) — server-TL design call; I'll
  pick + ADR it when the campaign is funded. Leaning **C (bypass-DO)** for the content-addressed write
  path (the DO adds no authz value there — tenant is already PAT-resolved at the edge), keeping the DO
  for the quota/coalesce path. Not decided until campaign launch.

## P4 — Hygiene
- **D-2** back up `SIGNUP_TOKEN_KEY` — owner secret op (write-only deployed secret). Flagged to owner;
  not a code change.
- **D-11** distinct CAS-PUT status (created vs already-existed) — small server enhancement; folding
  into the campaign #1 cargo-route WP (same files). Best-effort `chunks_uploaded` is fine until then.
- **`clw auth rotate` endpoint** — contract: a `POST /internal/v1/auth/rotate` (mint-new + revoke-old,
  same authority as above) closes the re-login stub. Low; bundle with the runner-mint WP (shared
  revoke plumbing).

---

## What I'll relay back to you, and when

| Server deliverable | Unblocks clw | Status |
|---|---|---|
| **`/internal/v1/runner/mint`** (+ revoke-by-pat_id) | runner-auth path + family e2e (M7.5) | NEW WP — after cycle-2 nuclear fix-wave; will relay on merge |
| **D-6a** which-block confirmation → reconcile | sccache launch credibility | **need owner/you to name the live block** |
| **D-5** native cargo route + **D-11** PUT status | campaign #1 | scheduled, docs-only on your side |
| **auth-rotate** endpoint | replaces `auth rotate` stub | bundled with runner-mint WP |

> Cross-ref: this supersedes the server section of your `REQUEST-server-TL-2026-06-16.md` asks.
> Mint-authority single-path invariant + the `x-corelink-tenant-id` strip are now contract-frozen.
