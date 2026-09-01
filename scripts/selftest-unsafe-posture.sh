#!/usr/bin/env bash
# selftest-unsafe-posture.sh — the regression corpus for scripts/check-unsafe-posture.sh.
#
# Eight rounds of cold review found ways to get `unsafe` into the shipped binary with
# every gate reporting clean. Each was fixed; none was ever recorded anywhere a machine
# could re-check. That corpus lived only in commit prose, which means the next rewrite of
# the guard could reopen any of them silently — and six of the eight rounds did exactly
# that to an earlier round's fix.
#
# This replays them. Each case mutates a COPY of the tree in a temp dir and asserts the
# named oracle rejects it. A case that goes green is a reopened bypass.
#
# CASE COUNT: do not maintain a number here. Round 11 found this header saying 42 while
# the suite printed 43, because case 8d was added without bumping it — a stale count in
# the file that exists to prove counts are honest. The suite reports its own totals on
# every run; read those. The SHAPE is: one clean baseline, N mutations that must each be
# REJECTED, and one that must
# be PERMITTED (8c: a symlinked `target/` is legitimate and must not fail the gate — a
# guard that fires on everything is as useless as one that fires on nothing). Most of
# the rejected cases additionally pin WHICH check must fire (`--because`, below) — that
# is an attribute of a case, not a case of its own, so nothing is double-counted. The
# suite prints TWO `green` lines for that reason, and it prints its own totals; read
# those. Round 12 found this very paragraph maintaining two numbers four lines after
# telling the reader not to.
#
# This suite is not self-validating, and has now proved that twice. Round 7 reported 30
# passed, 0 failed on a tree whose shipped binary ran `unsafe` (the bypass was scoped to
# a crate no case compiled); round 8 reported 39 passed, 0 failed on another, behind a
# symlinked `.cargo`. A green run means the RECORDED bypasses are still closed. It has
# never meant there are no others, and twice in a row it did not mean the tree was safe.
#
# Not wired into the scoped lane: it runs `cargo metadata` per case. `scripts/gates.sh`
# runs it in the FULL lane.

set -euo pipefail
cd "$(dirname "$0")/.."
SRC="$PWD"
GUARD=scripts/check-unsafe-posture.sh

TMPROOT="$(mktemp -d)"
trap 'rm -rf "$TMPROOT"' EXIT

# Deliberately NO shared CARGO_TARGET_DIR. The synthetic workspaces have no
# dependencies, so each builds in well under a second on its own, and sharing a
# target dir would only make them queue on cargo's lock.

# Cases are independent — each mutates its own copy of the tree — so they run
# CONCURRENTLY. Serially this suite was 2 minutes, which is too much to put in front
# of every merge; nobody waits twice for a check they can skip. Results are written to
# per-case files and reported in declaration order, so the output is deterministic
# even though the execution is not.
JOBS="${SELFTEST_JOBS:-$( (command -v nproc >/dev/null && nproc) || sysctl -n hw.ncpu 2>/dev/null || echo 4 )}"
[ "$JOBS" -gt 8 ] && JOBS=8
mkdir -p "$TMPROOT/res"
seq_no=0

wait_for_slot() {
  while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do wait -n 2>/dev/null || sleep 0.05; done
}

pass=0; fail=0
# Run one case: NAME, then a function body that mutates $W (a fresh copy of the tree).
# Asserts the guard FAILS. `expect_ok` asserts the opposite, for the clean baseline.
# run_case NAME (fail|ok) [--oracle guard|compiler:<pkg>] [--because <substring>] -- setup...
#
# WHICH ORACLE ANSWERS WHICH QUESTION matters, and getting it wrong is how a corpus
# becomes decorative. A permission attribute in a `forbid` crate is rejected BY THE
# COMPILER (error[E0453], verified for allow, expect and cfg_attr) — the guard does
# not look for it and must not, because five generations of attribute matching were
# each defeated by a spelling. Those cases assert `cargo check` fails. Only the
# configuration, the detachment tricks, the macro/include scans and the hash pin are
# the guard's job.
# `--because` pins WHICH check fired. Without it a case can go red for an unrelated
# reason and still look like proof — review found exactly that: the workspace.exclude
# case was actually being caught by the stray-manifest check, so deleting the exclude
# assertion left the self-test fully green.
run_case() {
  seq_no=$((seq_no + 1))
  wait_for_slot
  idx="$seq_no" _run_case "$@" &
}

