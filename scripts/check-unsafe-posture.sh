#!/usr/bin/env bash
# check-unsafe-posture.sh — protect the CONFIGURATION that makes the compiler enforce
# "no `unsafe` outside one audited crate", and PIN that crate's source.
#
# WHY THIS SCRIPT DOES NOT MODEL RUST'S GRAMMAR
# ---------------------------------------------
# Five generations of this guard tried to police Rust source with regular
# expressions, and cold review broke all five. The last one stripped comment spans
# before matching, on the argument that over-stripping was sound. It is not: a string
# literal `"/*"` … `"*/"` in ordinary source opens a comment span across everything
# between, so the guard blanked ACTIVE CODE and reported clean while `unsafe` ran in
# `fn main`. That was fail-OPEN in the exact direction the header swore was
# impossible. Earlier rounds died to `#[expect]`, to `#[cfg_attr(all(), allow(...))]`,
# to a `]` inside a string truncating the attribute match, to `unsafe` split across a
# newline, and to a lint level sitting in a comment.
#
# The lesson took too long: a checker that must model Rust's grammar is writing a
# lexer badly, and the adversary only needs one spelling it did not think of.
#
# So the two questions are asked of the two things that already answer them exactly:
#
#   THE COMPILER, for `unsafe` and for permission attributes. Every member except one
#   inherits `[workspace.lints.rust] unsafe_code = "forbid"`. `forbid` cannot be
#   locally re-allowed — `#[allow(unsafe_code)]`, `#[expect(unsafe_code)]` and
#   `#[cfg_attr(all(), allow(unsafe_code))]` are all `error[E0453]`, verified. There
#   is nothing for a regex to do in those crates.
#
#   A CONTENT HASH, for the one crate that must sit at `deny` so the audited Windows
#   DACL block can be allowed. Its whole source tree is pinned byte-for-byte against
#   `.github/win-acl-sources.expected`. A hash cannot be defeated by a spelling.
#
# WHAT IS LEFT FOR THIS SCRIPT
# ----------------------------
# 1. The configuration, parsed with `tomllib` (never grepped — a lint level in a
#    COMMENT satisfied a grep once): the root level really is `forbid`; every member
#    from `cargo metadata` either inherits it or is the one audited `deny`; a failed
#    enumeration is a hard failure, not zero iterations and a clean report.
# 2. That no crate is detached from the workspace. `workspace.exclude`, or a sibling
#    carrying its own `[workspace]` table, keeps a path dependency linked into the
#    binary while removing it from `cargo metadata` — so the lint stops applying and
#    nothing notices.
# 3. The hash pin on the `deny` crate.
# 4. That no in-repo cargo config sets ANY key that is not on a written allowlist
#    (empty today). A `.cargo/config.toml` decides both what runs as the compiler and
#    what is handed to it: `build.rustflags = ["--cap-lints", "allow"]` demotes every
#    lint in every workspace crate to `allow`, `forbid` included, and `rustc-wrapper` /
#    `rustc-workspace-wrapper` / `rustc` reach the same end through a shim script. Both
#    verified, each turning the compiler oracle off with fmt, host clippy, the cross
#    lane and this guard all still clean. It is an ALLOWLIST because the first version
#    named three dangerous keys and review used a fourth; enumerating dangerous keys is
#    the same losing game as enumerating dangerous flag values. Parsed with `tomllib`,
#    not grepped, and EVERY `.cargo/` directory in the repo, not just the root one —
#    cargo discovers config by walking cwd upward, so a config in a member directory is
#    live for anything that runs cargo there. `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS` and
#    `RUSTC_WRAPPER` in the AMBIENT ENVIRONMENT do the same and are NOT closable from
#    inside the repo — see the not-covered list.
# 5. That no member manifest points a compiled target (`[lib]`, `[[bin]]`, `[[test]]`,
#    `[[bench]]`, `[[example]]`) at a `path` outside its own member directory — which
#    would compile a file scan (6) never walks and the hash pin never reaches.
# 6. Four source scans — THREE monotone, one a form ledger — no comment stripping, on
#    every file under a member directory REGARDLESS OF EXTENSION. The extension does
#    not decide what rustc compiles: a `[lib] path = "src/lib.inc"` (or an `include!`,
#    or a `#[path]`) makes any filename a crate root, verified. So the walk takes every
#    file, and the only exclusion is `target/`.
#      * `macro_rules!` — `unsafe_code` is SUPPRESSED in external macro expansions, so
#        a macro body containing `unsafe` compiles clean under `forbid` at BOTH the
#        definition and the expansion site. The compiler cannot be made to see this.
#        The workspace defines none, so this asserts zero, with a written allowlist.
#      * `include!` and `#[path` — either can pull first-party source from outside a
#        member directory, where scan (6) and the hash pin (3) do not reach.
#      * `cfg_attr` — the one attribute that CONSTRUCTS another attribute, so
#        `#[cfg_attr(all(), path = "../../../shared/probe.rs")]` is a `#[path]` that no
#        `#[path` pattern matches (verified: it got a macro body carrying `unsafe` into
#        `fn main` with this guard reporting clean). Deciding by regex which `cfg_attr`
#        expands to what is the lexer game that failed five times — so this compares
#        the whole FORM against a ledger instead, taking the extent by counting
#        balanced parens. This scan is NOT assert-zero and NOT monotone: the workspace
#        uses four (`clw-hydrate`'s `cfg_attr(not(windows), allow(dead_code))`), a
#        blanket refusal false-positived on them, and a guard that fires on correct
#        code is a guard someone deletes. `cfg_attr`-wrapped permission attributes are
#        `error[E0453]` anyway; the ledger exists for the PATH half.
#    The patterns tolerate whitespace where Rust does (`macro_rules \n ! m` is a legal
#    definition and defeated `\bmacro_rules!`, verified).
#    Monotone — true of the first THREE — means they can only OVER-report: a hit inside
#    a comment is a false failure, never a missed one. That is the property an earlier
#    version claimed and did not have, and it is why the crate docs of clw-win-acl no
#    longer NAME an attribute spelling. The `cfg_attr` scan over-reports too (a
#    commented-out one is a hit, verified) but is not monotone in the same sense,
#    because an off-ledger form is judged by its text rather than by its presence.
# 7. That no `build.rs` exists. A build script runs arbitrary code at compile time and
#    can generate the source of the crate it belongs to; the workspace has none, so
#    this is an assert-zero with a written allowlist, the same shape as (6) — rather
#    than a point-in-time observation in a comment, which is what it was.
#
# WHAT IS STILL NOT COVERED — SECURITY.md carries the same list
# -------------------------------------------------------------
#   * PLATFORM-CONDITIONAL CODE. rustc strips inactive `cfg` before linting, so
#     `#[cfg(target_os = "macos")] { unsafe { … } }` in a `forbid` crate is invisible
#     to every lane that does not compile for that target. The gate lints the CI host
#     (Linux) and cross-lints windows-gnu; three of the five targets release.yml ships
#     — both macOS targets and aarch64-unknown-linux-gnu — are linted by no lane. This
#     is the widest remaining hole and it is a real one.
#   * LINT CAPPING, AND COMPILER REPLACEMENT, FROM THE ENVIRONMENT. Check (4) closes
#     every in-repo cargo config, but the same effects arrive from outside the repo and
#     nothing in a repo can assert about its own environment. `RUSTFLAGS` /
#     `CARGO_ENCODED_RUSTFLAGS` carry `--cap-lints allow` directly. `RUSTC_WRAPPER`,
#     `RUSTC_WORKSPACE_WRAPPER` and `RUSTC` REPLACE THE COMPILER with a script that can
#     add anything it likes to any crate it chooses — the environment twin of the keys
#     check (4) refuses. A `~/.cargo/config.toml` on the runner does either, and so does
#     a `.cargo/config.toml` in any ANCESTOR directory of the checkout — cargo walks CWD
#     upward to the filesystem root before it ever consults `$CARGO_HOME`, which is
#     itself relocatable by the `CARGO_HOME` variable. This is a property of the CI
#     definition, not of these scripts.
#   * `rust-toolchain.toml`. It is in-repo and committable, and it selects WHICH rustc
#     binary runs — a `channel` naming a `rustup toolchain link`-ed path points the
#     whole build at an arbitrary compiler. That is the same class as the cargo-config
#     keys this guard refuses, and it is NOT checked here. It is called out because the
#     rationale for checking cargo config is "a config decides what runs as the
#     compiler", and that rationale applies to this file verbatim.
#   * `unsafe` added INSIDE the audited block. The hash pin makes it a fixture diff,
#     which is the point — but reading that diff is a human's job.
#   * Third-party dependencies, `[patch]`-substituted paths, proc-macro expansions, and
#     `fuzz/` (a detached workspace by design).
#
# Regression corpus: scripts/selftest-unsafe-posture.sh

