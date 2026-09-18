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

# PR checkout is untrusted input. The CoreLink runner fabric can inject the
# single-use broker credential trio at env-0, but no PR workflow that checks out
# repository input may inherit it. Inventory every direct workflow YAML file
# below: a new direct-PR workflow is a failure until it supplies this boundary.
# Direct means `pull_request` or an event named `pull_request_*` (target, review,
# review-comment, and future siblings). Indirect events such as `issue_comment`
# and `workflow_dispatch` do not carry that name and are not classified by name;
# any current workflow that materializes a PR head is nevertheless audited because
# this policy guards every executable job of every direct-PR workflow, not checkout
# steps alone.
credential_null_repo_root() {
  CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd
}

assert_pr_credential_null_workflows() {
  local workflow_root="$1"
  command -v python3 >/dev/null 2>&1 || {
    echo "GATE FAILED: python3 is required to validate the PR credential-null workflow guard" >&2
    return 1
  }

  python3 - "$workflow_root" <<'PY'
import re
import sys
from pathlib import Path

workflow_root = Path(sys.argv[1])
credentials = ("CLW_CRED_TICKET", "CLW_LEASE_ID", "CLW_FABRIC_ENDPOINT")
first_step_guard = (
    "      - name: assert PR broker credentials are absent\n"
    "        if: github.event_name == 'pull_request' || startsWith(github.event_name, 'pull_request_')\n"
    "        shell: bash\n"
    "        run: |\n"
    "          set -euo pipefail\n"
    "          for credential in CLW_CRED_TICKET CLW_LEASE_ID CLW_FABRIC_ENDPOINT; do\n"
    "            [ -z \"${!credential:-}\" ] || {\n"
    "              echo \"::error::PR broker credential must be empty: ${credential}\"\n"
    "              exit 1\n"
    "            }\n"
    "          done"
)

def fail(workflow: str, reason: str) -> None:
    print(f"GATE FAILED: PR credential-null guard: {workflow}: {reason}", file=sys.stderr)
    raise SystemExit(1)

def yaml_double_quoted_key(value):
    """Decode the YAML 1.2 double-quoted escapes permitted in a mapping key.

    This is deliberately not a YAML parser.  It is only the scalar grammar the
    source-level guard needs to compare a key with a small, fixed vocabulary.
    Unknown escapes (and line folding) are refused by the caller, rather than
    being guessed at and possibly making a security-relevant key invisible.
    """
    escapes = {
        "0": "\0", "a": "\a", "b": "\b", "t": "\t", "n": "\n",
        "v": "\v", "f": "\f", "r": "\r", "e": "\x1b", " ": " ",
        '\"': '\"', "/": "/", "\\": "\\", "N": "\u0085", "_": "\u00a0",
        "L": "\u2028", "P": "\u2029",
    }
    decoded = []
    index = 0
    while index < len(value):
        character = value[index]
        if character != "\\":
            decoded.append(character)
            index += 1
            continue
        index += 1
        if index == len(value):
            raise ValueError("trailing backslash")
        escape = value[index]
        if escape in escapes:
            decoded.append(escapes[escape])
            index += 1
            continue
        digits = {"x": 2, "u": 4, "U": 8}.get(escape)
        if digits is None or index + digits >= len(value):
            raise ValueError("unsupported escape")
        codepoint = value[index + 1:index + 1 + digits]
        if not re.fullmatch(r"[0-9A-Fa-f]+", codepoint):
            raise ValueError("non-hex escape")
        codepoint_number = int(codepoint, 16)
        if 0xD800 <= codepoint_number <= 0xDFFF or codepoint_number > 0x10FFFF:
            raise ValueError("invalid Unicode code point")
        decoded.append(chr(codepoint_number))
        index += 1 + digits
    return "".join(decoded)

def yaml_mapping_key(line, indent, empty_value=False):
    """Return one conservative plain/single/double-quoted block-map key.

    The PR policy accepts a deliberately narrow block-mapping vocabulary.  This
    recognizer proves the spelling of the few keys we inventory; it does not try
    to interpret flow YAML, aliases, tags, or arbitrary YAML values.
    """
    prefix = " " * indent
    if not line.startswith(prefix) or len(line) == len(prefix) or line[len(prefix)].isspace():
        return None
    value = line[len(prefix):]
    key = None
    after_key = ""
    if value[0] == "'":
        index = 1
        decoded = []
        while index < len(value):
            if value[index] == "'":
                if index + 1 < len(value) and value[index + 1] == "'":
                    decoded.append("'")
                    index += 2
                    continue
                key = "".join(decoded)
                after_key = value[index + 1:]
                break
            decoded.append(value[index])
            index += 1
    elif value[0] == '\"':
        index = 1
        raw = []
        while index < len(value):
            if value[index] == '\"':
                try:
                    key = yaml_double_quoted_key("".join(raw))
                except ValueError:
                    return None
                after_key = value[index + 1:]
                break
            if value[index] == "\\" and index + 1 < len(value):
                raw.extend(value[index:index + 2])
                index += 2
                continue
            raw.append(value[index])
            index += 1
    else:
        plain = re.match(r"([A-Za-z0-9_-]+)(.*)$", value)
        if plain is None:
            return None
        key, after_key = plain.groups()
    if key is None:
        return None
    separator = re.match(r"^[ \t]*:(.*)$", after_key)
    if separator is None:
        return None
    tail = separator.group(1)
    if empty_value and tail.strip() and not tail.lstrip().startswith("#"):
        return None
    return key

def yaml_mapping_key_at_any_indent(line):
    if "\t" in line[:len(line) - len(line.lstrip(" \t"))]:
        return None
    indent = len(line) - len(line.lstrip(" "))
    return yaml_mapping_key(line, indent)

if not workflow_root.is_dir():
    fail("workflow corpus", f"missing directory {workflow_root}")

workflow_paths = sorted(
    path for path in workflow_root.iterdir()
    if path.is_file() and path.suffix.lower() in (".yml", ".yaml")
)
if not workflow_paths:
    fail("workflow corpus", "contains no workflow YAML files")

guarded_workflows = []
guarded_jobs = 0
for path in workflow_paths:
    workflow = path.name
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(workflow, f"cannot read workflow ({exc})")

    # Validate the top-level `on:` form BEFORE classifying events. YAML admits
    # sequences, flow maps, aliases, anchors, tags and quoted keys; accepting a
    # subset opportunistically would let a valid PR trigger fall through as
    # "not a PR". The repository policy intentionally permits only a plain
    # top-level mapping block with unquoted, unique event keys.
    raw_lines = text.splitlines()
    top_on_forms = [
        index for index, line in enumerate(raw_lines)
        if re.match(r"^(?:on|['\"]on['\"])\s*:", line)
    ]
    if len(top_on_forms) != 1 or raw_lines[top_on_forms[0]] != "on:":
        fail(workflow, "top-level on must be exactly one plain `on:` mapping block (quoted, flow, alias, anchor, tag, and duplicate forms are refused)")
    on_start = top_on_forms[0]
    on_end = len(raw_lines)
    for index in range(on_start + 1, len(raw_lines)):
        line = raw_lines[index]
        if line and not line[0].isspace() and not line.startswith("#"):
            on_end = index
            break
    on_events = []
    for line in raw_lines[on_start + 1:on_end]:
        active = line.split("#", 1)[0]
        if not active.strip():
            continue
        if active.startswith("    "):
            continue
        event = re.fullmatch(r"  ([A-Za-z_][A-Za-z0-9_-]*):\s*", active)
        if event is None:
            fail(workflow, "top-level on must be a plain event mapping (sequence, flow, alias, anchor, tag, and ambiguous forms are refused)")
        on_events.append(event.group(1))
    if not on_events or len(on_events) != len(set(on_events)):
        fail(workflow, "top-level on must contain non-empty unique canonical event keys")

    active_lines = [line.split("#", 1)[0] for line in text.splitlines()]
    if not any(event == "pull_request" or event.startswith("pull_request_") for event in on_events):
        continue
    guarded_workflows.append(workflow)

    # The marker lives inside the top-level env block. It makes the scope
    # auditable and lets this intentionally small parser reject a job-level
    # lookalike rather than guessing at arbitrary YAML constructs.
    if text.count("  # CI_CREDENTIAL_NULL_GUARD: workflow-env") != 1:
        fail(workflow, "must contain exactly one top-level-env guard marker")
    top_env = re.search(r"(?ms)^env:\n(.*?)(?=^[^ \t#]|\Z)", text)
    if top_env is None:
        fail(workflow, "has no top-level env block")
    env_block = top_env.group(1)
    if env_block.count("  # CI_CREDENTIAL_NULL_GUARD: workflow-env") != 1:
        fail(workflow, "guard marker is not in the top-level env block")
    if any(re.search(r"\benv:\s*\{[^}]*\\", line) for line in raw_lines):
        fail(workflow, "an inline env mapping with YAML escapes is refused because its credential keys cannot be safely inventoried")
    for credential in credentials:
        declaration = f"  {credential}: \"\""
        if env_block.count(declaration) != 1:
            fail(workflow, f"top-level {credential} must be declared exactly once as empty")
        declarations = [
            line for line in raw_lines
            if yaml_mapping_key_at_any_indent(line) == credential
        ]
        if declarations != [declaration]:
            fail(workflow, f"{credential} must use one unquoted canonical declaration; quoted, inline, and duplicate forms are refused")
        if any(re.search(rf"\benv:\s*\{{[^}}]*{credential}", line) for line in active_lines):
            fail(workflow, f"{credential} may not appear in an inline env mapping")

    try:
        jobs_start = active_lines.index("jobs:")
    except ValueError:
        fail(workflow, "PR workflow has no jobs mapping")
    # A reusable-workflow job receives a fresh called-workflow context: caller
    # workflow env does not cross that boundary.  Therefore it is forbidden
    # unless this policy grows an explicit, independently-audited treatment.
    #
    # Do not let YAML quoting or a valid double-quoted escape make a job
    # invisible to that rule.  Any other mapping-looking job key is refused
    # rather than silently omitted by this deliberately small parser.
    job_starts = []
    for index in range(jobs_start + 1, len(raw_lines)):
        line = raw_lines[index]
        if line.startswith("  #"):
            continue
        if not re.match(r"^  \S", line):
            continue
        job_name = yaml_mapping_key(line, 2, empty_value=True)
        if job_name is None or not re.fullmatch(r"[A-Za-z0-9_-]+", job_name):
            fail(workflow, "jobs mapping contains a non-canonical job key; only plain or single/double-quoted [A-Za-z0-9_-] IDs are accepted")
        job_starts.append((index, job_name))
    if not job_starts:
        fail(workflow, "PR workflow has no executable jobs")
    for position, (job_start, job_name) in enumerate(job_starts):
        job_end = job_starts[position + 1][0] if position + 1 < len(job_starts) else len(active_lines)
        job_lines = active_lines[job_start:job_end]
        raw_job_lines = raw_lines[job_start:job_end]
        if any(yaml_mapping_key(line, 4) == "uses" for line in raw_job_lines):
            fail(workflow, f"job {job_name} uses a reusable workflow; reusable jobs are refused until explicitly guarded")
        step_markers = [index for index, line in enumerate(job_lines) if line == "    steps:"]
        if len(step_markers) != 1:
            fail(workflow, f"job {job_name} must declare exactly one steps list")
        first_step = next(
            (index for index in range(step_markers[0] + 1, len(job_lines)) if re.match(r"      - ", job_lines[index])),
            None,
        )
        if first_step is None:
            fail(workflow, f"job {job_name} has no executable step")
        candidate = "\n".join(job_lines[first_step:first_step + len(first_step_guard.splitlines())])
        if candidate != first_step_guard:
            fail(workflow, f"job {job_name} must begin with the fail-closed PR credential runtime guard")
        guarded_jobs += 1

if not guarded_workflows:
    fail("workflow corpus", "contains no pull_request or pull_request_target workflow to guard")
print("PR credential-null workflow guard: clean (all PR workflows / jobs guarded: " + ", ".join(guarded_workflows) + f"; {guarded_jobs} jobs)")
PY
}