# synth_case NAME <attribute-line> — assert rustc rejects that spelling under an
# inherited `forbid`. Builds a two-file workspace with no dependencies.
synth_case() {
  seq_no=$((seq_no + 1))
  wait_for_slot
  idx="$seq_no" _synth_case "$@" &
}

_synth_case() {
  # `idx` arrives as a one-shot prefix assignment on the caller's `&` line, which
  # the linter cannot see (SC2154, which was RED on the PR). Assert it rather than
  # suppress it: unset would silently collapse every case onto one temp dir and
  # one result file, so the suite would report on whichever worker wrote last.
  : "${idx:?internal: run/synth case invoked without an index}"
  local name="$1" attr="$2"
  local W="$TMPROOT/synth$idx"
  mkdir -p "$W/m/src"
  cat > "$W/Cargo.toml" <<EOF
[workspace]
members = ["m"]
resolver = "2"
[workspace.lints.rust]
unsafe_code = "forbid"
EOF
  cat > "$W/m/Cargo.toml" <<EOF
[package]
name = "m"
version = "0.0.0"
edition = "2021"
[lints]
workspace = true
EOF
  printf '%s\npub fn probe() {}\n' "$attr" > "$W/m/src/lib.rs"
  local rc=0 out=""
  out="$( cd "$W" && cargo check 2>&1 )" || rc=$?
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qF E0453; then
    printf 'PASS\t  \033[32mred\033[0m    %s\n' "$name" > "$TMPROOT/res/$idx"
  else
    printf 'FAIL\t  \033[31mBYPASS REOPENED\033[0m  %s (rustc exit %d, no E0453)\n' \
      "$name" "$rc" > "$TMPROOT/res/$idx"
  fi
}