set -euo pipefail
cd "$(dirname "$0")/.."

command -v python3 >/dev/null 2>&1 || {
  echo "GATE FAILED: python3 is required by the unsafe-posture guard." >&2; exit 1; }

# tomllib is 3.11+. Without this the script dies on a ModuleNotFoundError traceback
# instead of the deliberate message above — and the self-hosted CI Mac is exactly
# where a system 3.9 shows up.
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' || {
  echo "GATE FAILED: the unsafe-posture guard needs python3 >= 3.11 (it parses" >&2
  echo "  Cargo.toml with tomllib). Found: $(python3 -V 2>&1)" >&2; exit 1; }

python3 - <<'PY'
import hashlib, json, os, re, subprocess, sys, tomllib

# The ONE member permitted to sit at `unsafe_code = "deny"`. Its whole source tree is
# hash-pinned; no attribute matching is done anywhere, by anyone.
ACL_CRATE = 'crates/clw-win-acl'
ACL_PIN = '.github/win-acl-sources.expected'

# Written allowlists. Both are deliberately empty today, and both exist so that a
# legitimate need is a REVIEWED EDIT HERE rather than a reason to delete the check —
# which is how guards die.
MACROS_OK = {}   # 'path:line' -> why this macro_rules! cannot carry unsafe into a binary
INCLUDES_OK = {} # 'path:line' -> why this include!/#[path] stays inside its member