assert_pr_credential_null_runtime() {
  local credential
  for credential in CLW_CRED_TICKET CLW_LEASE_ID CLW_FABRIC_ENDPOINT; do
    if [ -n "${!credential:-}" ]; then
      echo "GATE FAILED: PR credential-null guard: ${credential} is non-empty" >&2
      return 1
    fi
  done
  echo "PR credential-null runtime guard: broker credential trio is absent or empty"
}

assert_pr_credential_null() {
  local workflow_root="$1"
  assert_pr_credential_null_workflows "$workflow_root"
  assert_pr_credential_null_runtime
}

pr_credential_null_selftest() {
  local repo_root script out rc
  repo_root="$(credential_null_repo_root)"
  script="$repo_root/scripts/gates.sh"
  PR_CREDENTIAL_NULL_SELFTEST_FIXTURE="$(mktemp -d "${TMPDIR:-/tmp}/clw-pr-credential-null.XXXXXX")"
  trap 'rm -rf "${PR_CREDENTIAL_NULL_SELFTEST_FIXTURE:-}"' EXIT
  mkdir -p "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows"
  cp "$repo_root/.github/workflows/ci.yml" "$repo_root/.github/workflows/coverage.yml" \
    "$repo_root/.github/workflows/mutation.yml" "$repo_root/.github/workflows/shellcheck.yml" \
    "$repo_root/.github/workflows/windows.yml" "$repo_root/.github/workflows/pr-title.yml" \
    "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/"

  if "$script" --assert-pr-credential-null --workflow-dir "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows" >/dev/null; then
    echo "  ✓ clean PR workflow/job inventory passes"
  else
    echo "  ✗ clean PR workflow/job inventory failed" >&2
    return 1
  fi

  expect_pr_credential_null_failure() {
    local description="$1" expected="$2"
    out=""
    rc=0
    out="$("$script" --assert-pr-credential-null --workflow-dir "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows" 2>&1)" || rc=$?
    if [ "$rc" -ne 0 ] && printf '%s\n' "$out" | grep -qF "$expected"; then
      echo "  ✓ $description"
    else
      echo "  ✗ $description — static guard passed or did not name '$expected'" >&2
      printf '%s\n' "$out" >&2
      return 1
    fi
  }

  out=""
  rc=0
  out="$(CLW_CRED_TICKET=leaked "$script" --assert-pr-credential-null --workflow-dir "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows" 2>&1)" || rc=$?
  if [ "$rc" -ne 0 ] && printf '%s\n' "$out" | grep -qF 'CLW_CRED_TICKET is non-empty'; then
    echo "  ✓ a non-empty runtime broker credential is named"
  else
    echo "  ✗ a non-empty runtime broker credential was accepted" >&2
    printf '%s\n' "$out" >&2
    return 1
  fi

  # Change copies of the REAL PR workflows, then invoke the production
  # subcommand in a fresh process. If the guard is replaced by `return 0`, each
  # sabotage turns green and this self-test turns red.
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text = text.replace('  CLW_CRED_TICKET: ""', '  CLW_CRED_TICKET: "leaked"', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  expect_pr_credential_null_failure "a non-empty broker credential declaration is named" "coverage.yml"

  cp "$repo_root/.github/workflows/coverage.yml" "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml"
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text = text.replace('  CLW_LEASE_ID: ""\n', '', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  expect_pr_credential_null_failure "a removed broker credential declaration is named" "coverage.yml"

  cp "$repo_root/.github/workflows/coverage.yml" "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml"
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
assert '    runs-on: ubuntu-24.04\n' in text, 'coverage job runner anchor changed'
text = text.replace('    runs-on: ubuntu-24.04\n', '    runs-on: ubuntu-24.04\n    env:\n      "CLW_CRED_TICKET": "leaked"\n', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  expect_pr_credential_null_failure "a quoted credential key cannot bypass the static guard" "unquoted canonical declaration"

  cp "$repo_root/.github/workflows/coverage.yml" "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml"
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
assert '    runs-on: ubuntu-24.04\n' in text, 'coverage job runner anchor changed'
text = text.replace('    runs-on: ubuntu-24.04\n', '    runs-on: ubuntu-24.04\n    env:\n      "CLW_CRED_\\u0054ICKET": "leaked"\n', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  expect_pr_credential_null_failure "an escaped credential key cannot bypass the static guard" "unquoted canonical declaration"

  cp "$repo_root/.github/workflows/coverage.yml" "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/coverage.yml"
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/windows.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text = text.replace('      - name: assert PR broker credentials are absent\n', '      - name: bypassed PR broker credential guard\n', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  expect_pr_credential_null_failure "a missing first-step runtime guard is named" "windows.yml"

  cp "$repo_root/.github/workflows/windows.yml" "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/windows.yml"
  # fixture must write GitHub's expression literally.
  # shellcheck disable=SC2016
  printf '%s\n' \
    'name: unguarded-pr-target' \
    'on:' \
    '  pull_request_target:' \
    'env:' \
    '  # CI_CREDENTIAL_NULL_GUARD: workflow-env' \
    '  CLW_CRED_TICKET: ""' \
    '  CLW_LEASE_ID: ""' \
    '  CLW_FABRIC_ENDPOINT: ""' \
    'jobs:' \
    '  build:' \
    '    runs-on: ubuntu-24.04' \
    '    steps:' \
    '      - run: git fetch origin "${{ github.event.pull_request.head.sha }}"' \
    > "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/target-pr.yml"
  expect_pr_credential_null_failure "a pull_request_target workflow is inventoried" "target-pr.yml"
  rm -f "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/target-pr.yml"

  # A caller's workflow-level env does not protect a called reusable workflow.
  # These are valid YAML (and accepted by actionlint): quoted job IDs and quoted
  # `uses` keys must therefore be recognized, not skipped by the source guard.
  write_quoted_reusable_job_fixture() {
    printf '%s\n' \
      'name: quoted-reusable-pr-job' \
      'on:' \
      '  pull_request:' \
      'env:' \
      '  # CI_CREDENTIAL_NULL_GUARD: workflow-env' \
      '  CLW_CRED_TICKET: ""' \
      '  CLW_LEASE_ID: ""' \
      '  CLW_FABRIC_ENDPOINT: ""' \
      'jobs:' \
      "$1" \
      "$2" \
      > "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/quoted-reusable.yml"
  }
  write_quoted_reusable_job_fixture \
    '  "unguarded-reusable" :' \
    '    "uses" : owner/example/.github/workflows/reusable.yml@main'
  expect_pr_credential_null_failure \
    "a double-quoted reusable job/key is named despite YAML whitespace" \
    "job unguarded-reusable uses a reusable workflow"
  write_quoted_reusable_job_fixture \
    "  'unguarded-reusable' :" \
    "    'uses' : owner/example/.github/workflows/reusable.yml@main"
  expect_pr_credential_null_failure \
    "a single-quoted reusable job/key is named despite YAML whitespace" \
    "job unguarded-reusable uses a reusable workflow"
  write_quoted_reusable_job_fixture \
    '  "unguarded\u002dreusable" :' \
    '    "uses" : owner/example/.github/workflows/reusable.yml@main'
  expect_pr_credential_null_failure \
    "an escaped double-quoted reusable job ID is inventoried" \
    "job unguarded-reusable uses a reusable workflow"
  write_quoted_reusable_job_fixture \
    '  "unguarded-reusable" :' \
    '    "us\u0065s" : owner/example/.github/workflows/reusable.yml@main'
  expect_pr_credential_null_failure \
    "an escaped double-quoted uses key is inventoried" \
    "job unguarded-reusable uses a reusable workflow"
  rm -f "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/quoted-reusable.yml"

  cp "$repo_root/.github/workflows/coverage.yml" "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/review-pr.yml"
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/review-pr.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text = text.replace('  pull_request:\n', '  pull_request_review:\n', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  if "$script" --assert-pr-credential-null --workflow-dir "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows" >/dev/null; then
    echo "  ✓ a guarded pull_request_review workflow is accepted"
  else
    echo "  ✗ a guarded pull_request_review workflow was rejected" >&2
    return 1
  fi
  python3 - "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/review-pr.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text = text.replace('  pull_request_review:\n', '  pull_request_review_comment:\n', 1)
text = text.replace('      - name: assert PR broker credentials are absent\n', '      - name: bypassed PR broker credential guard\n', 1)
open(path, "w", encoding="utf-8").write(text)
PY
  expect_pr_credential_null_failure "a pull_request_review_comment workflow requires the first-step guard" "review-pr.yml"
  rm -f "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/review-pr.yml"

  write_ambiguous_on_fixture() {
    printf '%s\n' 'name: ambiguous-on' "$@" 'jobs: {}' \
      > "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/on-form.yml"
  }
  write_ambiguous_on_fixture 'on:' '  - pull_request'
  expect_pr_credential_null_failure "a sequence-form top-level on is refused" "plain event mapping"
  write_ambiguous_on_fixture 'on:' '  pull_request:' '  pull_request:'
  expect_pr_credential_null_failure "a duplicate pull_request event is refused" "unique canonical event keys"
  write_ambiguous_on_fixture 'on: &pr_events'
  expect_pr_credential_null_failure "an anchored top-level on is refused" "top-level on"
  write_ambiguous_on_fixture 'on: *pr_events'
  expect_pr_credential_null_failure "an aliased top-level on is refused" "top-level on"
  write_ambiguous_on_fixture 'on: !!map'
  expect_pr_credential_null_failure "a tagged top-level on is refused" "top-level on"
  write_ambiguous_on_fixture 'on: [pull_request]'
  expect_pr_credential_null_failure "a flow-form top-level on is refused" "top-level on"
  write_ambiguous_on_fixture 'on: {pull_request: null}'
  expect_pr_credential_null_failure "an inline-map top-level on is refused" "top-level on"
  write_ambiguous_on_fixture '"on":' '  pull_request:'
  expect_pr_credential_null_failure "a quoted top-level on is refused" "top-level on"
  rm -f "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/on-form.yml"

  printf '%s\n' \
    'name: unguarded-pr-checkout' \
    'on:' \
    '  "pull_request":' \
    'jobs:' \
    '  build:' \
    '    runs-on: ubuntu-24.04' \
    '    steps:' \
    '      - uses: actions/checkout@v4' \
    '      - run: cargo test' \
    > "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/new-pr.yml"
  expect_pr_credential_null_failure "a quoted PR trigger cannot evade workflow discovery" "plain event mapping"

  # fixture must write GitHub's expression literally.
  # shellcheck disable=SC2016
  printf '%s\n' \
    'name: unguarded-pr-checkout' \
    'on:' \
    '  pull_request:' \
    'env:' \
    '  # CI_CREDENTIAL_NULL_GUARD: workflow-env' \
    '  CLW_CRED_TICKET: ""' \
    '  CLW_LEASE_ID: ""' \
    '  CLW_FABRIC_ENDPOINT: ""' \
    'jobs:' \
    '  build:' \
    '    runs-on: ubuntu-24.04' \
    '    steps:' \
    '      - run: git fetch origin "${{ github.event.pull_request.head.sha }}"' \
    > "$PR_CREDENTIAL_NULL_SELFTEST_FIXTURE/.github/workflows/new-pr.yml"
  expect_pr_credential_null_failure "a new PR job without a first-step guard cannot be omitted" "new-pr.yml"

  echo "PR credential-null selftest: all red-on-revert cases passed"
}

case "${1:-}" in
  --assert-pr-credential-null)
    shift
    if [ "${1:-}" = "--workflow-dir" ] && [ -n "${2:-}" ] && [ "$#" -eq 2 ]; then
      assert_pr_credential_null "$2"
    elif [ "$#" -eq 0 ]; then
      assert_pr_credential_null "$(credential_null_repo_root)/.github/workflows"
    else
      echo "usage: $0 --assert-pr-credential-null [--workflow-dir DIR]" >&2
      exit 2
    fi
    exit 0
    ;;
  --selftest-pr-credential-null)
    [ "$#" -eq 1 ] || { echo "usage: $0 --selftest-pr-credential-null" >&2; exit 2; }
    pr_credential_null_selftest
    exit 0
    ;;
esac

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

# CI jobs start cold: there is deliberately no actions/cache build cache. Give every
# Cargo command in THIS gates invocation one target directory, but key it to the
# GitHub run attempt under the runner's temporary area. This preserves intra-job
# reuse (including the unsafe-posture compiler sentinel) without pretending that a
# runner's $HOME survives another job.
configure_ci_cargo_target_dir() {
  [ "${CI:-}" = "true" ] || return 0

  local run_id="${GITHUB_RUN_ID:-}" run_attempt="${GITHUB_RUN_ATTEMPT:-}"
  local runner_tmp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
  if [[ ! "$run_id" =~ ^[0-9]+$ ]] || [[ ! "$run_attempt" =~ ^[0-9]+$ ]]; then
    echo "GATE FAILED: CI must provide numeric GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT for a job-local Cargo target" >&2
    return 1
  fi

  # Do not inherit a runner-level target path: it can be a stale persistent cache.
  CARGO_TARGET_DIR="${runner_tmp%/}/clw-gates-target-${run_id}-${run_attempt}"
  export CARGO_TARGET_DIR
  mkdir -p -- "$CARGO_TARGET_DIR"
  echo "CI Cargo target: job-local ${CARGO_TARGET_DIR} (shared by this gates invocation; never a cross-run cache)"
}

print_ci_cargo_target_dir() {
  configure_ci_cargo_target_dir
  printf '%s\n' "${CARGO_TARGET_DIR:-}"
}

case "${1:-}" in
  --print-ci-cargo-target-dir)
    [ "$#" -eq 1 ] || { echo "usage: $0 --print-ci-cargo-target-dir" >&2; exit 2; }
    print_ci_cargo_target_dir
    exit 0
    ;;
esac

configure_ci_cargo_target_dir

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
# registry by name (and registering it is a hard failure). No observer outside this
# repo's Actions scheduler is configured; see docs/OBSERVABILITY.md (WP-36).
bash scripts/deadman-registry.sh check
# And the gate's own teeth: plant every defect class it claims to catch against a
# copy of the REAL workflows dir and assert it goes red AND names the thing — plus
# that the unmodified copy passes, so a gate that fires on everything cannot score
# full marks. Offline; no network.
bash scripts/deadman-selftest.sh
bash scripts/workflow-permissions-selftest.sh
bash scripts/fuzz-selftest.sh
bash scripts/pr-title-workflow-selftest.sh
bash scripts/ci-build-target-selftest.sh

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
  # The gap is not noise: CI assigns a job-local CARGO_TARGET_DIR that every self-test
  # case inherits, and by the time the suite runs, the steps above it in THIS job have
  # already populated that directory — so the one case that compiles the real tree
  # finds most of the graph built, while a local run gives each case its own empty
  # target dir. It is deliberately not a cross-run cache: no actions/cache exists and
  # every CI job starts cold. That is why widening that case from `-p clw-types` to
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
