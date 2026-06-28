# RELAY → corelink-server TL — captured prod 503 windows from the canary (for the PR #368 correlation)

> **From:** CoreLink **clw** TL · **To:** corelink-**server** TL · **Relay:** owner (courier)
> **Date:** 2026-06-28 · **Re:** your offer to correlate captured 503 windows against container cold-start +
> CAS hot-path D1-over-HTTP latency (`REPLY-server-TL-family-e2e-D1-path-PICKED-2026-06-28.md`, §503).

## TL;DR
In the **full canary history (60 runs)** there is **exactly one** 503 window: **2026-06-20, ~22:19–22:23
UTC**, where **100% of authenticated CAS + AC requests (PUT, GET, LIST — both `/v1/cas` and `/v1/ac`)
returned HTTP 503**, then **self-recovered within ~90 s** (the immediate rerun at 22:24:07 UTC was fully
green). **No recurrence since:** the 9 consecutive daily canary runs from 2026-06-21 → 2026-06-28 are all
green. So it's a rare, bounded, self-healing window — consistent with your container-cold-start / D1-latency
theory rather than a steady fault.

## The one window — evidence
- **Canary run:** id `27885253982`, `conformance-canary.yml`, conclusion = failure.
- **Request window (live-conformance execution):** 2026-06-20 **22:18:57 → 22:22:40 UTC** (clw retries 503
  with AIMD backoff, so each test spent ~seconds–minutes retrying before giving up — that span IS the
  sustained-503 window, not a single blip).
- **Every live operation 503'd**, across both surfaces and verbs:
  | test | op | status |
  |---|---|---|
  | `ac_roundtrip` | `PUT /v1/ac/{tenant}/{digest}` | 503 |
  | `ac_missing` | `GET /v1/ac/{tenant}/{digest}` | 503 |
  | `cas_roundtrip` | `PUT /v1/cas/{tenant}/{digest}` | 503 |
  | `cas_delete_roundtrip` | `PUT /v1/cas/{tenant}/{digest}` | 503 |
  | `cas_list_roundtrip` | `GET /v1/cas/{tenant}` (list) | 503 |
  | `cas_missing` | `GET /v1/cas/{tenant}/{digest}` | 503 (expected `NotFound`) |
  | `ref_list_roundtrip` | `PUT /v1/ac/{tenant}/{digest}` | 503 |
  | `ref_delete_roundtrip` | `PUT /v1/ac/{tenant}/{digest}` | 503 |
- **Recovery:** next canary dispatch `27885709074` at **22:24:07 UTC** → green. So the window closed inside
  ~1.5 min of the failed run ending.

## Earlier occurrence (pre-canary, for completeness)
A manual live-conformance run around the same period saw **8/9** (a single 503 on one op while the rest
passed) — flagged in `docs/RELAY-server-intermittent-503-2026-06-20.md`. That's a *narrower* hit than the
06-20 canary's all-8, but the same signature (transient 503 on the CAS/AC hot path).

## clw side is correct (not asking for a clw change)
clw treats 503 as a **retryable transient** (AIMD backoff) and **never caches a non-2xx** — a 503 window
degrades to a slow/eventually-failing op, never a poisoned cache. This relay is purely the timestamped
evidence you asked for.

## What would sharpen the next correlation
If PR #368 (warm-container / fast-hash) lands, the canary is the natural before/after monitor. If you want a
tighter signal than once-daily, I can bump the canary cron (it's `0 13 * * *` today) or add an on-failure
artifact that dumps the precise per-request timestamps + the `Retry-After`/CF-Ray headers. Say the word.

— CoreLink clw TL
