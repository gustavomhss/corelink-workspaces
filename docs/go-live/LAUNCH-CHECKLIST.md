# clw go-live launch checklist — the turnkey path to `v0.1.0` GA

> **Purpose.** The clw **code** is done: two full adversarial audit passes (71 findings fixed across
> PRs #42–#55), gate-green, live-conformance-green, real e2e coverage for the destructive commands.
> What remains for a GA tag is **owner/infra action**, not engineering. This checklist turns
> "owner/infra-gated" from a vague status into a **sequenced, do-this-then-that** list: each of the 11
> exit-gates, its current status, and the exact action + who owns it.
>
> Legend: ✅ done · ⏳ **owner action** (concrete, unblocked) · 🔒 infra/external (needs a key, a bill,
> or a third party) · 🧪 verifiable-now-by-you (a script exists).

---

## At a glance

| # | Exit gate | Status | Blocking action |
|---|---|---|---|
| 1 | Audit merged + defects fixed; zero open HIGH/MED | ✅ | — (done **twice** — pass-1 + pass-2) |
| 2 | Compiles + tests green every platform incl. Windows | ✅ / 🧪 | gates run on the **self-hosted Mac** (incl. the windows-gnu cross-compile check); a Windows **runtime** test needs a Windows machine (a future self-hosted Windows runner) — **not** billing. ACL **verifiable now** via the PowerShell script |
| 3 | Every tracked deferral closed (or logged waiver) | ✅ | — (shipped code clean; only `best-effort` *design* comments remain) |
| 4 | Signed reproducible binaries; install **and** uninstall verified | 🔒 | Provision signing secrets (below); run the matrix |
| 5 | Supply chain clean + SBOM + **signed** checksums | ⏳ / 🔒 | `cargo-deny`/`publish=false` ✅; SBOM in CI ✅; signed checksums need the signing key |
| 6 | Live conformance green as a pre-tag gate | ✅ / ⏳ | 8/8 green vs prod; the `release.yml` `conformance` job is wired as a REQUIRED predecessor but is **inert until `CLW_CI_TOKEN` is provisioned** (skip-not-fail) — provision that one secret to make it ship-blocking |
| 7 | End-to-end **family** path proven across repos | 🔒 / ⏳ | **clw half DONE** (runner-model #64 + per-tier #66 + solo-live #67 suites merged); **server D-9 mint SHIPPED** (#305) + per-tier seed delivered (#303); remaining = runner's clw-invocation build + a D1 test env + per-tenant PATs — all TL-routed |
| 8 | Region/residency correct + real right-to-erasure | ✅ | erasure ✅ (`clw erase`, audited); signup-worker region fix **LIVE** (D-3 #269 — `regionFromColo` derives `primary_region`). **Verify:** create a tenant from a non-NA colo (e.g. `LHR`) and assert its D1 row shows `primary_region = 'weur'` (not the old `enam` hardcode): `wrangler d1 execute <DB> --command "SELECT primary_region FROM tenants WHERE id='<new>'"` |
| 9 | Legal/compliance published | ✅ beta · ⏳ GA | Docs **operative for the open beta** (drafted ✅; none claims counsel-reviewed; DPA carries the beta→counsel banner). **NOT an open-beta blocker.** Counsel sign-off **deferred to post-open-beta / pre-GA by owner waiver (2026-06-17)** — see `docs/ROADMAP-go-live.md`. |
| 10 | Telemetry-off + key rotation runbook + support/EOL | ✅ | telemetry removed ✅; rotation runbook + EOL policy now published (this PR) |
| 11 | CI not on a single founder machine | ☑️ **owner-accepted** | self-hosted single-owner **dedicated** Mac is the chosen model (not GitHub billing). Owner accepts the single-machine risk (CI/release pause if the Mac is down); per-project toolchain isolation (#68) mitigates the cross-project rustup-nuke |

**The critical path to GA:** provision **signing secrets (gate 4/5)** + **legal sign-off (gate 9 — deferred to post-open-beta by owner waiver 2026-06-17)** + prove the **family e2e (gate 7)**. **For the OPEN BETA, gate 9 is satisfied** (docs operative; counsel engaged after beta, before GA). CI + release are **self-hosted** (gate 2/11 owner-accepted, the `release.yml` runs on the dedicated Mac); everything else is ✅ or follows from these.

---

## ⏳ Owner actions — do these (each is concrete + unblocked)

### A. Provision the signing & release secrets (gates 4, 5)
Decisions are already made in `docs/DISTRIBUTION.md`, and the signing/publish **step bodies are now implemented** in `.github/workflows/release.yml` (macOS codesign+notarize, Windows Azure Trusted Signing via the `sign` CLI, Linux minisign, signed `SHA256SUMS`, publish the GitHub Release on the public mirror + bump the Homebrew tap — each guarded by secret-presence, so it no-ops cleanly in dry-run and runs for real once the secret exists). You only **provision the secrets** the chosen options imply (each is flagged `TODO(OWNER)` in the step `env:` blocks); there are **no dry-run guards left to delete**. Caveat: the bodies are written to vendor spec but are first exercised end-to-end on your provisioning run (no signing infra exists to test against beforehand).

- **Windows code-signing — Azure Trusted Signing** (decision (b); exportable `.pfx` was rejected — no longer obtainable from a reputable CA). Provision as GitHub **secrets**:
  `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_SUBSCRIPTION_ID`, `AZURE_CODE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`, `AZURE_ENDPOINT` (e.g. `https://eus.codesigning.azure.net`), and `AZURE_CLIENT_SECRET` (OIDC via `azure/login` is the **preferred** alternative to the client secret).
- **macOS — Developer ID + notarization** (decision (c): notarized **zip** for the bare-binary PATH install). Provision:
  `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY`, plus the App Store Connect notarization creds (`MACOS_NOTARY_KEY_BASE64`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER_ID`).
- **Linux — minisign** (sigstore documented as the alternative). Generate a minisign keypair; provision the secret key as `MINISIGN_SECRET_KEY` and the passphrase as `MINISIGN_PASSWORD`; publish the **public** key in the repo + `docs/DISTRIBUTION.md` trust-root section.
- **Distribution — public mirror Release + Homebrew tap** (decision (a); the binary host is the **public** mirror repo `humangr-labs/clw-releases`, with `humangr-labs/homebrew-clw` as the public tap). Provision a cross-repo publish credential `CLW_RELEASES_TOKEN` (publish the GitHub Release + 5 binaries on the mirror) and `HOMEBREW_TAP_TOKEN` (bump `Formula/clw.rb` on real `v<version>` tags only). No R2 creds and no `CLW_INSTALL_TOKEN` — the download surface is public, so there is no authenticated route to coordinate (server dep **D-12 is dropped/moot**).

→ See `docs/go-live/RUNBOOK-key-rotation-revocation.md` for how these keys are rotated/revoked once provisioned (gate 10).

### B. Verify the Windows PAT-file ACL (gate 2, the security half) 🧪
You do **not** need the M2.4 CI lane to close this. On any Windows machine (a VM, a friend's PC, a
cloud box), run the script in **`docs/go-live/WINDOWS-ACL-VERIFICATION.md`** — it builds clw, runs
`clw init`, and asserts the `config.toml` DACL is protected + owner-only. Report the three `PASS`
lines and we mark the Windows ACL runtime-verified for v0.1.0.

### C. Verify install **and** uninstall per platform (gate 4)
After the signed binaries build: on each of macOS / Linux / Windows, run the public install
(`brew install humangr-labs/clw/clw`, the `install.sh` curl fallback, or the direct `.exe` on Windows),
confirm `clw doctor` is green, then run `clw uninstall --yes` and confirm `~/.clw` (config + PAT +
cache) is gone and the binary-removal instruction is printed. (The uninstall **logic** is audited +
e2e-tested; this is the per-platform packaging confirmation.)

### D. Route the drafts to counsel (gate 9)
The binding docs are written and self-consistent (`EULA.md` + `NOTICE` at the repo root;
`docs/PRIVACY-POLICY.md`, `docs/TERMS-OF-SERVICE.md`, `docs/DPA-subprocessors.md`, `SECURITY.md`,
and the export-control self-classification).
They carry the "operative, full counsel review pending" framing. **Owner decision (2026-06-17): no
counsel for now — the open beta launches on these operative docs, and counsel is engaged AFTER the
open beta, before GA** (logged as a waiver in `docs/ROADMAP-go-live.md`). The eventual sign-off flips
gate 9 from operative-for-beta → published.

---

## 🔒 Infra/external — needs a bill or a third party

- **~~M2.4 — GitHub-hosted CI matrix~~ → SELF-HOSTED (owner decision, gates 2, 11).** GitHub-hosted
  billing is **not** the model — CI (`ci.yml`) and the release (`release.yml`) run on the owner's
  **dedicated single-owner Mac** (macOS native, Linux + Windows-**gnu** cross via `cargo-zigbuild`),
  with per-project `CARGO_HOME`/`RUSTUP_HOME` isolation (#68) against the cross-project rustup-nuke.
  Gate 11 is **owner-accepted** (single-machine risk). A Windows **runtime** test (not just compile)
  remains a future self-hosted Windows runner — optional, not billing-gated.
- **M7 — family e2e (gate 7).** Needs corelink-server (D-9 mint **SHIPPED** #305 + seed #303) + the
  runner's clw-invocation build + a D1 test env. Cross-team, all TL-routed. See
  `docs/FAMILY-E2E-integration-plan-2026-06-16.md`.

---

## When all ⏳ + 🔒 clear: cut the tag

1. Confirm `main` is gate-green and live-conformance-green (the `release.yml` `conformance` job gates this).
2. Tag `v0.1.0`. The pipeline runs: preflight → conformance gate → build+sign (5 targets) → **signed**
   SHA-256 checksums + SBOM → publish the GitHub Release on the public mirror `humangr-labs/clw-releases`
   (`CLW_RELEASES_TOKEN`) → bump the Homebrew tap `humangr-labs/homebrew-clw` (`HOMEBREW_TAP_TOKEN`,
   real tags only).
3. Smoke-test the published, signed artifact on one machine per OS (download → verify signature →
   `clw doctor`).
4. Announce per the support policy in `docs/SUPPORT-AND-EOL.md`.

**Pre-GA, a `v0.1.0-rc1` code-freeze tag can be cut at any time** — the code is frozen-ready; an RC
marks "audited², release-engineering pending" without implying signed GA binaries.