_run_case() {
  # `idx` arrives as a one-shot prefix assignment on the caller's `&` line, which
  # the linter cannot see (SC2154, which was RED on the PR). Assert it rather than
  # suppress it: unset would silently collapse every case onto one temp dir and
  # one result file, so the suite would report on whichever worker wrote last.
  : "${idx:?internal: run/synth case invoked without an index}"
  local name="$1" want="$2"; shift 2
  local because="" oracle="guard"
  while :; do
    case "${1:-}" in
      --oracle)  oracle="$2"; shift 2 ;;
      --because) because="$2"; shift 2 ;;
      --)        shift; break ;;
      *)         break ;;
    esac
  done
  local W="$TMPROOT/case$idx"
  mkdir -p "$W"
  # Tracked files AND untracked-but-not-ignored ones. `git ls-files` alone silently
  # omitted new files, so a bypass staged as a brand-new file was not under test at
  # all — and the guard's own new fixture made the BASELINE fail. Excludes target/
  # and .git via --exclude-standard plus the ignore rules.
  (cd "$SRC" && git ls-files -z -co --exclude-standard) \
    | (cd "$SRC" && tar --null -cf - -T -) | (cd "$W" && tar -xf -)
  ( cd "$W" && "$@" ) \
    || { printf 'FAIL\t  SETUP FAILED: %s\n' "$name" > "$TMPROOT/res/$idx"; return; }
  local rc=0 out=""
  case "$oracle" in
    guard)
      out="$( cd "$W" && bash "$GUARD" 2>&1 )" || rc=$? ;;
    real-tree-compile)
      # COMPILE THIS REPOSITORY, as checked out, with its own cargo configuration.
      #
      # Every other compile case here builds a dependency-free SYNTHETIC workspace in
      # $TMPROOT, because the question they ask — does `forbid` reject this attribute
      # spelling? — is a fact about rustc that none of clw's dependencies participate
      # in, and compiling clw-types to ask it cost 41 seconds of cold graph.
      #
      # That optimisation deleted a property nobody noticed was load-bearing. A
      # synthetic workspace is HERMETIC: it cannot see this repo's `.cargo/config.toml`
      # or anything else that reaches rustc here. So the six fast cases would go on
      # passing while `[build] rustflags = ["--cap-lints", "allow"]` — two lines in a
      # file reviewers skim as linker config — demoted `forbid` to `allow` across the
      # whole workspace, with the guard, fmt, and BOTH clippy lanes clean. Verified.
      #
      # This case is the sentinel for that. It puts `unsafe` in a `forbid` crate of the
      # REAL tree and asserts the real toolchain refuses to compile it. Anything that
      # caps, redirects or otherwise defangs the lint turns this case GREEN, which the
      # harness reports as BYPASS REOPENED.
      #
      # ITS REACH IS EXACTLY THE CRATES `-p` COMPILES, and an earlier revision of this
      # comment claimed more than that. The version scoped to `-p clw-types` was walked
      # straight past by a `rustc-wrapper` shim that appended `--cap-lints allow` only
      # when it saw `clw_cli` — a different crate, so this case still failed to compile
      # and still passed, 30/30, while the shipped binary ran `unsafe`. So it is
      # `-p clw-cli` now: the crate that actually ships and handles the plaintext PAT,
      # and the one whose graph pulls in clw-types (and the rest) on the way, so a shim
      # scoped to EITHER is caught. A wrapper narrowed to a crate outside clw-cli's
      # graph still evades it; the guard's cargo-config allowlist is what closes that
      # class, and this is the backstop for the day something reaches rustc anyway.
      #
      # COST, because it should be a deliberate choice and not a surprise: 109s cold
      # locally, against 21s for the clw-types version. On CI it is close to free —
      # gates.sh exports a shared CARGO_TARGET_DIR when CI=true that every case
      # inherits and the job's earlier steps have already populated, so the whole
      # suite measured 14.0s there WITH this case widened (9.85s was the narrower
      # `-p clw-types` suite; the ~4s delta is this change). The expensive box is the
      # developer's; the box that pays in the merge path is warm.
      out="$( cd "$W" && cargo check -p clw-cli 2>&1 )" || rc=$? ;;
    *) printf 'FAIL\t  bad oracle: %s\n' "$oracle" > "$TMPROOT/res/$idx"; return ;;
  esac
  if [ "$want" = fail ] && [ "$rc" -ne 0 ] && [ -n "$because" ] \
     && ! printf '%s' "$out" | grep -qF -- "$because"; then
    printf 'FAIL\t  \033[31mWRONG CHECK FIRED\033[0m  %s (wanted: %s)\n' \
      "$name" "$because" > "$TMPROOT/res/$idx"; return
  fi
  if [ "$want" = fail ] && [ "$rc" -ne 0 ]; then
    printf 'PASS\t  \033[32mred\033[0m    %s\n' "$name" > "$TMPROOT/res/$idx"
  elif [ "$want" = ok ] && [ "$rc" -eq 0 ]; then
    printf 'PASS\t  \033[32mgreen\033[0m  %s\n' "$name" > "$TMPROOT/res/$idx"
  else
    printf 'FAIL\t  \033[31mBYPASS REOPENED\033[0m  %s (oracle exit %d, wanted %s)\n' \
      "$name" "$rc" "$want" > "$TMPROOT/res/$idx"
  fi
}

# `_run_case` copies the tree with `git ls-files`, so outside a work tree every case
# would report "NO RESULT ... the worker died" — fail-closed, but 39 identical corpses
# name the symptom and not the cause.
git rev-parse --git-dir >/dev/null 2>&1 || {
  echo "SELF-TEST CANNOT RUN: not inside a git work tree." >&2
  echo "Each case copies the tree with \`git ls-files\`; without one there is nothing" >&2
  echo "to copy and every case would fail for that reason rather than on its merits." >&2
  exit 1; }

