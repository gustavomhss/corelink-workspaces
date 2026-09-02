#!/usr/bin/env bash
# pr-title-check.sh — require the title that survives squash-merge to use the
# repository's Conventional Commits vocabulary (CONTRIBUTING.md and G4).
#
# The title is deliberately read from PR_TITLE rather than an argument so the
# workflow can pass GitHub event data through `env:` instead of interpolating it
# into a shell command. This script accepts no positional arguments.
set -euo pipefail

if [ "$#" -ne 0 ]; then
  echo "usage: $0 (reads PR_TITLE from the environment)" >&2
  exit 2
fi

if [ -z "${PR_TITLE+x}" ]; then
  echo "PR title check failed: PR_TITLE is not set" >&2
  exit 1
fi

# G4 permits seven repository Conventional Commit types. A scope is
# optional but, when present, starts with lowercase ASCII alphanumeric and then
# contains only lowercase ASCII alphanumeric characters, `.`, `_`, `/`, or `-`.
# The optional `!` is the Conventional Commits breaking-change marker and
# belongs immediately after the type or closing scope.
conventional_title_re='^(feat|fix|chore|docs|ci|refactor|test)(\([a-z0-9][a-z0-9._/-]*\))?!?: [^[:space:][:cntrl:]]([^[:cntrl:]]*[^[:space:][:cntrl:]])?$'

if [[ "$PR_TITLE" =~ $conventional_title_re ]]; then
  echo "PR title check: Conventional Commits title accepted"
  exit 0
fi

echo "PR title check failed: use type(scope)!: subject" >&2
echo "allowed types: feat, fix, chore, docs, ci, refactor, test; scope and ! are optional; subject must be non-empty and trimmed" >&2
exit 1
