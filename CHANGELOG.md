# Changelog

## [0.1.4] — post-launch hardening: data-safety, fuzzing, daily conformance canary (2026-06-21)

> Staged and tag-ready: `Cargo.toml` is bumped to 0.1.4. The release is cut with
> `git tag -a v0.1.4 -m "clw 0.1.4" && git push origin v0.1.4` once GitHub-hosted Actions minutes return
> (`release.yml` runs the 5-target cross-build on hosted runners). A few small, safe fixes landed on `main`
> after the initial stage and so will ship in this same un-cut tag — folded into the sections below (a
> security dependency bump and shared-cache concurrency/diagnosability hardening).
>
> Three post-launch hardening waves: (1) live user-simulation — 6 real-user personas drove the shipped
> binary against live prod, confirmed findings fixed to SOTA across severities; (2) a second brutal
> adversarial audit (40 agents → 7 findings, 0 high) closed to SOTA, the data-safety fixes below being the
> substantive ones; (3) in-tree fuzzing of every untrusted-input parser. Plus an adversarial security
> sweep (1 LOW, fixed) and a daily live-conformance canary.

### Security
- **The plaintext-guard rejection no longer echoes embedded userinfo.** A misconfigured
  `CLW_ENDPOINT=https://user:secret@host` is still refused (the PAT never leaves over plaintext), but the
  error message now redacts the embedded `user:pass@` instead of printing it to stderr/logs. (LOW — the
  CLW_TOKEN PAT travels in the header, never the URL; this is a secondary self-inflicted-credential hygiene
  fix.)
- **`quinn-proto` bumped 0.11.14 → 0.11.15** for **RUSTSEC-2026-0185** (remote memory exhaustion, HIGH).
  Lockfile-only; the crate is a phantom dependency (`reqwest`'s unused HTTP/3 feature), so there is no
  compiled path into the binary — this keeps the supply-chain gate green and the patched dep in the lock.

### Added
- **`clw snapshot --force`** — replace an existing workspace name whose content differs, via an atomic
  rm-then-write within the one command (vs the manual `rm`-then-snapshot, where a crash between the two
  leaves no ref). Re-snapshotting *identical* content needs neither — see Changed.
- **C2c credential-source: single-use cred-ticket redemption for untrusted runner boots (#165).** When a
  runner ref-domain (`CLW_REF_DOMAIN=runner`) is injected with a `CLW_CRED_TICKET` + `CLW_LEASE_ID` +
  `CLW_FABRIC_ENDPOINT` instead of a static `CLW_TOKEN`, clw redeems the ticket ONCE at trusted boot
  (`POST {CLW_FABRIC_ENDPOINT}/v1/leases/{lease_id}/cas-cred`, ticket in the body) into a scope-narrowed
  CAS PAT — so the PAT never lives in the untrusted container env. Fail-closed (no `CLW_TOKEN` fallback once
  a ticket is present); only network commands redeem (a no-network `doctor`/`whoami` never burns the
  single-use ticket); the ticket is never logged (redacting `Debug`). Unlocks the runner-fabric env-0 boot.

### Changed
- **Re-snapshotting identical content is now a no-op success** (was HTTP 409). The most natural loop —
  "save again, nothing changed" / a runner's idempotent re-snapshot — now exits 0 reporting "already up to
  date" instead of erroring. (Full in-place versioning of *changed* content remains a server-side
  capability, tracked separately; `--force` is the client-side replace path meanwhile.)
- **`ls --all` no longer floods the terminal.** It distinguishes `clw run` memo records (which share the
  tenant keyspace — counted as `run_cache_entries`) from genuinely-corrupt refs, and stopped emitting a
  duplicate per-record warning; corrupt records go to the diagnostic channel (`-v`) + `--json` + a summary.
- **`clw ls` shows human-readable timestamps** (`YYYY-MM-DD HH:MM:SS UTC`) in its text output; `--json`
  keeps `created_at_unix`.
- **`clw rm` is honest about a no-op** — removing a name that never existed reports "nothing to delete"
  (`--json {"deleted":false}`) instead of claiming it deleted something; still idempotent (exit 0).
- **Clearer messages** — not-found errors hint at `CLW_REF_DOMAIN=runner`; bare `clw ls` points at
  `--help`; `clw doctor`'s note clarifies the tenant ID is shown intentionally (only the token is
  redacted); snapshot/hydrate help notes the directory is a positional arg; `clw snapshot` prints a note
  when files were excluded by `.gitignore`/`.clwignore`.

