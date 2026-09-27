# Changelog

## [Unreleased]

### Fixed
- `clw run` can write an atomic `CLW_RUN_STATE_FILE` receipt so wrappers distinguish
  proven pre-spawn failure from a child result without parsing exit codes or output.
  The additive states are `NOT_STARTED`, `DISPATCHING`, and `EXECUTED`; uncertain
  dispatch never authorizes an automatic retry (#567).
- Unix hydration validates the complete manifest path tree and bounded symlink
  resolution before destination writes (#296). All implicit directories are
  created before links; link names are exclusively reserved to detect filesystem
  aliases before any link is published, and publication checks plain parents. Safe
  relative links and dangling normal suffixes remain supported. Unresolvable
  parent traversal, detected graph cycles and excessive resolution work are refused. This does
  not change manifest-v1 serialization or certify hostile local-writer exclusion
  or Windows copy/ACL behavior. Destination name-equivalence protection remains
  the complementary #303 guard rather than being reimplemented here.
- Namespace-probe cleanup removes only successfully created names in reverse
  order, preserving failed aliases and reporting unknown residue (#303).
- Hydrate rejects destination name collisions, including implicit directory
  aliases, before file-content fetches (#303). Literal manifest names remain
  unchanged; private filesystem probes include at most 1,000,000 namespace nodes.
  Complete regular-file publication now refuses occupied names instead of
  overwriting them. Native sensitive/insensitive-volume evidence is kept separate
  from Windows copy-policy and sibling-writer lifetime obligations.
- Cwd-only PATH lists preserve native script launch spelling and their exact
  child PATH in the Unix selected-tool correction (#301). On Linux, a selected
  empty component in a mixed PATH is explicitly refused before memo lookup;
  an explicit `./tool` or `PATH=.` avoids changing script `$0` or the environment.
- On Unix, memoization now identifies an executable with effective launch access
  and retains that selected path through lookup and dispatch (#301, Unix slice).
  Weak identity is refused, native argv0 is preserved, and detected tool drift
  suppresses replay or memo publication. Versioned tool identities invalidate
  older Unix keys without deleting records; non-Unix resolution is unchanged.
- Declared non-UTF-8 environment values are refused before run-cache lookup or
  child dispatch, without printing their contents (#302). The `clw/run/v4/` domain
  deliberately invalidates v3 memo keys, including invalid-value records that
  aliased an absent variable. Old records remain recognizable for GC; supported
  environment values and child-exit behavior are unchanged.
- Snapshot conflict guidance no longer claims a server rejection for local
  preflight refusal (#309). Exit codes, dry refusal and replacement behavior
  are unchanged; real remote conflicts retain actionable guidance.
- Forced unchanged snapshots repair an absent root manifest before reporting
  success, without deleting or rewriting the same-root ref (#294). The default
  non-force no-op remains dry. This repairs missing content, not corrupt-but-present
  blobs or concurrent publication/GC races.
- Local-cache PUT failures and unwinding now clean only the temporary file
  owned by that operation, after closing its handle. Successful publication
  disarms cleanup; removal errors preserve the primary error and emit a warning
  (#314). Crash scavenging and cache-budget scheduling are unchanged.
- `clw run` refuses replay of stored non-success records while retaining their
  stream references for garbage collection. The existing one-command fallback
  preserves create-only cache occupants and the real child result (#307).
- `clw run`: an oversized initial AC lookup response now reports the existing
  pre-spawn error (125), rather than incorrectly claiming the child was killed
  for output overflow (137). The response cap, actual output-cap termination,
  and legitimate child exit-code passthrough are unchanged (#305).
- Hydration rejects per-file size/chunk-sum mismatches and sum overflow before
  creating destination entries or fetching materialization chunks (#306). Named
  and direct-digest restores share this validation; valid manifest bytes are unchanged.
- Unix hydration creates owner-only temporary files, applies the intended file
  mode through the open handle, and syncs before publishing the final name (#297).
  Cleanup is armed only after exclusive creation succeeds, so an occupied temp
  path is never removed by a failed open. Metadata/sync failures abort before
  publication. Windows ACL semantics and whole-tree confinement are separate.

## [0.1.12] — required-hit cache replay + post-v0.1.11 hardening (2026-09-06)

### Fixed
- **Windows: a symlink whose target directory contains the link itself is now rejected (M17).**
  Windows has no unprivileged symlinks, so `hydrate` materialises a `Symlink` entry as a COPY of
  its target. For a self-referential target — canonically `d/up -> ..` — the recursive copy walked
  the tree it was writing into and re-read what it had just written, until it died on MAX_PATH or
  filled the disk. The manifest is untrusted remote input, so this was a resource-exhaustion vector.
  It is now refused with a clear `InvalidManifest` error. Also covers two shapes that a purely
  lexical check misses: a trailing space or dot (Win32 strips them, so `.. ` *is* `..`) and
  non-ASCII case (NTFS folds `ä`/`Ä` to one directory).

  **This is a materializer rule, not a format change** — no manifest that is valid on unix became
  invalid, and `d/up -> ..` still hydrates into a real symlink there.

### Changed
- **`--concurrency` now also governs destructive sweeps and `ls --all`.**
  `prune` and `erase` use the per-invocation value for both their bounded delete
  schedulers and their CoreLink request governor; `ls --all` uses that same value
  for its ref-fetch dispatcher and request governor. `0` is clamped to `1`,
  matching `snapshot` and `hydrate`. The knob remains a CLI flag only, with no
  environment/config setting; it does not apply to one-record `ls --name`.
- **Windows only: the symlink copy-fallback now refuses to descend past 64 directory levels**
  (`MAX_COPY_DEPTH`), as a fail-safe bound behind the M17 guard above — without it, any miss in
  that guard is unbounded rather than merely wrong. A legitimate manifest whose symlinked directory
  target nests deeper than 64 levels now fails to hydrate on Windows where it previously copied. No
  realistic workspace approaches this; the constant can be raised if one ever does.

### CI / Infrastructure
- CI build targets are now explicitly job-local and keyed to the GitHub run attempt:
  Cargo commands within one gate reuse compilation, but CI makes no cross-run cache
  claim (and has no `actions/cache` build cache).
- **The windows-gnu cross-check now runs with `--all-targets`.** Without it, `cargo check` compiles
  only lib/bin targets, so every `#[cfg(windows)] #[test]` in the tree was never type-checked by any
  lane. Also added a `windows.yml` step that runs the M17 containment predicate on the real
  windows-gnu binary under wine.
- **Added `docs/RUNBOOK-ci-fallback.md`** — the written procedure for returning CI to
  GitHub-hosted runners during a self-hosted outage. Carries the full `runs-on:` inventory
  (19 `corelink` fabric jobs + the single `[self-hosted, macOS, X64]` release matrix), the exact
  label-swap diff that unblocks merging, the measured hosted-minute cost (≈18 billed minutes per
  PR push; ≈77 per release, 40 of them the ×10 macOS legs), and the gotchas — reverting PR #233
  wholesale would also revert the clw-cli test-hermeticity fixes and delete `gate-check-selftest`;
  the `CARGO_TARGET_DIR` persistent cache in `gates.sh` is vestigial (no `actions/cache` exists,
  every run already compiles cold); and hosted Actions being *billable* is an unverified
  precondition. States plainly that a `clw-mac-1` outage **stops releases** — the release matrix
  has no label-swap fallback (`ci-use-host-toolchain.sh` hard-fails on hosted arm64 macOS and the
  zig step is fail-closed), only a revert of the PR #234 shape that last shipped green on
  2026-07-17. The runbook's own acceptance criterion 1 (a fresh hosted test PR) is recorded as
  **UNMET** inside the document rather than claimed.
- **The "no `unsafe`" posture is now a lint plus a gate, not a sentence in a doc.**
  `SECURITY.md` and `docs/THREAT-MODEL.md` claimed the shipped crates contain zero
  `unsafe`. That was false — one audited Windows-DACL block existed — and nothing
  enforced any version of the claim. Added `[workspace.lints.rust] unsafe_code =
  "forbid"` with every member opting in. The lint reaches every kind of crate root,
  verified by injecting `unsafe {}` into one of each and watching the compiler reject
  it: `lib.rs`, `src/bin/*.rs`, `tests/*.rs` and `benches/*.rs`.
  **The DACL block moved into its own crate, `clw-win-acl`.** `forbid` cannot be
  locally re-allowed, so whichever crate holds that block must sit at `deny` instead —
  and a `deny` crate is precisely where `unsafe` can be reintroduced with the compiler
  silent. It is now pinned BYTE FOR BYTE against `.github/win-acl-sources.expected`, so
  any change to it is a fixture diff a person reads — the regex that used to police it
  is GONE, not tightened. Leaving the block in `clw-cli` put 8,554 lines under that
  regex, and cold review found a spelling it did not model,
  `#[cfg_attr(all(), allow(unsafe_code))]`, which ran `unsafe` in the shipped binary
  with `cargo fmt`, full-workspace `cargo clippy -D warnings` and the guard all green.
  The new crate is ~180 lines and holds nothing else; `clw-cli`, the crate that
  handles the plaintext PAT, is back under `forbid` where the compiler enforces it.
  Shrinking what the regex must be right about was the only fix here that removes a
  regex from the trust path rather than patching it again.
  **Three more things the compiler does not see**, each now asserted:
  `unsafe_code` is suppressed inside EXTERNAL macro expansions, so a
  `#[macro_export] macro_rules!` body in one member expanding into another compiles
  clean under `forbid` at both sites — the workspace defines no macros, so the guard
  asserts zero and an author adding one must argue for it there; a crate detached by
  `workspace.exclude` or its own `[workspace]` table stays a path dependency linked
  into the binary while leaving `cargo metadata`, so the set of in-repo manifests must
  equal the member set; and the source walk no longer follows a hardcoded `crates/`
  path or silently skips symlinked directories.
  **How the guard got here, in order, because the pattern is the point.** Five
  generations were written and each was broken by review. An attribute counter fell to
  `#[allow(unsafe_code, dead_code)]`, to extra whitespace, and to
  `#[expect(unsafe_code)]`, which is not the same attribute. A token scan replaced it
  and fell to an `unsafe` split across a line break (grep is line-oriented), only
  looked under `crates/`, and raised false positives on doc comments. A lint-level
  assertion fell to the required string sitting in a *comment*; a member opt-in check
  written as `grep -q 'workspace = true'` could not fail at all, because
  `version.workspace = true` appears in every manifest in the repo. Each of those is
  the same mistake: matching TEXT where a PARSER or the COMPILER was the right tool.
  The guard now parses manifests with `tomllib`, takes the member list from
  `cargo metadata` (a failed enumeration is a hard failure, not zero iterations and a
  clean report), and leaves finding `unsafe` to the compiler, which does it exactly.
  **In-repo cargo configs are now held to an ALLOWLIST, which is empty.** A cargo
  config decides both what is handed to the compiler and what RUNS as it.
  `[build] rustflags = ["--cap-lints", "allow"]` demotes every lint in every workspace
  crate — `forbid` included — to `allow`; `rustc-wrapper`, `rustc-workspace-wrapper`
  and `rustc` reach the same end through a shim script, and can scope it to a single
  crate. Both verified end to end: `unsafe` running in the shipped binary with the
  guard, `cargo fmt --check`, host `clippy -D warnings` and the windows-gnu cross lane
  all clean. The first version of this check named three dangerous keys and review
  simply used a fourth, so it asserts instead that NO key is set that is not written
  down — every `.cargo/` directory in the tree, not just the root, since cargo
  discovers config by walking cwd upward. The guard's directory traversals were
  consolidated into ONE helper (`walk_or_fail`) that FAILS on a symlinked directory
  rather than skipping it — they used to disagree, and the newest and most
  security-critical of them failed OPEN on the construct the oldest already failed
  closed on: `ln -s ci/cargo-cfg .cargo` is
  one committable line that hid a capping config from the walk while cargo read it
  normally, and "we did not look there" had been reported as "there is nothing there". `RUSTFLAGS` and `RUSTC_WRAPPER` in the
  AMBIENT ENVIRONMENT do the same and are NOT closable from inside a repository; both
  are now named as uncovered.
  **`scripts/selftest-unsafe-posture.sh` is new and replays the corpus** against a copy
  of the tree: one clean baseline, every recorded bypass as a case that must be
  rejected (most pinning WHICH check must fire) and one asserting a symlinked `target/`
  stays permitted, so fail-closed does not become fail-on-everything. The suite reports
  its own totals; no count is carried here, because it went stale three times. The corpus previously
  existed only in commit prose, which is why five of the seven rounds reopened an
  earlier round's fix. One case compiles the real tree as the sentinel for anything
  that defangs the lint wholesale; its reach is exactly the crates `-p` compiles, so it
  builds `clw-cli`, the crate that ships. The full lane runs it — **~117s on a cold dev
  box, 14.0s in CI** (the latter on the 39-case revision); the gap is the
  `CARGO_TARGET_DIR` the job's earlier steps have already populated.
  A green suite means the recorded bypasses are still closed and nothing more: it
  reported 30/30 on a tree whose binary ran `unsafe` behind a crate-scoped compiler
  wrapper, and 39/39 on another behind a symlinked `.cargo`.
  Separately, the scoped-lane escalation regex was anchored to the ROOT `Cargo.toml`
  only, so a PR editing just `crates/*/Cargo.toml` could delete the enforcement and
  take the fast lane.
  A claim held up only by review attention is not a control, which is this change's
  own thesis — and `SECURITY.md` now also states plainly what remains uncovered:
  **platform-conditional code, which is the widest hole — three of the five targets
  `release.yml` ships (both macOS targets and `aarch64-unknown-linux-gnu`) are linted
  by no lane at all**; lint capping from the environment; `unsafe` added inside the
  audited block itself; third-party and `[patch]` code; proc-macro expansions; and
  `fuzz/`. (`build.rs` left that list — the guard now asserts none exists, rather than
  observing it in prose.)
  Also dropped the `Win32_System_Memory` windows-sys feature, dead since the SID
  buffer moved from `LocalAlloc` to `Vec<u64>` — confirmed by a clean
  `--target x86_64-pc-windows-gnu` build without it.
- **Added `scripts/pre-merge-gate-check.sh`, ported from corelink-server.** Guards against a
  failure mode that bit both repos within two hours of each other: a CONFLICTING PR gets NO
  `pull_request` check runs at all (GitHub cannot build the merge ref) — not queued, not
  failed, absent — so `gh pr checks` reports "nothing failing" and reads as green. Refuses to
  say OK to merge unless (1) `mergeable == MERGEABLE` (UNKNOWN is also refused — it means "ask
  again", not "fine"), (2) the always-present gates (`gates` + `dco` from `ci.yml`, and
  `llvm-cov (report-only)` from `coverage.yml` — the only workflows here with a bare
  `on: pull_request` and no `paths:` filter) actually appear in the check list, and (3) a
  report of zero checks fails rather than passing as "nothing to gate". Proven in both
  directions by `scripts/pre-merge-gate-check-selftest.sh` (stubbed `gh`, no network), wired
  into `ci.yml` as its own job so the safety script keeps proving itself on every PR.
- **The release pipeline is now zero-hosted too.** `release.yml` moved off GitHub-hosted runners entirely,
  mirroring the proven corelink-server CLI release: all five targets build on ONE self-hosted Mac
  (`corelink-builder`, $0 CF) via `cargo-zigbuild` — darwin native (system linker), and the Linux
  x86_64/aarch64 + windows-gnu targets cross-compiled with zig's libc+linker (a real macOS host is required
  for codesign/notarize anyway, so every leg folds onto it at no extra cost). Rust comes from the pinned
  host toolchain via the new `scripts/ci-use-host-toolchain.sh` (never `rustup install`/`default` on the
  shared-`$HOME` Mac fleet). The non-build jobs (preflight, conformance, sbom, publish) run on `corelink`.
  Ported the server's hard-won build fixes (zigcc linker-script perms / exit-126 guard; the `windows-rs`
  cache-integrity re-fetch) and added `SOURCE_DATE_EPOCH` + a `RUSTFLAGS` path-remap so the shipped binaries
  are reproducible and carry no dev-host `$HOME`/username in their strings. The Linux minisign signing leg
  now installs via `brew` (it runs on the Mac now, not apt-based ubuntu). Conformance nulls the ambient
  env-0 cred trio (same cross-tenant fix as the CI lanes).
- **Every GitHub-hosted Linux CI lane now runs on `corelink`** — CoreLink's own ephemeral
  CF-Firecracker runner fabric — so this repo spends **zero** GitHub-hosted Actions minutes, per the
  org mandate *"self-hosted tudo ou não roda"*. Migrated `ci` (gates + dco), `conformance-canary`,
  `coverage`, `live-journeys`, `mutation`, `fuzz`, `stress`, `shellcheck`, `canary-deadman` and
  `metered-probe-canary` from `ubuntu-latest`. Image-capability adaptations: `live-journeys` uses
  `rustup show` (the toolchain is baked) instead of `rustup toolchain install/default`; `shellcheck`
  and the `ci`-gates mingw cross-CC install via apt (passwordless sudo is present on the box);
  `metered-probe` installs `blake3` into a venv (Ubuntu 24.04 is PEP-668 externally-managed).
  `release.yml`, `windows.yml` and `e2e-evidence.yml` intentionally keep hosted targets pending the
  cross-platform release rework.

### Fixed
- **New releases no longer advertise SLSA provenance that is not published.** Every
  GitHub Release told users it carried "SLSA provenance"; it never did — the
  `attest-build-provenance` step is gated to PUBLIC repositories and this source repo is
  private, so it is skipped on every release (owner waiver, 2026-06-18). The notes then
  pointed at `docs/INSTALL.md`, which handed the reader
  `gh attestation verify … --repo HuGR-Labs/clw-releases` — a command that **cannot**
  succeed. A user verifying a download on a security-sensitive path hit a failing
  supply-chain check and could reasonably conclude the binary was compromised; the
  practical effect was to train people to ignore verification failures. The claim is
  removed from the notes template, `docs/INSTALL.md` and `docs/DISTRIBUTION.md`, and
  `docs/INSTALL.md` now states plainly that the check is unavailable and that its failure
  is expected, not evidence of tampering.
  The published body is no longer a string inside the workflow. It is a committed file,
  `.github/release-notes.md`, which `release.yml` renders by substituting `__VERSION__`
  and passes with `--notes-file`. That indirection is the fix: the text every user reads
  is now an ordinary file diff, which is what review is good at. The original defect
  survived for months because the sentence lived in a shell assignment two-thirds of the
  way down a thousand-line workflow, where a reviewer of a release-pipeline change has
  no reason to look.
  `scripts/check-release-notes.sh` (wired into `scripts/gates.sh`, so it runs in every
  lane) keeps that indirection in place. Within `release.yml` it covers three things.
  Every literal `gh release create|edit` must pass `--notes-file "$NOTES_FILE"` and none
  of `gh`'s other notes-bearing flags — `--notes`, `-n`, `-F`, `--notes-from-tag`,
  `--generate-notes`, `--notes-start-tag` — matched so that the attached forms `-nVALUE`
  and `-F-` are caught too, since `pflag` takes the LAST occurrence and a trailing `-F-`
  would source the body from stdin with the required flag still in plain sight. Neither
  the variable `$NOTES_FILE` may be referenced anywhere but its one assignment and one
  render, and the rendered file's basename is reserved to that assignment line — which
  catches the literal path spelling, and only that. A path the workflow ASSEMBLES
  (`> "$RUNNER_TEMP/release-notes.rendered".md`, `>> "$RUNNER_TEMP/$D.md"`, a `sed -i`
  naming it through a variable) is NOT caught, was demonstrated in review, and is named
  in the script header alongside the other escapes rather than implied away. A fourth
  is named there too, and is the one that most limits the headline claim: a step can
  rewrite `.github/release-notes.md` ITSELF before the render, so "the body is a
  reviewed file" means a human reviews that file — not that what ships is what they
  read. The mutation would happen on the runner, between checkout and render, and
  leave no trace in the diff. The render must match one permitted spelling
  exactly — a positive match, because the forbidden-option list it replaced named `-e`,
  `-f` and `-i` and could not see GNU sed's `--expression`, `--file` and `--in-place`.
  And `.github/release-publish.expected` pins the complete argv of both invocations, so
  `--title`, `--repo`, `--draft` and a `--latest`/`--prerelease` inversion are covered.
  It reads three files, runs no subprocess but `python3`, and takes ~0.1s.
  **What it does not cover, named rather than implied.** Two constructs inside
  `release.yml` itself escape it and are deliberately not chased: a `gh()` shell function
  shadowing the real binary, and a `run:` step whose shell is `python` or `pwsh` calling
  the API directly. Outside the file it proves nothing at all — a `curl` to the REST API,
  a step in another workflow, or a person with a browser all get there with this clean.
  Six rounds were spent trying to close *that* class by scanning text, and each version
  was defeated by an idiom its author had not imagined; the version before this one tried
  to close it by *executing* the pipeline's steps, which meant `sudo apt-get`,
  `cargo install`, and a call to the production CoreLink API on a developer's laptop in
  every lane, for 4m22s of a 7m38s gate. Enumerating how a shell reaches an HTTP endpoint
  is not winnable, and adding regexes for these two would restart that loop. Review of
  `release.yml` covers them; this guard keeps the words being published in a reviewable
  file and closes the flag set that *is* closed.
  **Still outstanding, and not fixed by this change:** all 20 releases already published
  on the mirror keep the false claim in their notes. The workflow fix reaches a published
  tag only if that tag's run is repeated, and nobody re-runs an old release. Correcting
  them means editing public content, which is an owner action and needs tooling reviewed
  on its own terms — a first attempt was written and withdrawn from this change after
  review found its target confirmation compared a variable against itself and a failed
  release listing reported success having done nothing.
- **Windows and Authenticode: the docs no longer demand a signature that does not
  exist.** §4d told the reader a `Valid` `Get-AuthenticodeSignature` status "is
  required", but Azure Trusted Signing is deferred and every shipped `.exe` is unsigned,
  so the documented check always reported `NotSigned` — the same defect as the SLSA
  claim, on the same security-sensitive path. §4d now says the check is not live, that
  `NotSigned` is expected rather than evidence of tampering, and points at the signed
  checksum manifest as the verification that works today, and the asset table now matches
  the hedge §4c already carried. `docs/go-live/RUNBOOK-key-rotation-revocation.md` had
  the same defect — it told a consumer verifying a key rotation to validate the
  Authenticode chain and the macOS notarization ticket, neither of which exists — and
  `docs/INSTALL.md` routes readers there when verification fails, so a corrected §4
  was handing them straight into two checks that cannot pass.
- **A re-run of a release tag now refreshes its notes.** The idempotent `--clobber` path
  only re-uploaded assets, so a release published with incorrect notes kept them
  permanently. Both the create and the re-run paths now render the same committed
  `.github/release-notes.md`, and a failed refresh is fatal rather than a warning — a re-run
  that cannot correct the notes has not achieved the idempotency it claims. Note the
  consequence: the workflow is now authoritative over the release body, so hand-edits
  made in the GitHub UI are overwritten by the next re-run.
- **A `CLW_*` variable that is SET BUT EMPTY is now treated as ABSENT, not as a value (M12).**
  clap hands an empty env var to the program as `Some("")`, which short-circuited the whole
  resolution chain — beating BOTH `~/.clw/config.toml` and the built-in default. A missing secret
  in CI materialises as exactly that, so this was routine, not exotic: `CLW_TOKEN=""` sent a real
  request with a bare `Bearer ` header and failed with "authentication failed" instead of the
  actionable "missing token" (and with no request at all); `CLW_ENDPOINT=""` failed with "refusing
  plaintext (got '')" instead of falling back to the default endpoint; `CLW_CACHE_DIR=""` silently
  made the process CWD the blob-cache root. All five clap-merged globals (`endpoint`, `tenant`,
  `token`, `cache_dir`, `ref_domain`) are now filtered empty-to-absent in ONE place, shared by every
  resolve entry point — matching the `CORELINK_*` fallback path, which has always done this.
  Credential semantics are unchanged: the C2c broker trio is read through that same `CORELINK_*`
  reader (an empty `CLW_CRED_TICKET` already meant absent) and the fail-closed rule — a NON-EMPTY
  ticket with a missing `CLW_LEASE_ID`/`CLW_FABRIC_ENDPOINT` is a hard error, never a fall-through
  to the static token — is now pinned by a test at the `resolve` level.
- **The local cache's eviction is now an approximate LRU instead of FIFO (audit M22).** `gc` sorts
  candidates oldest-`mtime`-first and the docs promised "LRU(mtime)", but nothing ever refreshed
  an `mtime`: `get` read the blob without touching it and a `put` of an already-present blob
  returned early as a silent no-op, so a blob's mtime was permanently its FIRST-WRITE time. That
  inverted the intent — the hottest blobs (fetched earliest in a hydrate, then read on every run)
  were the first evicted whenever the budget tightened, producing perpetual thrash of exactly the
  working set the cache exists to hold. A `get` HIT and a repeat `put` now stamp the blob's mtime
  to now via `std::fs::File::set_modified` (no new dependency). The refresh is strictly
  best-effort: every failure is swallowed, so a read-only mount, a foreign-owned blob in a shared
  cache, or a timestamp-less filesystem costs recency credit but can never turn a cache hit into
  an error. The stamp is **amortised** — a blob refreshed within the last 60s is skipped — because
  an unconditional `open(O_WRONLY)`+`futimens` measured at **+125% to +233%** over the bare
  read+BLAKE3-rehash a `get` already pays, whereas the amortised form (a `stat`, and the write-mode
  open only when genuinely stale) measured at **+7% to +64%**. The 60s resolution is why the policy
  is documented as *approximate* LRU: two blobs used within the same minute may evict in either
  order — a rounding error next to the FIFO it replaces.
- **`clw ls --all` no longer mislabels native ActionCache values as "corrupt ref records."**
  A tenant's AC keyspace is shared three ways: workspace refs, `clw run` memo records, and raw
  native/Bazel ActionCache values (arbitrary non-JSON bytes). `list_refs` enumerates the whole
  keyspace, so `ls --all` fetched every entry and — because a raw AC value is not a `RefRecord` —
  warned "skipping unparseable ref record" for each one and counted it under `--json` `errors`. On a
  dogfood tenant that had accumulated native AC blobs (e.g. `ac-roundtrip-value-*`) this flooded the
  listing with false-corruption warnings. A ref/run record is always JSON, so a value that does not
  parse as JSON at all is now classified as a benign foreign AC entry: counted separately (new
  `foreign_ac_entries` in `--json`, `; N non-ref AC entries` in the human summary), silent at debug,
  never warned. Only bytes that ARE valid JSON but do not match the `RefRecord`/`RunRecord` shape are
  still treated as a genuinely corrupt ref (warn + `skipped` + named by ref key). No data-loss surface
  (`prune` sweeps CAS blobs, not AC values; foreign AC entries were never touched).
- **`live-journeys.yml` wired to the actually-bound probe secrets.** The metered-quota 402 journey
  read `secrets.CLW_CI_PROBE_TENANT`/`CLW_CI_PROBE_TOKEN`, but the repo secrets bound for this purpose
  are `CLW_PROBE_TENANT`/`CLW_PROBE_TOKEN` (no `_CI_`) — the same ones `metered-probe-canary.yml`
  already uses. Because the journey soft-skips on a missing cred, the wrong names made it silently
  no-op every run instead of exercising `metered_writes_hit_the_402_quota_wall` (the clw-conformance
  library path to the 402 wall, distinct code from the canary's raw curl/python probe). Corrected to
  the real secret names so the journey actually runs.
- **Documented the `137` exit code in the frozen CLI exit-code table (`docs/CLI-SURFACE-EXIT-FREEZE.md`).**
  `CLW_RUN_OUTPUT_TOO_LARGE = 137` (`crates/clw-cli/src/subcmds/run.rs`) was a real, tested exit code —
  returned when `clw run`'s child DID run but was killed for exceeding the captured-output CAS blob
  cap — but was missing from the frozen table (only `125`/child-never-ran and the child-passthrough
  range were documented). Not exploitable (a `TooLarge` run errors out before `clw-run`'s memoization
  step is ever reached, so it is never cached), but it was undocumented conformance drift against the
  runner-facing contract. Added the `137` row + updated the runner-rule pseudocode and stability note.
- **`clw-cli` config-resolution unit tests are now hermetic against the ambient credential
  environment.** Seven `resolve`/`resolve_lenient` tests read the process env but did not clear the
  full `CLW_*`/`CORELINK_*` namespace or the C2c broker trio (`CLW_CRED_TICKET`/`CLW_LEASE_ID`/
  `CLW_FABRIC_ENDPOINT`); they passed only because a GitHub-hosted runner's env happened to be clean.
  On a `runs-on: corelink` box the fabric injects the broker trio into every job's env-0, which drove
  `resolve` down the credential-broker path (token resolved to `""`) and made the "missing
  tenant/token" tests see ambient values — four failed. A new `VarGuard::isolate()` snapshots and
  clears the complete `CRED_ENV_VARS` set (restored on drop), so every resolve test starts from a
  known-empty credential environment regardless of what the host/CI injected.

### Security
- **Bumped `h2` 0.4.14 → 0.4.17** to clear `RUSTSEC-2026-0258` (h2 unbounded empty DATA frames), a newly
  published advisory that failed the repo-wide `cargo-deny` advisories gate on every PR. Transitive via
  `reqwest`→`hyper`; lockfile-only change, no API surface touched. `cargo deny check advisories` → ok.
- **The release + install path no longer performs privileged writes through a GitHub org-transfer
  redirect.** All six CoreLink repos moved from the `HumanGuardrail` org to **`HuGR-Labs`** on
  2026-08-01. A GitHub org transfer leaves a `301` behind, so `git clone`, `gh` and `curl` kept working
  and the drift was invisible — but an **exact-string** comparison does not follow a redirect, and
  neither does a trust boundary. `release.yml` was reaching `HumanGuardrail/clw-releases` and
  `HumanGuardrail/homebrew-clw` **through that redirect while holding write-scoped tokens**
  (`CLW_RELEASES_TOKEN` for the mirror Release plus the `install.sh` contents-API `PUT`;
  `HOMEBREW_TAP_TOKEN` for the `git clone`/push of the tap). A redirect is only as durable as the org
  name behind it: were `HumanGuardrail` ever released and re-registered, those privileged writes — and
  the public `install.sh` download surface — would land in an attacker-controlled path.
  The whole publish path is now pinned directly at `HuGR-Labs`: ~20 references in
  `.github/workflows/release.yml` (the cross-repo `gh release create/upload/view --repo`, the
  `install.sh` contents-API push, the **`git clone` of the Homebrew tap**, the generated formula's
  `homepage`, and the `BASE_URL` baked into every published binary URL), `scripts/install.sh`'s
  `DEFAULT_BASE_URL` (the anonymous `curl | sh` download surface — a real customer path),
  `scripts/provision-release-secrets.sh` (its `[ "$REPO" = … ]` equality check plus the
  fine-grained-PAT resource-owner/scope instructions — exactly the exact-match class a redirect does
  *not* rescue), `Cargo.toml`'s `repository`, and every consumer-facing install doc
  (`brew install humanguardrail/clw/clw` → `brew install hugr-labs/clw/clw` in README,
  `docs/INSTALL.md`, `docs/DISTRIBUTION.md`, `docs/GUIDE.md`). Dated handoffs/relays/replies, the
  go-live rework plan, the roadmap decision ledger and the archived WP briefs are deliberately
  **left as-is** — they are historical records of what was true at the time, not live config.
  **Repinning narrows this exposure; it does not eliminate it.** Every reference *inside this repo* now
  points at the live org, but the chain still terminates in a URL a human types — a **name**, not a
  key. The genuinely open residual is **bootstrap/TOFU**: anyone who already holds the old one-liner in
  a bookmark, a blog post, a `Dockerfile` or a CI config keeps traversing the redirect, and those copies
  are outside this repo's reach. That is why the `HumanGuardrail` org **must stay registered forever**,
  repin or no repin. Closing this for real needs an out-of-band anchor — a vanity domain we own, a
  self-verifying channel (the Homebrew tap plus a formula `sha256`), or publishing the signing-key
  fingerprint somewhere a user can independently check it. None of those exist yet.
  **What a customer downloads is unchanged.** Both org paths resolve to the same repository
  (id `1272675315`) and the same release-asset GUIDs; `SHA256SUMS` and `SHA256SUMS.minisig` for
  v0.1.11 fetch byte-identical from either org — the repin removes a `301` hop and nothing else. The
  minisign trust root stays **embedded** in `install.sh` (key `4B57B8B54A0E396D`) and is still never
  fetched from the download surface, so signature verification is unchanged and unweakened. The
  published `Formula/clw.rb` in `HuGR-Labs/homebrew-clw` still carries old-org URLs; it is generated by
  `release.yml`, so it repins itself on the next real release and must **not** be hand-edited.
- **A `dead-org-slug` guard in `scripts/gates.sh` keeps it pinned.** Runs in *every* lane (including the
  docs-only fast path, which is precisely where an install-doc regression would otherwise slip through)
  and fails the gate if either dead org slug (`humangr-labs`, `HumanGuardrail`) reappears in live
  config — workflows, scripts, `Cargo.toml`, or the consumer-facing install docs. Historical documents
  are exempt by construction (the guard scans an explicit live-config allowlist, not the whole tree).

### Added
- **`CLW_CACHE_BUDGET_BYTES` — the local cache eviction budget is now configurable.** The `~/.clw/cache`
  GC budget was a hard-coded 8 GiB with no override, so a working set larger than 8 GiB silently evicted
  warm blobs (a cache miss, never data loss, but an invisible performance cliff on large monorepos / many
  parallel snapshots). It now reads a raw byte count from `CLW_CACHE_BUDGET_BYTES` (unset/`0`/invalid →
  the 8 GiB default); raise it to keep a big working set warm, or lower it on a disk-tight machine. The
  resolver is pure + unit-tested; `LocalCache::with_budget` stays the deterministic, env-independent path
  for tests. Surfaced by the 2026-07-17 doc-vs-code audit.

### Tests / CI — test-rigor & E2E drive (make robustness PROVABLE)
A robustness wave that makes test-adequacy *provable* rather than assumed. All lanes are **additive** —
they do not touch the required `gates`/`dco` checks (except the round-trip property test, which runs in the
gate). See `docs/TESTING.md` for the index + the measure→ratchet→gate posture.
- **Coverage lane** (`coverage.yml`, `cargo-llvm-cov`, per-PR) — measured baseline ≈ **88.8%** line coverage;
  a floor is active and blocks a regression (deterministic, not flaky).
- **Fuzz lane** (`fuzz.yml`, nightly) — runs the previously-dormant `cargo-fuzz` harness (the `clw-types`
  untrusted-input targets: symlink-target / rel-path / manifest-parse / digest-from-hex); a crash is red.
- **Mutation lane** (`mutation.yml`) — `cargo-mutants`. This entry described the weekly
  whole-workspace form and a follow-on that would "flip to block new survivors per-PR";
  that follow-on has since landed, so the per-PR `--in-diff` lane IS the lane now and
  the weekly sweep is gone. See the later entry for why (the six 300-minute shards were
  being evicted by the runner fabric and were much of what starved it).
- **Property round-trip** (`crates/clw-integration-tests/tests/roundtrip_prop.rs`, in the gate) — proptest
  arbitrary trees (unicode/space names, nested dirs, empty+multi-byte files, exec bits, in-tree symlinks) →
  `snapshot→hydrate` asserted byte-identical with structure/mode/symlink-target preserved.
- **Stress lane** (`stress.yml`, weekly) — aggressive magnitude: GB-scale file, 100k files, deep+wide tree,
  64-way concurrency; crash/hang/OOM/mismatch is red.
- **Windows-runtime lane** (`windows.yml`, `windows-latest`/gnu) — the workspace test suite now actually
  **runs** on Windows (was compile-only); it surfaced (and documents) the hydrate symlink→copy fallback as a
  real, expected platform divergence.
- **Shellcheck lane** (`shellcheck.yml`) — lints the distribution + CI shell scripts, including the public
  `install.sh`.

## [0.1.11] — stabilization wave: gate-hole closure + last unbounded reads (2026-07-13)

A quality-bar hardening wave from a 5-angle hidden-debt sweep. The core code was already
clean (panics, error-swallow, doc-vs-code drift, supply-chain: all clean); the recurring
debt lived in the **gate layer** — a hole there lets any future regression go green silently.
This closes them so green means green.

### Fixed
- **Release signing is now fail-closed on real releases.** A real `vX.Y.Z` with the minisign
  secret absent/rotated previously fell to `exit 0` with a buried warning — publishing
  UNSIGNED `SHA256SUMS` + Linux binaries AND still bumping the Homebrew formula. Both minisign
  steps now mirror the conformance guard: on a real (non-`-rc`) tag with the secret absent →
  hard error + `exit 1`. rc/prerelease keeps the dry-run skip. (macOS/Windows signing stay
  waived-open by design — those secrets are intentionally absent per the owner waiver.)
- **The last two size-unbounded reads are capped.** `redeem_cred_ticket` (clw-client) read the
  cred-ticket response with a raw `.text()` (no ceiling) — a buggy/hostile `CLW_FABRIC_ENDPOINT`
  could OOM the client at runner boot; it now routes through `read_body_capped` (64 KiB, a PAT
  envelope is <1 KiB). The snapshot small-file fast path chose the in-memory branch on the
  walk-time size but read the current bytes; a bounded read now retains at most 16 MiB and
  probes only one extra byte before returning an actionable retry message when the file grew
  past the threshold (both upload and local-manifest paths). During one live snapshot call,
  file concurrency N bounds simultaneous small-file buffers to N × 16 MiB. An already-running
  blocking read is not cancelled when its async caller is dropped, so it can overlap reads
  from a later invocation; concurrent local-manifest calls can also overlap their reads.

### Changed
- **The PR fast-lane (`scripts/gates.sh` scoped lane) no longer has coverage holes.** Scoped
  tests now include the sink crates (`clw-cli`, `clw-integration-tests`) so a cross-crate
  regression in a low-level crate is caught at PR time, not only on main's post-merge full lane.
  The windows-gnu cross-check is now a **universal tripwire** (both lanes), so a `#[cfg(windows)]`
  break can't slip the required gate. clippy runs `--all-features` in both lanes so feature-gated
  (`live`) code is held to the same `-D warnings` bar.
- **`cargo-audit` is pinned** (`@0.22.2`, was floating) in CI + release + the gates fallback, so
  a behavior-changed release can't silently alter the advisory gate; the advisory DB is still
  fetched fresh, so advisory freshness is unaffected.

## [0.1.10] — adversarial re-audit fixes (op-deadline + run-store hang + run-402) (2026-07-11)

### Fixed
- **The per-op deadline now actually bounds a queued op under load.** `CLW_OP_DEADLINE_SECS` previously did
  not cover `governor.acquire()` (the permit wait was un-timed) and re-used a stale pre-acquire `remaining`,
  so under 429/AIMD permit contention a single op could run far past its deadline (~40 min for a 300 s
  deadline). The acquire is now bounded by the deadline and `remaining` is recomputed after it, so total
  per-op wall-clock (acquire + request) is bounded by the deadline. (Found by an adversarial re-audit.)
- **`clw run` no longer hangs ~15 min AFTER a build already succeeded.** On success clw stores the
  stdout/stderr/record to the backend best-effort; these were three SERIAL stores (each with the full op
  deadline), so a backend that went unresponsive post-success stalled the (already-passed) run for up to
  ~15 min. The stores now run concurrently under a short 30 s phase deadline — a slow/unresponsive backend
  simply skips the cache write and the correct exit code returns promptly.
- **`clw run` surfaces the actionable 402 quota message.** The run path printed the raw `HTTP 402 from …`
  Display, bypassing the plan/billing/upgrade message — on the exact surface (a runner driving `clw run`)
  the 402 message was built for. It now routes through the actionable message (exit code unchanged).
- **A huge `CLW_OP_DEADLINE_SECS` no longer panics.** The value is clamped (≤ 24 h) so building the deadline
  can't overflow `Instant + Duration`.

## [0.1.9] — 402 upgrade link + macOS Gatekeeper doc (2026-07-11)

### Changed
- **The 402 quota-exceeded message now links the upgrade page.** Following the actionable 402 message added
  in 0.1.8, it now points the user straight at `https://corelink-admin.humangr.com/en/pricing` to self-serve
  the plan change, instead of only telling them to contact their account owner.

### Docs
- **Documented the macOS Gatekeeper recovery for a direct browser download.** The binary is not yet
  Apple-notarized (planned post-launch); `brew` and `install.sh` strip quarantine, but a raw asset downloaded
  from the Releases page is Gatekeeper-blocked. `docs/INSTALL.md` and the auto-generated release notes now
  show the `xattr -d com.apple.quarantine` / right-click-Open recovery.

## [0.1.8] — prod-breakage hardening: actionable 402 + bounded op deadline (2026-07-11)

### Fixed
- **A quota-exceeded (HTTP 402) failure now shows an actionable message instead of a cryptic HTTP error.**
  When a tenant crosses its monthly usage quota / plan $-ceiling, the backend returns 402; clw now says
  "monthly usage quota reached … this is a plan/billing limit on your CoreLink account, not a clw error;
  contact your CoreLink account owner to raise the monthly quota or upgrade the plan" instead of a bare
  `HTTP 402 from PUT …` that reads like a server fault. (Prod-breakage audit — the wall a real paying
  customer hits.)
- **Every backend operation is now bounded by an overall deadline, so an unresponsive backend fails in
  bounded time instead of hanging ~12 minutes.** A new `CLW_OP_DEADLINE_SECS` knob (default **300s**) caps
  the total wall-clock across the retry budget: each attempt is wrapped in a timeout, backoff is capped to
  the remaining budget, and on exhaustion clw returns a clear "operation exceeded the Ns deadline — the
  CoreLink backend may be unavailable" error. A black-hole backend (accepts the connection but never
  responds) previously ran 6 × 120s + backoff with no overall bound. The 429-AIMD / Retry-After / 5xx retry
  behavior is unchanged; a normal or legitimately-slow single request is unaffected.

## [0.1.7] — external/escaping-symlink snapshot fidelity (2026-07-10)

### Fixed
- **`clw snapshot` no longer fails an entire tree over one external symlink.** A symlink whose target is
  absolute or escapes the workspace (e.g. a stray `node_modules/.bin/x -> /usr/local/bin/x`) used to
  hard-fail the whole snapshot; it is now **skipped with an explicit per-link warning** (path → target) and
  the rest of the tree snapshots normally. Such a link points OUTSIDE the workspace — its target isn't in the
  snapshot and couldn't be restored meaningfully — so skipping it is correct. The containment model is
  UNCHANGED and strict: `validate_symlink_target_contained` is untouched, the manifest can never contain an
  escaping symlink, hydrate is unchanged, and intra-workspace relative symlinks (`bin/x -> ../src/x`) are
  still recorded. (Closes the last deferred LOW from the 5-angle audit; security cold-reviewed.)

## [0.1.6] — 5-angle adversarial audit remediation (2026-07-09)

> A ruthless 5-lens audit (correctness · security · robustness · performance · honesty) of the shipped
> binary, then a fix wave under independent cold review per change. No CRITICAL: the integrity core
> (digest-verify-before-write, path/symlink containment, the exit-125 contract) held. The fixes below
> close the real HIGH/MED/LOW findings in producer bookkeeping, the `run` child lifecycle, and
> concurrency shape. All additive under the CLI-surface freeze — no flag/behavior/exit-code contract broken.

### Security
- **`clw run` no longer leaks the CAS credential into the untrusted child's environment.** The child is
  spawned with the credential-bearing vars (`CLW_TOKEN` / `CORELINK_TOKEN` / `CLW_CRED_TICKET` /
  `CLW_LEASE_ID` / `CLW_FABRIC_ENDPOINT` and namesakes) stripped, so untrusted CI code can no longer read
  the bearer token/ticket from its env. Non-secret config (`CLW_ENDPOINT`/`CLW_TENANT`/…) and `PATH`/`HOME`
  are preserved so builds still run. (HIGH.)
- **`clw doctor` / `clw auth whoami` / `clw auth login` redact embedded endpoint userinfo.** An endpoint
  configured as `https://user:secret@host` no longer prints its `user:secret@` userinfo in text or `--json`
  — including the support-bundle output meant to be pasted into a ticket. Redaction takes the *last*
  authority `@`, so a password that itself contains `@` cannot leak. The real endpoint is still stored to
  config unchanged; only the echo is redacted. (MED.)
- **Cache blobs are owner-only.** Cache blob files are created `0600` and the cache tree is hardened to
  `0700` on every init (not only via `auth login`/`init`), so a snapshot/hydrate-only user on a shared host
  no longer exposes workspace content to other local uids. (LOW.)

### Fixed
- **`clw snapshot` can no longer publish an unrestorable snapshot.** Each File's `size` and the manifest
  `total_size` are now derived from the actual chunked bytes (Σ chunk sizes), not walk-time metadata — so a
  file that changes size between the directory walk and the read can no longer yield a manifest whose
  `total_size` disagrees with its chunks and is rejected by *every* future `hydrate`. The manifest digest is
  unchanged for unchanged input. (HIGH.)
- **`clw run` no longer hangs forever on a backgrounded grandchild.** A child that leaves a daemon or
  `make -j` sub-job holding the stdout/stderr pipe (e.g. `sh -c 'svc & exit 0'`) no longer wedges clw: once
  the direct child exits, a bounded drain grace collects the child's real output and stops. Fast and
  cap-exceeded paths unchanged. (HIGH.)
- **`clw prune` no longer sweeps live blobs under replica lag.** A ref whose record 404s mid-mark (a lagging
  replica) makes the live-set provably incomplete; prune now aborts and deletes nothing in both dry-run and
  `--yes`, with an actionable, re-runnable error, instead of treating that ref's blobs as orphans. (MED.)
- **`clw snapshot --force` re-run is idempotent.** A lost-ACK retry of the create-only ref PUT that 409s is
  reconciled by reading the existing ref: identical content is an idempotent success; only a genuine
  difference surfaces the `--force` guidance — removing the spurious "workspace already exists" on a
  brand-new name. (MED.)
- **`clw hydrate` fsyncs each file before its rename** (durability against a torn file on power loss),
  surfaces a real permission/IO error on the destination instead of a misleading "not empty", and removes
  its temp file on a mid-write failure. (MED/LOW.)
- **The AIMD request governor tolerates a poisoned lock** (recovers the guard) instead of cascading a single
  panic into a process-wide request abort. (LOW.)
- **Exit-code contract hardened** (per `docs/CLI-SURFACE-EXIT-FREEZE.md`): a panic during `clw run` setup now
  surfaces the reserved `125` (child-never-ran), not a raw `101`; a child killed by clw for oversized output
  maps to a distinct `137` (SIGKILL), never the reserved `125`; neither is cached. (LOW.)

### Performance
- **`clw snapshot` uploads chunks with global concurrency across files** (one shared semaphore) instead of
  one-file-at-a-time, so a tree of many small files finally parallelizes; the manifest digest is
  schedule-independent (byte-identical), and repeated chunks are de-duplicated within a run. (HIGH.)
- **`clw hydrate` fetches a file's chunks concurrently and in order**, so a workspace of a few large blobs
  (e.g. the runner's toolchain image) no longer downloads chunk-by-chunk at 1-deep. (MED.)

## [0.1.5] — content-addressed hydrate by manifest digest (2026-07-06)

> The W6 deliverable: `clw hydrate --manifest-digest` unblocks the CF-native check-host image, which
> hydrates a toolchain snapshot from a content-addressed `toolchain_ref = <digest>` at container start.
> Additive under the CLI surface freeze — no existing flag, behavior, or exit code changed. The by-digest
> path re-verifies the fetched manifest against the supplied digest **before** any filesystem write, and
> reuses the identical path-escape / symlink-containment validators as the named-ref path.

### Added
- **`clw hydrate --manifest-digest <HEX>`** — materialize a snapshot **directly by its content-addressed
  manifest digest**, as an alternative to `--name <NAME>` (mutually exclusive; exactly one required). The
  fetched manifest is verified to hash to the supplied digest, so the reference is self-verifying — the ref
  *is* the content. This is the content-addressed hydrate seam for callers that already hold the digest
  (e.g. a CF-native check-host hydrating `toolchain_ref = <digest>`); it skips the named-ref (AC) lookup
  entirely. Additive flag under the CLI freeze — see `docs/REPLY-clw-TL-toolchain-resolver-seam-2026-06-26.md`.

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
- **`anyhow` bumped 1.0.102 → 1.0.103** for **RUSTSEC-2026-0190** (unsoundness in `Error::downcast_mut()`,
  advisory 2026-06-25). Lockfile-only; keeps the release supply-chain gate green on the patched release.

### Added
- **`clw snapshot --force`** — replace an existing workspace name whose content differs, via an rm-then-write
  within the one command. (Correction: this is a sub-second window, **not** atomic — a crash between the rm
  and the write can still leave no ref, the same failure as the manual `rm`-then-snapshot, just narrower.
  The Unreleased section adds an idempotent-409 recovery for the lost-ACK retry case.) Re-snapshotting
  *identical* content needs neither — see Changed.
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
