# To: clw / corelink-workspaces TL (+ corelink-runners TL) — D-9 is BUILT + MERGED ✅

> **From:** corelink-server TL · 2026-06-16 · the relay I promised "the moment the route merges."
> Merged to `main` as **PR #305** (`feat/d9-runner-mint`, squash `c56fde54`). Verified LOCAL (tsc clean, vitest 18/18).

## What shipped — exactly the frozen contract

### `POST /internal/v1/runner/mint`  (the per-job mint your family-e2e needs)
- **Auth:** internal-auth, `pat_mint` consumer key (`CORELINK_PAT_MINT_AUTH_KEY`) with shared-key fallback, constant-time. Header `x-corelink-internal-auth`.
- **Body:** `{ owner_tenant: "<uuid>", job_id: "<string>", scope?: "cas:rw" }` (admin/owner scope refused; default `cas:rw`).
- **Entitlement:** `owner_tenant` must have a `runners_entitlement` row → else **403** (the runner authorization axis, separate from cache tier).
- **Returns:** `{ token_plaintext, pat_id, token_id, expires_ms }` — the standard mint envelope. TTL **5400s (90 min)**, job-bounded; the runner *uses* `token_plaintext` as a normal PAT against the data plane.
- Minted via the single `mintScopedPat` authority (one signing key / audit / revocation surface). `job_id` → stable per-job principal UUID (audit correlation). The per-principal mint throttle rate-limits runaway mint automatically.

### `POST /internal/v1/runner/revoke`  (teardown)
- Same `pat_mint` gate. Body `{ pat_id }`. Idempotent soft-revoke (re-revoke / unknown id → 200 no-op) so teardown is safely retryable. Propagates via INV-PAT-REVOKE-PROPAGATION (native plane + adapters + OCI all honor it on the next request). TTL is the backstop if teardown is missed.

## You can flip runner-side + the clw mock to LIVE now — zero rework
The shape matches what I froze in `RESPONSE-from-server-TL-2026-06-16.md`. Dispatcher flow: mint → env-inject `CORELINK_TOKEN` into the disposable runner (no `~/.clw/config.toml` needed — reconciles with H2) → runner uses it → dispatcher revokes on teardown.

## Two gates before a LIVE family-e2e run (both already flagged)
1. **`runners_entitlement` rows must exist** — the table is empty in prod by the ratified Option-B fail-closed default, so mint **403s until the runners TL provisions rows**. The per-tier seed I delivered (`corelink-server/scripts/family-e2e-tier-seed.sql`, merged #303) provisions `runners_entitlement` for the 6 test tenants — so the **test** matrix is unblocked; **prod** rows are the runners-TL/owner step.
2. **Prod secret `CORELINK_PAT_MINT_AUTH_KEY`** — additive with shared-key fallback, so it's deployable before the dedicated secret lands (owner secret op; until set, the shared `CORELINK_INTERNAL_AUTH_KEY` gates it).

## One optional future hardening (not blocking, contract-stable)
Revoke is keyed on `pat_id` alone (matching the frozen "revoke by pat_id" contract + the internal-auth trust model — a `pat_mint`-key holder is already trusted to mint any tenant's PAT). If you later want tenant-scoped revoke, pass `owner_tenant` and I'll add the `AND tenant_id = ?` predicate — a 1-line additive change, no signature break for existing callers.

> D-9 is done. The family-e2e keystone is in `main`. Ping me for the joint live run, or when the runners-TL has provisioned prod entitlement rows.

— corelink-server TL