### Fixed
- **`clw prune` no longer aborts — or corrupts — on a tenant that used `clw run`.** A run-cache memo record
  shares the AC keyspace with workspace refs; the live-set walk parsed every key as a `RefRecord`, so the
  first run record tripped the fail-safe and disabled prune. The walk now recognizes a `RunRecord` and
  **pulls its stdout/stderr CAS blobs into the live set** — so prune both runs and never orphans/deletes a
  run record's captured output. (Data-safety: a later cache hit can still fetch its output.)
- **`clw snapshot --force` can now recover a corrupt-but-present ref.** The replace path gated on a
  *parseable* existing ref; it now gates on raw AC presence, so a name whose stored record is unreadable
  can be overwritten with `--force` (previously only `rm --name` could clear it).
- **Paginated listings (`ls`/`prune`) are bounded against a hostile/buggy server.** A hard page-count cap
  bounds the cursor-cycle guard's memory, so a server emitting empty pages with ever-advancing cursors
  can no longer grow client memory without limit.
- **The cache GC no longer counts in-flight temp files toward the budget.** `.tmp-*` staging files are
  excluded from the cache-size accounting and from eviction, so a concurrent write can't skew GC.
- **A shared cache no longer spuriously fails under concurrent processes.** The removal paths
  (`clear_content`, the format-version-mismatch wipe run on every open; and `gc` eviction) now skip
  in-flight `.tmp-*` puts and treat a concurrently-removed blob (`NotFound`) as benign instead of erroring.
  Without this, two `clw` processes sharing a cache that start right after a cache-format bump could fail
  with a bare "No such file or directory". Cache fs errors now also carry the failing operation + path
  (e.g. `cache put: rename …`) so a rare concurrency failure is diagnosable instead of an opaque errno.

### Tests / CI
- Binary-level E2E coverage push: `clw run` exit-code contract (reserved 125, 128+signal), a resource cap
  + non-retryable tiers (402/403/410) surfaced cleanly, config-resolution precedence + identity
  persistence, single-command coverage fill + a full-lifecycle journey, process-level concurrency + a
  1500-file tree — plus assertion-depth hardening (validate output VALUES/state, not just exit codes).
- **In-tree fuzzing of every untrusted-input parser** — 26 stable-`proptest` fuzzers across manifest /
  ref-record / run-record decode, `Digest::from_hex`, the `validate_*` path checks, AC/CAS list-page
  decode, config parsing, and the endpoint-userinfo redaction (a security invariant: credentials never
  survive into a displayed endpoint). Gate-friendly: 1k cases per PR, 50k under `CLW_FUZZ_GATE=1`. No
  crashes found at 50k.
- **Daily live-conformance canary** (`conformance-canary.yml`) runs the prod round-trip in honest mode
  (`CLW_CONFORMANCE_REQUIRE=1`) on a schedule, so a prod-side availability regression surfaces as a
  recurring red signal rather than being noticed only on the next manual run.
- CI gains a **self-hosted runner fallback** (`clw-mac-1`) for the gates+dco jobs, for when GitHub-hosted
  Actions minutes are unavailable; the GitHub-hosted lane remains the preferred path.

## [0.1.3] — complete the symlink `..` fix + lock the power-user findings (2026-06-20)

> Completes the v0.1.2 intra-workspace `..` symlink fix on the hydrate path (it was only half-resolved,
> so a snapshot succeeded but the round-trip `hydrate` aborted) and lands binary-level regression tests
> for all four power-user findings so they cannot silently come back.

