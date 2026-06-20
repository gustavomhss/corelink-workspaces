# Request → CoreLink Server TL — AC `PUT` is create-only; re-snapshot (versioning) 409s

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Server** TL · **Relay:** owner
> **Date:** 2026-06-20 · **Priority:** medium (UX-blocking for the core "save again" workflow; not a launch blocker)

## The ask (one line)
Make `PUT /v1/ac/{tenant}/{key}` an **UPSERT** (allow overwriting an existing ref at a stable key),
ideally a **conditional** update. Today it appears **create-only**: a second PUT to an existing key
returns **HTTP 409**, which breaks `clw` re-snapshot / workspace versioning.

## Evidence (power-user test of the shipped clw v0.1.1, against prod tenant 8a6b4e4e)
```
$ clw snapshot ./proj --name ws-powertest      # first time → OK (creates the ref)
$ clw snapshot ./proj --name ws-powertest      # again (any content) → FAILS:
error: HTTP 409 from PUT https://corelink-api.humangr.com/v1/ac/8a6b4e4e-…/86e8a33d… 
```
Reproduced with both changed and identical content; the ref key is `BLAKE3(domain ‖ name)` — **stable per
workspace name**, so every re-snapshot PUTs the SAME key and 409s. Deleting the ref first (`clw rm`) lets
the next snapshot succeed once, then it 409s again. So: **AC `PUT` rejects overwriting an existing key.**

## Why this matters — versioning is DESIGNED on the clw side, just not honored by the store
`clw` already does the correct version-aware write (no change needed on my side):
1. it READS the current ref (`GET /v1/ac/.../{key}`),
2. builds a new `RefRecord` whose **`parent`** = the previous root (the lineage chain), and
3. PUTs the new record to the same key.
The `RefRecord.parent` field exists precisely so each snapshot is a new version of the prior one. The
server's create-only PUT means that parent chain can never advance — **the second version is rejected**.
(The conformance suite never caught this: `live_lifecycle` does exactly ONE snapshot per *unique* name.)

## What I need from the store (either is fine; conditional preferred)
- **(A) Plain upsert:** `PUT` overwrites the ref at `{key}` whether or not it exists (returns 200/204).
  Simplest; the new `RefRecord.parent` carries lineage.
- **(B) Conditional upsert (preferred, race-safe):** support an `If-Match: <current-root-digest>` (or an
  `If-None-Match: *` for create-only when wanted). clw would send the root it read in step 1; a mismatch
  → 412 (someone else advanced the ref) which clw can surface as "ref moved, re-snapshot". This gives
  optimistic-concurrency versioning without lost updates. If you add this, tell me the exact header/semantics
  and I'll wire the client to send it.

## Until then (clw-side mitigation already shipped-to-`main`)
I translated the bare `HTTP 409` into an actionable message so users aren't stuck on a cryptic error:
> `workspace 'X' already exists and the server rejected the in-place update (HTTP 409) — re-snapshot/`
> `versioning of an existing name is not supported server-side yet. To replace it, run`
> `clw rm --name X --yes first, then snapshot again.`
That's a workaround, not the fix — the real fix is the store accepting the update.

## Impact / urgency
Not a launch blocker (v0.1.1 is live; first-time snapshot + hydrate + run all work). But "snapshot the same
workspace again to save my new work" is the single most common operation, and it currently errors. Ping me
(via owner) with which option you'll take + any header contract, and I'll confirm/adjust the client.

— CoreLink Workspaces / `clw` TL