CLI=crates/clw-cli/src/config.rs
TYPES=crates/clw-types/src/lib.rs
ACL=crates/clw-win-acl/src/imp.rs

echo "unsafe-posture self-test — replaying the cold-review bypass corpus"

run_case "baseline: unmutated tree is clean" ok true

echo "round 1 — attribute spellings (oracle: the COMPILER, not the guard)"
synth_case "1.1 second #[allow(unsafe_code)] in another file" '#[allow(unsafe_code)]'
run_case "1.2 member drops [lints] workspace = true" fail \
  sh -c 'perl -0pi -e "s/\[lints\]\nworkspace = true\n//" crates/clw-types/Cargo.toml'

echo "round 2 — more spellings, same oracle"
synth_case "2.1 #[allow(unsafe_code, dead_code)]" '#[allow(unsafe_code, dead_code)]'
synth_case "2.2 #[allow( unsafe_code )] with whitespace" '#[allow( unsafe_code )]'
synth_case "2.3 #[expect(unsafe_code)]" '#[expect(unsafe_code)]'
synth_case "2.4 #[allow(clippy::all, unsafe_code)]" '#[allow(clippy::all, unsafe_code)]'
run_case "2.5 member sets unsafe_code = \"allow\"" fail \
  sh -c 'printf "\n[lints.rust]\nunsafe_code = \"allow\"\n" >> crates/clw-chunk/Cargo.toml
         perl -0pi -e "s/\[lints\]\nworkspace = true\n//" crates/clw-chunk/Cargo.toml'
run_case "2.6 root lint flipped to \"allow\"" fail \
  sh -c 'perl -0pi -e "s/unsafe_code = \"forbid\"/unsafe_code = \"allow\"/" Cargo.toml'
run_case "2.7 audited crate flipped to \"allow\"" fail \
  --because "outside the unsafe_code lint" -- \
  sh -c 'perl -0pi -e "s/unsafe_code = \"deny\"/unsafe_code = \"allow\"/" crates/clw-win-acl/Cargo.toml'

echo "round 3 — parser vs text"
run_case "3.3 forbid present only in a COMMENT" fail \
  sh -c 'perl -0pi -e "s/^unsafe_code = \"forbid\"/# unsafe_code = \"forbid\"\nunsafe_code = \"allow\"/m" Cargo.toml'
run_case "3.4 member outside crates/ without [lints]" fail \
  sh -c 'mkdir -p tools/helper/src
         printf "[package]\nname=\"helper\"\nversion=\"0.0.0\"\nedition=\"2021\"\n" > tools/helper/Cargo.toml
         printf "pub fn h() {}\n" > tools/helper/src/lib.rs
         perl -0pi -e "s{    \"crates/clw-types\",}{    \"tools/helper\",\n    \"crates/clw-types\",}" Cargo.toml'
run_case "3.6 cargo metadata fails -> must not report clean" fail \
  sh -c 'printf "\nthis is not toml [[[\n" >> Cargo.toml'

echo "round 4 — the three that compiled green"
synth_case "4.1 #[cfg_attr(all(), allow(unsafe_code))]" '#[cfg_attr(all(), allow(unsafe_code))]'
run_case "4.2 cross-crate macro_rules! body" fail \
  --because "define macro_rules! macros" -- \
  sh -c 'printf "\n#[macro_export]\nmacro_rules! clw_m { () => { 1 }; }\n" >> '"$TYPES"
run_case "4.3 workspace.exclude detaches an in-repo path dep" fail \
  --because "declares workspace.exclude" -- \
  sh -c 'mkdir -p vendor/evil/src
         printf "[package]\nname=\"evil\"\nversion=\"0.0.0\"\nedition=\"2021\"\n" > vendor/evil/Cargo.toml
         printf "pub fn e() {}\n" > vendor/evil/src/lib.rs
         perl -0pi -e "s/^members = \[/exclude = [\"vendor\/evil\"]\nmembers = [/m" Cargo.toml'
