#!/usr/bin/env bash
# The repo's load-bearing gates. Green or it doesn't merge — no exceptions without a human waiver.
#
# Two lanes, same rigor:
#   FULL   — main pushes, local runs, anything ambiguous: whole workspace.
#   SCOPED — pull_request CI only: gate exactly the crates the diff touches.
#            Docs-only diffs skip the Rust gates entirely (markdown can't break rustc).
#            Any touch to clw-types (the contract), Cargo.toml/lock, deny.toml,
#            scripts/ or .github/ escalates back to FULL.
# Rationale: cargo's dependency graph guarantees a scoped `test -p <crate>` still
# compiles everything that crate depends on; main is ALWAYS full-lane, so nothing
# can merge-then-rot unnoticed.
# Supply-chain invariant (made explicit so it is auditable): the FULL lane is the
# ONLY lane that runs `cargo deny check` + `cargo audit`. Therefore the inputs
# those gates read — Cargo.lock (the resolved dependency set) and deny.toml (the
# license/advisory/bans policy) — MUST be in the full-lane escalation list below,
# so a PR that edits either is re-checked against the supply-chain gates instead
# of slipping through the SCOPED lane. main is always full-lane as the backstop.
set -euo pipefail
# Run from the repo root regardless of the caller's cwd. Without this the gate
# silently ran against a DIFFERENT worktree when invoked with a `cd` that did not
# stick — a green that meant nothing.
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

# Persistent target dir on CI (outside the runner's _work checkout, immune to clean):
if [ "${CI:-}" = "true" ]; then
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/ci-cache/clw-target}"
  mkdir -p "$CARGO_TARGET_DIR"
fi

# --- DEAD-ORG-SLUG GUARD (runs in EVERY lane, including docs-only) -----------
# The GitHub org moved `humangr-labs` -> `HumanGuardrail` -> `HuGR-Labs` (2026-08-01).
# Renames leave a 301 redirect, so a stale slug KEEPS WORKING for `git clone` /
# `gh` and hides itself — but an exact-string comparison (repo allowlists,
# `[ "$REPO" = ... ]`, a brew tap path, an attestation `--repo`) does NOT follow
# redirects and silently fails. This guard fails the gate if a DEAD org slug
# reappears in live config (workflows, scripts, manifests, consumer-facing docs).
# Historical records (dated handoffs/relays/replies, the go-live plan + roadmap
# ledger, archived WP briefs) are deliberately EXEMPT — they record what was
# true then and must not be rewritten.
dead_org_hits="$(
  grep -rIn --exclude=gates.sh -E 'HumanGuardrail|humangr-labs|humanguardrail/clw' \
    .github scripts Cargo.toml README.md CLAUDE.md \
    docs/INSTALL.md docs/DISTRIBUTION.md docs/GUIDE.md \
    docs/go-live/LAUNCH-CHECKLIST.md docs/go-live/RUNBOOK-key-rotation-revocation.md \
    2>/dev/null || true
)"
if [ -n "$dead_org_hits" ]; then
  echo "GATE FAILED: dead GitHub org slug in live config — the org is 'HuGR-Labs'" >&2
  echo "$dead_org_hits" >&2
  exit 1
fi
echo "dead-org-slug guard: clean (org = HuGR-Labs)"

# --- RELEASE-NOTES GUARD (runs in EVERY lane, including docs-only) -----------
# Keeps the text the release pipeline publishes to users in a reviewed FILE
# (.github/release-notes.md) rather than a string inside release.yml, so that
# changing it is an ordinary file diff rather than an edit nobody re-reads. Note the
# limit: nothing stops a step from rewriting that file on the runner before it is
# rendered, so this is not a guarantee that what users read is what review saw.
#
# Within release.yml: every literal `gh release create|edit` must pass
# --notes-file "$NOTES_FILE" and none of gh's other notes-bearing flags; $NOTES_FILE
# may be referenced nowhere else, and the rendered file's BASENAME is reserved to its
# assignment line — which catches the literal path spelling, and only that; and the
# full argv of both invocations is pinned to .github/release-publish.expected.
# It does NOT cover a `gh()` shell function shadowing the binary, a non-bash `run:`
# step, a path to the rendered file ASSEMBLED from a variable or across a quote
# boundary, a step that rewrites .github/release-notes.md itself before the render,
# or anything outside this one file — the script header names all four and
# says why chasing them restarts a loop that already cost nine rounds. Review of
# release.yml covers them.
bash scripts/check-release-notes.sh

