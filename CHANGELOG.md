# Changelog

## [Unreleased] — go-live hardening waves (2026-06-14)

> Everything below shipped on `main` after the phase-1 baseline: the M0.5 audit-defect
> fixes, the M2 Windows port, and the M1/M4/M5/M6 hardening waves. The phase-1 section
> that follows is retained for the record.

### Added
- **`clw rm --name <ws>`** — delete a workspace ref (AC erasure). **LIVE:** the server `DELETE /v1/ac`
  endpoint (D-1) is deployed and conformance-green, so this performs a **real, idempotent** ref delete
  against production. Destructive-command rails: a y/N confirmation unless `--yes`, and a refusal to
  delete non-interactively / with `--json` without `--yes` (it never silently deletes in a pipeline).
  Removes only the ref, not the underlying CAS blobs. (M1.4, server dependency D-1)
- **`clw prune`** — live CAS garbage collection: a mark-and-sweep over the manifest roots reachable
  from refs that deletes unreferenced blobs. **Dry-run by default**; `--yes` to actually delete. Has a
  safety floor that refuses to wipe a non-empty CAS when the live set is empty (a likely stale ref
  index). (gated on D-7/D-8, now LIVE)
- **`clw erase`** — live GDPR Art.17 full-tenant erasure: irreversibly deletes **every ref and every
  CAS blob** for the tenant. Three rails: **dry-run by default**, `--yes` to execute, and a
  typed-tenant confirmation (interactive prompt, or `--confirm-tenant <TENANT>` for non-interactive /
  `--json`). (M6.4, gated on D-7/D-8, now LIVE)
- **`clw uninstall`** — remove the local `clw` footprint: the `~/.clw` config (including the stored
  PAT) and the local cache. Prompts for confirmation by default; `--yes` skips the prompt and
  `--dry-run` previews exactly what would be removed without deleting anything. It cannot delete its
  own executable, so it **prints the binary path** for you to remove manually.
- **`clw completions <bash|zsh|fish|powershell|elvish>`** — print a shell-completion script for the
  named shell to stdout, to source or install per your shell's convention.
- **`clw auth`** — manage the locally-stored CoreLink identity, fully offline (no network call):
  `login` (persist resolved endpoint/tenant/token to `~/.clw/config.toml`, forced `0600`),
  `logout` (drop the `token` key; `--all` clears the whole file, with a confirm unless `--yes`),
  `whoami` (print the resolved identity, token shown as `<set>`/`<unset>` only).
  In v1, `rotate` directs you to re-authenticate — run `auth login` with the new `--token` to
  atomically replace the stored credential; a dedicated one-step rotate (mint-new + revoke-old) is a
  post-launch nicety that needs a server endpoint. (M6.6)
- **`clw doctor`** — support-ready, **redacted** diagnostics bundle (version, os/arch, config
  path/existence/mode, endpoint, tenant + token presence, cache dir); the token is never
  printed; `--json` supported; points at `SECURITY.md` for disclosure; always exits 0. (M5.5)
- **`--no-color` / `NO_COLOR`** — accessibility flag; color is also auto-disabled when the
  `NO_COLOR` env var is set or stdout is not a TTY (single `color_enabled` policy seam). (M5.7)
- **`--json` coverage** audited across commands as a documented contract; `clw run` is the
  one **documented exception** (the wrapped command owns stdout, so it has no `--json` form). (M5.7)
- **No telemetry (by decision).** clw collects no usage data and ships with no mechanism to
  send any — there is no telemetry sink, opt-in flag, or phone-home path in the binary. The
  only data transmitted is the workspace content you explicitly `snapshot`/`run` plus the auth
  token. (M5.3 — owner decision: ship with zero telemetry rather than a dormant backend.)
- **Windows support** — the entire workspace now compiles and runs on Windows x86_64
  (`USERPROFILE` home resolution, `#[cfg]`-gated unix-only constructs). (M2.1)
- **Client resource-safety bounds** in `hydrate` — manifest entry-count / byte / symlink
  guards (anti-OOM, anti disk-fill, symlink-cycle guard) against a hostile manifest. (M1.6)
