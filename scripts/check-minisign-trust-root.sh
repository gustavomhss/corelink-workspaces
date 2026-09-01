#!/usr/bin/env bash
# check-minisign-trust-root.sh — keep the published public key tied to install.sh.
#
# The installer deliberately verifies releases with its embedded public-key pin, not
# with a key downloaded from the public mirror. docs/minisign.pub is nevertheless a
# consumer-facing copy of that same trust root. A drift between the two creates two
# incompatible verification stories, so reject it before a docs-only gate can exit.
#
# This parser intentionally accepts one narrow, auditable installer spelling instead
# of sourcing install.sh: executing an installer merely to read its constant would
# make this guard run arbitrary installer code. The constrained value is a minisign
# public-key line: exactly 56 base64 characters (no whitespace or CR).
set -euo pipefail

repo_root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
if [ "$#" -gt 1 ] || [ ! -d "$repo_root" ]; then
  printf 'GATE FAILED: minisign trust-root guard needs one repository root at most\n' >&2
  exit 1
fi

pubkey_file="$repo_root/docs/minisign.pub"
installer="$repo_root/scripts/install.sh"

fail() {
  printf 'GATE FAILED: minisign trust-root guard: %s\n' "$*" >&2
  exit 1
}

[ -f "$pubkey_file" ] || fail "docs/minisign.pub is absent (fail-closed)"
[ -f "$installer" ] || fail "scripts/install.sh is absent (fail-closed)"

# A minisign .pub file is exactly its comment and public-key line, both
# newline-terminated. `wc -l` therefore also rejects a truncated second line.
line_count="$(wc -l < "$pubkey_file")"
[ "$line_count" -eq 2 ] || fail "docs/minisign.pub is malformed: expected exactly two newline-terminated lines, found $line_count"

pubkey_comment="$(sed -n '1p' "$pubkey_file")"
pubkey_line="$(sed -n '2p' "$pubkey_file")"
[[ "$pubkey_comment" =~ ^untrusted\ comment:\ minisign\ public\ key\ [0-9A-F]{16}$ ]] \
  || fail "docs/minisign.pub is malformed: line 1 is not a minisign public-key comment"
[[ "$pubkey_line" =~ ^[A-Za-z0-9+/]{56}$ ]] \
  || fail "docs/minisign.pub is malformed: line 2 is not a 56-character base64 public-key line"

# Count every assignment-shaped line first, so a duplicate never becomes an ambiguous
# "first match wins" pin. We parse neither shell expansions nor comments as a key.
assignment_count="$(grep -c '^CLW_MINISIGN_PUBKEY_LINE=' "$installer" || true)"
case "$assignment_count" in
  0) fail "CLW_MINISIGN_PUBKEY_LINE is absent from scripts/install.sh (fail-closed)" ;;
  1) ;;
  *) fail "CLW_MINISIGN_PUBKEY_LINE appears $assignment_count times in scripts/install.sh; expected exactly one" ;;
esac

installer_assignment="$(grep '^CLW_MINISIGN_PUBKEY_LINE=' "$installer")"
if [[ ! "$installer_assignment" =~ ^CLW_MINISIGN_PUBKEY_LINE=\"([A-Za-z0-9+/]{56})\"$ ]]; then
  fail "CLW_MINISIGN_PUBKEY_LINE in scripts/install.sh is malformed"
fi
installer_key="${BASH_REMATCH[1]}"

# cmp is deliberately used instead of a normalized/text comparison: line 2 must be
# byte-for-byte the key that install.sh materializes for minisign.
if ! cmp -s <(printf '%s\n' "$installer_key") <(sed -n '2p' "$pubkey_file"); then
  fail "docs/minisign.pub line 2 does not byte-match CLW_MINISIGN_PUBKEY_LINE in scripts/install.sh"
fi

printf 'minisign trust-root guard: clean (docs/minisign.pub line 2 byte-matches install.sh)\n'
