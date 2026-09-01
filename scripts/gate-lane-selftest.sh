#!/usr/bin/env bash
# gate-lane-selftest.sh — prove the SCOPED-lane classifier cannot silently skip Rust.
#
# WHY THIS EXISTS
# ---------------
# `scripts/gates.sh` decides, from the changed-file list, whether a PR is docs-only and
# the Rust gates may be skipped. That decision was written as
#
#     if ! echo "$changed" | grep -qvE '(\.md$|^docs/)'; then   # docs-only
#
# and it is wrong in a way that is invisible on a small diff. `grep -q` exits at its
# first match; the `echo` then takes EPIPE; `pipefail` makes the pipeline non-zero; and
# the leading `!` reads that as "docs-only". A 33-file diff containing Rust was
# classified docs-only on PR #247 and the required check reported ALL GATES GREEN in 54
# seconds having run no fmt, no clippy, no tests and no cross lane.
#
# Nothing downstream could catch it: `pre-merge-gate-check.sh` reads check CONCLUSIONS,
# and the conclusion was success. A gate that can silently verify nothing is worse than
# no gate, because it is trusted.
#
# So the classification is pinned here: both directions (a Rust diff is never `docs`,
# a docs diff still is), BOTH inversion sites (the docs-only one and the `^crates/`
# narrowing, which has the identical EPIPE and skips `cargo deny`/`cargo audit`), the
# one escalation term that is load-bearing (`crates/clw-types/`), the empty-list
# fail-safe, and that the scoped lane is still reachable. The EPIPE condition is
# reproduced rather than described — Rust FIRST, then 20k paths, at both sites.
set -euo pipefail
cd "$(dirname "$0")/.."

# THE REAL FUNCTION, not a copy. The first draft of this file reimplemented the
# classification inline; sabotaging scripts/gates.sh then left it green, because it was
# agreeing with itself. That is the second-source-of-truth failure a gate must not have,
# and it is why classify_lane lives in its own file.
# shellcheck source=scripts/classify-lane.sh
. scripts/classify-lane.sh

pass=0; fail=0
check() { # NAME  EXPECT-VERDICT  FILE-LIST
  #
  # Compares the FULL verdict, not a docs/rust collapse. The first version of this
  # harness collapsed `full` and `crates:*` into "rust", and that single simplification
  # hid THREE reintroducible vacuous-greens at once: the sibling `^crates/` EPIPE, the
  # `crates/clw-types/` escalation, and the empty-list fail-safe. Every one of them
  # turns `full` into `crates:*` or `docs` — a distinction the collapse threw away.
  # The lesson is not "add cases"; it is that a test which discards the difference it
  # exists to protect cannot be repaired by more inputs.
  local name="$1" expect="$2" list="$3" got
  got="$(classify_lane "$list")"
  if [ "$got" = "$expect" ]; then
    printf '  \033[32mok\033[0m    %s\n' "$name"; pass=$((pass+1))
  else
    printf '  \033[31mFAIL\033[0m  %s — classified %s, expected %s\n' "$name" "$got" "$expect"
    fail=$((fail+1))
  fi
}

# The regression. A Rust path FIRST so grep exits immediately, then enough bytes to
# block the write — this is the exact shape that misclassified on PR #247. With the
# old `echo |` form this case reports `docs`.
big_rust="crates/clw-cli/src/config.rs
$(for i in $(seq 1 20000); do printf 'docs/f%d.md\n' "$i"; done)"
check "a big diff with Rust FIRST is not docs-only" full "$big_rust"

# The same list with the Rust path last: grep must read everything, so no EPIPE. This
# passed even with the broken form, which is why the bug looked unreproducible at first.
big_rust_last="$(for i in $(seq 1 20000); do printf 'docs/f%d.md\n' "$i"; done)
crates/clw-cli/src/config.rs"
check "a big diff with Rust LAST is not docs-only" full "$big_rust_last"

check "a small diff with Rust is not docs-only" full "docs/a.md
crates/clw-cli/src/config.rs"
check "a genuinely Markdown-only diff still is"  docs "docs/a.md
README.md
docs/nested/FOO.md"
check "the published trust root is never docs-only" full "docs/minisign.pub"
check "a docs image is never docs-only"             full "docs/diagram.png"
check "a docs YAML file is never docs-only"         full "docs/site.yml"
check "a Markdown file below docs/ remains docs-only" docs "docs/FOO.md"
check "the CONTRACT crate escalates to the full lane" full "crates/clw-types/src/lib.rs"
check "a workflow change escalates to the full lane" full ".github/workflows/ci.yml"
check "a script change escalates to the full lane"  full "scripts/gates.sh"