# Permitted `cfg_attr` FORMS, keyed by the normalised attribute text rather than by
# `file:line`, because a line-keyed entry rots the moment the code moves and the next
# author deletes the check instead of the entry.
#
# `cfg_attr` is refused in general because it CONSTRUCTS another attribute, so
# `#[cfg_attr(all(), path = "../../shared/probe.rs")]` is a `#[path]` that no `#[path`
# pattern matches — verified in review, it carried a macro body with `unsafe` into
# `fn main` with this guard clean. Deciding by regex which `cfg_attr` expands to what is
# the lexer game that failed five times. So: assert a closed set of forms instead.
#
# Adding a form here is a REVIEWED act. The bar is that the attribute it constructs can
# neither pull in source nor grant permission for `unsafe` — and note that the second
# half is already enforced by the compiler (`error[E0453]`) in every crate but the one
# audited `deny`, so the load-bearing half is the first.
CFG_ATTR_OK = {
    # Constructs `allow(dead_code)`. Cannot name a path, cannot grant unsafe
    # permission. Used by clw-hydrate for helpers whose only callers are cfg(windows),
    # which is the shape WP-16 itself asked for elsewhere in this workspace.
    'cfg_attr(not(windows), allow(dead_code))',
}
BUILD_RS_OK = {} # 'path'      -> why this build script cannot generate unsafe source
# Dotted key paths any in-repo `.cargo/config.toml` is permitted to set, e.g.
# 'build.jobs'. EMPTY: this repo needs no cargo config, and a cargo config decides what
# runs as the compiler. Adding a key here means having read the value AND, if it names a
# program (`rustc`, `rustc-wrapper`, `rustc-workspace-wrapper`, a `linker`), that program.
CARGO_CONFIG_OK = {}

def fail(msg, *extra):
    sys.stderr.write("GATE FAILED: %s\n" % msg)
    for e in extra:
        sys.stderr.write("  %s\n" % e)
    sys.exit(1)

# --- the one directory walk, and it FAILS CLOSED on a symlink --------------------
# There were three walks here and they disagreed about symlinked directories. The
# member-source walk failed on them, with a comment saying their contents "would be
# silently unscanned". The other two SKIPPED them — filtering `dirs[:]` on `islink`
# before anything could match — which is the same sentence describing a bug instead of
# a check. Review turned the newest and most security-critical of the three, the
# cargo-config walk, straight into a bypass:
#
#     ln -s ci/cargo-cfg .cargo        # committable, diffable, reads as "share the CI config"
#
# The symlink is dropped from `dirs`, so `basename(dirpath) == '.cargo'` never matches,
# so the config it points at is never in a directory this walk calls `.cargo` — while
# cargo resolves it through the OS and reads it normally. Measured: guard rc=0 printing
# "0 in-repo .cargo/config file(s), 0 key(s) off the allowlist", fmt clean, clippy
# `-D warnings` clean on the whole workspace, and the 39-case suite entirely green,
# with a `rustc-wrapper` shim capping the lint for clw-cache and `unsafe` compiled into
# the shipped binary.
#
# Skipping a symlinked directory is never sound in a check that is trying to prove an
# ABSENCE: not looking somewhere is indistinguishable from finding nothing there, which
# is precisely the property this whole file exists to stop claiming. So there is one
# walk now and it refuses to continue.
PRUNE_ALL = ('target', '.git', 'node_modules')

def walk_or_fail(top, prune, what):
    """os.walk, yielding (dirpath, files), that FAILS on any symlinked directory it
    would otherwise have to skip. Pruned names are skipped before the check — those
    are directories nothing here reads, and `target/` in particular is routinely a
    link to a shared build cache."""
    if os.path.islink(top):
        fail('the %s walk was handed a symlinked root: %s' % (what, top),
             'Its contents would not be walked, and a check that does not look cannot',
             'report clean.')
    for dirpath, dirs, files in os.walk(top, followlinks=False):
        keep = []
        for d in dirs:
            if d in prune:
                continue
            full = os.path.join(dirpath, d)
            if os.path.islink(full):
                fail('symlinked directory in the tree: %s' % os.path.relpath(full),
                     '-> %s' % os.readlink(full),
                     'This walk (%s) does not follow symlinks, so everything under it' % what,
                     'would be silently unexamined while the tools that DO resolve it —',
                     'cargo and rustc — read it normally. Review got a capping',
                     '`.cargo/config.toml` past this guard with exactly one such link.',
                     'Replace it with a real directory.')
            keep.append(d)
        dirs[:] = keep
        yield dirpath, files

# --- members, from cargo (never from a sed range over the manifest) -------------
# An earlier version parsed `members = [` with sed, which slurped unrelated quoted
# strings, and looped over the result WITHOUT checking that the enumeration
# succeeded — so a failing `cargo metadata` produced zero iterations and a clean
# report: fail-open on the one check standing between `forbid` and nothing.
try:
    proc = subprocess.run(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                          capture_output=True, text=True, timeout=120)
except subprocess.TimeoutExpired:
    fail("`cargo metadata` timed out after 120 seconds — the workspace member list is "
         "unknown, so this guard refuses to report clean.")
except Exception as e:
    fail("could not run `cargo metadata` to enumerate workspace members: %s" % e)
if proc.returncode != 0:
    fail("`cargo metadata` failed (exit %d) — the workspace member list is unknown, so "
         "this guard cannot verify the unsafe posture and refuses to report clean."
         % proc.returncode, (proc.stderr or '').strip()[:400])
try:
    md = json.loads(proc.stdout)
except json.JSONDecodeError as e:
    fail("`cargo metadata` output is not JSON: %s" % e)

