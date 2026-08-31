#!/usr/bin/env bash
# check-release-notes.sh — keep the published release-notes body in a REVIEWED FILE.
#
# THE DEFECT THIS EXISTS FOR
# --------------------------
# Twenty published releases advertise "SLSA provenance". The attestation step in
# release.yml is gated to public repositories and this one is private, so it always
# skips; the claim is false, and it sent users to a `gh attestation verify` that
# cannot succeed. It survived review for months because the text lived in a shell
# assignment two-thirds of the way down a thousand-line workflow, where a reviewer
# reading a release-pipeline diff has no reason to look.
#
# THE CONTROL
# -----------
# The body is now `.github/release-notes.md`, a committed file, rendered by
# substituting `__VERSION__`. What users read is therefore a file a human reviews,
# instead of a string buried in a thousand-line workflow — which is the whole point.
# It is NOT the same as "what users read is what review saw": a step could rewrite
# that file on the runner before the render, and nothing here stops it (escape 4).
# This script does not judge the TEXT; it keeps the INDIRECTION in place, so the text
# cannot migrate back into the workflow.
#
# THE SCOPE, STATED EXACTLY — read this before citing the script as coverage
# --------------------------------------------------------------------------
# It reasons about `.github/workflows/release.yml` and nothing else, and within that
# file it covers exactly three things:
#
#   * every LITERAL `gh release create|edit` — it must take its body from the
#     rendered file and must not carry any of gh's other notes-bearing flags, which
#     are a documented finite set and are all named below;
#   * the rendered file — the variable `$NOTES_FILE` may be referenced only in its one
#     assignment, its one render and those invocations, and the rendered file's BASENAME
#     is reserved to the assignment line. Read that second half narrowly: it catches the
#     literal spelling of the path and nothing else (see the third escape below);
#   * the full argv of those invocations, pinned to a committed fixture.
#
# It does NOT prove that nothing else in release.yml publishes a body. FOUR in-file
# constructs are known to escape it and are deliberately not chased:
#
#   1. a `gh()` shell FUNCTION shadowing the real binary — every invocation still reads
#      as literal and still carries the required flag;
#   2. a `run:` step whose shell is python or pwsh, calling the API directly;
#   3. a path to the rendered file ASSEMBLED rather than written literally —
#      `> "$RUNNER_TEMP/release-notes.rendered".md`, `> "${D}.rendered.md"`,
#      `>> "$RUNNER_TEMP/$D.md"`, an `ln -sf` over it, or a `sed -i` naming it through a
#      variable. All demonstrated in review, all with this guard clean. Bash builds
#      paths; there is no finite set of spellings to enumerate, which is the same
#      unwinnable class as the two above;
#   4. a step that rewrites the REVIEWED SOURCE, `.github/release-notes.md`, before the
#      render — `sed -i "s/…/…/" .github/release-notes.md`, a `printf >` over it, an
#      `echo >>` appending to it. Demonstrated in review, guard clean. This one is
#      different in kind and matters most: it means the body a user reads was NOT
#      necessarily the body review saw. The mutation happens between checkout and
#      render, on a runner, and leaves no trace in the file diff. Nothing here
#      constrains where `.github/release-notes.md` may be named; it is read for the
#      emptiness check and matched as a substring on the render line, and that is all.
#
# A `curl -X PATCH` or `gh api -X PATCH … -f body=…` in this same publish step does it
# too, in bash, with this guard clean. That is not a fifth item: it is the class named
# below, and it works in-file as readily as out. The four above are four constructs
# BESIDES it.
#
# Escapes 1-3 are the class round 6 was right to abandon —
# enumerating how a shell reaches an HTTP endpoint is not winnable — and adding
# regexes for them would restart the loop that has cost nine rounds. Outside
# this file it proves nothing at all: a `curl` to the REST API, a step in another
# workflow, or a person with a browser all reach the same Release with this clean.
# Review of `release.yml` is what covers every one of those.
#
# That boundary is the correction of two opposite mistakes, both made here:
#
#   * Rounds 1-6 tried to prove that NOTHING anywhere could publish a false body.
#     Each version was defeated by an idiom its author had not imagined, and the
#     last one tried to settle it by EXECUTING the pipeline's steps — which meant
#     `sudo apt-get`, `cargo install`, and a call to the PRODUCTION CoreLink API,
#     on every developer's laptop, in every lane, for 4m22s of a 7m38s gate.
#     Enumerating how a shell can reach an HTTP endpoint is not winnable.
#   * Round 7 over-corrected: it disclaimed that whole class and then checked only
#     `--notes`, so `gh release create -n "…SLSA provenance…"` — the original
#     defect, verbatim, in this very file — passed clean. `-n` is not an
#     unimaginable idiom; it is on the first screen of `gh release create --help`.
#     A closed set that a tool documents for you is exactly the kind you CAN
#     enumerate, and declining to is not a scope decision, it is the control
#     failing at its one job.
#
# Reads three files (the workflow, the body, the argv fixture) and runs no
# subprocess but this python3. ~0.1s. Every lane.

