# BUG REPORT — prod: fresh-signup tenant gets HTTP 500 on every /v1/ac/* op

> **Found by:** clw dogfood day-1 (customer #1 = founder) · 2026-06-05
> **Severity: LAUNCH-BLOCKING.** A brand-new self-serve customer cannot use
> any AC-backed feature (action cache, clw refs). CAS works; AC is 100% dead
> for the tenant. **Owner: corelink-server session** (this repo is a pure client).

## Repro (deterministic)

1. Fresh signup via the proven Clerk chain (same as `scripts/e2e-clerk-signup.sh`):
   tenant `8a6b4e4e-5d66-4ab7-9388-85ea5ed45c4c`, clerk user `user_3EivgGP9sapRXNtApIKs2YGegAi`,
   created 2026-06-05. PAT minted and accepted (96 chars).
2. CAS — all good:
   - `PUT /v1/cas/<tenant>/<hash>` → **201**
   - `GET` missing blob → **404** (correct)
3. AC — all broken, any key, GET and PUT:
   - `GET/PUT /v1/ac/<tenant>/<key>` → **500**, body `internal`

## Ruled out

- Tenant row is healthy in `corelink-prod-d1`:
  `primary_region='enam'` (valid per CHECK), `tenant_state='active'`, `tier='free'`.
- NOT the region-code mismatch it first looked like ("dub" in Clerk
  publicMetadata is informational colo, not the routing region).
- Not auth (CAS accepts the same PAT; bad PAT correctly 401s).

## Hypotheses to check (in likely order)

1. **AC envelope-signing key missing for fresh signups** — the AC path signs
   envelopes via the HKDF tenant-key signer; if signup-worker provisions
   tenant+PAT but NOT the tenant signing/derivation key the AC handler
   needs, every AC op would 500 while CAS (global bucket, prefix-at-insert)
   survives.
2. **Macro-region → AC-bucket mapping gap** — tenant carries `enam`, but AC
   buckets are colo-coded (`corelink-ac-{sam,iad,lhr,nrt,syd}`); if a
   regional worker lacks the `enam→iad` mapping (or the binding), AC 500s.
3. Tier-gated dependency absent for `tier='free'` on the AC path.

## Observability gap (secondary finding)

`wrangler tail` on BOTH `prod` and `prod-sam` captured **zero** log/exception
lines for the 500s — the container's error is swallowed before the Worker
edge. Check container-side logs/audit rows; consider surfacing container
errors to tail for prod debugging.

## Related findings from the same dogfood (separate, smaller)

- **Rate limit on parallel chunk PUTs (429):** a whole-repo `clw snapshot`
  (concurrency 8) sustains 429s past an 18s backoff ladder. What is the
  free-tier write rate? clw will add a client-side global throttle; the
  number shapes it. (clw-side mitigation already merged: PR #19.)
- **`SIGNUP_TOKEN_KEY` is NOT in `.env.local`** — exists only as a deployed
  Worker secret (write-only). Violates the secrets-backup rule: if lost,
  it's unrecoverable. Back it up.
- `scripts/admin/mint-pilot-token.sh` header references the DEAD dotted
  hostname (`signup.corelink.humangr.com`); live is flat
  (`corelink-signup.humangr.com`).

## Probe kit (rerun any time)

```bash
source ~/.clw/dogfood.env
curl -sS -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer $CLW_TOKEN" \
  "$CLW_ENDPOINT/v1/ac/$CLW_TENANT/$(printf 'a%.0s' {1..64})"   # expect 500 (bug) / 404 (fixed)
```

---

# ✅ RESOLVED (2026-06-05) — confirmed root cause & fix trail

**Three stacked failures**, each masking the next:

1. **Original D-day R2 S3 token had no AC-bucket scope** (CAS-only era) → rotated.
2. **Replacement token was created read-only** (dashboard default) → owner
   upgraded to Admin Read & Write. (Note: container instances only read env at
   start — a credential change requires instance recycle to take effect.)
3. **THE root cause:** the Worker DO forwards container env as
   `R2_AC_BUCKET: this.env.R2_AC_BUCKET ?? ""` (`worker/src/durable_object.ts:476`)
   with a documented "absent/empty → default" contract — but the Rust side used
   `std::env::var(..).unwrap_or_else(..)`, which only defaults when the var is
   ABSENT; `Ok("")` passes through → S3 client with an EMPTY bucket name →
   `failed to construct request` → `AcHandlerError::Internal` → 500 on every
   `/v1/ac/*` op since the 2026-05-30 prod deploy. Reproduced locally
   byte-for-byte; CAS only survived because the DO does NOT forward
   `R2_CAS_BUCKET` at all (absent → Rust default works).

**Prod hotfix applied (2026-06-05):** `R2_AC_BUCKET=corelink-ac-iad` +
`R2_AC_REGION=iad` added as plain vars on the `corelink-prod` worker via
settings-PATCH (inherit pattern; 19/19 secrets preserved; rehearsed on
`corelink-prod-sam` first) + container rollout. **Code root fix:** corelink-server
PR `fix/container-empty-env-vars` (`storage::env_or` honors empty-as-default in
ac.rs + cas.rs; wrangler.toml codifies the prod vars; cf-deploy-prod.yml
wrangler@4).

**Verified end-to-end after fix:** live conformance 4/4 · snapshot with prod
dedupe (8/48 chunks uploaded) · hydrate 100%-from-cache · roundtrip 0 diffs.

## Related findings (recorded, owners assigned)

- **Burst 429s on parallel writes — CORRECTED diagnosis.** An earlier note here
  guessed "Bot Fight Mode / WAF". **That was wrong.** Header capture + code trace +
  live capacity check (all 2026-06-05) prove the 429 is **Cloudflare platform
  backpressure on the single per-tenant Durable Object** under bursty parallel CAS
  writes — not Bot Fight Mode (no `cf-mitigated`), not app quota (our `retry-after`
  was 10; the app's only 429 uses `secondsUntilNextMonthStart()` ≈ millions), not
  container exhaustion (20 instances live). `checkRequestQuota` is a deferred stub.
  Full analysis + scaling options → **`docs/FINDING-per-tenant-write-ceiling.md`**.
  **Owner decision** belongs to the CI-acceleration campaign, not a WAF tweak.
- **`signup-worker` hardcodes `primary_region='enam'`** for every tenant
  (`apps/signup-worker/src/webhooks/clerk.ts:456`; the colo-derived region param
  is discarded — `configureTenant` is a no-op). Latent multi-region correctness
  bug; NOT touched here (active GAP-5 lane) — route to the signup-worker session.
- `SIGNUP_TOKEN_KEY` had no `.env.local` backup (write-only secret) → rotated
  2026-06-05 + backed up; any previously-minted pilot links are invalidated
  (re-mint with `scripts/admin/mint-pilot-token.sh`).