manifests = sorted({os.path.abspath(p['manifest_path']) for p in md.get('packages', [])})
if len(manifests) < 2:
    fail("`cargo metadata` reported %d workspace member(s) — implausible; refusing to "
         "report clean on an enumeration that is probably broken." % len(manifests))

root = os.path.abspath('Cargo.toml')
ACL_MANIFEST = os.path.abspath(os.path.join(ACL_CRATE, 'Cargo.toml'))

def load(path):
    try:
        with open(path, 'rb') as fh:
            return tomllib.load(fh)
    except Exception as e:
        fail("cannot parse %s as TOML: %s" % (path, e))

# --- the workspace lint level, as a PARSED VALUE --------------------------------
rootdoc = load(root)
ws = rootdoc.get('workspace', {})
level = (ws.get('lints', {}).get('rust', {}) or {}).get('unsafe_code')
if level != 'forbid':
    fail('root Cargo.toml no longer sets [workspace.lints.rust] unsafe_code = "forbid" '
         '(found: %r).' % level,
         'Without it the compiler stops rejecting `unsafe`, and every other check here',
         'is checking the configuration of a lint that does nothing.')

# --- nothing may CAP the lint out of existence -----------------------------------
# `[build] rustflags = ["--cap-lints", "allow"]` in a repo-level cargo config demotes
# EVERY lint in EVERY workspace crate to `allow`, `forbid` included. Review proved it:
# with those two lines and a plain `unsafe { … }` on the first line of `fn main`, this
# guard exited 0, `cargo fmt --check` was clean, and BOTH clippy lanes — host and
# windows-gnu, each at `-D warnings` — were clean, and the binary ran the `unsafe`. No
# macros, no includes. It is the cheapest bypass in the whole corpus and it lives in a
# file reviewers read as linker configuration.
#
# AN ALLOWLIST, NOT A DENYLIST, AND THAT DISTINCTION IS THE WHOLE CHECK.
#
# The first version of this asserted three keys absent: `build.rustflags`,
# `build.rustdocflags`, `target.*.rustflags`. Review walked past it in one line:
#
#     [build]
#     rustc-wrapper = "scripts/rustc-shim.sh"      # reads as ordinary build tuning
#
# with a shim that appends `--cap-lints allow` when it sees the crate it wants. Guard
# rc=0, printing "uncapped by any repo cargo config"; fmt clean; both clippy lanes
# clean; the self-test 30/30 — the shim was scoped to ONE crate so the real-tree
# sentinel, which compiles a different one, still failed to compile and still passed.
# `rustc-workspace-wrapper` is better for an attacker still (workspace members only),
# and `build.rustc` replaces the compiler outright.
#
# The header below argues that sorting harmless flag VALUES from harmful ones is the
# same losing game as parsing Rust with regexes. That is right, and it applies one
# level up: enumerating dangerous KEYS is the same game again, and review is faster at
# finding a key than I am at listing them. The question with a closed answer is not
# "which keys are dangerous" — it is "which keys is this repository allowed to set at
# all", and today that answer is NONE.
#
# So: every table/key any repo cargo config sets must appear in CARGO_CONFIG_OK, which
# is empty. A legitimate need becomes a reviewed edit there, the same ledger shape as
# MACROS_OK / INCLUDES_OK / BUILD_RS_OK. New cargo keys, and keys nobody here has
# thought of, are closed by default rather than pending the next round of review.
#
# Parsed, not grepped, for the reason every other check here is parsed: `# rustflags =`
# in a comment is not a setting, and a setting spelled across a line continuation is.
#
# EVERY directory, not just the root: cargo discovers config by walking CWD upward, so
# `crates/clw-cli/.cargo/config.toml` is inert from the repo root and fully live the
# moment anything runs cargo in that directory — a `cd` in a Makefile, a script, a
# developer. Nothing in this repo does that today; the check does not depend on that
# staying true.
def flatten_cfg(doc, prefix=()):
    """Every leaf setting as a dotted key path. A table with no leaves under it sets
    nothing and is not reported."""
    for k, v in doc.items():
        if isinstance(v, dict):
            yield from flatten_cfg(v, prefix + (k,))
        else:
            yield '.'.join(prefix + (k,))