set -euo pipefail

command -v python3 >/dev/null 2>&1 || {
  echo "GATE FAILED: python3 is required by the release-notes guard." >&2; exit 1; }

python3 - <<'PY'
import os, re, sys

WF = '.github/workflows/release.yml'
BODY = '.github/release-notes.md'
FIXTURE = '.github/release-publish.expected'

# The only notes-bearing form permitted, written exactly.
REQUIRED = '--notes-file "$NOTES_FILE"'
# Every OTHER way `gh release create|edit` can set or replace the body, short and
# long, from `gh release create --help` and `gh release edit --help`. This is the
# closed set the round-7 version failed to enumerate: it checked `--notes` alone,
# so `-n` — the same flag, one screen up in the same help output — walked through.
# If `gh` ever grows another, this list is the thing that must change.
FORBIDDEN = ['--notes', '-n', '-F', '--notes-from-tag', '--generate-notes',
             '--notes-start-tag']

def fail(msg, *extra):
    sys.stderr.write("GATE FAILED: %s\n" % msg)
    for e in extra:
        sys.stderr.write("  %s\n" % e)
    sys.exit(1)

for p in (WF, BODY, FIXTURE):
    if not os.path.isfile(p):
        fail("%s is missing. The published release notes must come from %s, rendered "
             "by %s, and pinned by %s." % (p, BODY, WF, FIXTURE))

if not open(BODY, encoding='utf-8').read().strip():
    fail("%s is empty — the pipeline would publish a release with no body." % BODY)

raw = open(WF, encoding='utf-8').read().splitlines()

# Drop whole-line comments, then JOIN backslash continuations. Both matter:
# a `#` mid-line can sit inside a string or a `${#var}`, so only a line whose first
# non-space character is `#` is commentary; and without joining, an invocation split
# across lines is invisible to every check below — which is how `--notes-file` on a
# continuation line both evaded matching AND produced a nonsense failure message.
lines, buf, start = [], '', None
for n, t in enumerate(raw, 1):
    if t.lstrip().startswith('#'):
        continue
    if buf:
        buf += ' ' + t.strip()
    else:
        buf, start = t, n
    if buf.rstrip().endswith('\\'):
        buf = buf.rstrip()[:-1]
        continue
    lines.append((start, ' '.join(buf.split())))
    buf, start = '', None
if buf:
    lines.append((start, ' '.join(buf.split())))

def hits(pattern):
    rx = re.compile(pattern)
    return [(n, t) for n, t in lines if rx.search(t)]

