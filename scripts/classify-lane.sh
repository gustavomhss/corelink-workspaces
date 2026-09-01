#!/usr/bin/env bash
# classify-lane.sh — decide which gate lane a changed-file list needs.
#
# EXTRACTED SO IT CAN BE TESTED. `scripts/gates.sh` sources this, and
# `scripts/gate-lane-selftest.sh` calls THIS function — not a copy of it. A selftest
# that reimplements the logic it checks is a second source of truth and agrees with
# itself forever; the first draft of that selftest did exactly that, and sabotaging
# `gates.sh` left it green.
#
# WHAT THIS DECIDES, AND WHY IT IS LOAD-BEARING
# ---------------------------------------------
# A `docs` verdict SKIPS every Rust gate — fmt, clippy, the workspace tests, the
# supply-chain lanes, the cross-compile lane — and `gates.sh` then prints
# ALL GATES GREEN. So a wrong `docs` verdict is not a slow build; it is a required
# check that verified nothing and said everything was fine.
#
# That happened. PR #247, job 99505002408: 54 seconds on a 33-file diff containing
# `crates/clw-hydrate/src/lib.rs`, with `echo: write error: Broken pipe` in the log,
# then `ALL GATES GREEN (docs-only)`. The classification was
#
#     ! echo "$changed" | grep -qvE '(\.md$|^docs/)'
#
# `grep -q` exits at its first match, the `echo` takes EPIPE, `pipefail` makes the
# pipeline non-zero, and the leading `!` reads that as "docs-only". It needs the first
# non-doc path EARLY and enough bytes after it to block the write, so it is invisible
# on a small diff — which is why it survived, and why `pre-merge-gate-check.sh` could
# not catch it: that tool reads check CONCLUSIONS, and the conclusion was `success`.
#
# Every comparison here therefore uses a here-string. No pipes.
set -euo pipefail

# classify_lane <changed-file-list>
#   prints one of:  full | docs | crates:<name> <name> ...
#
# FAIL-SAFE DIRECTION: everything unrecognised is `full`. The cheap verdict must be the
# one that runs MORE, never less.
classify_lane() {
  local changed="$1"

  # An empty list is not "nothing changed" — it is "we could not tell", which the
  # caller reaches on a shallow clone or a missing merge base.
  [ -n "$changed" ] || { printf 'full\n'; return 0; }

  # docs-only: EVERY path must be Markdown. `docs/` is not itself a safe surface:
  # docs/minisign.pub is the published trust root, while images and YAML can change
  # rendered or consumed release material. Keep the here-string: a pipe to grep -q
  # can turn a large adversarial diff into a false docs verdict through EPIPE.
  if ! grep -qvE '\.md$' <<<"$changed"; then printf 'docs\n'; return 0; fi

  # The contract, the workspace manifests, the supply-chain policy inputs and anything
  # under scripts/ or .github/ escalate to the full lane. INVARIANT: the full lane is
  # the only one that runs `cargo deny` and `cargo audit`, so their inputs must be
  # listed here or a PR editing them would skip them.
  # `crates/[^/]+/Cargo\.toml` is WP-16's term. A member manifest is a SUPPLY-CHAIN
  # input — it can add, bump or re-source a dependency — and the full lane is the only
  # one that runs `cargo deny` and `cargo audit`. The regex was anchored to the ROOT
  # manifest, so a PR editing only `crates/<x>/Cargo.toml` skipped both.
  #
  # NOT the reason, though an earlier version of this comment claimed it was: the
  # `[lints] workspace = true` opt-in that applies `unsafe_code = "forbid"` also lives
  # in these files, but `scripts/gates.sh` runs `check-unsafe-posture.sh`
  # UNCONDITIONALLY, before any classification, in every lane including docs-only — so
  # deleting an opt-in is caught whatever lane this returns. Review proved it. The
  # supply-chain reason stands on its own; the other was a guess that read well.
  if grep -qE '^(Cargo\.(toml|lock)|deny\.toml|scripts/|\.github/|crates/clw-types/|crates/[^/]+/Cargo\.toml)' <<<"$changed"; then
    printf 'full\n'; return 0
  fi

  # Confined to crates/: narrow to those packages.
  if ! grep -qvE '^crates/' <<<"$changed"; then
    printf 'crates:%s\n' "$(grep -oE '^crates/[^/]+' <<<"$changed" | sort -u | sed 's|crates/||' | tr '\n' ' ')"
    return 0
  fi

  printf 'full\n'
}