### Fixed
- **Hydrating a relative `..` symlink that stays inside the workspace** (e.g. `bin/x -> ../src/x`) no
  longer fails with "symlink target escapes workspace". The v0.1.2 fix updated the producer and the
  manifest parse but missed the hydrate *materialization* path, which still re-validated with the
  strict (any-`..`) check — so a snapshot succeeded but the round-trip `hydrate` aborted. Hydrate now
  uses the same contained validator; targets that genuinely escape the workspace root are still
  rejected. Caught by a new binary-level snapshot→hydrate regression test.

### Tests
- **Binary-level power-user acceptance regressions** for all four v0.1.2 findings (re-snapshot 409 is
  actionable + the rm-then-retry workaround works, intra-workspace `..` symlink round-trips, usage
  errors are not mislabeled `transport error`, `rm --ref-key` deletes a dangling/corrupt ref). These
  run in the default CI lane against the wiremock mock server — the bugs the v0.1.2 fixes addressed
  previously had zero automated coverage. A new create-only AC mock reproduces the live store's
  409-on-overwrite so the real re-snapshot path is exercised.

## [0.1.2] — power-user acceptance findings (2026-06-20)

> Fixes the 4 findings from an end-to-end power-user test of the shipped v0.1.1 binary. The core
> engine (snapshot/hydrate byte-identity, run memoization, exit-125 contract, prune safety-floor) was
> confirmed solid; these address the rough edges around it.

### Added
- **`clw rm --ref-key <KEY>`** — delete a ref by its raw 64-hex key, for removing a dangling/corrupt
  ref that `ls --all` / `prune` surface (by key) but that has no usable name (and that blocks `prune`).
  Mutually exclusive with `--name`; existing `clw rm --name` is unchanged.

### Fixed
- **Relative symlinks with `..` that stay inside the workspace** (e.g. `bin/x -> ../src/x`) are now
  snapshotted instead of rejected with "symlink target escapes workspace". Resolved on both the
  producer and consumer; symlinks that genuinely escape the workspace root are still rejected.
- **Re-snapshot error message.** Re-snapshotting an existing workspace name returned a bare
  `HTTP 409`; it now explains the situation and the `clw rm --name <X> --yes` workaround. (The
  underlying in-place versioning needs server-side upsert support — tracked separately.)
- **Error labels.** Authentication, usage, not-found, prune-refusal and similar application errors are
  no longer mislabeled `transport error: …` (a new verbatim message class); genuine transport/HTTP
  capability errors keep their label.

## [0.1.1] — post-launch audit remediation (2026-06-19)

