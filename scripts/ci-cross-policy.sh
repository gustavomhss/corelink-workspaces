#!/usr/bin/env bash
# Validate local cross-check shortcuts, not the identity of an adversarial caller.
# GitHub sets CI and GITHUB_ACTIONS true; CI can be overridden, so inspect both.
# See https://docs.github.com/en/actions/reference/workflows-and-actions/variables
# Empty, false and 0 mean local here; unknown nonempty values refuse the shortcut.
# Only CLW_SKIP_CROSS=1 selects the existing shortcut. Other values still run cross.
# Without that shortcut, this helper does not change the existing lane selection.
assert_ci_cross_policy() {
  [ "${CLW_SKIP_CROSS:-}" = "1" ] || return 0
  local marker value
  for marker in CI GITHUB_ACTIONS; do
    value="${!marker:-}"
    case "$value" in
      ''|false|0) ;;
      *)
        printf 'GATE FAILED: CLW_SKIP_CROSS=1 is local-only; %s marks CI or an unknown context. Unset the override.\n' "$marker" >&2
        return 1
        ;;
    esac
  done
  return 0
}
