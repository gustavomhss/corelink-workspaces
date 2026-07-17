# CLAUDE.md — project guide for Claude Code

> Durable, in-repo context for any Claude session on `clw` / corelink-workspaces. Keep this current.
> Point-in-time/resume state lives in `docs/HANDOFF-*-clw-TL-state.md` (read the latest first).

## What this is
`clw` (CoreLink Workspaces) — a Rust workspace CLI: `snapshot` / `hydrate` / `run` over a
content-addressed backend (CAS + AC). Auth = `Authorization: Bearer <PAT>` + tenant-in-URL-path
(never `x-corelink-tenant-id`). Config is env-driven (`CLW_ENDPOINT/CLW_TENANT/CLW_TOKEN/CLW_REF_DOMAIN`),
no config file required. Single human owner = Gustavo (stakeholder + approver).

**Status: SHIPPED — v0.1.11 is live (open beta; v0.1.0 launched 2026-06-18).**
`brew install humanguardrail/clw/clw`. v0.1.11 (2026-07-13) is a stabilization wave from a 5-angle
hidden-debt sweep (core code was clean; the recurring debt lived in the GATE layer): signing is now
FAIL-CLOSED on real releases (a real `vX.Y.Z` with the minisign secret absent hard-fails instead of
publishing unsigned + bumping brew — #182); the PR scoped-lane no longer has coverage holes (tests
now include the sink crates clw-cli/clw-integration-tests, the windows-gnu cross-check is a UNIVERSAL
tripwire in both lanes, clippy runs `--all-features` — #183); the last two size-unbounded reads are
capped (cred-ticket body + snapshot small-file fast-path — #184); `cargo-audit` pinned `@0.22.2`.
v0.1.6 (2026-07-09) shipped the 5-angle adversarial-audit remediation (4 HIGH + 6 MED + LOW cluster).
v0.1.7 (2026-07-10) closes the external-symlink LOW. v0.1.5 shipped `clw hydrate --manifest-digest`
(#153). **`main` is branch-protected (`gates`+`dco` required, enforce_admins).**

## Build / test / gate
- The repo gate is `bash scripts/gates.sh` (FULL) or `bash scripts/gates.sh --scoped [base]` (change-scoped
  fast lane). It runs fmt/clippy/test/deny(+windows-cross) lanes. (There is no root `verify.sh`.)
- **Test-rigor lanes (make coverage PROVABLE, additive — NOT part of the required gate): see `docs/TESTING.md`.**
  `coverage.yml` (llvm-cov, per-PR, measure→ratchet→gate), `fuzz.yml` (nightly cargo-fuzz on the clw-types
  untrusted-input targets, crash=red), `mutation.yml` (weekly cargo-mutants, report-only→follow-on blocks
  new survivors). All on github-hosted ubuntu.
- On the self-hosted CI Mac, the toolchain is ISOLATED (a shared rustup-nuke happens) — use
  `CARGO_HOME=$HOME/.clw-ci-rust/cargo RUSTUP_HOME=$HOME/.clw-ci-rust/rustup $HOME/.clw-ci-rust/cargo/bin/cargo`.
  Pinned `RUST_TOOLCHAIN` (currently 1.91.1). Cross-builds use cargo-zigbuild (ziglang in an isolated venv).

## Repo conventions (enforced — do not relearn the hard way)
- **DCO: commit with `-s`** (sign-off gate blocks otherwise). Conventional Commits for titles.
- **Flow (main is branch-protected — no direct-to-main):** as of 2026-07-09 `main` requires the `gates` +
  `dco` status checks GREEN and enforces admins, force-push/deletion blocked, linear history. So EVERYTHING
  goes: short-lived branch → PR → independent cold review → **squash-merge on green**. No approval-review is
  required (the cold review is the gate, not the GitHub button), so the TechLead self-merges once checks pass.
  Run the FULL gate LOCALLY first, then merge. ~1 PR in flight.
- **Never force-push / history-rewrite on main.** The destructive-guard blocks the agent from force-push
  (a human runs it out-of-band) → prefer fresh-branch / add-a-commit. Never `--no-verify`. Don't commit
  telemetry/state (`.techlead/`, `__pycache__`, audit logs) — the git-hygiene-guard blocks it.
- **The TechLead owns merge/approval end-to-end** (owner delegated: "prs, merge etc sao responsabilidade
  sua, revisar, aprovar/reprovar, pedir review independente"). The local clone still goes stale → always
  `git fetch` + diff `origin/main` before assuming. Trust the repo over any in-context/pre-compaction summary.

## Release (proven end-to-end)
- Tag `v*` triggers `.github/workflows/release.yml`: preflight → conformance → 5-target build+sign
  (mac×2 native, linux×2 + windows-gnu cross) → SBOM → checksum + publish.
- **`vX.Y.Z-rc.N` = dry-run** (⇒ `--prerelease`, NO formula bump, reversible). Cut an rc to exercise the
  tag-only legs before a real release — rc dry-runs have repeatedly caught latent, tag-only bugs.
- **Real `vX.Y.Z`** publishes the GitHub Release on the PUBLIC mirror `HumanGuardrail/clw-releases` (latest)
  and bumps the formula in the PUBLIC tap `HumanGuardrail/homebrew-clw`.
- Binaries are minisign-signed (`SHA256SUMS.minisig`, trust-root `docs/minisign.pub`). SLSA provenance is
  gated to public repos only (deferred per waiver — this repo is private). macOS Developer-ID + Azure
  Windows signing are deferred.
- **Transient failure:** the self-hosted runner can hit a network `ENOTFOUND` on artifact upload — NOT a
  code bug. Just `gh run rerun <id> --failed` (same tag; the fail-safe skips publish if any build fails,
  so there is never a partial release).

## Distribution model
clw stays proprietary-LICENSED (EULA governs use) but the binary is PUBLICLY downloadable. Main repo
stays PRIVATE; binaries live on the public mirror (a private repo's Release assets aren't public).
`install.sh` is the curl fallback (public; strips macOS quarantine). Windows = direct `.exe`.

## Cross-team seam (frozen — `docs/CLI-SURFACE-EXIT-FREEZE.md`)
The runner DRIVES `clw snapshot→hydrate→run` (never reimplements). `clw run` exits the child's code
verbatim; a clw-internal failure where the child NEVER ran exits the RESERVED `125`. Non-zero is never
cached. Inject identity via `CLW_*` env; `CLW_REF_DOMAIN=runner` → keyspace `clw/ref/runner/v1/`.