> Addresses all 20 cold-verified findings from a post-launch adversarial audit of v0.1.0
> (9 PRs, #109–#117). No customer-facing behavior change beyond the `clw run` exit-code
> correction; the rest hardens supply-chain, distribution glue, and docs.

### Fixed
- **`clw run` exit code (frozen-contract violation).** A clw-internal failure where the child
  **never ran** (e.g. config/identity resolution failure) now exits the reserved **`125`** instead
  of `2`. `2` collided with a child legitimately returning 2, so the CoreLink runner could
  mis-attribute a clw substrate failure to the user's build. Now matches
  `docs/CLI-SURFACE-EXIT-FREEZE.md`; regression-tested.
- **`install.sh` trust root.** The installer now verifies the release signature against a
  minisign public key **embedded in the script** (the pin), instead of trust-on-first-use against a
  key fetched from the same surface as the signature.
- **Third-party attribution.** `THIRD-PARTY-LICENSES.md` (the bundle `NOTICE` promises) is now
  generated with `cargo-about` and shipped as a release asset — previously promised but never produced.
- **Memoization key.** `clw run` now folds the (canonicalized) working directory into the cache key,
  preventing a stale-replay across different directories with otherwise-identical inputs.
- **Cache temp files.** Hardened against cross-process collisions (process id + atomic sequence),
  mirroring the hydrate path.
- **Docs.** Corrected the `INSTALL.md` trust-root fingerprint (the key **is** committed), removed a
  non-existent `clw@<version>` Homebrew install claim, and made the brew-verification wording honest.

### Changed
- **Release pipeline.** SLSA provenance is gated to public repos (skipped with a loud warning on this
  private repo — deferred per waiver); macOS notarization now asserts the notarytool JSON status; the
  live-conformance gate is **ship-blocking for real tags** (requires `CLW_CI_TOKEN`); `install.sh` is
  published to the public mirror; the SLSA subject set includes the checksums + SBOM.
- **Supply-chain gates.** Added a `bytes` version floor, `cargo audit --deny unmaintained/unsound`, and
  a committed `fuzz/Cargo.lock` + opt-in fuzz deny/audit lane.

### Determinism note
- `clw run` memoizes on declared `--input` + `--env` names + command + cwd. **Undeclared** environment
  variables that affect a command's output remain the caller's responsibility to declare; clw does not
  sandbox the child environment in this release (env-sandboxing is tracked as a post-beta hardening).

## [0.1.0] — go-live hardening waves (2026-06-14)

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

### Security & correctness — go-live audit (2 adversarial passes, 71 findings closed)
> Two read-only multi-agent audits (168 agents each: many finder lenses → dedup → 3-skeptic
> refute-default verification) hardened the client before tag. Every finding was lead-cold-reviewed
> and fixed gate-green; the headline items by category:

- **PAT confidentiality.** Closed two `@`-userinfo PAT-exfiltration vectors — a loopback bypass
  (`http://127.0.0.1:8080@evil.com`) and the same class on the TLS path (`https://legit@evil.com`),
  both now parsed with the `url` crate (embedded credentials rejected). Redacted the PAT from the
  derived `Debug` of `Config` and `Cli`. Added a Windows owner-only protected DACL on the PAT file
  (`config.toml`), an over-permissive-config warning on unix, and SSRF hardening (`redirect::none`).
- **Destructive-command safety.** `clw uninstall` now refuses to `remove_dir_all` a misconfigured
  `cache_dir` (a `.` no longer wipes the CWD); `clw prune` is **fail-safe** — it aborts (deletes
  nothing) on an incomplete live set rather than sweeping live blobs as false orphans; `clw erase`
  re-enumerates to **verify** the tenant is empty before claiming a GDPR Art.17 erasure (and reports
  distinct counts + progress on a mid-delete failure); declined confirmations exit non-zero uniformly.
- **A `clw run` hang.** Fixed a deadlock in the command-output capture (a concurrent-stream `join!`
  that hung when one stream exceeded the cap while the child wrote the other) — now kills the child
  and returns on the first cap-blow, with a dual-stream-flood regression test under a real timeout.
- **DoS / resource bounds** (hostile server or malicious manifest): disk-fill budget keyed on the real
  summed chunk sizes, ref/blob list-enumeration caps, streamed `run` output capture, wired cache
  eviction, and per-file/total chunk-count caps. Fetched chunks are length-checked against their
  declared size; an over-cap root manifest is rejected before upload.
- **Manifest & cache integrity.** Manifest canonical form (sorted + unique paths) is enforced in
  release (was a stripped `debug_assert`), defeating a duplicate-path hydrate race; the `clw run`
  memoization key now folds in file mode, empty input dirs, and separator-normalized paths; `clw
  status` is mode-aware. Tenant ids are validated against URL-path injection.
- **Real test coverage.** Added binary-level end-to-end tests for the destructive commands
  (prune/erase/ls/rm against a stateful mock incl. DELETE + LIST routes) and replaced several vacuous
  unit tests that asserted on their own reimplementation.
- **Honest tracked items (not silent debt):** the Windows ACL FFI is compile-verified + cold-reviewed
  but its *runtime* check is owner-runnable now via `docs/go-live/WINDOWS-ACL-VERIFICATION.md` (no
  Windows CI needed); the Windows **test lane** itself remains M2.4 (billing-gated).

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