# --- WORKFLOW-PERMISSIONS GUARD (runs in EVERY lane, including docs-only) -----
# Every workflow must DECLARE a top-level `permissions:` block. A workflow that
# declares nothing inherits the repository default, which is an ambient,
# out-of-band setting a repo admin can widen at any time without touching this
# repo — so "the default happens to be read today" is not an invariant, it is a
# coincidence. Declaring the block in-tree pins the token scope to the file, where
# it is reviewable in a diff, and (because declaring ANY block resets every
# undeclared scope to `none`) narrows the token from "read on every scope" to
# exactly the scopes the workflow names.
#
# The assertion is about PRESENCE, not value: a lane that legitimately needs a
# different top-level scope (canary-deadman.yml needs `actions: read` to read
# other workflows' run history) declares that instead — the point is that the
# scope is written down, per job where possible, with a justification.
#
# EXEMPT (temporary, named — never a wildcard): release.yml is owned by an
# in-flight work package (WP-08b) that adds its permissions block; editing it
# here would collide. When WP-08b lands, this list must go back to empty — the
# retirement notice below says so out loud.
bash scripts/workflow-permissions.sh

# --- UNSAFE-POSTURE GUARD (runs in EVERY lane, including docs-only) ----------
# Protects the CONFIGURATION that makes the compiler enforce "no `unsafe` outside
# one audited crate". The compiler is the oracle WITHIN a crate: with the workspace
# lint at `forbid`, the clippy run below rejects `unsafe` in any target of any
# inheriting member, exactly — but only for the crates and targets some lane actually
# compiles, and only while something is still running the real rustc with the real
# lint level. This guard covers the seven places that leaves open: the lint
# configuration itself; any in-repo cargo config, which decides what runs AS the
# compiler and what is handed to it; crates detached from the workspace; a compiled
# target rooted outside its member directory; build scripts; external macro
# expansions; and source pulled in by `include!` / `#[path]` / `cfg_attr`. It parses
# manifests and cargo configs with tomllib rather than grepping, because three grep
# generations were each broken by review. See the script header and SECURITY.md —
# both of which also name what NONE of this reaches.
bash scripts/check-unsafe-posture.sh

# --- DEADMAN-REGISTRY GUARD (runs in EVERY lane, including docs-only) --------
# Every SCHEDULED workflow must be registered in canary-deadman.yml's lane
# registry, and every registered lane must actually be scheduled — equality in
# BOTH directions. Without the first leg, adding a scheduled lane silently adds a
# lane nobody watches: it can stop running and produce no red, which is the exact
# blind spot the deadman exists to close. Without the second, the registry
# overstates its own reach and the deadman false-alarms forever on a lane GitHub
# was never going to run.
#
# This runs here — at PR time — and AGAIN inside canary-deadman.yml, because the
# two catch different things: here it fails the PR that introduces the drift; there
# it fails if the registry rots on main by some path that skipped this gate.
#
# EXPLICITLY NOT COVERED: the deadman's own liveness. A dead-man's-switch cannot be
# an element of the set it watches — if canary-deadman.yml stops running, its entry
# stops being evaluated and the silence looks like health. It is excluded from the
# registry by name (and registering it is a hard failure). An observer outside this
# repo's Actions scheduler is tracked as WP-36; until then, nothing detects a dead
# deadman.
bash scripts/deadman-registry.sh check
# And the gate's own teeth: plant every defect class it claims to catch against a
# copy of the REAL workflows dir and assert it goes red AND names the thing — plus
# that the unmodified copy passes, so a gate that fires on everything cannot score
# full marks. Offline; no network.
bash scripts/deadman-selftest.sh
bash scripts/workflow-permissions-selftest.sh

# The lane classifier decides whether the Rust gates run at all, so it is itself
# gated — in EVERY lane, before any classification happens, because a broken
# classifier would otherwise route around the check that would have caught it.
# The published minisign key is also load-bearing: it must not drift from the
# installer pin, even when a PR would otherwise take the Markdown-only fast exit.
bash scripts/check-minisign-trust-root.sh
bash scripts/minisign-trust-root-selftest.sh
bash scripts/gate-lane-selftest.sh
bash scripts/mutation-verdict-selftest.sh