- **`SECURITY.md` + threat model** (`docs/THREAT-MODEL.md`) — disclosure policy and the
  client threat model summarized from the audit. (M4.4)
- **Legal/attribution bundle** — `EULA.md`, `NOTICE`, `about.toml` (cargo-about), and a
  finalized `deny.toml` license posture; M6 legal drafts (privacy/ToS/export-control/DPA/DR). (M4.2/M6)

### Changed
- **Windows hydrate copies symlinks instead of linking them.** Windows has no unprivileged
  symlinks, so a hydrated symlink entry becomes a **copy** of its target (file → bytes,
  directory → recursive subtree). This is a documented, non-silent copies-not-links
  divergence; it closes the prior "symlink-on-Windows" v1 limitation. (M2.2)
- **Windows-aware path validation.** `validate_rel_path` / `validate_symlink_target` now reject
  Windows backslash, drive-letter, and UNC forms on **every** platform (not unix-only). (M2.2)
- **Adaptive rate-limit client.** On HTTP 429 the client now honors `Retry-After` and applies
  **AIMD** adaptive concurrency (per-invocation) instead of blind retry; `--concurrency` is
  wired into hydrate as well as snapshot. (M1.3)
- **User-Agent header** `clw/<ver> (<os>; <arch>)` sent on every CoreLink request. (M5.2)
- **Quota-aware 429 UX** — distinguish "rate-limited (retrying)" from "plan/quota limit
  reached (actionable)" in the surfaced message. (M5.6)
- **Cache-format versioning** — `~/.clw/cache` is stamped with a format version; on mismatch
  the cache is safely invalidated rather than silently replaying stale content. (M5.4)
- **Run-key hermeticity** — `clw run` now distinguishes an **unset** env var from an **empty**
  one in the cache key (key domain `v2`→`v3`); secret-looking files are warned on at snapshot. (M1.5 / M6.5)
- **Toolchain de-hack** — removed the founder-Mac rustup hack; `rust-toolchain.toml` declares
  `components = ["rustfmt", "clippy"]` so a clean machine builds correctly. (M2.0)

### Security
- **Erasure is LIVE and destructive.** The server dependencies **D-1** (`DELETE /v1/ac`), **D-7**
  (ref enumeration), and **D-8** (CAS list/delete) are deployed and conformance-green, so the
  erasure surfaces are live: `clw rm` (**M1.4**, AC-ref delete) performs a real idempotent delete with
  confirm rails; `clw prune` (CAS GC) and `clw erase` (**M6.4**, GDPR Art.17 full-tenant erasure) are
  live, dry-run-by-default, and guarded (`--yes`, and a typed-tenant confirmation for `erase`). These
  commands delete real production data — the safety rails, not a server gap, are now what stand
  between the operator and an irreversible delete.
- Threat model updated to reflect the shipped Windows path validation and the erasure gap
  (see `docs/THREAT-MODEL.md`).

### CI
- **`cargo-deny`** (advisories + licenses + bans + sources) wired into the local gate and the
  release lane. The **`cargo-audit`** RustSec scan is **CI-provisioned / best-effort locally**: the
  local gate runs it only if `cargo-audit` is installed, otherwise it prints "cargo-audit not
  installed — CI provisions it; skipping locally" and continues — so the authoritative RustSec scan
  runs in CI, not the local gate. (M4.1)
- **Cross-platform CI matrix** — Linux blocking, Windows informational (to flip to required
  once green); release pipeline scaffold + distribution decisions; **`publish = false`** on all
  crates (proprietary-leak guard). (M2.4 / M3 / M4)
- Per-release **CycloneDX SBOM** + live-conformance pre-tag gate scaffolding. (M4.3 / M4.5)

## [Unreleased] — phase-1 complete (2026-06-05)

