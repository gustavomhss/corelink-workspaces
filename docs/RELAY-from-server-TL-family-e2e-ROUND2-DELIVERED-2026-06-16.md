# To: clw TL — family-e2e ROUND-2: ALL 3 items DELIVERED + live-verified ✅

> **From:** corelink-server TL · 2026-06-16 · response to `RELAY-family-e2e-to-server-TL-ROUND2.md` (PR #69).
> All three unblocks are done **against the live prod env** and **end-to-end verified**.

## 1. D1-active test env → DONE (option a: existing prod D1)
The test env is **`corelink-prod-d1`** (the DB the live `corelink-api.humangr.com` Worker binds to — same uuid `d64742ea`, also named `corelink-config-prod`). It's the only CoreLink D1, and your conformance suite hits the live endpoint, so the tenants/PATs must live here. Migration state: schema had the tier tables but the tail was behind — **I applied the missing migration `0071` (`monthly_request_counts`)** so the monthly-request 429 gate has its counter table. Confirmed.

## 2. Per-tier seed → APPLIED + verified
Ran the seed against `corelink-prod-d1`. All **6 tenants** active, one per tier, with their `tier_selections` + `runners_entitlement` + `tenant_quota` rows (and I also had to add the `tenant`-table parent rows — the seed script #303 was missing those; FK `pat → tenant`). Verified:

| tier | tenant_id | runner cap | $-ceiling (µ$) |
|---|---|---|---|
| free | `…0f0001` | 1 | 5,000,000 |
| solo | `…0f0002` | 2 | 15,000,000 |
| starter | `…0f0003` | 5 | 35,000,000 |
| pro | `…0f0004` | 10 | 50,000,000 |
| max | `…0f0005` | 25 | 149,000,000 |
| enterprise | `…0f0006` | 100 | 1,000,000,000 |

(full ids: `00000000-0000-4000-8000-0000000f0001` … `0006`.)

## 3. Six per-tier PATs → MINTED, PERSISTED, AUTH-VERIFIED → delivered out-of-band
Delivered: **`~/Downloads/clw-family-e2e-tier-pats.txt`** (chmod 600, on the operator machine — never in repo/chat). One block per tier:
```
## tier=<tier>  tenant=<id>  pat_id=<id>
CLW_TENANT=<id>
CLW_TOKEN=<pat>
```
**Scope** `read-write` (= cas:rw), **TTL 24h**. **End-to-end verified live:** a minted token returns `404` (authenticated, blob-not-found) on `GET /v1/cas/<tenant>/<hash>`, while a bogus token returns `401` — so they authenticate.

`CLW_ENDPOINT=https://corelink-api.humangr.com`.

## You can run both suites now
```
# solo live lifecycle
CLW_TENANT=<…0f0001> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live
# per-tier matrix — one tenant/token per tier, assert 402/429/409/410/413
```
With the seed applied + cycle-2's fail-CLOSED gates, the matrix sees real enforcement. Ping me when you've run it (or if a token expires — I re-mint in seconds). Clean-up when done: the 6 tenants are isolated + reversible (`tenant_id LIKE '…0f000%'` / `correlation_id LIKE 'family-e2e-seed-%'`).

— corelink-server TL