# shellcheck source=scripts/classify-lane.sh
. scripts/classify-lane.sh
# shellcheck source=scripts/scoped-package-set.sh
. scripts/scoped-package-set.sh

SCOPE="full"
if [ "${CI:-}" = "true" ] && [ "${GITHUB_EVENT_NAME:-}" = "pull_request" ]; then
  base_ref="${GITHUB_BASE_REF:-main}"
  git fetch -q origin "$base_ref" || true
  if ! git merge-base "origin/${base_ref}" HEAD >/dev/null 2>&1; then
    changed=""  # shallow clone / no merge base → fall through to full lane
    echo "no merge base vs origin/${base_ref} — full lane (fail-safe)"
  else
    changed="$(git diff --name-only "origin/${base_ref}...HEAD")"
  fi
  # The classification lives in scripts/classify-lane.sh so that
  # scripts/gate-lane-selftest.sh can exercise THE REAL FUNCTION rather than a copy of
  # it. It used to be inline here, and a wrong `docs` verdict skips every Rust gate and
  # still prints ALL GATES GREEN — see that file's header for the incident.
  lane="$(classify_lane "$changed")"
  case "$lane" in
    docs)
      echo "SCOPED lane: docs-only diff — Rust gates not applicable"
      echo "ALL GATES GREEN (docs-only)"
      exit 0 ;;
    crates:*)
      SCOPE="${lane#crates:}" ;;
    full)
      echo "diff needs the full lane (contract/workspace/CI surface, outside crates/, or undetermined)" ;;
    *)
      echo "GATE FAILED: classify_lane returned an unrecognised verdict: $lane" >&2
      exit 1 ;;
  esac
fi

# The reverse-dependency expansion only affects Rust lanes. Exercise its real
# metadata parser and graph walk after the docs-only exit so a Markdown-only PR
# keeps its no-toolchain/no-index contract, but every full/scoped Rust run still
# proves the expander before consuming its result.
bash scripts/scoped-package-set-selftest.sh

cargo fmt --all -- --check

