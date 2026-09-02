#!/usr/bin/env bash
# Structural and red-on-revert checks for the scheduled fuzz workflow's
# credential-null boundary and persistent-corpus proof wiring.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

python3 - .github/workflows/fuzz.yml <<'PY'
import re
import sys
from pathlib import Path

workflow = Path(sys.argv[1])
source = workflow.read_text(encoding="utf-8")

first_guard = """      - name: assert broker credentials are absent
        shell: bash
        run: |
          set -euo pipefail
          for credential in CLW_CRED_TICKET CLW_LEASE_ID CLW_FABRIC_ENDPOINT; do
            [ -z "${!credential:-}" ] || {
              echo "::error::fuzz broker credential must be empty: ${credential}"
              exit 1
            }
          done"""

restore_pin = "uses: actions/cache/restore@0400d5f644dc74513175e3cd8d07132dd4860809 # v4.2.4"
save_pin = "uses: actions/cache/save@0400d5f644dc74513175e3cd8d07132dd4860809 # v4.2.4"
coverage_call = "run: bash scripts/fuzz-coverage-check.sh \"${{ matrix.target }}\" \"${{ steps.corpus-cache.outputs.cache-matched-key != '' }}\""


def validate(text: str) -> None:
    marker = "  # CI_CREDENTIAL_NULL_GUARD: workflow-env"
    if text.count(marker) != 1:
        raise ValueError("top-level credential-null marker")
    env_match = re.search(r"(?ms)^env:\n(.*?)(?=^[^ \t#]|\Z)", text)
    if env_match is None or marker not in env_match.group(1):
        raise ValueError("top-level credential-null env block")
    for credential in ("CLW_CRED_TICKET", "CLW_LEASE_ID", "CLW_FABRIC_ENDPOINT"):
        declaration = f'  {credential}: ""'
        if len(re.findall(rf"^\s*{credential}\s*:.*$", text, re.MULTILINE)) != 1:
            raise ValueError(f"unique {credential} declaration")
        if declaration not in env_match.group(1):
            raise ValueError(f"empty {credential} declaration")

    lines = text.splitlines()
    try:
        jobs_start = lines.index("jobs:")
    except ValueError as error:
        raise ValueError("jobs mapping") from error
    job_starts = [
        (index, match.group(1))
        for index, line in enumerate(lines[jobs_start + 1 :], jobs_start + 1)
        if (match := re.fullmatch(r"  ([A-Za-z0-9_-]+):", line))
    ]
    if [name for _, name in job_starts] != ["supply-chain", "fuzz"]:
        raise ValueError("exact fuzz job inventory")
    for position, (start, name) in enumerate(job_starts):
        end = job_starts[position + 1][0] if position + 1 < len(job_starts) else len(lines)
        job_lines = lines[start:end]
        try:
            steps = job_lines.index("    steps:")
            first = next(i for i in range(steps + 1, len(job_lines)) if job_lines[i].startswith("      - "))
        except (ValueError, StopIteration) as error:
            raise ValueError(f"{name} steps") from error
        candidate = "\n".join(job_lines[first : first + len(first_guard.splitlines())])
        if candidate != first_guard:
            raise ValueError(f"{name} first-step credential guard")

    if text.count(restore_pin) != 1 or text.count(save_pin) != 1:
        raise ValueError("immutable restore/save cache pins")
    if text.count(coverage_call) != 1:
        raise ValueError("restored-cache signal reaches coverage helper")


validate(source)

negative_controls = {
    "non-empty broker credential": source.replace('  CLW_CRED_TICKET: ""', '  CLW_CRED_TICKET: "leaked"', 1),
    "missing first job guard": source.replace("      - name: assert broker credentials are absent", "      - name: compromised first step", 1),
    "mutable cache restore action": source.replace(restore_pin, "uses: actions/cache/restore@v4", 1),
    "inverted cache-restored signal": source.replace("cache-matched-key != ''", "cache-matched-key == ''", 1),
}
for description, mutated in negative_controls.items():
    try:
        validate(mutated)
    except ValueError:
        continue
    raise SystemExit(f"negative control unexpectedly passed: {description}")

print("fuzz workflow selftest GREEN: credential guards and restored-corpus proof have red-on-revert controls")
PY
