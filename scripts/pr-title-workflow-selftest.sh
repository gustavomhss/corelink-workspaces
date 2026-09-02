#!/usr/bin/env bash
# pr-title-workflow-selftest.sh — fail closed if the trusted PR-title metadata
# workflow stops isolating its checker from PR-controlled code or title syntax.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$REPO_ROOT/.github/workflows/pr-title.yml"

python3 - "$WORKFLOW" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
try:
    text = path.read_text(encoding="utf-8")
except OSError as exc:
    print(f"PR title workflow selftest failed: cannot read {path}: {exc}", file=sys.stderr)
    raise SystemExit(1)

def require(fragment: str, reason: str) -> None:
    if fragment not in text:
        print(f"PR title workflow selftest failed: {reason}", file=sys.stderr)
        raise SystemExit(1)

def forbid(fragment: str, reason: str) -> None:
    if fragment in text:
        print(f"PR title workflow selftest failed: {reason}", file=sys.stderr)
        raise SystemExit(1)

require(
    "on:\n  pull_request_target:\n    types: [opened, synchronize, reopened, edited]",
    "pull_request_target must retain its default activities plus edited",
)
require(
    "concurrency:\n  group: pr-title-${{ github.event.pull_request.number }}\n  cancel-in-progress: true",
    "per-PR cancelling concurrency is required",
)
require("permissions:\n  contents: read", "top-level contents: read permission is required")
require("  # CI_CREDENTIAL_NULL_GUARD: workflow-env", "workflow-level credential-null marker is required")
for credential in ("CLW_CRED_TICKET", "CLW_LEASE_ID", "CLW_FABRIC_ENDPOINT"):
    require(f'  {credential}: ""', f"workflow-level {credential} must be empty")

guard = (
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
job_start = text.find("jobs:\n  pr-title:\n")
if job_start < 0:
    print("PR title workflow selftest failed: missing pr-title job", file=sys.stderr)
    raise SystemExit(1)
job = text[job_start:]
steps_start = job.find("    steps:\n")
if steps_start < 0:
    print("PR title workflow selftest failed: pr-title job has no steps", file=sys.stderr)
    raise SystemExit(1)
steps = job[steps_start + len("    steps:\n"):]
first_step = next((i for i, line in enumerate(steps.splitlines()) if line.startswith("      - ")), None)
if first_step is None or "\n".join(steps.splitlines()[first_step:first_step + len(guard.splitlines())]) != guard:
    print("PR title workflow selftest failed: credential guard must be the exact first step", file=sys.stderr)
    raise SystemExit(1)

checkout = "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2"
if text.count("actions/checkout@") != 1 or checkout not in text:
    print("PR title workflow selftest failed: checkout must use the immutable v4.2.2 pin exactly once", file=sys.stderr)
    raise SystemExit(1)
require(
    "        with:\n          ref: ${{ github.event.pull_request.base.sha }}\n          persist-credentials: false",
    "checkout must use only base.sha and disable persisted credentials",
)
forbid("github.event.pull_request.head.sha", "head SHA must never be checked out")
forbid("github.event.pull_request.head.ref", "head ref must never be checked out")
forbid("draft", "the title gate must not skip draft PRs")

title_env = "          PR_TITLE: ${{ github.event.pull_request.title }}"
if text.count("github.event.pull_request.title") != 1 or title_env not in text:
    print("PR title workflow selftest failed: title must appear exactly once as PR_TITLE env data", file=sys.stderr)
    raise SystemExit(1)
require("        run: bash scripts/pr-title-check.sh", "checker must be invoked without title interpolation")

print("PR title workflow selftest: trusted metadata workflow policy is intact")
PY