def has_flag(text, flag):
    r"""A flag occurrence, attached or detached.

    SHORT flags get no trailing boundary: pflag accepts `-nVALUE` and `-F-` with the
    value glued on, and requiring a separator after the flag let both through — which
    matters, because pflag takes the LAST occurrence of a repeated flag, so a trailing
    `-F-` sources the body from stdin while the required `--notes-file "$NOTES_FILE"`
    is still sitting there in plain sight. The leading `(?<![\w-])` keeps `-n` from
    matching inside `--notes-file`: the character before that inner `-n` is `-`.

    LONG flags keep the trailing boundary, because without it `--notes` matches the
    prefix of `--notes-file` and the guard rejects its own required form.
    """
    tail = r'' if len(flag) == 2 else r'(?=[\s=]|$)'
    return re.search(r'(?<![\w-])%s%s' % (re.escape(flag), tail), text) is not None

# --- 1. every release-publishing invocation is a literal `gh release …` ----------
# `$GH release edit`, `xargs gh release edit` and `gh release "$SUB"` all publish
# while matching nothing named below. Requiring the literal `gh release <verb>` form
# turns the check from "does a bad string appear" into "is this the shape we allow".
for n, t in hits(r'release\s+(create|edit)\b'):
    if not re.search(r'(^|[\s;&|(])gh\s+release\s+(create|edit)\b', t):
        fail("a release is created or edited through an indirect invocation.",
             "line %d: %s" % (n, t[:200]),
             "Write it as a literal `gh release create` / `gh release edit`. Reaching",
             "the same command through a variable, `xargs`, or a computed subcommand",
             "hides the arguments from review and from this check.")

invocations = hits(r'(^|[\s;&|(])gh\s+release\s+(create|edit)\b')
if not invocations:
    fail("release.yml contains no `gh release create` or `gh release edit`.",
         "Nothing publishes %s. Either the publish step was removed, or it now runs"
         % BODY, "through a form this guard cannot see — both need a human.")

# --- 2. each one takes its body from the rendered file, and only that ------------
for n, t in invocations:
    for flag in FORBIDDEN:
        if has_flag(t, flag):
            fail("a `gh release` invocation sets the body with `%s`." % flag,
                 "line %d: %s" % (n, t[:200]),
                 "The body must come from %s so that changing what users read is a" % BODY,
                 "reviewable file diff. Note `-n` is a synonym for `--notes`, `-F` for",
                 "`--notes-file`; the original SLSA defect was re-introduced in review",
                 "as a plain `-n \"…\"` while this guard reported clean.")
    if REQUIRED not in t:
        fail("a `gh release` invocation does not pass `%s`." % REQUIRED,
             "line %d: %s" % (n, t[:200]),
             "Every publishing invocation must name the one rendered body, written",
             "exactly that way. `gh release upload` does not touch the body and is",
             "not matched here.")

# --- 3. $NOTES_FILE: one assignment, one write, no other reference ---------------
# Naming the tools that could rewrite it (`sed -i`, `tee -a`, `>>`, `cp`, `mv`,
# `truncate`, a heredoc) would be a denylist, and denylists are what failed six
# times. Inverting it closes them all without knowing any of their names — but it
# has to be inverted on BOTH the variable and the path. Round 8 did only the
# variable, and a `printf ... > "$RUNNER_TEMP/release-notes.rendered.md"` replaced
# the published body without ever mentioning NOTES_FILE. `gh` reads a file.
assigns = [h for h in hits(r'(?:^|\s)(?:export\s+)?NOTES_FILE=')]
if len(assigns) != 1:
    fail("NOTES_FILE must be assigned exactly once in release.yml (found %d)."
         % len(assigns), *("line %d: %s" % (n, t[:200]) for n, t in assigns))

render = [h for h in hits(r'>\s*"\$NOTES_FILE"') if BODY in h[1]]
if len(render) != 1:
    fail("exactly one line must write $NOTES_FILE from %s (found %d)." % (BODY, len(render)),
         *("line %d: %s" % (n, t[:200]) for n, t in render))

allowed_lines = {assigns[0][0], render[0][0]} | {n for n, _ in invocations}
for n, t in hits(r'\$\{?NOTES_FILE\b'):
    if n not in allowed_lines:
        fail("$NOTES_FILE is referenced outside its assignment, its render, and the "
             "`gh release` invocations.",
             "line %d: %s" % (n, t[:200]),
             "Anything else — an in-place edit, an append, a copy, a second writer —",
             "would change the published text after the reviewed file was rendered.")