run_case "4.3b sibling crate with its own [workspace] table" fail \
  --because "are NOT workspace members" -- \
  sh -c 'mkdir -p vendor/evil/src
         printf "[workspace]\n[package]\nname=\"evil\"\nversion=\"0.0.0\"\nedition=\"2021\"\n" > vendor/evil/Cargo.toml
         printf "pub fn e() {}\n" > vendor/evil/src/lib.rs'

echo "round 5 — the four that compiled green"
run_case "5a fake comment span hides a macro_rules!" fail \
  --because "define macro_rules! macros" -- \
  sh -c 'printf "\nconst _O: &str = \"/*\";\n#[macro_export]\nmacro_rules! p { () => { 1 }; }\nconst _C: &str = \"*/\";\n" >> '"$TYPES"
# The included file holds a plain fn, not a macro, ON PURPOSE. Dropping the `*.rs`
# filter means `probe.inc` is now SCANNED, so a macro in it trips the macro_rules check
# first and this case would stop testing the thing it is named after — `--because`
# caught exactly that. The strengthening is real and 6c is where it is asserted; here
# the payload is benign so the include! check is the only one that can fire.
run_case "5b include! of a non-.rs file" fail \
  --because "pull source in by path" -- \
  sh -c 'printf "pub fn p() {}\n" > crates/clw-types/src/probe.inc
         printf "\ninclude!(\"probe.inc\");\n" >> '"$TYPES"
run_case "5c #[path] escaping the member directory" fail \
  --because "pull source in by path" -- \
  sh -c 'mkdir -p shared && printf "pub fn p() {}\n" > shared/probe.rs
         printf "\n#[path = \"../../../shared/probe.rs\"]\nmod probe;\n" >> '"$TYPES"
run_case "5d ] inside a string, in the audited crate" fail \
  --because "no longer matches" -- \
  sh -c 'printf "\n#[cfg_attr(all(), doc = \"]\", allow(unsafe_code))]\npub fn p() {}\n" >> '"$ACL"

echo "round 6 — the .rs filter, the attribute that builds attributes, and cap-lints"
# The guard's escape scan was anchored on the literal `#[path`, so an attribute that
# CONSTRUCTS a path attribute went straight past it: verified rc=0, printing "no
# macro_rules!, no include!/#[path]", with the macro body's `unsafe` running in fn main.
run_case "6a #[cfg_attr(all(), path = …)] escaping the member dir" fail \
  --because "pull source in by path, or use cfg_attr" -- \
  sh -c 'mkdir -p shared
         printf "#[macro_export]\nmacro_rules! p { () => { unsafe {} }; }\n" > shared/probe.rs
         printf "\n#[cfg_attr(all(), path = \"../../../shared/probe.rs\")]\nmod probe;\n" >> '"$TYPES"
# `macro_rules` and `!` are separate tokens; rustc accepts a newline between them and
# `\bmacro_rules!` did not. Verified rc=0. `cargo fmt --check` catches it incidentally,
# which is luck: fmt is a style gate and nothing here may lean on it.
run_case "6b macro_rules and ! split across a newline" fail \
  --because "define macro_rules! macros" -- \
  sh -c 'printf "\n#[macro_export]\nmacro_rules\n! p { () => { unsafe {} }; }\n" >> '"$TYPES"
# The scans were filtered to `*.rs` while the guard's own header said "regardless of
# extension". Cargo takes any filename as a crate root, verified — so the one file that
# mattered was the one file the guard never opened.
run_case "6c [lib] path = \"src/lib.inc\" — a crate root that is not .rs" fail \
  --because "define macro_rules! macros" -- \
  sh -c 'printf "#[macro_export]\nmacro_rules! p { () => { unsafe {} }; }\n" > crates/clw-types/src/lib.inc
         printf "\n[lib]\npath = \"src/lib.inc\"\n" >> crates/clw-types/Cargo.toml'
