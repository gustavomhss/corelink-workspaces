#!/usr/bin/env bash
# release-guards.sh — pure, offline-testable decisions used by release.yml.
#
# WP-0A extracted these decisions without changing release policy. `release_kind`
# replaced the workflow's existing tag parser exactly. WP-05 now wires the release-
# file and self-verification decisions into release.yml. The remaining tested seams
# stay reserved for their own policy WPs: WP-04 (tag ancestry) and WP-13b (quota JSON
# parsing). Keeping those calls out of release.yml here avoids anticipating either WP.

set -uo pipefail

_release_guard_fail() {
  printf '::error::%s\n' "$*" >&2
  return 1 # RELEASE_GUARD_STRICT_FAIL_CLOSED
}

_release_guard_warn() {
  printf '::warning::%s\n' "$*" >&2
}

# release_kind <tag>
# Print exactly one of: real | rc | invalid.
#
# This is the ONE place that decides whether a syntactically valid tag is a real
# release or a release candidate. Consumers receive the resulting kind; they must
# never re-parse the tag.
release_kind() {
  local tag="${1:-}"

  if ! printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.?[0-9]+)?$'; then
    printf 'invalid\n'
  elif printf '%s' "$tag" | grep -Eq -- '-rc\.?[0-9]+$'; then
    printf 'rc\n'
  else
    printf 'real\n'
  fi
}

# _require_release_file <kind> <path> <role>
# Missing release files block a real release and remain tolerated for an rc dry-run.
_require_release_file() {
  local kind="${1:-invalid}" path="${2:-}" role="${3:-release file}"

  case "$kind" in
    real|rc) ;;
    *)
      _release_guard_fail "invalid release kind '$kind' while checking ${role} '${path:-empty}'"
      return
      ;;
  esac

  if [ -n "$path" ] && [ -f "$path" ]; then
    return 0
  fi

  case "$kind" in
    real)
      _release_guard_fail "required ${role} '${path:-empty}' is absent; refusing a real release"
      ;;
    rc)
      _release_guard_warn "${role} '${path:-empty}' is absent; rc remains a tolerant dry-run"
      return 0
      ;;
  esac
}

# require_trust_root <kind> <path>
require_trust_root() {
  _require_release_file "${1:-invalid}" "${2:-}" "release trust root"
}

# require_release_file <kind> <path>
# Used for legal files that must ship with a real release (EULA.md and NOTICE).
require_release_file() {
  _require_release_file "${1:-invalid}" "${2:-}" "release file"
}

# require_selfverify_ok <kind> <verification-exit-code>
# Once signature verification runs, a non-zero result is fatal for both real and rc.
# An rc may omit the trust root and skip verification, but it may never turn an
# executed cryptographic failure into a warning; that preserves workflow behavior.
require_selfverify_ok() {
  local kind="${1:-invalid}" rc="${2:-}"

  case "$kind" in
    real|rc) ;;
    *)
      _release_guard_fail "invalid release kind '$kind' after signature self-verification exited ${rc:-empty}"
      return
      ;;
  esac

  case "$rc" in
    0) return 0 ;;
    ''|*[!0-9]*)
      _release_guard_fail "signature self-verification returned an invalid status '${rc:-empty}'"
      return
      ;;
  esac

  _release_guard_fail "signature self-verification failed with exit ${rc}; refusing ${kind} release artifacts"
}

# assert_tag_is_ancestor <tag-commit> <base-ref>
# Fail closed if either commit cannot be resolved, the ancestry check itself errors,
# or the tag commit is not an ancestor. WP-04 decides when release.yml calls this
# (real releases only); this function intentionally has no rc exception of its own.
assert_tag_is_ancestor() {
  local sha="${1:-}" base="${2:-}" base_commit rc

  if [ -z "$sha" ] || ! git cat-file -e "${sha}^{commit}" 2>/dev/null; then
    _release_guard_fail "tag commit '${sha:-empty}' is unreachable; cannot prove release ancestry"
    return
  fi

  if [ -z "$base" ] || ! base_commit="$(git rev-parse --verify "${base}^{commit}" 2>/dev/null)"; then
    _release_guard_fail "release base '${base:-empty}' is unreachable; cannot prove tag ancestry (fail-closed)"
    return
  fi

  if git merge-base --is-ancestor "$sha" "$base_commit"; then
    return 0
  else
    rc=$?
  fi

  if [ "$rc" -eq 1 ]; then
    _release_guard_fail "tag commit '$sha' is not an ancestor of '$base'"
  else
    _release_guard_fail "git could not determine whether tag commit '$sha' is an ancestor of '$base' (exit $rc; fail-closed)"
  fi
}

# parse_quota_body <json>
# Print exactly one of: unmetered | metered | invalid.
# Parsing is structural so whitespace/pretty-printing cannot change the verdict.
# WP-13b replaces release.yml's current one-line grep with this tested seam.
parse_quota_body() {
  local body="${1:-}" verdict

  if ! verdict="$(printf '%s' "$body" | jq -er '
      if .unmetered == true then "unmetered"
      elif .unmetered == false then "metered"
      else "invalid"
      end
    ' 2>/dev/null)"; then
    printf 'invalid\n'
    return 0
  fi

  case "$verdict" in
    unmetered|metered|invalid) printf '%s\n' "$verdict" ;;
    *) printf 'invalid\n' ;;
  esac
}