cargo_cfgs = []
cfg_dirs_walked = 0
for dirpath, files in walk_or_fail('.', PRUNE_ALL, 'cargo-config'):
    cfg_dirs_walked += 1
    base = os.path.basename(dirpath)
    # Case-INSENSITIVE, and a case-variant is a hard failure rather than a silent
    # accept. macOS and Windows filesystems fold case, so a committed `.Cargo/` is
    # invisible to an `== '.cargo'` test and fully live to cargo — reproduced in
    # review, with `[build] rustflags = ["--cap-lints","allow"]` inside it and
    # executing `unsafe` compiling clean. This box and the self-hosted macOS release
    # runners are both case-insensitive; the Linux CI runner is not, which is why
    # this was a real hole on the machines that build releases and inert on the one
    # that gates PRs.
    #
    # Refusing the variant rather than reading it is deliberate: on a case-sensitive
    # filesystem `.Cargo/` is a DIFFERENT directory that cargo ignores, so a repo
    # containing one means different things on different machines. That is not a
    # state to normalise silently.
    if base.lower() == '.cargo':
        if base != '.cargo':
            fail('a case-variant of `.cargo` exists in the tree: %s' % os.path.relpath(dirpath),
                 'On a case-folding filesystem (macOS, Windows) cargo reads it as `.cargo`;',
                 'on a case-sensitive one it is ignored. The same commit would then mean two',
                 'different things depending on the machine. Rename it to `.cargo`.')
        # Fold the FILENAME too, and refuse a non-canonical spelling — the same rule
        # as the directory, for the same reason. Enumerating four spellings of a
        # case-folded name was the closed-set error this guard's own header warns
        # about, moved one path component to the right: `config.TOML` is READ BY CARGO
        # on a case-folding filesystem (positive control run: `[build] rustflags` in
        # `.cargo/config.TOML` reached rustc) and was invisible here.
        for fn in files:
            low = fn.lower()
            if low in ('config.toml', 'config'):
                if fn != low:
                    fail('a case-variant of `.cargo/%s` exists: %s'
                         % (low, os.path.relpath(os.path.join(dirpath, fn))),
                         'Cargo reads it on a case-folding filesystem and ignores it on a',
                         'case-sensitive one, so the same commit would mean two different',
                         'things depending on the machine. Rename it to `%s`.' % low)
                cargo_cfgs.append(os.path.join(dirpath, fn))

for cfg_name in sorted(cargo_cfgs):
    rel = os.path.relpath(cfg_name)
    try:
        with open(cfg_name, 'rb') as fh:
            cargo_cfg = tomllib.load(fh)
    except Exception as e:
        # Fail CLOSED. An unparseable cargo config is a config whose effect on the lint
        # level is unknown, and "unknown" is not "clean".
        fail('%s exists but cannot be parsed as TOML: %s' % (rel, e),
             'This file can demote every lint in the workspace to `allow` — directly via',
             '`--cap-lints` in rustflags, or by replacing rustc itself through a wrapper.',
             'Its contents must be readable for this guard to report clean.')
    hits = sorted(k for k in flatten_cfg(cargo_cfg) if k not in CARGO_CONFIG_OK)
    if hits:
        fail('a repo cargo config sets keys that are not on the allowlist:',
             *('%s sets %s' % (rel, k) for k in hits),
             'A cargo config decides what actually runs as the compiler and what is',
             'passed to it. `rustflags` can carry `--cap-lints allow`; `rustc-wrapper`,',
             '`rustc-workspace-wrapper` and `rustc` can add it, or anything else, from a',
             'shim script — both verified in review to demote `unsafe_code = "forbid"`',
             'to `allow` with fmt, the guard and both clippy lanes still clean.',
             'This is an ALLOWLIST on purpose. An earlier version named the dangerous',
             'keys and review simply used one it had not named; enumerating dangerous',
             'keys is the same losing game as enumerating dangerous flag values, or',
             'parsing Rust with regexes. CARGO_CONFIG_OK is empty because this repo',
             'needs no cargo config at all. If one is genuinely needed, add the exact',
             'keys there with a written reason — and read the wrapper script it names.')

# --- blind spot 3: workspace.exclude ---------------------------------------------
# An excluded in-repo crate stays a path dependency that links into the binary, but
# leaves `cargo metadata --no-deps`, so BOTH the compiler lint and every check below
# stop applying to it. Review demonstrated this with a `vendor/evil` path dep.
if ws.get('exclude'):
    fail('root Cargo.toml declares workspace.exclude = %r.' % (ws['exclude'],),
         'An excluded crate can still be a path dependency linked into the shipped',
         'binary while being invisible to `cargo metadata` — so neither the workspace',
         'lint nor this guard applies to it. If an exclusion is genuinely needed, it',
         'must be reviewed here and this assertion updated deliberately.')

# --- blind spot 3 (continued): every in-repo crate is a member --------------------
# `exclude` is not the only way to detach a crate: a sibling directory carrying its
# own `[workspace]` table is simply not a member of this one. Enumerate the manifests
# that exist on disk and require the set to match.
DETACHED_OK = {os.path.abspath('fuzz/Cargo.toml')}  # deliberate; see gates.sh
on_disk = set()
for dirpath, files in walk_or_fail('.', PRUNE_ALL, 'stray-manifest'):
    if 'Cargo.toml' in files:
        on_disk.add(os.path.abspath(os.path.join(dirpath, 'Cargo.toml')))
on_disk.discard(root)
stray = sorted(on_disk - set(manifests) - DETACHED_OK)
if stray:
    fail('in-repo crate(s) exist that are NOT workspace members:', *stray,
         'A non-member can be a path dependency linked into the binary while the',
         'workspace `unsafe_code = "forbid"` does not apply to it and this guard does',
         'not see it. Add it to [workspace] members, or if it is deliberately detached,',
         'add it to DETACHED_OK here with a written reason.')

# --- per-member opt-in, as a PARSED VALUE ---------------------------------------
problems, exceptions = [], []
for m in manifests:
    doc = load(m)
    lints = doc.get('lints', {}) or {}
    rel = os.path.relpath(m)
    if lints.get('workspace') is True:
        local = (lints.get('rust', {}) or {}).get('unsafe_code')
        if local is not None:
            problems.append('%s inherits the workspace lints AND overrides unsafe_code = %r'
                            % (rel, local))
        continue
    local = (lints.get('rust', {}) or {}).get('unsafe_code')
    if local == 'deny':
        exceptions.append((rel, m))
        continue
    problems.append('%s does not opt into the workspace lints (lints.workspace is %r) and '
                    'does not carry the audited unsafe_code = "deny" override (found %r) — '
                    'the lint does not apply to it and cargo reports nothing'
                    % (rel, lints.get('workspace'), local))

