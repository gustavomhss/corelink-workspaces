# Relay → corelink-server TL — CHASE: deploy PR #269 + the enam conformance credential

> Paste-ready. From the clw / corelink-workspaces TL. **2026-06-15.** This is a
> status-chase on the one item now blocking the clw go-live: the D-1/D-7/D-8
> deploy + a scoped credential to verify it.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** Where's the #269 deploy + the enam sandbox PAT? clw is fully ready and waiting only on you.

Quick, friendly chase — no new asks, just closing the loop.

## State on my side: DONE, gate-green, merged
- Client wired to the **ratified wire envelopes** (D-7 `{"refs":[{"ref_key"}],"next_cursor"}` / D-8 `{"blobs":[{"hash"}],"next_cursor"}`), `?limit=1000&cursor=` pagination, idempotent-`204` delete. `rm`/`prune`/`erase`/`ls --all` integrate the moment your container is up — **zero rework expected**.
- Destructive paths **adversarially reviewed and hardened** (9 findings closed).
- Live conformance suite **armed** (`crates/clw-conformance/tests/live_conformance.rs`) and now **honest**: a new require gate (`CLW_CONFORMANCE_REQUIRE=1`) makes a skip a hard failure, so the pre-tag run can't false-green on a missing endpoint or credential.

The only thing I cannot do without you is **prove the live round-trip** — that needs your endpoints in the air and a credential.

## The two things I need from you
1. **Deploy PR #269** (the D-1/D-7/D-8 implementation). Ratified + security-APPROVE'd already — this is the deploy step.
2. **A scoped conformance credential**, delivered **out-of-band** (not in a doc/repo):
   - a **PAT** with CAS read+write+delete and AC read+write+delete,
   - a **throwaway/sandbox tenant in `enam`** (ideally not one of the 26 prod tenants),
   - **short TTL is fine** — it's a one-shot deploy verification.

Drop both in the same "Deployed" ping and I run conformance immediately:
```
CLW_CONFORMANCE_REQUIRE=1 CLW_TENANT=<enam-tenant> CLW_TOKEN=<pat> \
  cargo test -p clw-conformance --features live
```
…and report the D-1/D-7/D-8 round-trip back the same hour, then flip our DPA/Privacy erasure language to the real GDPR Art.17 path.

## The ask
**What's the ETA on the deploy?** If it's blocked on something on your side, tell me what and I'll help unblock or re-sequence. If it's just queued, a rough date lets me schedule the conformance + the legal flip around it.

Prior thread (for context, no action needed): `docs/RELAY-server-D1-D7-D8.md`, `-WIRED.md`, `-FOLLOWUP.md`, `docs/RELAY-server-conformance-credential.md`.

Thanks 🙏 — clw TL

---

> Out-of-band reminder (your earlier note, just confirming we're aligned): you'll mint the sandbox tenant + short-TTL read-write PAT at deploy time and hand it over off-channel — never exposed in a doc or the repo. 👍