# The manifest form of #[path]: a compiled target rooted outside the member directory,
# where the source walk never reaches and the hash pin does not cover.
run_case "6d [lib] path escaping the member directory" fail \
  --because "outside their own directory" -- \
  sh -c 'mkdir -p shared && printf "pub fn p() {}\n" > shared/lib.rs
         printf "\n[lib]\npath = \"../../shared/lib.rs\"\n" >> crates/clw-types/Cargo.toml'
# THE CHEAPEST BYPASS IN THE CORPUS. Two lines in a file that reads as linker config.
run_case "6e .cargo/config.toml caps every lint to allow" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p .cargo
         printf "[build]\nrustflags = [\"--cap-lints\", \"allow\"]\n" > .cargo/config.toml'
# ...and the sentinel that would have caught it without anyone thinking of it: the one
# case that compiles the REAL tree. Asserts `unsafe` in a `forbid` crate does not build
# here. Turning the lint off by ANY means — a cargo config, a target override, a
# `[lints]` edit — flips this green.
run_case "6f real tree: unsafe in the SHIPPED crate must not compile" fail \
  --oracle real-tree-compile --because 'usage of an `unsafe` block' -- \
  sh -c 'printf "\npub fn _probe() { unsafe {} }\n" >> '"$CLI"
# `build.rs` used to be a sentence in the boundary list ("of which none exists") with
# no check behind it, next to two scans that mechanized the identical claim.
run_case "6g a build script appears" fail \
  --because "carry a build script" -- \
  sh -c 'printf "fn main() {}\n" > crates/clw-types/build.rs'

echo "round 7 — the cargo config decides what RUNS as the compiler"
# The round-6 assertion was a three-key denylist (build.rustflags, build.rustdocflags,
# target.*.rustflags). Review walked past it with a key it had not named: a wrapper
# script that appends `--cap-lints allow` when it sees the crate it wants. Guard rc=0
# printing "uncapped by any repo cargo config", fmt clean, both clippy lanes clean, and
# this suite 30/30 — the shim was scoped to clw_cli so the then-clw-types sentinel still
# passed. Hence an ALLOWLIST, and hence 6f now compiles clw-cli.
run_case "7a .cargo/config.toml sets build.rustc-wrapper" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p .cargo
         printf "[build]\nrustc-wrapper = \"scripts/rustc-shim.sh\"\n" > .cargo/config.toml'
# Strictly better for an attacker than rustc-wrapper: applies to WORKSPACE MEMBERS only,
# so it never touches a dependency and is that much less likely to be noticed.
run_case "7b .cargo/config.toml sets build.rustc-workspace-wrapper" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p .cargo
         printf "[build]\nrustc-workspace-wrapper = \"scripts/shim.sh\"\n" > .cargo/config.toml'
# And the blunt one: replace the compiler outright.
run_case "7c .cargo/config.toml sets build.rustc" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p .cargo && printf "[build]\nrustc = \"./shim\"\n" > .cargo/config.toml'
# Cargo discovers config by walking CWD upward, so this file is inert from the repo root
# and fully live the moment anything runs cargo in that directory. The root-only check
# never opened it.
run_case "7d cargo config in a MEMBER subdirectory, not the root" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p crates/clw-cli/.cargo
         printf "[build]\nrustflags = [\"--cap-lints\", \"allow\"]\n" > crates/clw-cli/.cargo/config.toml'
# The extensionless spelling cargo still honours.
run_case "7e the legacy .cargo/config filename" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p .cargo && printf "[build]\nrustflags = [\"--cap-lints\"]\n" > .cargo/config'
# An allowlist has to bite on keys nobody classified as dangerous, or it is a denylist
# with extra steps. `build.jobs` is harmless; it is still not on the list.
run_case "7f an innocuous cargo key is still not on the allowlist" fail \
  --because "not on the allowlist" -- \
  sh -c 'mkdir -p .cargo && printf "[build]\njobs = 2\n" > .cargo/config.toml'
