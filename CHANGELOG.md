# Changelog

## [Unreleased] — go-live hardening waves (2026-06-14)

> Everything below shipped on `main` after the phase-1 baseline: the M0.5 audit-defect
> fixes, the M2 Windows port, and the M1/M4/M5/M6 hardening waves. The phase-1 section
> that follows is retained for the record.

### Added
- **`clw auth`** — manage the locally-stored CoreLink identity, fully offline (no network call):
  `login` (persist resolved endpoint/tenant/token to `~/.clw/config.toml`, forced `0600`),
  `logout` (drop the `token` key; `--all` clears the whole file, with a confirm unless `--yes`),
  `whoami` (print the resolved identity, token shown as `<set>`/`<unset>` only).
  `rotate` is a stub — not yet supported pending a server token-rotation endpoint
  (use `auth login` with a new `--token` to replace the stored token). (M6.6)
- **`clw doctor`** — support-ready, **redacted** diagnostics bundle (version, os/arch, config
  path/existence/mode, endpoint, tenant + token presence, cache dir); the token is never
  printed; `--json` supported; points at `SECURITY.md` for disclosure; always exits 0. (M5.5)
- **`--no-color` / `NO_COLOR`** — accessibility flag; color is also auto-disabled when the
  `NO_COLOR` env var is set or stdout is not a TTY (single `color_enabled` policy seam). (M5.7)
- **`--json` coverage** audited across commands as a documented contract; `clw run` is the
  one **documented exception** (the wrapped command owns stdout, so it has no `--json` form). (M5.7)
- **Telemetry consent scaffolding** — off by default, with kill-switches (`DO_NOT_TRACK=1` and
  `CLW_TELEMETRY=0` always win; `CLW_TELEMETRY=1` opts in; `telemetry = "on"|"off"` in config).
  **No network backend** — events route to a documented no-op sink; the `Event` type is a
  fixed enum (no String/PathBuf/token field) so PII is redaction-proof by construction; never
  prompts in CI/non-TTY. (M5.3)
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
- **No erasure command exists yet.** `clw` has no `clw rm`; a future `clw rm` (AC-ref delete)
  is tracked as **M1.4**, gated on server dependency **D-1**. CAS blob GC / right-to-erasure
  is **M6.4** (gated on D-7/D-8). Documented as a tracked residual, not silently accepted.
- Threat model updated to reflect the shipped Windows path validation and the erasure gap
  (see `docs/THREAT-MODEL.md`).

### CI
- **`cargo-deny`** (advisories + licenses + bans + sources) + `cargo-audit` wired into the
  gate and the release lane. (M4.1)
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
