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
workflow_perms_exempt='release.yml'
missing_perms=""
overbroad_perms=""
# BOTH extensions. GitHub runs `.yml` and `.yaml` alike; globbing only `*.yml`
# let a permissions-less `evil.yaml` sit in the directory with this gate green —
# found in review, and it defeated the one invariant this guard exists to create.
# The `[ -e ]` guard is because an unmatched glob stays literal here.
for wf in .github/workflows/*.yml .github/workflows/*.yaml; do
  [ -e "$wf" ] || continue
  case " $workflow_perms_exempt " in *" $(basename "$wf") "*) continue ;; esac
  grep -q '^permissions:' "$wf" || missing_perms="${missing_perms}${wf}"$'\n'
  # PRESENCE is the assertion, with one exception: `write-all` is strictly worse
  # than the read-all default this guard exists to stop relying on, so blessing it
  # would make declaring a block a downgrade. A lane needing write says which
  # scope, per job.
  grep -qE '^permissions:[[:space:]]*write-all[[:space:]]*$' "$wf" \
    && overbroad_perms="${overbroad_perms}${wf}"$'\n'
done
if [ -n "$missing_perms" ]; then
  echo "GATE FAILED: workflow(s) without a top-level 'permissions:' block —" >&2
  echo "the GITHUB_TOKEN scope must be declared in-tree, not inherited from the repo default:" >&2
  printf '%s' "$missing_perms" >&2
  exit 1
fi
if [ -n "$overbroad_perms" ]; then
  echo "GATE FAILED: workflow(s) declare 'permissions: write-all' —" >&2
  echo "that is wider than the repository's read-all default, so declaring it is a" >&2
  echo "downgrade. Name the scopes the workflow needs, per job:" >&2
  printf '%s' "$overbroad_perms" >&2
  exit 1
fi
# Self-retiring nudge. NON-fatal, but not for the reason first written here: the
# claim was that a fatal notice would turn WP-08b red on the run that fixes it,
# which is wrong — WP-08b deletes this exemption in the same commit that adds
# release.yml's block, so a fatal notice would be atomically green. It is
# non-fatal because a gate that fails on someone else's correct change, purely to
# chase a stale list, trains people to ignore it. The follow-up is tracked in the
# work package, not by this line.
for wf in $workflow_perms_exempt; do
  if [ -f ".github/workflows/$wf" ] && grep -q '^permissions:' ".github/workflows/$wf"; then
    echo "NOTE: .github/workflows/$wf now declares 'permissions:' — remove it from workflow_perms_exempt in scripts/gates.sh"
  fi
done
echo "workflow-permissions guard: clean (all workflows declare 'permissions:'; exempt: ${workflow_perms_exempt})"

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
  if [ -z "$changed" ]; then
    echo "empty diff vs origin/${base_ref} — full lane (fail-safe)"
  elif ! echo "$changed" | grep -qvE '(\.md$|^docs/)'; then
    echo "SCOPED lane: docs-only diff — Rust gates not applicable"
    echo "ALL GATES GREEN (docs-only)"
    exit 0
  elif echo "$changed" | grep -qE '^(Cargo\.(toml|lock)|deny\.toml|scripts/|\.github/|crates/clw-types/)'; then
    echo "diff touches contract/workspace/CI surface — full lane (fail-safe)"
  elif ! echo "$changed" | grep -qvE '^crates/'; then
    SCOPE="$(echo "$changed" | grep -oE '^crates/[^/]+' | sort -u | sed 's|crates/||' | tr '\n' ' ')"
  else
    echo "diff reaches outside crates/ — full lane (fail-safe)"
  fi
fi

cargo fmt --all -- --check

if [ "$SCOPE" = "full" ]; then
  echo "FULL lane: whole workspace"
  # --all-features so feature-gated code (e.g. clw-conformance's `live` feature) is
  # held to the SAME -D warnings bar. clippy only type-checks/lints, so compiling the
  # `live` path here needs no network. (test stays default-features — see below — so
  # the live suite is not RUN without a backend.)
  cargo clippy --workspace --all-targets --all-features -- -D warnings
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
  # A `test -p <changed>` compiles the changed crate's DEPENDENCIES (downward) but not
  # its DEPENDENTS (upward) — so a change to a low-level crate (e.g. clw-snapshot) would
  # never exercise clw-cli / clw-integration-tests, letting a cross-crate regression pass
  # the required PR gate green (caught only by main's post-merge full lane). Close that:
  # always fold in the SINK crates where the whole graph converges, so any crate change
  # is tested through its top-level consumers too. (main is always full-lane regardless.)
  # Union the changed crates with the sink crates, de-duped by NAME (a change to
  # clw-cli / clw-integration-tests must not pass `-p` for it twice).
  crate_set="$(printf '%s\n' $SCOPE clw-cli clw-integration-tests | sort -u | tr '\n' ' ')"
  pkgs=""
  for c in $crate_set; do pkgs="$pkgs -p $c"; done
  # shellcheck disable=SC2086
  cargo clippy $pkgs --all-targets --all-features -- -D warnings
  # shellcheck disable=SC2086
  cargo test $pkgs
fi

# Cross-platform COMPILE gate — UNIVERSAL tripwire (runs in BOTH lanes). CI runs on a
# Linux (ubuntu) GitHub-hosted runner, which natively compiles the entire `cfg(unix)`
# path; the one cfg-DIVERGENT target — Windows (`cfg(windows)` vs `cfg(unix)`) — is
# cross-CHECKED here (a `cargo check`, type-check only, so no cross-linker/CC is needed)
# so Windows breakage is caught at PR time, not only on main's post-merge full lane
# (a `#[cfg(windows)]` break inside a scoped-lane crate change used to slip the required
# gate). Per-OS NATIVE release binaries are built by release.yml's matrix.
# Set CLW_SKIP_CROSS=1 for fast local iteration; CI/main must NOT skip it.
if [ "${CLW_SKIP_CROSS:-}" != "1" ]; then
  echo "cross-compile gate: windows-gnu (the cfg-divergent target)"
  rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || true
  # --all-targets is load-bearing (M17/WP-28): WITHOUT it, `cargo check` compiles
  # only lib/bin targets, so every `#[cfg(windows)] #[test]` in the tree was never
  # even TYPE-CHECKED by any lane — a systemic hole, not a per-WP detail. With it,
  # cfg(windows) test code is at least compiled on every PR. (Most of it is still
  # not RUN: this is a check, and windows.yml's wine job runs only the pure-lexical
  # `lexical_` tests of the M17 predicate on the windows-gnu binary — the
  # filesystem-touching ones do not run reliably under wine. Which is exactly why
  # that predicate is a plain function whose table also runs under normal
  # `cargo test` on unix.)
  cargo check --workspace --all-targets --target x86_64-pc-windows-gnu
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
