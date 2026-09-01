#!/usr/bin/env bash
# release-guards.sh — pure, offline-testable decisions used by release.yml.
#
# WP-0A extracts the decisions without changing release policy. `release_kind` is
# wired immediately because it replaces the workflow's existing tag parser exactly.
# The other functions are tested seams for the follow-on policy WPs that will wire
# them into release.yml: WP-04 (tag ancestry), WP-05 (trust root/self-verification)
# and WP-13b (quota JSON parsing). Keeping those calls out of release.yml here is
# deliberate: merely extracting a guard must not make a previously-tolerated release
# fatal, or vice versa.

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

# require_trust_root <kind> <path>
# A missing trust root blocks a real release and remains tolerated for an rc.
# WP-05 wires this tested decision into release.yml; WP-0A only establishes the seam.
require_trust_root() {
  local kind="${1:-invalid}" path="${2:-}"

  case "$kind" in
    real|rc) ;;
    *)
      _release_guard_fail "invalid release kind '$kind' while checking trust root '${path:-minisign.pub}'"
      return
      ;;
  esac

  if [ -n "$path" ] && [ -f "$path" ]; then
    return 0
  fi

  case "$kind" in
    real)
      _release_guard_fail "required release trust root '${path:-minisign.pub}' is absent (expected minisign.pub); refusing a real release"
      ;;
    rc)
      _release_guard_warn "release trust root '${path:-minisign.pub}' is absent; rc remains a tolerant dry-run"
      return 0
      ;;
  esac
}

# require_selfverify_ok <kind> <verification-exit-code>
# A failed signature self-verification blocks a real release and is tolerated for rc.
# WP-05 wires this into the workflow after the minisign invocation.
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

  case "$kind" in
    real)
      _release_guard_fail "signature self-verification failed with exit ${rc}; refusing a real release"
      ;;
    rc)
      _release_guard_warn "signature self-verification failed with exit ${rc}; rc remains a tolerant dry-run"
      return 0
      ;;
  esac
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
