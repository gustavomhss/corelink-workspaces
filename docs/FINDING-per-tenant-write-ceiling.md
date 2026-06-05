# FINDING — per-tenant CAS write throughput ceiling (the real "429")

> **For:** corelink-server architecture owner. **From:** clw dogfood, 2026-06-05.
> **Severity:** medium now / **HIGH for the CI-acceleration product (campaign #1)**.
> Not a launch blocker for the cache-storage product; IS a design question the
> CI/build-acceleration expansion must answer, because its core workload is
> exactly the thing that hits this ceiling.

---

## Correction of an earlier claim

The first incident note (`BUGREPORT-prod-ac-500.md`, "related findings") guessed
the dogfood 429s were **Cloudflare Bot Fight Mode**. **That was wrong** — corrected
here, with evidence. Bot Fight Mode issues challenges/blocks with a `cf-mitigated`
header; our 429s carry `retry-after: 10` and **no** `cf-mitigated`. Lesson logged:
the guess was made mid-incident without capturing headers; this finding is built
on reproduced, header-level + code-level + capacity-level evidence.

---

## What actually happens (reproduced, three independent confirmations)

**Network (controlled bursts against prod, tenant 8a6b4e4e…):**
| Workload | Result |
|---|---|
| 8 parallel **GET** /v1/cas | 8× 404, zero 429 — reads scale fine |
| 8 sequential **PUT** /v1/cas | 8× 201, zero 429 — serial writes fine |
| 8 parallel **PUT** /v1/cas | alternates 201 / **429 `retry-after:10`** / 25s timeout |
| 429 headers | `retry-after: 10`, `server: cloudflare`, **no `cf-mitigated`**, no app header |

**Code (trace by an independent agent):**
- The only app-code 429 on the CAS path is storage-quota
  (`worker/src/index.ts:1314` → `worker/src/lib/quota.ts:153`), and it sets
  `retryAfterSec = secondsUntilNextMonthStart()` ≈ **up to 2,678,400s** — never 10.
  ⟹ our `retry-after:10` is **not** app code.
- `checkRequestQuota` (`quota.ts:205`) is a deferred **stub** (always `ok`).
- `routes/cas.rs` PUT has **no** rate limiter / quota / 429 path.
- The per-tenant DO (`idFromName(tenantId)`, `index.ts:1418`) does **not**
  `blockConcurrencyWhile` the request path — it forwards concurrently — but **all
  of one tenant's traffic funnels through that single DO instance**.

**Capacity (live prod):** container app `max_instances=20`, 7 running, 0.5 vCPU /
4 GiB each. 20 instances would absorb 8 concurrent PUTs trivially ⟹ **not** container
exhaustion. (Note: `wrangler.toml` says `max_instances=5` — **config drift vs the
live value of 20**; worth reconciling separately.)

## Conclusion

The 429 is **Cloudflare platform backpressure on the single per-tenant Durable
Object** under bursty parallel writes — not Bot Fight Mode, not a WAF rule, not app
rate-limiting, not container-instance exhaustion. Every tenant's writes serialize
into one DO funnel; the platform sheds load (429) or queues to timeout when that one
DO is pushed past its per-instance throughput under a burst.

**Why it matters:** parallel per-tenant chunk upload is the *defining* workload of
sccache / bazel / Turborepo / clw — i.e. the CI/build-acceleration product
(`marketing/expansion/ci-build-acceleration.md`). The cache-storage product is fine
(normal clients aren't this bursty); the CI expansion must raise this ceiling first.

## The lucky part

**CAS is content-addressed and immutable.** A PUT needs zero per-tenant
coordination — the key IS the BLAKE3 hash of the bytes; two writers of the same
content write the same key idempotently; there is no ordering, no mutable state,
no conflict. So the per-tenant DO funnel is **architecturally unnecessary for CAS
writes** — it exists for lifecycle/coordination the CAS path doesn't use. That
makes this one of the cheapest possible things to scale.

## Options (owner decision — trade-offs honest)

| # | Option | Effort | Raises ceiling? | Notes |
|---|---|---|---|---|
| A | **clw client: honor `Retry-After` + adaptive concurrency** | S | No (smooths) | clw already retries 429 w/ backoff; add Retry-After honor + back off concurrency on 429. Ship regardless — it's correct client behavior. |
| B | **Shard CAS writes across N sub-DOs by hash** | M | **N×** | `idFromName(tenant + ":" + hashPrefix)`. Safe because CAS is content-addressed (no ordering). Directly multiplies per-tenant write throughput. |
| C | **Bypass the DO for CAS PUT entirely** | M–L | **Large** | Pure CAS writes are stateless/idempotent → Worker→container (or Worker→R2 binding) directly, skipping the DO. Biggest win; biggest change; keep the DO for AC/admin/lifecycle. |
| D | **Document the ceiling, revisit at scale** | XS | No | Acceptable for the cache product launch; explicitly NOT acceptable before selling CI acceleration. |

**Recommendation:** ship **A now** (correct client behavior, helps immediately,
zero server change), and put **B or C on the CI-acceleration campaign's critical
path** (it cannot launch credibly without one of them). Don't gold-plate before the
cache product launches — A + this doc is enough until campaign #1 is greenlit.

## Reproduce

```bash
# auth header in a 0600 file (no secret echoed); then:
EP=https://corelink-api.humangr.com; T=<tenant>
for i in $(seq 1 8); do ( curl -s -o /dev/null -w "%{http_code}\n" -X PUT \
  -H @/tmp/clw-auth.hdr --data-binary "x$i" \
  "$EP/v1/cas/$T/$(printf x$i | python3 -c 'import sys,hashlib;print(hashlib.blake2b(sys.stdin.buffer.read(),digest_size=32).hexdigest())')" ) & done | sort | uniq -c
# bursts intermittently show 429 (retry-after:10) and/or 25s timeouts
```
