# SESSION SUMMARY — `clw` go-live hardening (2026-06-14)

> **Read this first when you're back.** Executive consolidation of everything the
> 2026-06-14 hardening waves landed on `main`, the current go-live status, and the
> short list of things that **cannot be done in-repo by an agent** — they need
> **you** (owner), **legal**, or a **server/infra** change.
>
> **Baseline:** `main @ f8f5edf`. Authoritative detail lives in
> `docs/ROADMAP-go-live.md` (milestones, dependency ledger, exit gate),
> `CHANGELOG.md` (what shipped), and `docs/HANDOFF-corelink-server-track.md`
> (the server deps). This doc points at them; it does not replace them.

---

## (a) What landed this session — by theme

All of the below shipped on `main` across the day's waves (`70ba1cb`, `6e06f17`,
`b11c890`, the `clw auth`/M5 wave, and the docs-sync wave). Full per-commit detail:
`docs/ROADMAP-go-live.md` §Progress log; user-facing detail: `CHANGELOG.md`.

### Security / audit
- **M0 / M0.5** — standing audit merged (#26) **and its own defects fixed**: the
  https loopback allow-list was **prefix-bypassable** (`http://127.0.0.1.evil.com`
  → PAT-in-cleartext leak) → now an **exact-host** check (CRIT); `clw run`
  tool-identity resolved against the wrong cwd (HIGH); config values written
  without escaping (HIGH). Full local gate green.
- **M4.4** — `SECURITY.md` + `docs/THREAT-MODEL.md` (disclosure policy + client
  threat model from the audit).
- Invariants now enforced and documented: path-traversal closed, PAT `0600`/`0700`
  + redaction + https-only, **zero `unsafe`**, no prod `unwrap`/`panic`.

### Windows port (the clw-side long pole) — **M2**
- **M2.0** — toolchain de-hack: removed the founder-Mac rustup hack;
  `rust-toolchain.toml` pins `1.91.1` + `rustfmt`/`clippy`.
- **M2.1** — the **entire workspace now compiles on Windows x86_64** (`USERPROFILE`
  home resolution, `#[cfg]`-gated unix-only constructs). Proven locally via
  `cargo check --workspace --target x86_64-pc-windows-gnu` (0 errors).
- **M2.2** — Windows-aware path validation (reject backslash / drive-letter / UNC
  on **every** platform) + Windows symlink **copy-fallback** (closes the prior
  "symlink-on-Windows" v1 limitation as a documented copies-not-links divergence).
- **M2.4** — cross-platform CI matrix (Linux blocking, Windows informational —
  to flip to required once green).

### Rate-limit / governor — **M1.3**
- Honor `Retry-After` + **AIMD** adaptive concurrency on HTTP 429 (the real 429
  relief), replacing blind retry; `--concurrency` wired into hydrate as well as
  snapshot. **M1.2 was PULLED** (always-PUT rejected — it would kill zero-upload
  dedupe and *increase* write pressure; see ROADMAP decision record).
- Also: **M1.5** run-key hermeticity (unset ≠ empty env, key domain `v2`→`v3`),
  **M1.6** hydrate resource-safety bounds (anti-OOM / anti disk-fill / symlink-cycle),
  **M1.7** hydrate hardening (re-verify fetched digests, mask mode bits,
  materialize files before symlinks).

### CLI surface — auth / doctor / accessibility / telemetry
- **M6.6** — `clw auth login/logout/whoami` (fully offline, redacted;
  `rotate` is a **server-dependent stub**).
- **M5.5** — `clw doctor` (redacted diagnostics bundle; `--json`; always exits 0).
- **M5.7** — accessibility: `--no-color`/`NO_COLOR` + auto-off on non-TTY;
  `--json` coverage as a documented contract (`clw run` the documented exception).
- **M5.3** — telemetry consent: **off by default**, `DO_NOT_TRACK`/`CLW_TELEMETRY`
  kill-switches, redaction-proof `Event` enum, **no network backend** (no-op sink).
- **M5.2** user-agent header; **M5.4** cache-format versioning + invalidate-on-mismatch;
  **M5.6** quota-aware 429 message; **M6.5** secret-file warning on snapshot.

### Supply-chain / legal
- **M4.1** — `cargo-deny` (advisories + licenses + bans + sources) + `cargo-audit`
  in the gate **and** the release lane.
- **M4.2** — license posture: removed invalid-SPDX `UNLICENSED`, `publish = false`
  on **all** crates, `EULA.md` + `NOTICE` + `about.toml` + finalized `deny.toml`.
- **M4.3 / M4.5** — CycloneDX SBOM + live-conformance pre-tag gate scaffolding.
- **M6 legal drafts** — `docs/PRIVACY-POLICY.md`, `docs/TERMS-OF-SERVICE.md`,
  `docs/EXPORT-CONTROL.md`, `docs/DPA-subprocessors.md`, `docs/DR-business-continuity.md`.

### Docs
- **M3 scaffold** — `release.yml` skeleton + `docs/DISTRIBUTION.md`.
- Brought README/usage/CHANGELOG/THREAT-MODEL/ROADMAP **in sync with the shipped
  binary** (they were frozen at phase-1); fixed the install/license contradiction.
- Server-track handoffs written: `docs/HANDOFF-corelink-server-track.md` (master)
  + the D-3/D-7/D-8/D-9 spec docs.

---

## (b) Current go-live status — DONE

Done on `main`: **M0/M0.5, M1.1, M1.3, M1.5, M1.6, M1.7, M2.0, the M2 Windows port
(incl. symlink copy-fallback + Windows-aware path validation), M2.4, M4 (publish=false
+ deny + gate + EULA posture), M4.3 SBOM + M4.5 conformance-gate scaffold, M4.4,
M5.2/5.3/5.4/5.5/5.6/5.7, M6.6 `clw auth` login/logout/whoami, M3 scaffold, M6 legal
drafts + M6.5, and the D-7/D-8/D-9 handoff specs.**

**In-tree stubs (landed, not yet wired — both server-gated):** `clw auth rotate`
(needs a server token-rotation endpoint) · the telemetry network backend (the
consent gate + kill-switches + no-op sink shipped; no backend).

**Next clw-side (an agent *can* do these in-repo):** M5.1 structured `tracing`
(serial, cross-cutting) · flip the Windows CI lane from informational to required
once it's green.

---

## (c) REMAINING — REQUIRES HUMAN OR SERVER (cannot be done in-repo by an agent)

These are the **only** things blocking go-live that an in-repo agent cannot close.
Each needs **you**, **legal**, or a **server/infra** change. This is your to-do list.

| # | Item | Who | Blocks (exit gate / milestone) |
|---|---|---|---|
| 1 | **Legal sign-off** of the EULA / Privacy / ToS / export-control (ECCN) / DPA drafts | **legal** | Exit gate #9 (compliance published); export-control gates international release |
| 2 | **GitHub Actions BILLING** — the cross-platform (Windows/Linux) CI is **blocked on a "spending limit"**; raise/enable billing | **you / org admin** | M2.4 Windows lane green → exit gate #2 & #11 (CI off the single founder Mac) |
| 3 | **Release signing infra** — Apple Developer ID + notarization, Windows Azure Trusted Signing (HSM, *not* an exportable `.pfx`), Linux minisign — **plus the authenticated download surface** | **you / infra** | M3.1/M3.3; exit gate #4 (signed binaries + authenticated install/uninstall) |
| 4 | **corelink-server track** — D-1, D-3, D-4, D-6 (+D-6a), D-7, D-8, D-9 | **server session** | residency, GDPR erasure, campaign #1, family e2e — see below |
| 5 | **M6.4 right-to-erasure** (GDPR Art.17) — needs server **D-7 (ref-LIST) + D-8 (CAS DELETE/LIST)** as a unit before the client `prune`/CAS-GC can exist | **server, then clw** | Exit gate #8 (tenant data actually deletable) — launch-blocking |
| 6 | **M7 family integration** + the **CoreLink runners repo** — a *third* repo with **no owner and no WP today** | **you (assign an owner)** | M7.3 → M7.5 family e2e |
| 7 | **`clw auth rotate` + the telemetry backend** — both depend on server/product decisions (token-rotation endpoint; telemetry destination + retention) | **you / server** | unblocks the two in-tree stubs |

### The server track (item 4), in one screen

Full brief: **`docs/HANDOFF-corelink-server-track.md`**. Critical path:

- **Confirmations first (cheap, day one):** **CONFIRM-a** Clerk PAT scope grants
  **CAS+AC** (the **#1 blocker** — gates D-7/D-8/D-9 auth + the whole family e2e);
  **CONFIRM-b** region-routing mechanism a/b/c (decides if **D-10** even exists);
  **D-6a** reconcile the `max_instances` 5-vs-20 drift (before D-6).
- **GDPR / launch-blocking:** **D-3** signup-worker region (`enam`) fix (residency);
  **D-7 + D-8 together** (ref-LIST + CAS DELETE/LIST = the erasure unit → M6.4);
  **D-1** `DELETE /v1/ac` (`clw rm`, erasure half).
- **Campaign #1 (CI-acceleration, the long pole — start D-4 in parallel now):**
  **D-4** `R2KvStore` keystone → **D-5** native cargo route → **D-6** per-tenant
  CAS write scaling.
- **Family auth + e2e:** **D-9** per-job runner PAT minting (scope **must** be
  CAS+AC) → freeze the runner contract → **M7.5** family e2e.

> Shared server-side invariant for D-7/D-8/D-9: **Bearer tenant-PAT + tenant-in-URL-path
> is the sole client tenant authority; `x-corelink-tenant-id` is server-internal and
> must be ignored/rejected when set by a client.**

---

## One-line owner action queue

1. Raise the **GitHub Actions spending limit** (unblocks cross-platform CI). 2. Send
the **5 legal drafts** to counsel for sign-off. 3. Open the **corelink-server session**
and hand it `docs/HANDOFF-corelink-server-track.md` (start CONFIRM-a/b + D-3 + D-4).
4. Assign an **owner for the CoreLink runners repo** (M7.3). 5. Decide the **signing +
download surface** (Developer ID / Azure Trusted Signing / minisign + the auth'd host).
