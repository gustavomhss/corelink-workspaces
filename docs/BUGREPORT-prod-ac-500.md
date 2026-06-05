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
