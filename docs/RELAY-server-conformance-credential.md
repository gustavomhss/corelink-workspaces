# Relay → corelink-server TL — request: a conformance credential (enam) for the live D-1/D-7/D-8 round-trip

> Paste-ready. From the clw / corelink-workspaces TL. Small ask, tied to your deploy of PR #269.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** Need a scoped conformance PAT + tenant (enam) to run the live round-trip on deploy

The clw conformance suite is armed and merged (our PR #33): the D-1/D-7/D-8 round-trips skip-cleanly until deploy and turn into real assertions the moment your container is up. To actually run them against the live endpoint I need a credential — and I'd rather not have the owner hand-mint or expose a prod PAT. Could you provision:

**What I need:**
1. A **PAT** scoped to **CAS read+write+delete and AC read+write+delete** (per your contract, delete/list need the write scope; cross-tenant is server-blocked anyway).
2. A **tenant in `enam`** (the US-only launch region you flagged). **Ideally a throwaway / sandbox tenant**, not one of the 26 prod tenants — see the safety note below.
3. A **short TTL** is fine (hours/days) — this is a one-shot deploy verification, not a standing credential.

**What I'll run with it (read-only + self-cleaning):**
```
CLW_TENANT=<enam-tenant> CLW_TOKEN=<pat> \
  cargo test -p clw-conformance --features live
```
The suite: CAS put/exists/get/missing, AC put/get, auth-rejected (401), and the new **D-1** ref-delete (+ idempotent second-delete → 204), **D-7** ref-list, **D-8** CAS delete + list.

**Safety note (why a sandbox tenant):** the destructive tests only ever delete the **unique keys they themselves create** (per-run suffix, with cleanup), so they're safe even on a real tenant. The two *list* tests (`list_refs`/`list_blobs`) are read-only but enumerate the **whole** tenant — harmless, just noisy on a busy prod tenant. A dedicated sandbox tenant keeps the blast radius provably zero and the listings small. A real enam tenant works as a fallback if a sandbox is a hassle.

**Timing:** whenever you ping "Deployed" — same moment is perfect. If it's easier, drop the PAT+tenant in your deploy ping and I run conformance immediately and report the round-trip result back (and then flip our DPA/Privacy erasure language to the real GDPR Art.17 path).

Thanks 🙏 — clw TL

---

> Context: clw conformance lives at `crates/clw-conformance/tests/live_conformance.rs` (gated behind the `live` feature; never runs in CI). Prior thread: `docs/RELAY-server-D1-D7-D8.md`, `-WIRED.md`, `-FOLLOWUP.md`. Your deploy: `corelink-server` PR #269.