### Security & hardening (audit 2026-06-09)
- **Path traversal / symlink-escape closed in `hydrate`** — `Manifest::from_bytes` now rejects any entry path or symlink target that is absolute or contains a `..`/`.` component; `hydrate` re-validates (defense-in-depth). Closes a write-outside-`dest` (zip-/symlink-slip) vector when hydrating an untrusted or tampered workspace. (H1)
- **PAT no longer world-readable at rest** — `~/.clw/config.toml` written `0600` (was umask-dependent, often `0644`), `~/.clw` set `0700`, local cache root `0700`. (H2)
- **PAT never sent over plaintext** — `HttpClient` requires `https://` (loopback excepted for dev/tests). (M2)
- **No stale memoized replays after a toolchain upgrade** — `clw run` key now folds the content digest of `command[0]`; key domain `clw/run/v1/`→`v2/`. (M3)
- **Removed an avoidable `unsafe`** raw-pointer cache borrow in `hydrate` (now direct sync access, matching `snapshot`); `hydrate` streams each chunk to disk instead of buffering whole files in memory. (M1, M4-hydrate)
- **`--concurrency` flag** (default 8) to throttle snapshot/hydrate; retry backoff jittered to avoid synchronized herds. (M5)
- **Hygiene** — non-deprecated `home_dir`; config parser documents its minimal-TOML subset and ignores inline comments after quoted values; `--token` help warns it is visible in the process list. (LOWs)

### Audit follow-up — defects found in the audit itself (M0.5, 2026-06-14)
> A cold re-audit of the landing branch found three defects *inside* the audit's own fixes; closed here before merge.
- **HTTPS loopback allow-list was prefix-bypassable** — `http://127.0.0.1.evil.com` / `http://localhostx` passed the old `starts_with` check and would have leaked the PAT in cleartext. Now an **exact-host** check (`127.0.0.1` / `localhost` / `[::1]`, optional `:port`/path). Regression test added. (CRIT)
- **`clw run` tool-identity resolved against the wrong cwd** — a relative `command[0]` was resolved against the process cwd instead of the run cwd, yielding an unstable/incorrect key. Now resolved against the run cwd. (HIGH)
- **Config values written without escaping** — a token containing `"` or `\` was silently truncated on round-trip. Writer now escapes (`toml_escape`) and the parser honors `\"`/`\\`. Round-trip test added. (HIGH)
- **Windows-unblock hygiene** — the two shell-script-based `clw run` tool tests are now `#[cfg(unix)]`-gated (they were unconditional and would block the Windows target in M2).
- **Tracked (not done here, by design):** folding the tool binary read into a cheaper metadata fingerprint (perf) → roadmap **M1.5**; a hydrate defense-in-depth regression test → **M1.7**. See `docs/ROADMAP-go-live.md`.

### Added
- **The workspace primitive, client-side** — `clw` CLI: `init`, `snapshot`, `hydrate`, `status`, `run`, `ls` (#13, #17).
- `clw-types`: frozen contract — BLAKE3 digests, manifest v1 (canonical JSON, Merkle-style), RefRecord namespace over the AC, CasTransport/AcTransport traits (contract hash fe251941).
- `clw-chunk`: FastCDC chunking, frozen params 256K/1M/4M (#7).
- `clw-client`: HTTP CAS/AC transports with content-integrity verification, retry, PAT auth (#5).
- `clw-manifest`: ignore-aware tree walk + manifest diff (#3).
- `clw-cache`: atomic local content store, self-healing reads, LRU gc (#4).
- `clw-snapshot`: walk→chunk→dedupe→upload→ref pipeline; second snapshot of an unchanged tree uploads zero chunks (#10).
- `clw-hydrate`: ref→manifest→cache-first parallel materialization, atomic writes, mode bits (#11).
- `clw-run`: AC-backed command memoization — deterministic commands cached, replayed byte-exact, success-only (#9).
- Integration suite over real HTTP (wiremock stateful CoreLink) (#12) and binary-level e2e: snapshot→wipe→hydrate bit-identical roundtrip (#16).
- Live-API conformance suite, feature-gated (`--features live`, env-skip without PAT) (#14).
- Docs: SPEC-manifest v1 + ARCHITECTURE (#2), real README + usage recipes (#15).

### CI
- DCO gate, PR template (#1); change-scoped gates — PR lane scoped to the diff, main always full-lane; persistent CI target dir (#8).

### Fixed
- Compile-time const-assert for chunk params (clippy clean) (#6).
