# Changelog

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
