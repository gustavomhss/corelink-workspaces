# Security Policy

`clw` (CoreLink Workspaces CLI) is a proprietary, commercial client of the
CoreLink API. It handles a tenant credential (a Personal Access Token) and
uploads/downloads tenant file contents to/from content-addressed storage. We
take its security posture seriously.

## Reporting a vulnerability

**Do not open a public issue, PR, or discussion for a suspected vulnerability.**

Report privately to **security@humangr.com** (PGP available on request).

Please include, where possible:

- the `clw` version (`clw --version`) and the OS/architecture you are on;
- a description of the issue and its impact (e.g. credential exposure, write
  outside the hydrate destination, integrity bypass);
- the minimal steps, manifest, or server response needed to reproduce;
- any logs — **with your PAT and file contents redacted** (`clw` already
  redacts tokens in its own diagnostics; please double-check before sending).

### What to expect

| Stage | Target window |
|---|---|
| Acknowledgement of your report | within **2 business days** |
| Initial triage + severity assessment | within **5 business days** |
| Fix or mitigation plan communicated | within **15 business days** for HIGH/CRITICAL |

We will keep you updated through remediation and will credit you in the release
notes if you wish. Please give us a reasonable window to ship a fix before any
public disclosure (coordinated disclosure).

## Supported versions

`clw` is **pre-1.0**. Only the **latest released version** is supported and
receives security fixes. There are no long-term-support or back-port branches
before `1.0`. Upgrade to the most recent release before reporting, and expect
fixes to ship as a new release rather than a patch to an older one. The semver /
stability charter and an EOL policy land with `1.0` (roadmap M3.6 / M5.5).

## Security posture

The posture below reflects the findings closed in the 2026-06-09 full-crate
audit (`docs/AUDIT-clw-2026-06-09.md`) and its M0.5 follow-up. See
`docs/THREAT-MODEL.md` for the asset/boundary/threat breakdown and the explicit
out-of-scope / residual-risk list.

- **Path traversal closed.** Manifest entry paths and symlink targets are
  validated (`clw-types::validate_rel_path` / `validate_symlink_target`):
  absolute paths and any `..`/`.` component are rejected at
  `Manifest::from_bytes`, and `hydrate` re-validates as defense-in-depth. A
  malicious or tampered manifest cannot write outside the hydrate destination
  (zip-slip / symlink-slip). *(audit H1)*
- **PAT protected at rest.** `~/.clw/config.toml` is written `0600`, `~/.clw`
  and the local cache root `0700` — no longer umask-dependent (was often
  `0644`). *(audit H2)*
- **PAT never sent in cleartext.** The HTTP client requires an `https://`
  endpoint. A loopback exception (dev/tests) uses an **exact-host** check
  (`127.0.0.1` / `localhost` / `[::1]`, optional `:port`/path) — the earlier
  prefix match that let `http://127.0.0.1.evil.com` through is closed, with a
  regression test. *(audit M2; M0.5 CRIT)*
- **No stale replay across toolchain changes.** The `clw run` memoization key
  folds the content digest of the invoked tool (`command[0]`, resolved against
  the run cwd), so a cached result is not replayed after the underlying
  toolchain changes. *(audit M3; M0.5 HIGH)*
- **Integrity verified.** Every chunk is BLAKE3 content-addressed and verified
  on fetch; manifests are canonical-JSON, Merkle-style.
- **Token redaction.** PATs are redacted in `clw`'s own output/diagnostics; the
  `--token` flag's help warns it is visible in the process list (prefer the
  config file or env).