if [ "$SCOPE" = "full" ]; then
  echo "FULL lane: whole workspace"
  # --all-features so feature-gated code (e.g. clw-conformance's `live` feature) is
  # held to the SAME -D warnings bar. clippy only type-checks/lints, so compiling the
  # `live` path here needs no network. (test stays default-features — see below — so
  # the live suite is not RUN without a backend.)
  cargo clippy --workspace --all-targets --all-features -- -D warnings

  # Replay the cold-review bypass corpus against the unsafe-posture guard. Five
  # generations of that guard were each defeated by a construct the previous one
  # had not modelled, and the corpus existed only in commit prose — so a rewrite
  # could reopen any of them silently, which is exactly what four of the six rounds
  # did. RUNTIME IS ENVIRONMENT-DEPENDENT AND THE TWO NUMBERS ARE 10x APART, so
  # quoting one of them alone has now been wrong three times in a row:
  #   * ~117s wall on a cold 8-worker dev box. Contended, it measures much
  #     higher — a reviewer got 159.85s at load-avg 37 — so treat the cold figure as the
  #     floor, not the cost you will see.
  #   * 14.0s in CI, 4 workers (run 33361045294: 05:36:11.34 -> 05:36:25.32), on an
  #     earlier, smaller revision of the corpus. Every case added since is a
  #     guard-oracle case with no compile in it, so treat this as a FLOOR. Carrying a
  #     case count here is what went stale three times; the suite prints its own.
  # The gap is not noise: CI=true exports a shared CARGO_TARGET_DIR that every
  # self-test case inherits, and by the time the suite runs, the steps above it in
  # THIS job have already populated that directory — so the one case that compiles
  # the real tree finds most of the graph built, while a local run gives each case
  # its own empty target dir. (Shared within the job, not across runs:
  # docs/RUNBOOK-ci-fallback.md records that no actions/cache exists and every CI
  # run starts cold.) That is why widening that case from `-p clw-types` to
  # `-p clw-cli` cost 109s locally and ~4s on the box in the merge path.
  # Earlier revisions of this comment claimed ~11s (a warm-box figure) and then
  # 70-80s (a cold-box figure), each stated as if it were THE cost. Say which box.
  # Also: the suite copies the tree per case, so anything large and tracked shows up
  # here once per case — untracking 227 stray files moved a 30-case run from ~172s
  # to ~72s. Worth checking before blaming a case.
  bash scripts/selftest-unsafe-posture.sh
  cargo test --workspace
  # Supply-chain gates are a whole-workspace concern — full lane only.
  # INVARIANT: this is the only lane that runs them, so their inputs (Cargo.lock,
  # deny.toml) escalate to this lane in the SCOPED-lane gate above. Keep the two
  # in lockstep: adding an input these gates read means adding it to that regex.
  cargo deny check
  # cargo-audit advisory lane. FAIL-CLOSED on every axis:
  #   * --deny unmaintained / --deny unsound turn those advisory classes into hard
  #     errors (the default only fails on vulnerabilities/yanked), closing the gap a
  #     cold audit flagged: an unmaintained or unsound dep could previously pass.
  #   * cargo-audit exits non-zero if it cannot fetch/update the RustSec advisory DB,
  #     and `set -euo pipefail` (top of file) propagates that — so an advisory-DB
  #     fetch FAILURE fails the gate rather than passing silently. We do NOT pass
  #     --no-fetch; the gate must run against an up-to-date DB or fail.
  #   * cargo-deny's own vulnerability/yanked fail-closed posture (above) is left
  #     intact — this is defence in depth, not a replacement.
  if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit --deny unmaintained --deny unsound
  elif [ "${CI:-}" = "true" ]; then
    # CI is the AUTHORITATIVE supply-chain gate: a missing cargo-audit here is a HARD
    # FAILURE, never a silent skip — otherwise an advisory could ship unchecked. Try to
    # provision it; if that fails, fail loudly with an ::error:: rather than skip.
    echo "cargo-audit not installed in CI — installing (the advisory gate is mandatory here)"
    # Pin the TOOL version (matches the install-action pin in ci.yml/release.yml) for
    # reproducibility — a behavior-changed cargo-audit release can't silently alter the
    # gate. The advisory DB is fetched fresh at run time regardless, so freshness of
    # advisories is unaffected by pinning the binary.
    if ! cargo install cargo-audit --version 0.22.2 --locked; then
      echo "::error::cargo-audit is unavailable in CI and could not be installed — refusing to skip the supply-chain advisory gate"
      exit 1
    fi
    cargo audit --deny unmaintained --deny unsound
  else
    echo "cargo-audit not installed — CI provisions it; skipping locally"
  fi
else
  echo "SCOPED lane: crates [${SCOPE}]"
  # Cargo's normal `-p changed` walk only follows dependencies DOWNWARD.  Expand the
  # reverse workspace closure as well, then add the named historical integration
  # sinks.  The helper validates every metadata and directory->package mapping and
  # returns a sorted, de-duped list; ambiguity is a hard failure, never a narrower run.
  if ! package_lines="$(scoped_package_set "$SCOPE")"; then
    exit 1
  fi
  [ -n "$package_lines" ] || { echo "GATE FAILED: scoped package set is empty" >&2; exit 1; }
  crate_set=()
  while IFS= read -r c; do
    [ -n "$c" ] && crate_set+=("$c")
  done <<<"$package_lines"
  [ "${#crate_set[@]}" -gt 0 ] || { echo "GATE FAILED: scoped package set is empty" >&2; exit 1; }
  pkgs=()
  for c in "${crate_set[@]}"; do pkgs+=(-p "$c"); done
  cargo clippy "${pkgs[@]}" --all-targets --all-features -- -D warnings
  cargo test "${pkgs[@]}"
fi

