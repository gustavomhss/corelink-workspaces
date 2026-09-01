#!/usr/bin/env bash
# workflow-permissions.sh — require an explicit, non-overbroad token policy.
#
# GitHub applies the repository default to a workflow that declares no
# permissions block. That default is an out-of-band setting an administrator can
# widen without a reviewed repository diff. A top-level block makes the policy
# visible here and resets every unmentioned scope to none.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Production callers inspect the repository corpus.  Tests may pass an explicit
# fixture directory, but an inherited environment variable must never be able to
# redirect this guard away from the checked-out workflows.
if [ "$#" -eq 0 ]; then
  WF_DIR="$REPO_ROOT/.github/workflows"
elif [ "$#" -eq 2 ] && [ "$1" = "--workflow-dir" ]; then
  WF_DIR="$2"
else
  echo "usage: $0 [--workflow-dir DIR]" >&2
  exit 2
fi

if [ ! -d "$WF_DIR" ]; then
  echo "GATE FAILED: workflow directory is missing or not a directory: $WF_DIR" >&2
  exit 1
fi

# release.yml is intentionally temporary: WP-08b owns its permission block.
# Keep this named (never a wildcard), so a new workflow cannot silently inherit
# an exemption.
workflow_perms_exempt='release.yml'
missing_perms=""
overbroad_perms=""
ambiguous_perms=""
inspected=0
allowed_scalar_re='^permissions:[[:space:]]*(read-all|'\''read-all'\''|"read-all")[[:space:]]*(#.*)?$'

# GitHub runs both extensions, including dotfiles. nullglob prevents an empty
# pattern from becoming a literal filename; dotglob keeps a hidden workflow from
# escaping the same policy as its visible siblings.
shopt -s nullglob dotglob
workflow_files=("$WF_DIR"/*.yml "$WF_DIR"/*.yaml)
shopt -u nullglob dotglob
for wf in "${workflow_files[@]}"; do
  case " $workflow_perms_exempt " in *" $(basename "$wf") "*) continue ;; esac
  inspected=$((inspected + 1))

  permissions_count="$(grep -c '^permissions:' "$wf" || true)"
  if [ "$permissions_count" -eq 0 ]; then
    missing_perms="${missing_perms}${wf}"$'\n'
    continue
  fi
  if [ "$permissions_count" -ne 1 ]; then
    ambiguous_perms="${ambiguous_perms}${wf}"$'\n'
    continue
  fi

  permissions_line="$(grep '^permissions:' "$wf")"
  # Permit only a block mapping, an explicitly empty mapping, or the exact
  # read-all scalar. This fail-closed spelling whitelist catches YAML aliases,
  # anchors and folded scalars without pretending this shell guard is a YAML
  # parser. Scoped write access belongs in the readable block-mapping form.
  if grep -qE '^permissions:[[:space:]]*(#.*)?$' <<<"$permissions_line" \
    || grep -qE '^permissions:[[:space:]]*\{\}[[:space:]]*(#.*)?$' <<<"$permissions_line" \
    || grep -qE "$allowed_scalar_re" <<<"$permissions_line"; then
    continue
  fi
  if [[ "$permissions_line" == *write-all* ]]; then
    overbroad_perms="${overbroad_perms}${wf}"$'\n'
  else
    ambiguous_perms="${ambiguous_perms}${wf}"$'\n'
  fi
done

if [ "$inspected" -eq 0 ]; then
  echo "GATE FAILED: no non-exempt workflow files found in $WF_DIR" >&2
  exit 1
fi

if [ -n "$missing_perms" ]; then
  echo "GATE FAILED: workflow(s) without a top-level 'permissions:' block —" >&2
  echo "the GITHUB_TOKEN scope must be declared in-tree, not inherited from the repo default:" >&2
  printf '%s' "$missing_perms" >&2
  exit 1
fi

if [ -n "$overbroad_perms" ]; then
  echo "GATE FAILED: workflow(s) declare 'permissions: write-all' —" >&2
  echo "that is wider than the repository's read-all default, so declaring it is a" >&2
  echo "downgrade. Name the scopes the workflow needs, per job:" >&2
  printf '%s' "$overbroad_perms" >&2
  exit 1
fi

if [ -n "$ambiguous_perms" ]; then
  echo "GATE FAILED: workflow(s) have duplicate or unsupported top-level 'permissions:' syntax —" >&2
  echo "use a block mapping, {}, or the exact read-all scalar so review is unambiguous:" >&2
  printf '%s' "$ambiguous_perms" >&2
  exit 1
fi

# Non-fatal retirement nudge: WP-08b removes this exemption atomically with the
# release workflow's new block. Keeping the notice non-fatal avoids failing a
# correct change solely because this temporary list was not retired yet.
for wf in $workflow_perms_exempt; do
  if [ -f "$WF_DIR/$wf" ] && grep -q '^permissions:' "$WF_DIR/$wf"; then
    echo "NOTE: $WF_DIR/$wf now declares 'permissions:' — remove it from workflow_perms_exempt in scripts/workflow-permissions.sh"
  fi
done

echo "workflow-permissions guard: clean (all workflows declare 'permissions:'; exempt: ${workflow_perms_exempt})"