# Fail CLOSED, twice over. A broken config at the ROOT breaks cargo itself, so the
# `cargo metadata` check catches it before the guard ever parses the file — pinned here
# so that is a recorded fact and not a lucky red.
run_case "7g an unparseable ROOT cargo config" fail \
  --because '`cargo metadata` failed' -- \
  sh -c 'mkdir -p .cargo && printf "[build\nrustflags = [\n" > .cargo/config.toml'
# ...and the same file in a member subdirectory, which cargo (run from the root) never
# loads. This is what actually exercises the guard's own fail-closed parse branch: a
# config whose effect on the lint level cannot be read is not "clean".
run_case "7g2 an unparseable cargo config cargo does not load" fail \
  --because "cannot be parsed as TOML" -- \
  sh -c 'mkdir -p crates/clw-cli/.cargo
         printf "[build\nrustflags = [\n" > crates/clw-cli/.cargo/config.toml'
# The LIST form of the build-script key. It fell through every branch of the build.rs
# check and was silently ignored there — but it is NIGHTLY-only, and the pinned stable
# toolchain rejects it in `cargo metadata`, so the guard is fail-closed today by a
# different check. Pinned to the one that really fires rather than to the one I wanted
# to be firing. The parse branch that handles the list (and hard-fails on a `build`
# value it does not recognise) is the backstop for the day it stabilises, and this case
# is where that will be noticed: it will start failing --because and want re-pinning.
run_case "7h [package] build in its list form" fail \
  --because '`cargo metadata` failed' -- \
  sh -c 'printf "fn main() {}\n" > crates/clw-types/gen.rs
         perl -0pi -e "s/^(\\[package\\]\n)/\$1build = [\"gen.rs\"]\n/m" crates/clw-types/Cargo.toml'

echo "round 8 — not looking is not the same as finding nothing"
# THE BYPASS THE ALLOWLIST DID NOT CLOSE. The cargo-config walk dropped symlinked
# directories from `dirs[:]` before `basename(dirpath) == ".cargo"` could match, so a
# symlinked `.cargo` was never entered and the config behind it never sat in a directory
# the walk called `.cargo` — while cargo resolves it through the OS and reads it. One
# committable, diffable line that reads as "share the CI cargo config". Measured on the
# round-8 head: guard rc=0 printing "0 in-repo .cargo/config file(s), 0 key(s) off the
# allowlist", fmt clean, workspace clippy -D warnings clean, and this suite 39/39 green,
# with a rustc-wrapper shim capping the lint for clw-cache and `unsafe` in the binary.
run_case "8a a SYMLINKED .cargo directory hides the config" fail \
  --because "symlinked directory in the tree" -- \
  sh -c 'mkdir -p ci/cargo-cfg
         printf "[build]\nrustc-wrapper = \"ci/shim.sh\"\n" > ci/cargo-cfg/config.toml
         if [ -e .cargo ] || [ -L .cargo ]; then mv .cargo .cargo-original; fi
         ln -s ci/cargo-cfg .cargo'
# The same construct anywhere else the guard walks. A symlink is a refusal to look, and
# the three walks used to disagree about that: the member-source one failed closed while
# the two that mattered more skipped silently. One helper now, one policy.
# NOTE the link target has to RESOLVE. A dangling symlink is not a directory, so
# `os.walk` files it under `files` and the check never sees it — my first draft of this
# case pointed at a path that did not exist and went green for that reason, which is
# the same "not looking" mistake one level down. `--because` caught it.
run_case "8e a ] inside a cfg_attr made it INVISIBLE" fail \
  --because "pull source in by path" -- \
  sh -c 'printf "macro_rules! p { () => { 1 }; }\n" > probe.rs
         printf "\n#[cfg_attr(all(), doc = \"]\", path = \"../../../probe.rs\")]\nmod probe;\n" >> crates/clw-types/src/lib.rs'