- **`unsafe` forbidden (one audited exception).** Shipped `clw` crates contain zero
  `unsafe` **except one audited block in `clw-win-acl`** — the `cfg(windows)` DACL
  hardening in `crates/clw-win-acl/src/imp.rs`, which must go through Win32 FFI
  because there is no safe Rust equivalent. That crate exists only to hold it: ~180
  lines, and the ONLY member that restates the workspace lint as `deny` so the
  exception can carry one reviewed `#[allow(unsafe_code)]`. Every other member,
  `clw-cli` included, inherits `[workspace.lints.rust] unsafe_code = "forbid"`.
  **Two mechanisms, and neither models Rust's grammar.**
  1. **The compiler**, for `unsafe` AND for permission attributes. `forbid` cannot be
     locally re-allowed: `#[allow(unsafe_code)]`, `#[expect(unsafe_code)]` and
     `#[cfg_attr(all(), allow(unsafe_code))]` are each `error[E0453]` — verified, and
     replayed by the self-test below. The clippy run in `scripts/gates.sh` carries
     `-D warnings` on the host (`--workspace` in the full lane; in the scoped lane the
     changed crates plus the sink crates, a set that always contains any crate whose
     own source changed) and on the windows-gnu cross target. `CLW_SKIP_CROSS=1` skips
     the cross lane; it exists for local iteration, and "CI/main must NOT skip it" is
     currently a COMMENT in `scripts/gates.sh`, not an assertion — nothing fails if the
     variable is set in CI.
  2. **A content hash**, for the one `deny` crate. Its whole source tree is pinned
     byte-for-byte in `.github/win-acl-sources.expected`. Changing anything there is a
     fixture diff a person reads.
  This replaced five generations of guard that tried to police Rust with regular
  expressions. Cold review broke all five — by `#[expect]`, by `cfg_attr`, by a `]`
  inside a string truncating the match, by an `unsafe` split across a newline, by a
  lint level sitting in a comment, and finally by a string literal `"/*"` … `"*/"`
  opening a fake comment span that made the guard blank live code and report clean.
  A checker that must model Rust's grammar is writing a lexer badly.
  **What `scripts/check-unsafe-posture.sh` still does** (every lane): asserts the
  configuration by PARSING manifests with `tomllib` — the root level is `forbid`, and
  every member from `cargo metadata` either inherits it or is the one audited `deny`,
  with a failed enumeration a hard failure rather than a silent pass; asserts that NO
  in-repo cargo config sets ANY key that is not on a written allowlist (empty today),
  searching every `.cargo/` directory rather than only the root one, because cargo
  discovers config by walking cwd upward — and **failing outright on any symlinked
  directory it meets**, since skipping one is indistinguishable from finding nothing in
  it (`ln -s ci/cargo-cfg .cargo` hid a capping config from the previous version of this
  check with every gate green); asserts no crate is detached by `workspace.exclude` or
  its own `[workspace]` table,
  which would keep it linked into the binary while removing it from `cargo metadata`;
  asserts no member manifest roots a compiled target (`[lib]`, `[[bin]]`, …) at a
  `path` outside its own directory; asserts no `build.rs` exists; checks the hash pin;
  and runs four scans over EVERY file under a member directory, whatever its
  extension — for `macro_rules!`, `include!`, `#[path` and `cfg_attr`. The first three
  are MONOTONE: they match raw bytes and can only over-report, so a hit inside a
  comment is a false failure a human clears, never a miss. **The `cfg_attr` scan is
  not**, and the difference is worth stating rather than glossing: it takes the
  attribute's extent by counting balanced parens so it can compare the whole FORM
  against a ledger, and an unbalanced attribute is treated as a hit. An earlier
  version bounded the extent with `[^\]]`, which made any `cfg_attr` containing a `]`
  invisible — not a ledger miss, an invisibility — and review used
  `#[cfg_attr(all(), doc = "]", path = "…")]` to walk an earlier bypass straight back
  in. The extension does not decide what rustc compiles: `[lib] path =
  "src/lib.inc"` makes any filename a crate root, verified, and an earlier `*.rs`
  filter meant the guard never opened it. `unsafe_code` is suppressed inside external
  macro expansions, so a macro body carrying `unsafe` compiles clean under `forbid` at
  both the definition and the expansion site; `include!` and `#[path` can pull
  first-party source in from outside a member directory, where neither the scan nor the
  hash pin reaches; and `cfg_attr` BUILDS attributes, so
  `#[cfg_attr(all(), path = "…")]` is a `#[path]` no `#[path` pattern matches — it got
  a macro carrying `unsafe` into `fn main` past an earlier round of this guard.
  The workspace uses none of the first three, so those are assert-zero with a written
  allowlist. `cfg_attr` is NOT assert-zero: `clw-hydrate` carries four legitimate
  `#[cfg_attr(not(windows), allow(dead_code))]`, which a blanket refusal rejected as a
  false positive on correct code — and a guard that fires on correct code is a guard
  someone deletes. So one FORM is permitted, in a ledger keyed by the normalised
  attribute text rather than by `file:line`, because a line-keyed entry rots when code
  moves and then the next author removes the check instead of the entry. The bar for
  adding a form is written beside it: the attribute it constructs must be unable to
  name a path or to grant permission for `unsafe`. `allow(dead_code)` can do neither.
  The cargo-config check is an ALLOWLIST, not a list of dangerous keys, and that is the
  point. Its first version named `build.rustflags`, `build.rustdocflags` and
  `target.*.rustflags`; review walked past it with `build.rustc-wrapper` naming a shim
  that appends `--cap-lints allow` for the crate it wants — guard clean, fmt clean, both
  clippy lanes clean, self-test 30/30, and `unsafe` running in the shipped binary.
  `rustc-workspace-wrapper` and `rustc` do the same. Enumerating dangerous keys is the
  same losing game as enumerating dangerous flag values; the question with a closed
  answer is which keys this repo may set at all, and that is none.
  `scripts/selftest-unsafe-posture.sh` replays the whole corpus: one clean baseline,
  every recorded bypass as a case that must be rejected — most of them also pinning
  WHICH check must fire — and one that asserts a symlinked `target/` is still
  *permitted*, so fail-closed does not quietly become fail-on-everything. The suite
  reports its own totals on each run; this document deliberately does not carry a
  count, because the count went stale three times.
  Each routes to the oracle that owns it — an attribute case asserts `error[E0453]`
  from `cargo check` on a dependency-free synthetic workspace, not a guard result. One
  case deliberately compiles the REAL tree (`unsafe` in `clw-cli` must not build): the
  synthetic workspaces are hermetic and cannot see this repo's cargo configuration, so
  without it nothing in the suite would notice the whole lint being capped out of
  existence. Its reach is exactly the crates `-p` compiles, which is why it is
  `clw-cli` — the crate that ships — after a shim scoped to `clw_cli` walked past the
  `clw-types` version of it with the suite reporting 30/30.
  The full lane runs it. **Runtime is environment-dependent by ~10x** and stating one
  number as if it were the cost has been wrong three times: **~117s on a cold dev box**
  against **14.0s in CI** (that CI figure was measured on an earlier, smaller revision
  of the corpus, and every case added since is a guard-oracle case with no compile in
  it — so treat it as a floor, not as the current cost), because `gates.sh` exports
  a shared `CARGO_TARGET_DIR` when `CI=true` that the earlier steps of the same job have
  already populated — shared within a job, not across runs.
  A green suite means the RECORDED bypasses are
  still closed; it has never meant there are no others, and that has now been shown
  twice — 30/30 on a tree whose binary ran `unsafe` behind a crate-scoped wrapper, then
  39/39 on another behind a symlinked `.cargo`.
  **What is NOT covered, and is governed by human review alone:**
  * **Platform-conditional code.** rustc strips inactive `cfg` before linting, so
    `#[cfg(target_os = "macos")] { unsafe { … } }` in a `forbid` crate is invisible to
    every lane that does not compile for that target. The gate lints the CI host
    (Linux) and cross-lints windows-gnu. `release.yml` ships FIVE targets, so
    **three of the five shipped targets — `aarch64-apple-darwin`, `x86_64-apple-darwin`
    and `aarch64-unknown-linux-gnu` — are linted by no lane at all.**
    **This is the widest remaining hole.**
  * **`rust-toolchain.toml`.** In-repo, committable, and it selects WHICH rustc binary
    runs — a `channel` naming a `rustup toolchain link`-ed path aims the whole build at
    an arbitrary compiler. Same class as the cargo-config keys the guard refuses, and
    NOT checked. Named here because the rationale for checking cargo config is "a config
    decides what runs as the compiler", and it applies to this file verbatim.
  * **Lint capping, and compiler replacement, from the environment.** The guard closes
    every in-repo cargo config, but the same effects arrive from outside the repo.
    `RUSTFLAGS="--cap-lints allow"` and `CARGO_ENCODED_RUSTFLAGS` carry the flag
    directly; **`RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER` and `RUSTC` replace the
    compiler outright** with a script free to add anything to any crate it picks — the
    environment twin of the config keys the guard now refuses. A `~/.cargo/config.toml`
    on the runner does either — as does a `.cargo/config.toml` in any **ancestor**
    directory of the checkout, since cargo walks CWD upward before consulting
    `$CARGO_HOME`, which `CARGO_HOME` itself relocates. Nothing in a repo can assert
    about its own environment;
    this is a property of the CI definition, not of these scripts.
  * **Cargo target `path` keys** are asserted to stay inside their member directory,
    but a path that stays inside it is otherwise untreated — the file it names is
    scanned like any other, and nothing checks that a target root is the file the
    package layout implies.
  * `unsafe` added inside the audited block itself — the hash pin makes it a fixture
    diff, but reading that diff is a person's job. Keeping the crate tiny is what
    makes that tractable.
  * Third-party dependencies, `[patch]`-substituted paths, and proc-macro expansions,
    which have no first-party source to inspect.
  * `fuzz/`, a deliberately detached workspace.
  The one avoidable raw-pointer borrow found in the audit was removed. Prod
  paths avoid `unwrap/expect/panic`.

## Scope

This policy covers the `clw` CLI and its crates in this repository. The CoreLink
**server** API is a separate system with its own coordination; vulnerabilities
in server-side endpoints should still be reported to the address above and will
be routed appropriately.