if problems:
    fail("workspace member(s) are outside the unsafe_code lint:", *problems)

# Exactly one crate may sit at `deny` instead of `forbid`, and it must be clw-win-acl.
# `deny` exists only because cargo rejects mixing `lints.workspace = true` with a local
# override, and `forbid` by definition cannot be re-allowed.
if len(exceptions) != 1 or exceptions[0][1] != ACL_MANIFEST:
    fail('exactly one member may use unsafe_code = "deny" (%s, which holds the audited '
         'Windows-DACL block). Found: %s'
         % (ACL_CRATE, ', '.join(r for r, _ in exceptions) or 'none'))

# --- source scan over MEMBER directories, not a hardcoded `crates/` --------------
# EVERY file, not just `*.rs`. The extension does not decide what rustc compiles: a
# `[lib] path = "src/lib.inc"` makes `lib.inc` the crate root (verified — it builds
# clean), and so do `include!` and `#[path]`. An extension filter here meant the guard
# never opened the one file that mattered while its own header claimed otherwise.
member_dirs = [os.path.dirname(m) for m in manifests]
sources = []
for d in member_dirs:
    for dirpath, files in walk_or_fail(d, ('target',), 'member-source'):
        for fn in files:
            sources.append(os.path.join(dirpath, fn))
scanned = {os.path.abspath(p) for p in sources}

# --- no compiled target may point outside its member directory --------------------
# Cargo lets any target name its own root: `[lib] path`, `[[bin]] path`, and the same
# key on `[[test]]`, `[[bench]]` and `[[example]]`. A path escaping the member dir
# compiles a file the walk above never reaches and the hash pin never covers — the
# manifest equivalent of `#[path]`, and invisible to a scan of source files because it
# lives in TOML. Parsed, like every other manifest question here.
target_escapes = []
for m in manifests:
    doc = load(m)
    mdir = os.path.dirname(m)
    entries = []
    lib = doc.get('lib')
    if isinstance(lib, dict):
        entries.append(('lib', lib))
    for kind in ('bin', 'test', 'bench', 'example'):
        for t in (doc.get(kind) or []):
            if isinstance(t, dict):
                entries.append((kind, t))
    for kind, t in entries:
        p = t.get('path')
        if p is None:
            continue
        resolved = os.path.abspath(os.path.join(mdir, p))
        if resolved not in scanned:
            target_escapes.append('%s: [%s] path = %r -> %s'
                                  % (os.path.relpath(m), kind, p,
                                     os.path.relpath(resolved)))
if target_escapes:
    fail('workspace member(s) compile a target from outside their own directory:',
         *target_escapes,
         'That file is compiled INTO the member — so the workspace lint applies to it',
         'only as far as the compiler reaches, while this guard never opens it and the',
         'hash pin never covers it. Keep every target root inside its member directory.')

# --- no build.rs ------------------------------------------------------------------
# A build script runs arbitrary code at compile time and can emit the source of the
# crate it belongs to, so nothing downstream of it is what the repository says it is.
# The workspace has none. This used to be a sentence in the boundary list saying so —
# a point-in-time observation with no check behind it, while the macro and include
# scans mechanized the identical "none today" as an assert-zero. Now it is one too.
build_scripts = {os.path.relpath(p) for p in sources
                 if os.path.basename(p) == 'build.rs'}
# `[package] build = "gen.rs"` names a build script under any filename at all, so the
# convention-based check above is not sufficient on its own.
for m in manifests:
    b = (load(m).get('package', {}) or {}).get('build')
    mdir = os.path.dirname(m)
    if b is True:
        build_scripts.add(os.path.relpath(os.path.join(mdir, 'build.rs')))
    elif isinstance(b, str):
        build_scripts.add(os.path.relpath(os.path.join(mdir, b)))
    elif isinstance(b, list):
        # Nightly `build = ["a.rs", "b.rs"]`. Previously fell through every branch and
        # was silently ignored — a manifest key that names build scripts, read by a
        # check whose whole job is that no build script exists.
        for one in b:
            if isinstance(one, str):
                build_scripts.add(os.path.relpath(os.path.join(mdir, one)))
    elif b is not None and b is not False:
        fail('%s sets [package] build = %r, which this guard does not understand.'
             % (os.path.relpath(m), b),
             'A build script runs arbitrary code at compile time. An unrecognised value',
             'here is not a reason to report clean.')
build_scripts = sorted(p for p in build_scripts if p not in BUILD_RS_OK)
if build_scripts:
    fail('workspace member(s) carry a build script:', *build_scripts,
         'A build script executes at compile time and can GENERATE the source that is',
         'then compiled, so the `unsafe` posture of the generated code is decided by',
         'code no scan here reads. The workspace has had none. If one is genuinely',
         'needed, add it to BUILD_RS_OK in this script with a written reason, and say',
         'so in SECURITY.md.')