# B1 — the SIBLING inversion. `! echo "$changed" | grep -qvE '^crates/'` has the
# identical EPIPE, and under it a diff reaching outside crates/ is scoped to
# `crates:…` — the one lane that skips `cargo deny` and `cargo audit`. Rust FIRST so
# grep exits immediately, then enough bytes to block the write. This case is why the
# header may say "at both sites".
b1="crates/clw-cli/src/config.rs
$(for i in $(seq 1 20000); do printf 'docs/f%d.md\n' "$i"; done)"
check "outside-crates/ is NOT scoped (the sibling EPIPE)" full "$b1"
check "a mixed docs+crates diff is full, not scoped"      full "docs/a.md
crates/clw-cli/src/config.rs"

# B2 — the ONLY load-bearing escalation term. Cargo.toml, Cargo.lock, deny.toml,
# scripts/ and .github/ all fall through to `full` on their own if their regex term is
# deleted, so removing them changes no verdict. `crates/clw-types/` does not: without
# it a contract-crate PR narrows to `crates:clw-types` and stops running the
# supply-chain gates. It was the one term with a unique effect and the one untested.
# A MEMBER MANIFEST escalates, because it is a supply-chain input and the full lane is
# the only one that runs `cargo deny` and `cargo audit`. The escalation regex was
# anchored to the ROOT manifest, so a PR editing only `crates/<x>/Cargo.toml` skipped
# both; that was WP-16's round-1 hole and it survived into the extracted classifier
# unpinned. Source files in the same crate must still narrow, or the term is too wide.
# (An earlier version of this comment justified the term by the `[lints] workspace =
# true` opt-in instead. That reason is wrong: `check-unsafe-posture.sh` runs in EVERY
# lane before classification, so the opt-in is guarded regardless of the verdict.)
check "a member manifest escalates to the full lane" full "crates/clw-cli/Cargo.toml"
check "member SOURCE still narrows"      "crates:clw-cli " "crates/clw-cli/src/x.rs"

check "clw-types alone does not narrow to crates:"  full "crates/clw-types/src/lib.rs"
check "clw-types mixed with another crate is full"  full "crates/clw-types/src/lib.rs
crates/clw-cli/src/config.rs"

# B3 — the documented fail-safe. Empty means "we could not tell" (shallow clone, no
# merge base), NOT "nothing changed". If it ever returns docs, every such PR prints
# ALL GATES GREEN having run nothing.
check "an EMPTY list is 'could not tell' => full"   full ""

# The scoped lane must still be REACHABLE, or the narrowing is dead and every PR pays
# the full lane. A gate that only ever says `full` is as wrong as one that only ever
# says `docs`, just in the expensive direction.
check "a single crate DOES narrow"                  "crates:clw-cli " "crates/clw-cli/src/config.rs"
check "two crates narrow to both, deduped"          "crates:clw-cli clw-snapshot " "crates/clw-cli/src/a.rs
crates/clw-cli/src/b.rs
crates/clw-snapshot/src/c.rs"

# Anti-vacuity: this harness must be able to FAIL. If the classifier were replaced by a
# constant, the mixed cases above would still pass in one direction — so assert that the
# two directions genuinely disagree on some input.
if [ "$(classify_lane 'README.md')" = docs ] && [ "$(classify_lane 'crates/a/src/x.rs')" != docs ]; then
  printf '  \033[32mok\033[0m    the harness distinguishes the two directions\n'; pass=$((pass+1))
else
  printf '  \033[31mFAIL\033[0m  the classifier cannot tell docs from Rust at all\n'; fail=$((fail+1))
fi

# The lane the caller actually uses. `docs` skips every Rust gate, so a verdict this
# harness does not model would route around it silently.
fail_before_shape=$fail
for v in "$(classify_lane 'crates/clw-cli/src/x.rs')" "$(classify_lane 'scripts/gates.sh')" "$(classify_lane '')"; do
  case "$v" in docs|full|crates:*) ;; *)
    printf '  \033[31mFAIL\033[0m  unrecognised verdict %s\n' "$v"; fail=$((fail+1)) ;;
  esac
done
if [ "$fail" -eq "$fail_before_shape" ]; then
  printf '  \033[32mok\033[0m    every verdict is one gates.sh handles\n'; pass=$((pass+1))
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || { echo "GATE-LANE SELFTEST FAILED: the docs-only classifier can skip Rust gates." >&2; exit 1; }
echo "gate-lane selftest: the docs-only classifier is correct in both directions."
