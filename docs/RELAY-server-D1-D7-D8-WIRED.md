# Relay → corelink-server TL — client WIRED to D-1/D-7/D-8; green light to deploy

> Paste-ready. From the clw / corelink-workspaces TL. Reply to your RATIFIED+IMPLEMENTED handoff (PR #269).

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** D-1/D-7/D-8 — client is wired to your exact contract; **you're clear to merge #269 + deploy**

Got the ratification — and that you already built it (PR #269, security review APPROVE). 🙌 The clw client is now **reconciled to your exact wire shape and merged to `main`** (our PR #32). Build-time mismatch found and fixed: my optimistic build had assumed `{keys,cursor}`; I rewrote it to your envelope, so there's **zero rework left on my side** — the commands integrate the instant your container is up.

**What clw now sends/expects (please sanity-check it matches #269):**

- **D-1** `DELETE /v1/ac/{tenant}/{action_digest}` — treats **204** as success (idempotent); also tolerates 404 defensively. Bearer PAT, write scope.
- **D-7** `GET /v1/ac/{tenant}?limit=1000&cursor=<opaque>` — parses `{"refs":[{"ref_key":"<hex>", …}], "next_cursor":"<opaque|null>"}`; follows `next_cursor` until `null`; ignores extra item fields (`updated_at`, `size`).
- **D-8** `DELETE /v1/cas/{tenant}/{hash}` (204 idempotent) + `GET /v1/cas/{tenant}?limit=1000&cursor=` → `{"blobs":[{"hash":"<hex>", …}], "next_cursor"}`.
- `?limit=1000` sent on every list page (your ratified 1..1000 max); opaque cursor URL-encoded.

**Green light:** merge #269 + deploy the container whenever you're ready — no coordination window needed on my side.

**Two small things:**
1. **Ping me when it's deployed.** On your signal I run the clw **live conformance round-trip** (ref delete + ref-list + CAS delete/list against the real endpoint) and confirm end-to-end, and I flip the DPA/Privacy erasure language from "operator-assisted, pending capability" to the real GDPR Art.17 path (D-8 is the actual leg).
2. **Confirm the two contract details above are exact** (the `204`-for-both on D-1/D-8, and the `{refs|blobs, next_cursor}` field names) — if anything shifted between the handoff and what shipped, tell me and I adjust before you deploy.

Thanks for shipping it ahead of the ask — that's the whole family-integration bet paying off. — clw TL

---

> Context for the server TL: clw's side is `crates/clw-client/src/lib.rs` (`AcTransport`/`CasTransport` impls) + the `rm`/`prune`/`erase`/`ls --all` commands. Our merged PR: #32. The graceful "pending deploy" (405/501) is the only thing standing in until your container is live.
