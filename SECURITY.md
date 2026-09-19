# Security Policy

> **#302 source update:** memoization paragraphs below additionally describe the
> `clw/run/v4/` transition and rejection of declared values not representable as
> UTF-8. Earlier source dates remain historical; this is not release evidence.

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
- any logs — **with your PAT and file contents redacted**. Managed identity
  reports mask their token fields, but generated help and arbitrary command
  output are separate paths. Inspect every attachment before sending; see
  the redaction scope and the open #333 limitation below.

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

This section describes source `be96b8394c66d1dc36ce7ca2f186780f0ea48308`
(workspace version 0.1.12), reviewed on 2026-09-18. The June audit and its
follow-up remain historical records, not proof that later findings are closed.
See [THREAT-MODEL.md](docs/THREAT-MODEL.md) for the boundary model and
[the audit tracker](https://github.com/HuGR-Labs/corelink-workspaces/issues/325)
for outstanding fixes. This documentation does not certify a deployed service
or a released executable.


> **Integrated changes at this source:** [PR #329](https://github.com/HuGR-Labs/corelink-workspaces/pull/329)
> supplies Unix hydrate temporary ownership and pre-publication permissions;
> [PR #334](https://github.com/HuGR-Labs/corelink-workspaces/pull/334) supplies
> checked per-file size validation. Both are already in the pinned main commit.
> Earlier test records retain their original source identities; this documentation
> update is not a new runtime-test or release result.

- **Paths require semantic validation as well as integrity.**
  `Manifest::from_bytes`, `validate_rel_path` and
  `validate_symlink_target_contained` check literal entry paths and relative
  targets. Entry paths reject dot components; contained symlink targets may
  contain `..`. The combined-link confinement finding
  [#296](https://github.com/HuGR-Labs/corelink-workspaces/issues/296) and
  destination-alias finding [#303](https://github.com/HuGR-Labs/corelink-workspaces/issues/303)
  remain open in this source. Files-before-links is not proof of full confinement.
- **Local credential and content permissions are platform-specific.** Unix
  config/cache writers use owner-only modes for their managed paths. The integrated
  #297 fix creates hydrate temporaries owner-only, arms cleanup only after exclusive
  creation and applies the intended final mode through the owned descriptor before
  sync/rename. Intentionally public final modes may be applied before rename; this
  is not a promise that such files remain private indefinitely.
  Windows config hardening explicitly warns and continues if applying the ACL
  fails. Config and default/custom cache require separate effective-access
  verification ([#317](https://github.com/HuGR-Labs/corelink-workspaces/issues/317)).
  A child running as the same user may read a reachable plaintext config; known
  secret environment names being removed does not create a filesystem sandbox.
- **Transport destination is parsed, not prefix-matched.** `clw-client` permits
  HTTPS with a host and no embedded username/password, or HTTP for a parsed
  loopback IP (IPv4 127.0.0.0/8 or IPv6 ::1) or the domain `localhost`.
  Redirects are disabled. The HTTP exception is for development/tests, not
  encrypted transport. HTTPS acceptance does not authorize an arbitrary host:
  the operator must choose a trusted destination before supplying a bearer token.
- **Memoization is a declared-input contract, not a hermetic toolchain.** The
  `clw/run/v4/` key includes selected input digests/modes, declared environment,
  command/tool identity and cwd. It does not capture every transitive tool,
  network response, clock or concurrent edit. Actual executable selection remains
  [#301](https://github.com/HuGR-Labs/corelink-workspaces/issues/301). Declared values
  not representable as UTF-8 are rejected before memo lookup/dispatch (#302);
  the v4 domain prevents reuse of v3 entries polluted by the old absent encoding.
  Only the producer is success-only in this baseline; received-record eligibility
  is [#307](https://github.com/HuGR-Labs/corelink-workspaces/issues/307).
- **Digest verification is not a proof of metadata, retention or safety.** CAS
  fetches and the local cache verify BLAKE3 against the expected content address.
  The root is one flat JSON manifest. Deterministic producer encoding and
  tolerant reader acceptance are different contracts. Integrated
  [#306](https://github.com/HuGR-Labs/corelink-workspaces/issues/306) checks each
  file's chunk sum for overflow and agreement with its declared size before
  materialization. The aggregate and fetched-byte checks remain. Broader parser
  acceptance is still [#308](https://github.com/HuGR-Labs/corelink-workspaces/issues/308).
- **Redaction has a defined scope.** Managed identity reports mask the bearer
  token and redact endpoint userinfo. Tenant IDs and paths can be shown in full;
  command output and snapshotted file contents are not automatically sanitized.
  Avoid credentials in command-line flags (process listings/history) and inspect
  any support bundle before sharing it. **Parser-generated help is a separate
  path:** this baseline displays configured token environment values in help
  ([#333](https://github.com/HuGR-Labs/corelink-workspaces/issues/333)). Do not share help captured with credentials
  present; identity-report redaction does not cover that output.
  Token authority is enforced by the
  server's scope, not inferred from a tenant name or a ref hash domain.
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
     own source changed) and on the windows-gnu cross target. `CLW_SKIP_CROSS=1` is
     an explicit local-only shortcut. The early `ci-cross-policy.sh` guard now
     refuses it when either `CI` or `GITHUB_ACTIONS` has an active or unknown
     nonempty value. Unset, empty, `false` and `0` are the accepted local markers;
     other skip values leave the cross command enabled. This #319 / PR #330
     enforcement supplements the source baseline above. Legitimate docs-only
     classification is unchanged. The guard checks trusted configuration, not
     the identity of a caller able to rewrite the workflow.
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
  it — so treat it as a floor, not as the current cost), because `gates.sh` assigns a
  job-local `CARGO_TARGET_DIR` that the earlier steps of the same job have already
  populated. It is shared only within that job; CI has no cross-run build cache.
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
  * **Toolchain selection and provenance.** `rust-toolchain.toml` selects the
    compiler channel. Release preflight in `.github/workflows/release.yml`
    explicitly checks that its channel matches `RUST_TOOLCHAIN`; it is not
    wholly unchecked. Agreement between two configuration values is not an
    attestation of the compiler binary or its installation. The selected
    toolchain and build environment still require trusted provisioning and review.
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
  The audit removed one avoidable raw-pointer borrow. These guards do not
  establish that every runtime path is panic-free; output-delivery behavior is
  separately tracked in [#310](https://github.com/HuGR-Labs/corelink-workspaces/issues/310).

## Scope

This policy covers the `clw` CLI and its crates in this repository. The CoreLink
**server** API is a separate system with its own coordination; vulnerabilities
in server-side endpoints should still be reported to the address above and will
be routed appropriately.
