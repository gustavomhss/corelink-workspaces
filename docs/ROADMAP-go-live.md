# ROADMAP — `clw` / CoreLink Workspaces go-live (mature, SOTA, impeccable) — **v2**

> **Owner seat:** TechLead (CoreLink). **v2 drafted:** 2026-06-14, after a 26-agent hardening fleet
> (`clw-golive-harden`) produced execution-ready WP briefs + an independent fresh audit + adversarial
> critique. **Baseline:** `main @ f8f5edf`. **Execution specs:** `docs/go-live/WP-BRIEFS.md`.
>
> **Mandate:** ship to production with **no loose ends, no debt, nothing deferred** without an
> explicit, scoped, owner-scheduled cross-repo dependency. `clw` is a **pure client** of
> `corelink-server` and never edits it; server work is coordinated via the dependency ledger.

## Progress log

| Date | Landed | Notes |
|---|---|---|
| 2026-06-09 | **Audit (#26)** + **M1.1 StreamCDC (#27)** | `fix/clw-audit-2026-06-09` and `fix/clw-deferrals-2026-06-09` merged to `main` (`87f9842`). M1.1 (snapshot streaming, dedupe-identical) is **DONE**. |
| 2026-06-14 | **M0.5 audit-defect fixes** (`1422fa6`) | Closed 3 defects *inside* the merged audit: https loopback prefix-bypass (CRIT), `clw run` tool-cwd (HIGH), config value escaping (HIGH) + Windows test gating. Full local gate green (145 tests). |
| 2026-06-14 | **7-lane parallel wave** (`70ba1cb`) | Worktree-isolated, conflict-free fleet: **M2.0** toolchain de-hack · **M4** (publish=false ×11 + `deny.toml`, cargo-deny green) · **M1.5** run-key unset≠empty (domain v3) · **M1.7** hydrate hardening (verify fetched digests, mask mode bits, symlink-after-files) · **SECURITY.md + THREAT-MODEL** · **release.yml scaffold + DISTRIBUTION** · **handoff specs D-7/D-8/D-9**. Full gate green (149 tests). |
| 2026-06-14 | **6-commit Windows/rate-limit wave** (`6e06f17`) | clw-types Windows-aware path contract (drive-letter/backslash) · **M1.3** Retry-After + AIMD adaptive concurrency (the real 429 relief) · **M2.1** clw-hydrate/clw-manifest compile on Windows (cfg-gated) · **M2 cli/run** home_dir USERPROFILE + shell-test gating · **M4.1** cargo-deny in the gate · **M2.4** cross-platform CI (Linux blocking, Windows informational). **The entire workspace now compiles on Windows** — proved locally via `cargo check --workspace --target x86_64-pc-windows-gnu` (0 errors). Full gate green (153 tests). |

> **Status:** M0, M1.1, M1.3, M1.5, M1.7, M2.0 done; **M2 Windows port compiles workspace-wide** (CI Windows lane informational pending green); M4 metadata + M4.1 gate-wired + SECURITY + release-scaffold + D-7/8/9 handoffs done. **Next:** flip the Windows CI lane to required once green · M4.2 EULA swap (remove `UNLICENSED`) · M5 (observability/telemetry/uninstall/cache-migration/doctor/quota-UX/accessibility) · M6 (legal/privacy/right-to-erasure) · M7 (family integration) · the corelink-server track (D-1/D-3/D-4/D-6/D-7/D-8/D-9).
>
| 2026-06-14 | **7-lane M2/M4/M5/M6 wave** (`b11c890`) | **M5.2** user-agent header · **M5.6** quota-aware 429 message · **M5.4** cache-format version + invalidate-on-mismatch · **M5.5** `clw doctor` (redacted) · **M2.2** Windows symlink **copy-fallback** (closes the symlink limitation) · **M4.2** EULA + NOTICE + about.toml + final deny.toml posture · **M6** legal drafts (privacy/ToS/export-control/DPA/DR) · **M6.5** secret-file warning on snapshot · + fixed a HOME-env parallel-test race. Full gate green (171 tests) + workspace still compiles on Windows. |

> **Status (wave 3 done):** Done — M0/M0.5, M1.1, M1.3, M1.5, M1.7, M2.0, **M2 Windows port incl. symlink copy-fallback**, M2.4, M4 (publish=false + deny + gate + EULA posture), M4.4, M5.2/5.4/5.5/5.6, M3 scaffold, M6 legal drafts + M6.5, D-7/8/9 handoffs. **Next clw-side:** M5.1 tracing (serial, cross-cutting) · M5.3 telemetry-consent + M5.7 accessibility + M6.6 `clw auth` (next clw-cli wave — they share main.rs) · M4.3 SBOM + M4.5 conformance gate · flip the Windows CI lane to required once green. **Owner/server-gated:** M6.4 right-to-erasure (D-7/D-8), M7 family integration, the corelink-server track.

> ~~**Known v1 limitation:** symlink-on-Windows~~ — **CLOSED (M2.2):** Windows hydrate now copies the symlink target (file → bytes, dir → subtree); a documented, non-silent copies-not-links divergence.

> **DECISION — M1.2 PULLED (2026-06-14):** "drop the exists-HEAD, always PUT" is **rejected**. The snapshot already does `exists`(HEAD, no body) then `put` only on a miss. Always-PUT would trade cheap HEADs for full PUT-with-body on *existing* chunks — re-uploading bytes on every re-snapshot (killing the zero-upload dedupe property) and **increasing** write pressure on the rate-limited per-tenant DO (the opposite of the 429 goal). The 429 relief comes from **M1.3** (Retry-After + AIMD), not from M1.2. Net: keep the exists-HEAD; M1.2 is closed as won't-do.

## What v2 changed (the fleet's high-signal corrections)

1. **Windows does not even compile.** `clw-hydrate` and `clw-manifest` use unconditional `std::os::unix`
   (`PermissionsExt`, `from_mode`, symlink). With Windows now a release target, this is **2× CRITICAL**
   and makes the cross-platform port the **clw-side long pole** — re-sized S→**M/L**, promoted to its own milestone (**M2**).
2. **The standing audit branch has its own bugs.** Its https-loopback allow-list is **prefix-bypassable**
   (plaintext-PAT leak), its `tool-in-key` resolves the tool against the **wrong cwd** and is a **silent
   no-op on Windows**, and its `home_dir` "fix" **breaks Windows**. M0 gains **M0.5** to fix the audit before/with merge.
3. **M1 is a serial spine, not a parallel wave.** M1.1→M1.2→M1.3 all edit the same `clw-client` `with_retry`/`put`
   path + `clw-snapshot` `snapshot()` loop + frozen `clw-types`. Only **M2-toolchain-dehack** parallelizes cleanly.
4. **M1.2 premise corrected.** Prod CAS PUT returns **201 for both** new and repeat blobs — created-vs-existed is
   **not** distinguishable from HTTP. The guaranteed win is **dropping the HEAD** (2 req→1); `chunks_uploaded`
   becomes best-effort. Requires a **wiremock request-count test as a DoD blocker**.
5. **GDPR Art.17 violation.** `clw rm` only deletes the AC ref pointer; it **cannot enumerate or GC the CAS blobs**,
   so a tenant **cannot actually delete their data** — fatal for a product that sells regional residency.
   `ls --all` and `prune` are **doubly-blocked** on new server capabilities (ref-LIST + CAS-DELETE/LIST).
6. **Whole new compliance surface.** No Privacy Policy, ToS, export-control/ECCN crypto classification (the binary
   ships TLS+BLAKE3), DPA/sub-processor list. → new go-live-blocking milestone **M6**.
7. **Family integration was under/mis-specified.** Runner auth conflates the client PAT path with the server-internal
   `x-corelink-tenant-id` header (clw never sends it); per-job PAT minting is undefined; the **CoreLink runners repo is a
   third repo with no WP/owner**; the region-endpoint mechanism is an **open question** (clw may need no change); sccache C1
   is almost certainly **docs-only** (clw is not in the sccache data path).
8. **Product-surface gaps:** credential/key lifecycle + signing-key rotation, telemetry consent, `clw uninstall`,
   cache-format migration, `clw doctor`/support, billing/quota-aware UX, output accessibility (NO_COLOR/`--json`),
   i18n decision, resource-safety/abuse bounds, DR/business-continuity.

## Locked scope decisions (owner, 2026-06-14)

| Decision | Choice | Consequence |
|---|---|---|
| License / distribution | **Proprietary / commercial EULA** | No public repo/crates.io; authenticated binary download; EULA+attribution bundle; `publish=false` everywhere. |
| Target platforms (signed) | **macOS arm64 + macOS x86_64 + Windows x86_64 + Linux x86_64 (+ aarch64)** | Full matrix; current single self-hosted-Mac CI is insufficient; **Windows requires real porting**. |
| Family integration | **CoreLink API + CoreLink runners + hugit/githugr (where it fits)** — *not* wallet/forge/Smith | Integration centers on the runner + the git host. |
| Campaign #1 (CI accel / sccache) | **Inside this go-live (critical path)** | Pulls server `R2KvStore` + cargo route + write-scaling onto the path. clw role is likely docs-only. |

## Exit gate — "impeccable go-live" (100% green before tag)

1. Standing audit **merged** *and its own defects fixed* (M0/M0.5); zero known HIGH/MED findings open.
2. **Compiles + tests green on every target platform**, incl. Windows; zero `unsafe`, zero prod-path `unwrap/expect/panic`, zero `TODO/FIXME/best-effort` in shipped code.
3. Every tracked deferral closed (or a logged human waiver — none expected).
4. Signed, reproducible binaries for all targets (HSM/cloud signing); authenticated install **and** clean **uninstall** verified per platform; no founder-machine hack.
5. Supply chain clean: `cargo-deny` + `cargo-audit` green; SBOM published; **signed** checksums; valid license posture (no invalid-SPDX `UNLICENSED`); all crates `publish=false`.
6. Live conformance suite green against prod as a **pre-tag gate** (incl. an AC delete round-trip once D-1 lands).
7. End-to-end **family** path proven across all repos: Clerk signup → scoped PAT (CAS+AC) → snapshot → hydrate on a runner → memoized run → CI cache hit.
8. **Region/residency** correct end-to-end (signup-worker fix landed) **and** a real **right-to-erasure** path exists (tenant data actually deletable).
9. **Legal/compliance published:** EULA, Privacy Policy, ToS, export-control self-classification, DPA + sub-processor list, SECURITY.md.
10. **Telemetry off by default with test-proven redaction**; signing keys have a documented **rotation+revocation** runbook; runbook + `clw doctor` + support/EOL policy published.
11. CI no longer depends on a single founder machine.

---

## The standing asset & its defects: `fix/clw-audit-2026-06-09` (UNMERGED)

Closes (on `main`, once merged): H1 path-traversal, H2 PAT perms `0600/0700`, M1 `unsafe` removal, M2 https-only,
M3 tool-in-key, M4-hydrate streaming, M5 jitter, + LOWs. **But the fresh audit found defects in the audit itself:**

| New finding | Sev | Action |
|---|---|---|
| https loopback allow-list **prefix-bypassable** (e.g. `http://127.0.0.1.evil.com`) → PAT leak | HIGH | **M0.5** — exact-host/loopback parse, not prefix match |
| `tool-in-key` resolves relative tool vs **wrong cwd**; **silent no-op on Windows** | MED×2 | **M0.5** — resolve vs PATH/correct cwd; cfg Windows (PATHEXT) |
| `home_dir()` "fix" reads `$HOME` only → **breaks Windows** | HIGH | **M0.5/M2** — use a portable home resolver (`dirs`/`%USERPROFILE%`) |
| `resolve_tool_id` reads the **entire toolchain binary into RAM every run** (incl. cache hits) | MED | **M0.5** — hash metadata/path, not full read, or cache it |
| `validate_rel_path`/`validate_symlink_target` accept **Windows backslash / drive-letter / UNC** → traversal on Windows | HIGH×2 | **M2** — Windows-aware path validation + tests |
| env **absent-vs-empty collision** in `compute_key` (`unwrap_or_default`) — open on **both** main and branch | MED | **M1** new WP — distinguish unset vs empty in the run key |

**Audit's verification caveat:** WP agents did not run `cargo build/clippy/test` locally (host load ~85).
**M0 re-verifies with a full local gate** now that the host is free.

---

## Milestones

### ✅ M0 — Baseline integrity: merge the audit **and fix its defects**  *(clw, blocking) — DONE 2026-06-14*
> Audit merged via #26 (2026-06-09); M0.5 defect fixes landed `1422fa6` (2026-06-14), full gate green.
- **M0.1** Cold-review `fix/clw-audit-2026-06-09` diff-by-diff (TechLead verify L0–L3); confirm clean merge onto `main`.
- **M0.2** **M0.5 defect fixes** folded in (https exact-host, tool-in-key cwd + Windows, portable home, resolve_tool_id RAM).
- **M0.3** Full local gate (`scripts/gates.sh`) — fmt + clippy `-D warnings` + whole-workspace test (the check the audit deferred to CI).
- **M0.4** Merge; consolidated CHANGELOG entry.
- **M0.5** Prune merged branches (`docs/ac500-final-root-cause`, `docs/concurrency-finding`, `docs/sccache-*`, `docs/changelog-phase1`, `docs/handoff-enam`, `fix/429-backoff`, `docs/clw-deferrals-*`).
- **Gate for everything below.**

### M1 — Correctness & deferral **spine** (SERIAL) + new correctness WPs  *(clw)*
> The fleet proved M1.1→M1.2→M1.3 cannot be fanned out (shared `clw-client` `with_retry`/`put`, shared
> `clw-snapshot snapshot()`, shared frozen `clw-types`). Run as a serial chain. M1.4 is a **conditional tail** gated on D-1.
- ✅ **M1.1 — StreamCDC snapshot streaming** — **DONE (#27, 2026-06-09):** `fix(clw-snapshot): stream large files (M4) — bounded memory, dedupe-identical`. *(Original spec retained below for the record.)* Boundary-identical to FastCDC. **Non-vacuous equivalence proptest** (must force a `>4 MiB` body, assert ≥1 max-length cut + ≥1 multi-chunk case) **+ a frozen golden-root vector** so drift is caught even if both chunker paths drift together. `clw-types` edit is a **doc-only** decision-record comment — keep it in a **separate commit/PR** from M1.2's signature change. *Scope = remove the duplicate whole-file buffer; O(1)-per-file memory is a larger refactor — owner to confirm (loose end).*
- **M1.2 — Drop the exists-HEAD (audit L4).** `CasTransport::put → PutOutcome{Created,Existed}` so the HEAD-before-PUT is removed (2 req→1). **Frozen-trait change** rippling to exactly **4 impls** (real `clw-client` + fakes in snapshot/hydrate/run). **Premise correction:** server returns 201 for both → outcome only distinguishable in fakes; `chunks_uploaded` becomes best-effort. **DoD blocker:** a wiremock test asserting **zero HEAD requests** (the in-memory fakes can't observe the dropped HEAD). Faithful accounting needs a server change → **D-11**.
- **M1.3 — Full Option A rate-limit client.** Honor `Retry-After` + **AIMD** adaptive concurrency on 429 (today: blind retry). Reconcile with the audit's `--concurrency` flag + jitter (overlap — define once). Wire `--concurrency` into **hydrate** too (GETs get 429'd). Note: AIMD is per-invocation (CLI). **Must land after M1.2** (same `put` status-arm).
- **M1.4 — `clw rm` (conditional, gated on D-1).** `AcTransport::delete` (frozen-trait add, ripples same 4 fakes — **serialize with M1.2**) + server `DELETE /v1/ac`. **`ls --all` and `prune` are carved OUT** — they need ref-enumeration (**D-7**) and CAS-DELETE/LIST (**D-8**); see M6 erasure track. `rm` alone orphans CAS blobs (documented until prune exists).
- **M1.5 — Run-key hermeticity (new).** Fix env **absent-vs-empty collision** in `compute_key` (open on both branches). Make the tool-identity claim actually true (ties to M0.5).
- **M1.6 — Client resource-safety / abuse bounds (new).** Enforce bounds: manifest size / entry count / per-snapshot chunks (anti-OOM), hydrate disk-space/total-download guard (anti disk-fill from a malicious manifest), symlink-cycle guard. Complements H1 (traversal) with resource-exhaustion hardening.
- **M1.7 — Hydrate hardening (new, from fresh audit).** Re-verify fetched-chunk digests in hydrate; apply file mode **masked** (strip setuid/setgid/sticky/world-writable from untrusted manifests); materialize files **before** creating symlinks (ordering) so a symlink can't redirect a later write.
- **DoD:** zero `best-effort`/`TODO` in shipped code; non-vacuous proptest + golden vector green; wiremock HEAD-count test green; contract decision records written **separately**; M1.4 landed or explicitly blocked-on-D1.

### M2 — Cross-platform port (Windows + Linux) — **the clw-side long pole**  *(clw, NEW milestone)*
- **M2.0 — Toolchain de-hack** (was M2.1; the one clean **parallel** lane). Remove the founder-Mac rustup hack from `rust-toolchain.toml`/`README`/`gates.sh`; **add `components = ["rustfmt","clippy"]`** (clean-machine correctness); note the `ci.yml` edit is a no-op (hack lives in `gates.sh`). Move `CARGO_TARGET_DIR` persistent-cache assumption to `actions/cache` (consumed by M2.4 CI).
- **M2.1 — Make it compile on Windows.** `#[cfg]`-gate every unix-only prod construct with Windows equivalents: `clw-hydrate` (symlink, `PermissionsExt`, `from_mode`), `clw-manifest` `walk()` (`PermissionsExt`/`mode()`), `clw-cli` `home_dir`, mode synthesis. **Contract note:** `clw-types` mode doc currently says "Windows out of scope" — contradicts the platform mandate → **decision record** to resolve.
- **M2.2 — Windows-correct path & symlink semantics.** Reject backslash/drive-letter/UNC in `validate_rel_path`/`validate_symlink_target`; separator-normalize manifest paths on join; decide symlink strategy (real symlink needs Dev-Mode/admin → else copy-fallback default).
- **M2.3 — `clw run` on Windows (owner decision).** Today spawns `/bin/sh` (6 hardcoded). Decide: does the Windows binary ship `run` at all, and with what shell? Implement or gate the feature per OS.
- **M2.4 — Cross-platform CI/test matrix.** GitHub-hosted runners (+ `cargo-zigbuild`/cross for `aarch64-linux`). **Windows test lane** (today every e2e/integration test is unix-only). Note `aarch64-linux` is **build-only/untestable** on hosted infra → tracked decision (qemu / self-hosted-arm / accept build-only). Replaces the single self-hosted Mac (delivers exit-gate #11).
- **Sequencing:** M2.0 may run in parallel with the M1 spine; **M2.1–M2.4 run after** M0 (more unix-perms code to port) **and** the M1 cluster settles (they re-touch the same hydrate/run/snapshot test modules).

### M3 — Release engineering & distribution (proprietary, cross-platform)  *(clw + infra)*
- **M3.1 — Release pipeline** (`release.yml`): tag → build all targets → **sign** (macOS Developer ID + notarize; **Windows via cloud/HSM** — Azure Trusted Signing/DigiCert KeyLocker, *not* an exportable `.pfx`; Linux minisign/sigstore) → **signed** SHA-256 checksums → publish. Leave explicit **insertion points** for the M4 deny/audit + conformance gates.
- **M3.2 — Artifact format decisions.** A bare CLI Mach-O **can't be stapled** → ship notarized `.pkg`/`.dmg` or notarized zip (couple with install). **Minisign public-key distribution/trust-root + rotation** documented.
- **M3.3 — Authenticated download surface (owner decision + possible new dep).** Concrete host: R2-behind-CoreLink-auth (likely a **new corelink-server route → D-12**), private GitHub Release behind SSO, or a vendor. Define the **install-token vs tenant-PAT** bootstrap (how a fresh user authenticates before clw is configured).
- **M3.4 — Reproducible builds.** `SOURCE_DATE_EPOCH` + path-remap + pinned runner image; `strip`/`codegen-units=1` already in `[profile.release]`.
- **M3.5 — Install & uninstall.** Authenticated one-line installer (macOS/Linux) **+ `install.ps1`** for Windows (owner: in scope?); README/usage rewrite with **no PATH hack**; fix `repository`/`git clone` metadata (inconsistent with no-public-repo). **`clw uninstall`** removing binary + `~/.clw` (config+PAT) + cache + keychain + completions + telemetry artifacts (with confirm). DoD: **clean install AND uninstall verified on every platform.**
- **M3.6 — Versioning & stability charter.** Semver policy; CHANGELOG release cut; tag `v0.1.0` → road to `1.0`; **logged i18n decision** (English-only for 1.0, strings centralized); **cache-format compat policy** (see M5).

### M4 — Supply chain & security posture  *(clw)*
- **M4.1** `cargo-deny` (advisories+licenses+bans+sources) + `cargo-audit` in CI **and** the release gate; provision both tools on the runner. **Single source of truth** for the license allow-list (`deny.toml`, consumed by `cargo-about`). Until M5/M2.4 land: `bans.wildcards=warn` (internal path deps), and the invalid `UNLICENSED` needs a deny clarify-workaround.
- **M4.2** **License posture:** replace invalid-SPDX `UNLICENSED` (note: `license-file` **cannot** be inherited via `.workspace=true` — keep `license` consistent, bind EULA via shipped files); add `publish = false` to **all** crates (proprietary leak guard); decide on `webpki-roots` **CDLA-Permissive-2.0**; scope down the workspace-wide `tokio` feature union (only `clw-run` needs `process`).
- **M4.3** Per-release **CycloneDX SBOM**; attribution bundle (`cargo-about`) shipped **inside** the artifact (legal obligation → M3.1 DoD).
- **M4.4** `SECURITY.md` + vuln disclosure + threat-model summary (from the audit). Enforce the secrets-backup rule (`SIGNUP_TOKEN_KEY` → **D-2**).
- **M4.5** Promote the live conformance suite to a **pre-tag gate** (env-skip safe; fix the skip-equals-pass false-green).

### M5 — Observability, ops & product surface  *(clw)*
- **M5.1** Structured logging via `tracing` + PAT redaction; `-v/-q/--log-json`; replace ~35 ad-hoc `println/eprintln`.
- **M5.2** **Version/user-agent header** on every CoreLink request: `clw/<ver> (<os>; <arch>)` (and on the installer download).
- **M5.3** **Telemetry & crash-reporting WP** (was one bullet): first-run consent; **CI/non-interactive default OFF**; honor `DO_NOT_TRACK` + `CLW_TELEMETRY` kill-switch; **test-proven** redaction (no PAT/path/content); documented payload + retention + crash-dump destination; tie to Privacy Policy (M6).
- **M5.4** **Cache-format versioning & migration.** Stamp `~/.clw/cache` with a format version; on mismatch auto-migrate or **safely invalidate** (never silent stale-replay). The manifest `schema_version` + the run-key `v1→v2` bump prove the format evolves.
- **M5.5** **Support & customer docs.** `clw doctor` (redacted diagnostics bundle); published troubleshooting/FAQ; supported-version/EOL policy; in-product "report a bug / get help" pointer; runbook + SLOs.
- **M5.6** **Billing/quota-aware UX.** Distinguish 429 "rate-limited (retrying)" from "plan/quota limit reached (actionable: upgrade/contact)"; surface a usage/quota readout. Coordinate with D-6.
- **M5.7** **Output accessibility.** Honor `NO_COLOR` + auto-disable color on non-TTY; **`--json` on every command** (documented coverage contract); progress degrades gracefully on CI/non-TTY; errors not by color alone (508/EN 301 549).

### M6 — Legal, Privacy, Compliance & Right-to-Erasure  *(clw + legal + server, GO-LIVE BLOCKING, NEW)*
- **M6.1** **Privacy Policy** enumerating every datum clw transmits/stores (PAT, file **contents**, paths, hostnames, telemetry) with redaction guarantees; **Terms of Service**/acceptable-use.
- **M6.2** **Export-control self-classification** (ECCN/EAR Crypto Note for the TLS+BLAKE3 binary) — prerequisite to international distribution of a signed commercial binary (gates M3.1 international release).
- **M6.3** **DPA + sub-processor list** (Cloudflare R2/DO is a sub-processor).
- **M6.4** **Right-to-Erasure (GDPR Art.17) — go-live blocking.** A tenant must be able to actually delete their data. Requires: client `prune`/CAS-GC (mark-and-sweep over the tenant's live root set) **+** server **ref-enumeration LIST (D-7)** **+** server **CAS DELETE/LIST (D-8)**. Until these land, the residency promise is non-compliant.
- **M6.5** **Client data-protection.** `.clwignore`/exclude + secret-scanning warning so PII/secrets aren't blindly uploaded; documented data-retention.
- **M6.6** **Credential & key lifecycle.** Client `clw auth login/logout/whoami/rotate` + token-expiry/revocation; release-**signing-key** rotation/revocation runbook (Developer ID / Authenticode / minisign expiry & compromise).
- **M6.7** **Disaster recovery / business continuity.** DR runbook for signing-key loss/compromise (re-key, re-sign, customer notification), loss of the download surface, CI/org continuity; document the cache as ephemeral/reconstructible and the R2 snapshot durability/backup guarantee (server-side commitment — schedule as a dep if a backup SLA is promised).

### M7 — HuGR family integration (CoreLink + runners + hugit/githugr)  *(coordinated, multi-repo)*
- **M7.1 — Region/residency (corrected).** Two tracks: **(server)** D-3 signup-worker `enam` fix; **(clw)** *only if needed* — the region-endpoint mechanism is an **open question (a/b/c)**: (a) server redirects to a regional host and clw follows; (b) clw learns the tenant `primary_region` (needs a new lookup API → **D-10**) and maps region→host (a new `clw-types` contract); (c) the single global host region-routes internally → **clw needs no change** and the "clw resolves the regional endpoint" clause is deleted. **Confirm with the edge/server owner before scoping.**
- **M7.2 — Runner ↔ clw contract (corrected + freeze).** **Auth two-layer model:** the runner authenticates with a (per-job, scoped) **tenant PAT** via `Authorization: Bearer` + tenant-in-URL-path, exactly like any client; `x-corelink-tenant-id` is **server-internal** and **never** set by clw/runner. Decide **library-link vs shell-out**; define the runner's **config-resolution** path (a disposable runner has no `~/.clw/config.toml`, an injected secret — conflicts with the H2 `$HOME`-config hardening); define the **snapshot-back ref-naming** convention/ownership. Freeze an **anchor contract** before any runner-side or e2e work.
- **M7.3 — CoreLink runners repo (third repo, NEW track).** A runner-side WP to consume the frozen clw contract — **owner needed**; no WP exists today. Gates M7.5.
- **M7.4 — Campaign #1 / CI-acceleration (server-gated).**
  - **S1 `R2KvStore`** (persistent KV, per-tenant prefix, no hash-verify) — closes turbo `TODO(v2)`; backs cargo+turbo. Keystone. **→ D-4.**
  - **S2 Native cargo route (Option A header-trust)** on S1 + mount + tests — mirror `bazel_v2`. **→ D-5.**
  - **S3 Per-tenant write-ceiling scaling** — Option **B** (shard CAS writes across N sub-DOs by hash; safe because content-addressed) or **C** (bypass DO for stateless CAS PUTs). **→ D-6.** **Split out** the `wrangler.toml max_instances=5 vs live 20` drift as **D-6a**, ordered **before** B/C (else scaling is silently capped).
  - **C1 sccache role (disambiguate).** Evidence says **docs-only**: sccache (key≠hash(value)) speaks its own HTTP protocol to the cargo route; **clw the binary is not in the data path**. Rename "clw/sccache wiring" → **"sccache→CoreLink configuration docs"** unless the owner wants a new clw KV transport (which would be a frozen-contract addition, not "wiring").
- **M7.5 — End-to-end family conformance.** Gated CI smoke spanning **4 repos + Clerk**: signup → scoped PAT (**must grant CAS+AC** — the drifted D1 single-`scope` enum may not; **confirm**) → snapshot → hydrate on a runner → memoized run → CI cache hit. **Gates on:** D-3, the runner-side impl (M7.3), D-4/D-5/D-6, **D-9 per-job PAT minting**, and the Clerk scope confirmation.

---

## Cross-repo dependency ledger (corelink-server / runners / Clerk — coordinated, NOT this repo's debt)

| ID | Item | Spec | Blocks | Owner |
|---|---|---|---|---|
| D-1 | `DELETE /v1/ac/...` (idempotency + tenant-scope) | audit L5 | M1.4 `clw rm` | corelink-server |
| D-2 | Back up `SIGNUP_TOKEN_KEY` (write-only secret) | BUGREPORT §secrets | M4.4 | corelink-server |
| D-3 | signup-worker region fix (`enam` hardcode + `regionFromColo` + no-op `configureTenant`) | HANDOFF-signup-region-hardcode | M7.1 | corelink-server (GAP-5) |
| D-4 | `R2KvStore` (persistent KV behind turbo+cargo) | FINDING-sccache step 1 | M7.4 S2, C1 | corelink-server |
| D-5 | Native cargo route (Option A header-trust) | FINDING-sccache step 2 | M7.4 C1 | corelink-server |
| D-6 | Per-tenant CAS write scaling (shard-DO **B** / bypass **C**) | FINDING-per-tenant-write-ceiling | M7.4 S3, M5.6 | corelink-server |
| D-6a | `wrangler.toml max_instances=5` vs live `20` config drift | same finding | order before D-6 | corelink-server |
| **D-7** | **Ref-enumeration LIST endpoint** (tenant's refs) | M1.4/M6.4 briefs | `ls --all`, `prune`, **Art.17 erasure** | corelink-server |
| **D-8** | **CAS DELETE + CAS LIST** (blob GC) | M6.4 brief | `prune`, **Art.17 erasure** | corelink-server |
| **D-9** | **Per-job runner PAT minting** (scope/TTL/trigger) | family-integration critic | M7.2, M7.5 | corelink-server / CoreLink API |
| **D-10** | **Tenant `primary_region` lookup API** (only if M7.1 path (b)) | family-integration critic | M7.1 | corelink-server (conditional) |
| **D-11** | Distinct CAS-PUT status (created vs existed) for faithful accounting | M1.2 brief | accurate `chunks_uploaded` (optional) | corelink-server (low) |
| **D-12** | Authenticated binary **download route** (if R2-behind-CoreLink) | M3.3 brief | M3.3 | corelink-server (conditional) |
| CONFIRM | Clerk PAT **scope grants CAS+AC** (drifted single-`scope` enum) | FINDING-sccache blocker 2 | M7.5 e2e | Clerk/signup |
| CONFIRM | Region-routing mechanism a/b/c | family-integration critic | M7.1 | edge/server |

**Already landed upstream:** AC-500 root cause (PR #139, empty `R2_AC_BUCKET`); wrangler@4 deploy fix.

---

## Corrected sequencing / waves

```
M0+M0.5 (audit merge + audit-defect fixes)  ── gate ──┐
                                                       ▼
WAVE A (parallel after M0):
  • M2.0 toolchain de-hack         ── fully independent, fast, Sonnet-able
  • M1.1 StreamCDC                 ── first link of the spine (isolated to chunk/snapshot)
WAVE B (SERIAL spine — cannot parallelize internally):
  M1.1 → M1.2 → M1.3   [+ M1.4 if D-1]   (shared clw-client put/with_retry + snapshot() + clw-types)
  M1.5/M1.6/M1.7 fold in around the spine (disjoint files where possible)
WAVE C (after Wave A+B settle):
  M2.1 → M2.2 → M2.3 → M2.4   (Windows port re-touches hydrate/run/snapshot test modules + ci.yml)
M3 (release) → M4 (supply chain) → M5 (obs/product) ── then ── TAG v1.0-rc
M6 (legal/compliance) runs in parallel with M3–M5 (legal-led) but is exit-gate-blocking
─────────────────────────────────────────────────────────────────────
corelink-server track (parallel, owner-scheduled — START NOW, the long pole):
  D-3 → M7.1   |   D-6a → D-6   |   D-4 → D-5 → M7.4   |   D-1 → M1.4   |   D-7+D-8 → M6.4 erasure
  D-9 per-job PAT, M7.3 runner repo, Clerk-scope CONFIRM  → all gate M7.5 family e2e
```

**The clw-only long pole is internal:** the serial M1 spine **+ the M2 Windows port** (M-sized, downstream
of M2.0 and the M1 cluster). The full-family long pole remains the **corelink-server track** — schedule
D-3, D-4, D-6/6a, D-7, D-8, D-9 into that session **now**, in parallel.

## Immediate next actions (this week)

1. **M0 + M0.5** — verify, **fix the audit's own defects**, full local gate, merge. Highest leverage; unblocks all.
2. **M0.4** — prune merged branches.
3. **Open the corelink-server track** — hand D-3, D-4, D-6/6a, **D-7, D-8, D-9** (the new erasure + runner-PAT deps) to that session; they are the critical-path long poles and several are GDPR-blocking.
4. **Scope M2 (Windows port) immediately in parallel** (M2.0 can start now) — it is the clw-side long pole.
5. **Kick off M6 legal track** in parallel (export-control + privacy policy have lead time and gate international release).

## Confirm-with-owner queue (cannot resolve from this repo)

- Region-routing mechanism (a/b/c) → shapes M7.1 and whether **D-10** exists.
- Runner: library-link vs shell-out; per-job PAT minting trigger/scope/TTL (**D-9**).
- hugit/githugr seam (does githugr trigger snapshot/run on push? does hugit consume clw refs?) — M7.4-adjacent, design-first.
- `clw run` on Windows: ship the feature or gate it per-OS (M2.3).
- Download surface host (R2-behind-CoreLink **D-12** vs GitHub-SSO vs vendor) + install-token bootstrap.
- M1.1 memory target: "remove duplicate buffer" (scoped) vs "O(1)-per-file" (larger refactor).
- `install.ps1` for Windows one-line install: in scope?
