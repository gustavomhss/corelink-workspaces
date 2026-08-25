# FINDING → server TL: every authenticated request costs ~1 second (PAT verification)

> ## ⚠️ SUPERSEDED (partially)
> Edge PAT verification was fixed: the Worker's `auth` phase now measures 5-6 ms, KV-served
> (`Server-Timing: auth;dur=5;desc="kv"`), measured 2026-08-24 in production.
>
> This finding measured tenant `…0000000f0006`, which is on the **enterprise** tier. Enterprise is
> uncapped for requests and unlimited for storage, so that tenant skips quota work other tenants pay.
> A latency measurement that does not state the tenant's tier is not reproducible.
>
> The finding is **not** fully refuted. It blamed PAT verification; the EDGE half was fixed, but the
> CONTAINER still re-verifies the PAT on every request — including Argon2id — and that work has never
> been measured. It sits inside the container's unattributed `oother` residue, which measures
> 172-194 ms in production. Instrumentation to split it is in flight.
>
> No number is claimed for the container's Argon2id cost. It has not been measured.

**Date:** 2026-07-19 · **Reporter:** clw TL · **Severity:** HIGH (dominates all clw command latency)
**Backend:** `corelink-api.humangr.com` · **Tenant measured:** f0006 (`…0000000f0006`, enterprise)

## The finding
Every **authenticated** request to the API takes **~1.0–1.3 seconds** server-side, while an
**unauthenticated** (401) request to the *same endpoint* returns in **~40–75 ms**. The ~1s delta is
per-request work gated behind auth — the shape points at **PAT verification that is not cached**
(e.g. an Argon2/bcrypt verify or an un-cached DB lookup on every call).

Because clw is a chatty content-addressed client (a snapshot is ~6–8 sequential requests; a listing is
1 + N), this ~1s/request tax is the **dominant** latency in every command.

## Evidence (real prod, reproducible)
Same endpoint `GET /v1/ac/{f0006}?limit=1000`, back-to-back:

| Request | Auth | Wall time |
|---|---|---|
| `curl` with a **valid** PAT | ✅ | **1.34 s**, then **1.09 s** |
| `curl` with `Authorization: Bearer badtoken` | ❌ 401 | **0.073 s** |

`clw -vv snapshot` (1-byte file, 1 chunk) wire trace — HTTP/2, **connection pooled/reused** (so this is
NOT TLS/connect overhead), consecutive request→response gaps:

```
req→resp: 1.37s · 1.12s · 1.01s · 1.23s ...   (~6-7 requests) → total snapshot ≈ 9.3 s for 1 byte
```

Derived command latencies (f0006):
- **1-byte snapshot: ~9.3 s** (≈7 sequential authenticated requests).
- **`ls --all` (46 keys): was ~50 s** — 1 list + 46 record fetches. clw side now fetches concurrently
  (PR #221) → **~9 s**; the residual is this ~1s/request × the Governor's in-flight cap.

## Ask (server side)
1. **Cache PAT verification** (per-token, short TTL) so a valid token is verified once, not per request —
   this alone should drop authenticated requests from ~1s toward the ~40–75 ms floor.
2. **Secondary (removes the N+1 for listings):** have `GET /v1/ac/{tenant}` optionally return the ref
   **record** (or at least the name) inline, not just the key hash, so `ls --all` needs 1 request, not 1+N.

## What clw already did
PR #221 parallelises the listing's record fetches (50 s → ~9 s) so the client no longer *multiplies* the
per-request cost. The remaining ~1s/request is server-side and only (1) closes.