# Cross-platform COMPILE gate — UNIVERSAL tripwire (runs in BOTH lanes). CI runs on a
# Linux (ubuntu) GitHub-hosted runner, which natively compiles the entire `cfg(unix)`
# path; the one cfg-DIVERGENT target — Windows (`cfg(windows)` vs `cfg(unix)`) — is
# cross-LINTED here (`cargo clippy -D warnings`; CI installs gcc-mingw-w64-x86-64 for it,
# because ring's C build does need a cross-CC — the older comment said none was needed)
# so Windows breakage is caught at PR time, not only on main's post-merge full lane
# (a `#[cfg(windows)]` break inside a scoped-lane crate change used to slip the required
# gate). Per-OS NATIVE release binaries are built by release.yml's matrix.
# Set CLW_SKIP_CROSS=1 for fast local iteration; CI/main must NOT skip it.
if [ "${CLW_SKIP_CROSS:-}" != "1" ]; then
  echo "cross-compile gate: windows-gnu (the cfg-divergent target)"
  rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || true
  # clippy, not check, and at the SAME -D warnings bar as the host lane — and with
  # --all-targets. Both halves are strengthenings from different work packages and
  # both reasons still hold:
  #   * --all-targets (M17/WP-28): without it `cargo check` compiles only lib/bin
  #     targets, so every `#[cfg(windows)] #[test]` in the tree was never even
  #     TYPE-CHECKED by any lane — a systemic hole, not a per-WP detail.
  #   * clippy over check (WP-16): `cargo check` let a real clippy error ride in
  #     crates/clw-win-acl — the one crate in the workspace that contains `unsafe`,
  #     and the only one host clippy never compiles, because it is entirely
  #     cfg(windows).
  #   * rustc strips inactive `cfg` before linting, so `unsafe_code = "forbid"` only
  #     fires on code compiled for a target some lane actually builds. Linting this
  #     target as well as the host closes windows-gnu. It does NOT close every
  #     target: a `#[cfg(target_os = "macos")]` block is still unlinted by any lane
  #     here, and SECURITY.md says so rather than implying otherwise.
  #
  # Most cfg(windows) code is still not RUN here: this lints, and windows.yml's wine
  # job runs only the pure-lexical `lexical_` tests of the M17 predicate on the
  # windows-gnu binary — the filesystem-touching ones do not run reliably under
  # wine. Which is why that predicate is a plain function whose table also runs
  # under normal `cargo test` on unix.
  cargo clippy --workspace --all-targets --all-features \
    --target x86_64-pc-windows-gnu -- -D warnings
else
  echo "CLW_SKIP_CROSS=1 — skipping cross-compile gate (local fast path only)"
fi

# ---------------------------------------------------------------------------
# FUZZ supply-chain lane (opt-in; dev-only — the fuzz harness NEVER ships in clw).
#
# `fuzz/` is a DETACHED workspace (empty `[workspace]` table, not in the root
# members list), so the workspace-wide `cargo deny check` / `cargo audit` above
# never see its dependency graph. A cold audit flagged that gap. This lane closes
# it by gating fuzz/Cargo.lock with the SAME two tools, against fuzz/'s own
# manifest + lockfile.
#
# It is OPT-IN (CLW_FUZZ_GATE=1) so it never slows or breaks the normal fast lane:
#   * the deps here are libfuzzer-sys/arbitrary, pulled in only under nightly
#     `cargo fuzz`, and they are dev-only — nothing here is distributed;
#   * fuzz/Cargo.lock is committed, so this lane is reproducible without nightly
#     (it reads the manifest+lock; it does not build the fuzz targets).
# Run it explicitly in the nightly fuzz CI lane:  CLW_FUZZ_GATE=1 bash scripts/gates.sh
if [ "${CLW_FUZZ_GATE:-}" = "1" ]; then
  echo "FUZZ supply-chain lane: gating fuzz/ (detached workspace) — dev-only"
  # fuzz/ uses its OWN deny config (fuzz/deny.toml): the dev-only fuzz driver
  # libfuzzer-sys carries `AND NCSA`, a permissive license that must NOT be added
  # to the root deny.toml allow-list (that list mirrors about.toml `accepted` =
  # the SHIPPED-binary licenses; nothing in fuzz/ ships). Same sources/advisories
  # rigor, allow-list extended only for the non-shipping fuzz harness.
  cargo deny --manifest-path fuzz/Cargo.toml check --config fuzz/deny.toml
  if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit --file fuzz/Cargo.lock --deny unmaintained --deny unsound
  else
    echo "::error::cargo-audit unavailable — refusing to skip the fuzz advisory gate"
    exit 1
  fi
  echo "FUZZ supply-chain lane GREEN"
fi

echo "ALL GATES GREEN"
