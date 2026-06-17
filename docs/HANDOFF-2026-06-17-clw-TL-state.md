# clw TL — session handoff / state (2026-06-17)

> **Purpose.** Single source of truth to resume *fresh* after a `/clear`. Reflects the **corrected**
> state after reading the inbound server-TL correspondence (which had advanced past the prior summary).
> I am TL of `clw` / corelink-workspaces (single human owner = Gustavo). Standing mandate: SOTA go-live,
> no debt/deferral without explicit human waiver, I OWN all merge/PR/commit, keep all judgment, delegate
> only verifiable execution, **never assume without evidence**.

---

## TL;DR — where we actually are

- **clw code + release pipeline: DONE and VERIFIED.** All PRs #61–#71 merged, `main` clean, 0 open PRs,
  only `main` locally. Release reconciliation (#71) **proven**: all 5 targets build on the self-hosted
  Mac (mac×2 native + linux×2 + **windows-gnu** cross via cargo-zigbuild 0.23.0) → correct artifacts
  (`file`-verified: Mach-O arm64/x86_64, ELF x86-64/aarch64, **PE32+ for MS Windows**).
- **Server TL: ZERO open family-e2e work.** Everything shipped + delivered (details below).
- **clw has NO open *code* work** (the `auth rotate`/#306 item was VOID-ed 2026-06-17 — owner-decided
  post-launch + a dispatcher-side capability, not clw code; see Actionable §1). The ball is in the
  **owner's + runner-TL's** court: re-mint PATs + runner build + signing/legal gates. clw is waiting on
  them, not the reverse.

---

## What the server TL DELIVERED (read these — they're the corrected reality)

Inbound docs now committed to `docs/` (no secrets in them — credentials are out-of-band):

| Item | Status | Evidence / location |
|---|---|---|
| **D-9 per-job runner mint** | ✅ shipped | `POST /internal/v1/runner/mint` + `/revoke`, server **#305** (`c56fde54`); 90-min TTL, cas:rw, admin refused. Bugfix **#307** (persists `pat` row, fail-CLOSED). |
| **`clw auth rotate` server endpoint** | ✅ shipped (but **NOT a clw consumer** — see Actionable §1) | `POST /internal/v1/auth/rotate {pat_id, scope?}` → mint-new-then-revoke-old, server **#306** (`06cf4849`). **Internal-auth only** (*"an end-user PAT cannot call it"*) ⇒ consumed **dispatcher-side**, not by the clw CLI. clw's own `auth rotate` stays the owner-decided re-login (post-launch for one-step). `RELAY-from-server-TL-auth-rotate-SHIPPED-2026-06-16.md` |
| **Per-tier seed + live D1** | ✅ applied to live `corelink-prod-d1` | 6 tenants/tier `…0f0001..0006`, `tier_selections`+`runners_entitlement`+`tenant_quota` (#303/#308); migration `0071` applied; gates fail-CLOSED (#302). |
| **6 per-tier PATs** | ✅ delivered out-of-band | `~/Downloads/clw-family-e2e-tier-pats.txt` (chmod 600). scope `read-write`, **TTL 24h, minted 2026-06-16 → almost certainly EXPIRED now → must re-mint** (ping server TL, re-mints in seconds). |
| **enam conformance credential** (D-1/D-7/D-8) | ✅ delivered out-of-band | enam/active/free sandbox tenant + ~72h PAT, minted 2026-06-15 (`~/Downloads`, owner has path). `RELAY-server-DEPLOYED-credential-READY.md` |
| **#269 (D-1/D-7/D-8 + residency + GDPR)** | ✅ deployed to prod | IAD container live, `/_health` green. |
| **rustup-nuke** | ✅ root-caused + fixed server-side | server **#312** (two vectors: missed `dtolnay/rust-toolchain@nightly` in `corelink-client-verify.yml`/`nightly.yml` + unguarded `cargo install cargo-fuzz` race). Residual = shared-host disk (98%) + shared CARGO_HOME, owner-infra. clw's own isolation (#68) stands regardless. |

---

## ⏳ ACTIONABLE NOW for me (clw TL) — newly unblocked, NOT yet done

1. **~~Implement `clw auth rotate` against #306~~ — VOID (resolved 2026-06-17, do NOT implement).**
   On review this item rested on a flawed premise. Evidence:
   - **Owner already decided (ROADMAP 2026-06-14):** clw's `auth rotate` IS the re-login workflow
     (`clw auth login` with the new token); a one-step mint-and-revoke command is an explicit
     **post-launch** nicety. The current stub is the ratified behavior — no hidden debt.
   - **#306 is a backend/dispatcher capability, not an end-user CLI one.** It requires **internal-auth**
     (`CORELINK_PAT_MINT_AUTH_KEY`) — the relay says verbatim *"an end-user PAT cannot call it"* — plus a
     `pat_id`. clw is an end-user client (Bearer-PAT + tenant-in-path only); the two-layer model
     (`HANDOFF-runner-pat-minting.md`) makes internal-auth *"untouchable by the client"*. Putting the
     mint key on end-user machines would be a **security regression**.
   - **clw structurally can't call it:** `pat_id` exists nowhere in the code (`Config` stores only the
     `token` plaintext), and clw has no internal-auth concept. In the D-9 model the **dispatcher** mints
     server-side and injects the PAT; clw only consumes. There is no clw-side consumer for #306.
   - **Net:** keep the stub as-is. #306 is consumed dispatcher-side (corelink-runners), not by clw.
     I over-trusted the server-TL relay's "your move" framing in the prior handoff. The only remaining
     gate-7 clw work is the LIVE family-e2e (item 2), which is owner/runner-gated — see below.
2. **Run the LIVE family-e2e** once PATs are re-minted (NOTE 2026-06-17: not yet ready — runner-build
   gate + owner re-mint still pending; do not attempt prematurely):
   - solo live lifecycle: `CLW_TENANT=<…0f0001> CLW_TOKEN=<pat> CLW_ENDPOINT=https://corelink-api.humangr.com cargo test -p clw-conformance --features live` (suite #67)
   - per-tier matrix (suite #66): one tenant/token per tier, assert 402/429/409/410/413 enforcement.
3. **D-1/D-7/D-8 conformance round-trip** against the enam sandbox credential (if not already run) —
   confirm green, then the DPA/Privacy erasure language can flip to the real Art.17 path. Note server
   heads-up: if D-8 (erasure) returns 403 on read-write, ping for an `admin`-scope re-mint.

> ⚠️ **Blocker on the live runner-mint path specifically:** the prod Worker still needs a **redeploy to
> activate #307** (owner/launch-gated). If a live `/runner/mint` token 401s, that's the pending deploy,
> not clw wiring. The 6 per-tier PATs were minted directly so they don't depend on #307.

---

## Still genuinely waiting on others

- **Runner TL (corelink-runners):** the clw-invocation build on the runner image — dispatcher calls
  `/internal/v1/runner/mint`, env-injects `CLW_*`/`CORELINK_TOKEN`, invokes `clw snapshot/hydrate/run`.
  Contract frozen + accepted both sides (`docs/CLI-CONTRACT.md`). This is the last non-clw, non-owner gate.
- **Owner (Gustavo):**
  - Re-mint trigger / point me to `~/Downloads/clw-family-e2e-tier-pats.txt` (+ enam cred) so I can run live.
  - **Prod Worker redeploy** to activate server #307 (for the live runner-mint path).
  - **Signing + publish secrets** (gate 4/5) — Azure Trusted Signing, macOS Developer-ID+notarization,
    minisign, and the public-distribution publish tokens (`CLW_RELEASES_TOKEN` for the mirror Release,
    `HOMEBREW_TAP_TOKEN` for the tap bump). See `docs/go-live/LAUNCH-CHECKLIST.md §A`.
  - **Legal sign-off** (gate 9) — drafts done, need counsel.

---

## Go-live gate status (from LAUNCH-CHECKLIST.md, current)

✅ done: 1 (audit²), 2 (self-hosted incl. windows-gnu, **build-verified**), 3, 8 (erasure), 10, 11 (owner-accepted single-Mac).
Open: **4/5** (signing secrets — owner), **6** (`CLW_CI_TOKEN` to make conformance ship-blocking — owner),
**7** (family-e2e LIVE — runner build + re-mint + my 3 actionables above), **9** (legal — owner).

**Critical path to GA:** signing secrets + legal sign-off + prove family-e2e live. Nothing else blocks.

---

## Repo hygiene snapshot

- Branch: `main` only (local). 0 open PRs. Working tree clean after this handoff commit.
- This commit lands: the 7 inbound server-TL relay/response docs (correspondence record) + this handoff.
- No orphan branches/worktrees/processes. self-hosted runner `clw-mac-1` installed as launchd service.

---

## Hard constraints (preserve — never violate)

- Credentials are **out-of-band only**; never in chat/repo. clw **never** touches the server DB (security boundary).
- Never fabricate signing keys (owner generates/holds). PAT always redacted, never logged.
- secret-read-guard hook blocks secret-named files + bash with `.p12`/`.p8`/`minisign.key` strings
  (bypass `TECHLEAD_ALLOW_SECRET_READ=1` for isolated cp only). destructive-guard bypass
  `TECHLEAD_ALLOW_DESTRUCTIVE=<≥6-char phrase from the command>`.
- Rigor compact: no debt/deferral/gambiarra without an explicit logged human waiver.

---

## First move on resume

Re-read this doc. **There is NO open clw *code* item** — the `clw auth rotate` item was VOID-ed on
2026-06-17 (see Actionable §1; it's an owner-decided post-launch nicety + a dispatcher-side capability,
not clw code). The only remaining gate-7 clw work is running the **live family-e2e**, which is gated on
the owner re-minting PATs and the runner build — **per owner 2026-06-17, not yet ready; do not attempt
prematurely.** So on resume, clw is effectively **waiting on owner/runner**, not on its own code.
Confirm current state with `git status` + `gh pr list` before assuming.