# The extent used to be bounded by `[^\]]`, so ANY cfg_attr containing a `]` produced
# no match at all — invisibility, not a ledger miss. Review walked the round-7 path
# bypass straight back in with this exact shape, guard green and clippy green. The
# corpus ALREADY held the shape at case 5d, where the hash pin catches it — so the
# lesson was in the suite and did not transfer to the regex. It transfers now.

run_case "8f a case-variant .cargo/config.TOML is refused" fail \
  --because "case-variant of \`.cargo/config.toml\`" -- \
  sh -c 'mkdir -p .cargo
         printf "[build]\nrustflags = [\"--cap-lints\", \"allow\"]\n" > .cargo/config.TOML'
# The directory basename was folded in round 10 and the FILENAME was left as a
# hand-typed tuple of four spellings — the same closed-set error one path component to
# the right. Positive control on a case-folding box: `[build] rustflags` in
# `.cargo/config.TOML` reached rustc.

run_case "8d a CASE-VARIANT .Cargo directory hides the config" fail \
  --because "case-variant of \`.cargo\`" -- \
  sh -c 'if [ -e .cargo ] || [ -L .cargo ]; then
           mv .cargo .cargo-case-tmp
           mv .cargo-case-tmp .Cargo
         else
           mkdir -p .Cargo
         fi
         printf "[build]\nrustflags = [\"--cap-lints\", \"allow\"]\n" > .Cargo/config.toml'
# macOS and Windows fold case, so `.Cargo/` IS `.cargo/` to cargo there and was invisible
# to an `== ".cargo"` test. Reproduced in review on this box with executing `unsafe`
# compiling clean under it. The Linux CI runner is case-sensitive, where the same file is
# inert — so the hole was real on the machines that BUILD RELEASES and absent on the one
# that gates PRs, which is the worst possible split and the reason this case exists.
# The guard REFUSES the variant rather than reading it: on a case-sensitive filesystem
# `.Cargo/` is a different directory cargo ignores, so one commit would mean two things
# depending on the machine.

run_case "8b a symlinked directory inside a workspace member" fail \
  --because "symlinked directory in the tree" -- \
  sh -c 'mkdir -p outside/src && printf "pub fn p() {}\n" > outside/src/lib.rs
         ln -s ../../../outside crates/clw-types/src/vendored'
# THE OTHER HALF: fail-closed must not mean fail-on-everything, or it gets deleted the
# first time it inconveniences someone. `target/` is pruned before the symlink check
# precisely because pointing it at a shared build cache is normal and nothing here reads
# it. This asserts the exemption still works — the only `ok` case besides the baseline.
run_case "8c a symlinked target/ is exempt, not a failure" ok \
  sh -c 'mkdir -p ../shared-build-cache && ln -s ../shared-build-cache target'

echo "the audited crate itself"
run_case "6.1 second permission attribute inside clw-win-acl" fail \
  --because "no longer matches" -- \
  sh -c 'printf "\n#[allow(unsafe_code)]\nfn _x() {}\n" >> '"$ACL"
run_case "6.2 the audited allow moved to another crate" fail \
  --because "no longer matches" -- \
  sh -c 'perl -0pi -e "s/    #\[allow\(unsafe_code\)\]\n//" '"$ACL"'
         printf "\n#[allow(unsafe_code)]\nfn _x() {}\n" >> '"$CLI"

wait
echo
for i in $(seq 1 "$seq_no"); do
  if [ ! -f "$TMPROOT/res/$i" ]; then
    printf 'FAIL\t  NO RESULT for case %d — the worker died\n' "$i" > "$TMPROOT/res/$i"
  fi
  IFS=$'\t' read -r verdict line < "$TMPROOT/res/$i"
  printf '%s\n' "$line"
  if [ "$verdict" = PASS ]; then pass=$((pass+1)); else fail=$((fail+1)); fi
done

echo
printf '%d passed, %d failed (%d workers)\n' "$pass" "$fail" "$JOBS"
[ "$fail" -eq 0 ] || { echo "SELF-TEST FAILED: a known bypass is reachable again." >&2; exit 1; }
echo "unsafe-posture self-test: all ${pass} cases behave as recorded."