# The same inversion on the PATH. `gh` reads a file, not a variable: a `printf` or a
# heredoc writing "$RUNNER_TEMP/release-notes.rendered.md" directly replaces the body
# while never mentioning NOTES_FILE, and review demonstrated exactly that. Whatever
# basename the assignment chooses may appear only on the assignment line.
basename = re.search(r'NOTES_FILE=.*?/([\w.-]+)', assigns[0][1])
if not basename:
    fail("cannot determine the rendered file's name from the NOTES_FILE assignment.",
         "line %d: %s" % (assigns[0][0], assigns[0][1][:200]),
         "It must assign a path whose last component is a literal filename, so that",
         "name can be reserved to that one line.")
for n, t in hits(re.escape(basename.group(1))):
    if n != assigns[0][0]:
        fail("the rendered notes file is named outside its single assignment.",
             "line %d: %s" % (n, t[:200]),
             "Writing that path directly — `printf ... > \"$RUNNER_TEMP/%s\"`, a heredoc,"
             % basename.group(1),
             "a `cp` — replaces the published body without ever mentioning $NOTES_FILE,",
             "which is why the variable-name check above is not sufficient on its own.")

# --- 4. the render is a pure __VERSION__ substitution ----------------------------
# `-e` allows a second expression, `-f` moves the script into another file, `-i`
# edits in place. Each puts the published wording back outside review; requiring a
# single literal expression is simpler to state and to check than modelling them.
rline = render[0][1]
RENDER_FORM = re.compile(
    r'^sed "s/__VERSION__/\$\{VERSION\}/g" ' + re.escape(BODY) + r' > "\$NOTES_FILE"$')
if not RENDER_FORM.match(rline):
    fail("the render of %s is not written in the one permitted form." % BODY,
         "line %d: %s" % render[0],
         'Expected exactly:',
         '  sed "s/__VERSION__/${VERSION}/g" %s > "$NOTES_FILE"' % BODY,
         "",
         "This is a positive match, not a list of forbidden options, because the",
         "forbidden list was the wrong shape: it named -e, -f and -i and could not see",
         "GNU sed's documented --expression, --file and --in-place, each of which can",
         "rewrite any part of the reviewed text on its way to the Release page. One",
         "permitted spelling is closed by construction; a list of bad ones never is.")

# --- 5. the full argv of every publishing invocation is PINNED -------------------
# Checks 1-4 constrain the BODY. They say nothing about --title, --repo, --draft, or
# a --latest/--prerelease inversion, all of which are user-visible and all of which
# the deleted fixture did cover. This restores that, statically: no execution, no
# stubs, no network — just the normalised text of the invocations, which must equal
# a committed file. Any change to how a release is published is then a two-file diff
# a human must write on purpose.
actual = '\n'.join(t for _, t in invocations) + '\n'
expected = open(FIXTURE, encoding='utf-8').read()
if actual != expected:
    fail("the `gh release` invocations in %s do not match %s." % (WF, FIXTURE),
         "This pins the whole argv, not just the notes: --title, --repo, --draft, and",
         "the --latest/--prerelease flag are user-visible too. If the change is",
         "intended, update the fixture in the same commit so it appears in review.",
         "", "--- expected ---", *expected.splitlines(),
         "--- actual ---", *actual.splitlines())

print("release-notes guard: clean. Every `gh release create|edit` in %s takes its body "
      "from %s via `%s` (rendered: __VERSION__ only), and their full argv matches %s. "
      "Scope: this file only, and FOUR constructs inside it that it does not see "
      "(a shadowing gh() function, a non-bash run: step, an assembled path to the "
      "rendered file, and a rewrite of %s itself before the render) — plus anything "
      "outside it. Read this script\'s header before citing it as coverage."
      % (WF, BODY, REQUIRED, FIXTURE, BODY))
PY
