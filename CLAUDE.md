# CLAUDE.md — project guide for Claude Code

> Durable, in-repo context for any Claude session on `clw` / corelink-workspaces. Keep this current.
> Point-in-time/resume state lives in `docs/HANDOFF-*-clw-TL-state.md` (read the latest first).

## What this is
`clw` (CoreLink Workspaces) — a Rust workspace CLI: `snapshot` / `hydrate` / `run` over a
content-addressed backend (CAS + AC). Auth = `Authorization: Bearer <PAT>` + tenant-in-URL-path
(never `x-corelink-tenant-id`). Config is env-driven (`CLW_ENDPOINT/CLW_TENANT/CLW_TOKEN/CLW_REF_DOMAIN`),
no config file required. Single human owner = Gustavo (stakeholder + approver).

**Status: SHIPPED — v0.1.5 is live (open beta; v0.1.0 launched 2026-06-18, v0.1.5 cut 2026-07-06).**
`brew install humanguardrail/clw/clw`. v0.1.5 ships `clw hydrate --manifest-digest` (#153, the W6
content-addressed-hydrate deliverable that unblocks the runners' CF-native check-host image).

## Build / test / gate
- The repo gate is `bash verify.sh` (FULL) or `bash verify.sh --scoped [base]` (change-scoped fast lane).
  `gates.sh` runs fmt/clippy/test/deny lanes.
- On the self-hosted CI Mac, the toolchain is ISOLATED (a shared rustup-nuke happens) — use
  `CARGO_HOME=$HOME/.clw-ci-rust/cargo RUSTUP_HOME=$HOME/.clw-ci-rust/rustup $HOME/.clw-ci-rust/cargo/bin/cargo`.
  Pinned `RUST_TOOLCHAIN` (currently 1.91.1). Cross-builds use cargo-zigbuild (ziglang in an isolated venv).

## Repo conventions (enforced — do not relearn the hard way)
- **DCO: commit with `-s`** (sign-off gate blocks otherwise). Conventional Commits for titles.
- **Tiered flow:** tiny self-verified docs/≤50 LOC → direct to main is OK; anything bigger or
  agent-authored → short-lived branch → PR → independent cold review → squash-merge. ~1 PR in flight.
- **Never force-push / history-rewrite on main.** The destructive-guard blocks the agent from force-push
  (a human runs it out-of-band) → prefer fresh-branch / add-a-commit. Never `--no-verify`. Don't commit
  telemetry/state (`.techlead/`, `__pycache__`, audit logs) — the git-hygiene-guard blocks it.
- **The owner merges via GitHub; the local clone goes stale** → always `git fetch` + diff `origin/main`
  before assuming. Trust the repo over any in-context/pre-compaction summary.

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
