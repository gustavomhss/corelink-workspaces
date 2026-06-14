#!/usr/bin/env bash
# The repo's load-bearing gates. Green or it doesn't merge — no exceptions without a human waiver.
#
# Two lanes, same rigor:
#   FULL   — main pushes, local runs, anything ambiguous: whole workspace.
#   SCOPED — pull_request CI only: gate exactly the crates the diff touches.
#            Docs-only diffs skip the Rust gates entirely (markdown can't break rustc).
#            Any touch to clw-types (the contract), Cargo.toml/lock, scripts/ or
#            .github/ escalates back to FULL.
# Rationale: cargo's dependency graph guarantees a scoped `test -p <crate>` still
# compiles everything that crate depends on; main is ALWAYS full-lane, so nothing
# can merge-then-rot unnoticed.
set -euo pipefail
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

# Persistent target dir on CI (outside the runner's _work checkout, immune to clean):
if [ "${CI:-}" = "true" ]; then
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/ci-cache/clw-target}"
  mkdir -p "$CARGO_TARGET_DIR"
fi

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
  elif echo "$changed" | grep -qE '^(Cargo\.(toml|lock)|scripts/|\.github/|crates/clw-types/)'; then
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
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  # Supply-chain gates are a whole-workspace concern — full lane only.
  cargo deny check
  if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit
  else
    echo "cargo-audit not installed — CI provisions it; skipping locally"
  fi
  # Cross-platform COMPILE gate. GitHub-hosted runners are not used (billing); the
  # self-hosted macOS runner cross-checks the one cfg-DIVERGENT target — Windows
  # (`cfg(windows)` vs `cfg(unix)`) — so Windows breakage is caught on main without a
  # native Windows runner. Linux is NOT cross-checked here: macOS natively compiles the
  # entire `cfg(unix)` path (so a Linux cross-check adds little over the native build),
  # AND `ring`'s C build needs an `x86_64-linux-gnu` cross-CC the runner lacks. The Linux
  # release artifacts are produced on-demand via `cargo zigbuild` (see release.yml /
  # docs/DISTRIBUTION.md). Runtime testing on Windows/Linux stays native/on-demand.
  # Set CLW_SKIP_CROSS=1 for fast local iteration; CI/main must NOT skip it.
  if [ "${CLW_SKIP_CROSS:-}" != "1" ]; then
    echo "cross-compile gate: windows-gnu (the cfg-divergent target)"
    rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || true
    cargo check --workspace --target x86_64-pc-windows-gnu
  else
    echo "CLW_SKIP_CROSS=1 — skipping cross-compile gate (local fast path only)"
  fi
else
  echo "SCOPED lane: crates [${SCOPE}]"
  pkgs=""
  for c in $SCOPE; do pkgs="$pkgs -p $c"; done
  # shellcheck disable=SC2086
  cargo clippy $pkgs --all-targets -- -D warnings
  # shellcheck disable=SC2086
  cargo test $pkgs
fi

echo "ALL GATES GREEN"
