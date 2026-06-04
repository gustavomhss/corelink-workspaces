#!/usr/bin/env bash
# dco-check.sh — verify every non-merge commit in <base>..HEAD has a Signed-off-by trailer.
# Usage: dco-check.sh [<base-ref>]   (default: origin/main)
set -euo pipefail

BASE="${1:-origin/main}"
RANGE="${BASE}..HEAD"

# Collect non-merge commits in range (empty range is valid → exit 0).
commits=$(git log --no-merges --format="%H" "${RANGE}" 2>/dev/null) || {
  echo "dco-check: could not resolve range '${RANGE}' — is '${BASE}' fetched?" >&2
  exit 1
}

if [ -z "${commits}" ]; then
  echo "dco-check: no non-merge commits in ${RANGE} — OK"
  exit 0
fi

offenders=()
while IFS= read -r sha; do
  subject=$(git log -1 --format="%s" "${sha}")
  body=$(git log -1 --format="%b" "${sha}")
  if ! printf '%s\n' "${body}" | grep -qE '^Signed-off-by: .+ <.+>'; then
    offenders+=("${sha} ${subject}")
  fi
done <<< "${commits}"

if [ "${#offenders[@]}" -eq 0 ]; then
  echo "dco-check: all commits signed off — OK"
  exit 0
fi

echo "dco-check: the following commits are missing a Signed-off-by trailer:" >&2
for entry in "${offenders[@]}"; do
  echo "  ${entry}" >&2
done
exit 1
