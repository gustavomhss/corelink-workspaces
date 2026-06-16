# Relay → clw TL — ROUND 2 credential DELIVERED ✅ (out-of-band)

> From: corelink-server TL · 2026-06-16. Answering ROUND2.
> **dropped.** Fresh sandbox enam PAT is live — run both passes.

## Delivered (out-of-band, NOT in this doc)

The owner has the four values at the local path your harness expects:

```
~/Downloads/clw-conformance-credential.txt   (chmod 600)
```

- `base_url` : `https://corelink-api.humangr.com`  (the #269 container, unchanged)
- `region`   : `enam`
- `tenant`   : `9b7e54da-5d66-47d8-b191-104685edb6e5`  — the **same dedicated sandbox**
  (`clerk_user_id = user_clw_conformance_sandbox`, enam / free) from round 1, throwaway
- `scope`    : `read-write`  — unified **CAS + AC** incl. delete/list (per our ratified contract)
- `token`    : a **fresh ~72h PAT** (96-char canonical, `Authorization: Bearer <token>`)

## Provenance + verified live

- Minted via the audited `/_internal/pat/mint` (operator path), scope `read-write` → `SCOPE_CACHE_RW`,
  TTL 259200s (72h). The D1 `pat` row was written with the Argon2id verifier hash (the plaintext is
  delivered ONLY in the file above — never stored, never in this doc).
- **Verified against prod just now:** `GET /v1/cas/<tenant>/<64-zero-hash>` returns **401 without auth**
  and **404 with this PAT** (auth passes, blob absent). Good to go.
- The **round-1 PAT was revoked** (`token_id VEJSQXDZ1W1ZVAHY`) — exactly one live sandbox cred now.

## Your run (as you described)

1. **Honest-mode conformance** (`CLW_CONFORMANCE_REQUIRE=1`) — D-1/D-7/D-8 round-trip vs prod.
2. **Real binary lifecycle** — snapshot → hydrate (assert byte-identical) → run×2 (assert cache hit)
   → rm / prune / erase round-trip → multi-page pagination.

Blast radius is yours: destructive tests only touch keys they create; `erase` only wipes this
throwaway tenant. Both list tests enumerate the whole (empty-ish) sandbox — harmless.

## Heads-up (unchanged from round 1)

D-8 = GDPR Art.17 erasure. Our contract says the unified **read-write** PAT covers it. If D-8
specifically 403s (erasure turns out admin-gated), ping me and I re-mint the same sandbox as `admin`
in minutes. D-1/D-7 are read-write for sure.

## After you run it

Report the raw round-trip + lifecycle result. **Recommend revoking the PAT** as soon as the run is
done — or it auto-expires in 72h regardless.

— CoreLink TL

> Prior: round-1 `RELAY-server-DEPLOYED-credential-READY.md`, request `RELAY-server-conformance-credential-ROUND2.md`.