def read(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except OSError:
        return ''

# --- the hash pin on the one `deny` crate ----------------------------------------
# This replaces five generations of attribute matching. In every other member the
# compiler rejects a permission attribute outright (E0453, verified for `allow`,
# `expect` and `cfg_attr`), so there is nothing to scan there. Here — the one place an
# attribute works — the question is not "is there a second one" but "did anything in
# this crate change without a human reading it", and a content hash answers that
# exactly, with no grammar to model.
acl_files = []
for dirpath, files in walk_or_fail(ACL_CRATE, ('target',), 'audited-crate-pin'):
    for fn in files:
        acl_files.append(os.path.join(dirpath, fn))
if not acl_files:
    fail('%s contains no files — the audited crate is missing.' % ACL_CRATE)

actual_pin = []
for p in sorted(acl_files):
    with open(p, 'rb') as fh:
        blob = fh.read()
    # git's blob hash, so the fixture can be regenerated with `git hash-object` and
    # checked by hand without this script.
    h = hashlib.sha1(b'blob %d\0' % len(blob) + blob).hexdigest()
    actual_pin.append('%s %s' % (h, os.path.relpath(p)))
actual_pin = '\n'.join(actual_pin) + '\n'

if not os.path.isfile(ACL_PIN):
    fail('%s is missing — the audited crate is unpinned.' % ACL_PIN,
         'Regenerate it with:',
         "  (cd %s && find . -type f | sort | xargs -I{} sh -c "
         "'printf \"%%s %s/%%s\\n\" \"$(git hash-object {})\" \"${0#./}\"' {}) > %s"
         % (ACL_CRATE, ACL_CRATE, ACL_PIN))
expected_pin = open(ACL_PIN, encoding='utf-8').read()
if actual_pin != expected_pin:
    fail('%s no longer matches %s.' % (ACL_CRATE, ACL_PIN),
         'This crate holds the workspace\'s only `unsafe`, and it sits at `deny` rather',
         'than `forbid` precisely so that block can be allowed — which means it is the',
         'one place an attribute can reintroduce `unsafe` with the compiler silent.',
         'Any change here must be read by a person. If the change is intended, update',
         'the fixture in the SAME commit so it appears in the diff.',
         '', '--- expected ---', *expected_pin.splitlines(),
         '--- actual ---', *actual_pin.splitlines())

# --- source scans: three monotone, one form ledger -------------------------------
# No comment stripping. The previous version stripped comment spans and argued that
# over-stripping was sound; a string literal "/*" … "*/" opened a span that swallowed
# a live `macro_rules!` carrying `unsafe`, and the guard reported clean while it ran
# in `fn main`. These scans can only OVER-report: a hit inside a comment is a false
# failure a human clears in one line, never a miss.
def scan(pattern, allowlist):
    out = []
    for p in sources:
        src = read(p)
        for mt in re.finditer(pattern, src):
            loc = '%s:%d' % (os.path.relpath(p), src[:mt.start()].count('\n') + 1)
            if loc not in allowlist:
                out.append(loc)
    return out

# `\s*` before the `!`, because `macro_rules` and `!` are separate tokens and rustc
# accepts a newline between them. `macro_rules\n! p { … }` defeated `\bmacro_rules!`
# with the guard exiting 0 — verified. (`cargo fmt --check` happens to normalise it,
# but nothing here may depend on that: fmt is a style gate, not part of this trust
# path, and `--check` failing is not what makes the posture hold.)
macros = scan(r'\bmacro_rules\s*!', MACROS_OK)
if macros:
    fail('workspace member(s) define macro_rules! macros:', *macros,
         '`unsafe_code` is SUPPRESSED inside external macro expansions, so a macro body',
         'containing `unsafe` compiles clean under `forbid` at BOTH the definition and',
         'the expansion site — verified in review. The compiler cannot be made to see',
         'it, and scanning macro bodies for `unsafe` is the grep-for-the-token game that',
         'failed three times. The workspace has defined none, so the posture held for',
         'free. If one is genuinely needed, add it to MACROS_OK in this script with a',
         'written reason, and say so in SECURITY.md. Note this scan does not read',
         'comments: a `macro_rules!` mentioned in prose trips it, which is the safe',
         'direction and is cleared the same way.')

# `cfg_attr` is in here, and it is not decoration. `#[cfg_attr(all(), path = "…")]` is
# a `#[path]` that the `#\[\s*path` pattern never sees — verified: it pulled a
# `macro_rules!` carrying `unsafe` in from OUTSIDE every member directory, with this
# guard printing "no macro_rules!, no include!/#[path]" and both clippy lanes clean.
# There is no honest regex for "which cfg_attr expands to a path attribute"; that is
# the lexer game that lost five times. So `cfg_attr` is judged by FORM against a
# ledger, NOT asserted absent: the workspace uses four
# (`clw-hydrate`'s `cfg_attr(not(windows), allow(dead_code))`), a blanket refusal
# false-positived on them, and a guard that fires on correct code is a guard someone
# deletes. The extent is taken by counting balanced parens — an earlier bound of
# `[^\]]` made any `cfg_attr` containing a `]` invisible and reopened the path bypass.
# The `cfg_attr`-wrapped PERMISSION attribute is `error[E0453]` anyway; the ledger
# exists for the PATH half.
escapes = scan(r'\binclude\s*!|#\s*\[\s*path\s*=', INCLUDES_OK)

# `cfg_attr` separately, because it is judged by FORM and the others by location.
def cfg_attr_extent(src, start):
    r"""Return the text of the cfg_attr(...) beginning at `start`, by BALANCED PARENS.

    The previous form was `cfg_attr\s*\([^\]]*?\)`, and `[^\]]` meant a cfg_attr
    containing a `]` ANYWHERE produced no match at all — not a ledger miss, an
    invisibility. Review walked the round-7 bypass straight back in with

        #[cfg_attr(all(), doc = "]", path = "../../../probe.rs")]

    which put executing `unsafe` into a workspace crate with this guard GREEN and
    `clippy -D warnings` green, while the stdout line said `0 hits for … cfg_attr`.
    The corpus already contained that exact shape (case 5d) — caught there by the
    hash pin, not by this scan — so the lesson was in the suite and did not transfer
    to the regex. Counting parens cannot be defeated by a bracket.

    Returns None if the parens never balance, which is treated as a hit: an
    unparseable attribute is not an absent one.
    """
    i = src.find('(', start)
    if i < 0:
        return None
    depth, j = 0, i
    while j < len(src):
        if src[j] == '(':
            depth += 1
        elif src[j] == ')':
            depth -= 1
            if depth == 0:
                return src[start:j + 1]
        j += 1
    return None

cfg_attr_seen = 0
for p in sources:
    src = read(p)
    for mt in re.finditer(r'\bcfg_attr\b', src):
        cfg_attr_seen += 1
        text = cfg_attr_extent(src, mt.start())
        line = src[:mt.start()].count('\n') + 1
        if text is None:
            escapes.append('%s:%d  cfg_attr with unbalanced parens' %
                           (os.path.relpath(p), line))
            continue
        form = ' '.join(text.split())
        if form not in CFG_ATTR_OK:
            escapes.append('%s:%d  %s' % (os.path.relpath(p), line, form))
if escapes:
    fail('workspace member(s) pull source in by path, or use cfg_attr:', *escapes,
         '`include!` and `#[path]` can bring first-party code in from OUTSIDE a member',
         'directory — where this scan does not walk and the hash pin does not reach —',
         'so `unsafe` arrives in a compiled crate with nothing having looked at the',
         'file. `cfg_attr` BUILDS attributes, so it is a `#[path]` (or an `include!`)',
         'that no pattern for those spells matches; review got exactly that past this',
         'guard. None of the three is used in this workspace today; if one is needed,',
         'add it to INCLUDES_OK with a reason establishing that its target stays inside',
         'the member. Note this scan does not read comments: any of these three',
         'mentioned in prose trips it, which is the safe direction.')

# WHAT WAS CHECKED, NEVER WHAT WAS CONCLUDED.
#
# This line used to end "...uncapped by any repo cargo config". In the state review
# built — a `.cargo/config.toml` naming a `rustc-wrapper` shim that appends
# `--cap-lints allow` — a repo cargo config WAS capping the lint, and this line said
# into the CI log that it was not. Six rounds of this change were spent on headers
# asserting properties the mechanism did not have; printing a conclusion instead of a
# result is that same defect relocated to stdout, and it is the one place every CI run
# looks. So each clause below names the check that ran, and a reader who wants to know
# whether that adds up to "safe" is pointed at the list that says what it misses.
#
# ROUND 9 MOVED THE SAME DEFECT ONE STEP FURTHER ALONG. "0 in-repo .cargo/config
# file(s), 0 key(s) off the allowlist" is a RESULT, not a conclusion, and it was still
# false while a symlinked `.cargo` capped the lint — because a directory the walk
# skipped deflates that count to zero exactly as if the repo were clean. A count of
# what was FOUND says nothing about whether the search could find it. So the
# cargo-config clause reports the SEARCH: how many directories were walked, and that
# zero symlinked directories were encountered (the walk fails outright on one, so any
# number but zero is unreachable here — printing it is what makes that visible).
print('unsafe-posture guard: checks passed (root Cargo.toml sets unsafe_code = '
      '"forbid"; %d member(s), of which %d declare lints.workspace = true and 1 is the '
      'audited deny; %d dir(s) walked (excluding target/, .git/, node_modules/, which are '
      'pruned BEFORE the symlink test and so are not counted here) with 0 symlinked dir(s) '
      'encountered, finding %d '
      '.cargo/config file(s) and 0 key(s) off the allowlist; %s matches its pin at %d '
      'file(s); %d file(s) scanned under member dirs, 0 unledgered hits for '
      'macro_rules!/include!/#[path]/cfg_attr (%d cfg_attr found, all on the permitted-'
      'form ledger); 0 build.rs; 0 target path outside its '
      'member; no workspace.exclude; no stray in-repo crate). NOT CHECKED HERE, by '
      'design: `unsafe` itself and permission attributes in inheriting members (the '
      'compiler rejects those, and only for targets a lane actually compiles — 3 of '
      'the 5 shipped targets are linted by no lane), and anything reaching rustc from '
      'the AMBIENT ENVIRONMENT (RUSTFLAGS, CARGO_ENCODED_RUSTFLAGS, ~/.cargo/config.toml, '
      'RUSTC_WRAPPER). See SECURITY.md.'
      % (len(manifests), len(manifests) - 1, cfg_dirs_walked, len(cargo_cfgs),
         ACL_CRATE, len(acl_files), len(sources), cfg_attr_seen))
PY
