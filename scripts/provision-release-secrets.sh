#!/usr/bin/env bash
# provision-release-secrets.sh — one-shot, owner-run provisioning of the clw
# release-pipeline secrets. Run this on YOUR machine; secret VALUES never leave
# it (they are piped to `gh secret set` over stdin, never echoed, never passed
# on a command line, never committed).
#
#   bash scripts/provision-release-secrets.sh
#
# What it provisions (idempotent — safe to re-run; re-running overwrites):
#   1. minisign keypair  -> MINISIGN_SECRET_KEY (base64 of the key) + MINISIGN_PASSWORD
#                           and writes docs/minisign.pub (the PUBLIC half, safe to commit)
#   2. CLW_RELEASES_TOKEN -> fine-grained PAT scoped to HuGR-Labs/clw-releases
#   3. HOMEBREW_TAP_TOKEN -> fine-grained PAT scoped to HuGR-Labs/homebrew-clw
#
# It CANNOT mint the PATs for you (that needs the GitHub web UI + your 2FA); it
# walks you through minting each, then reads the pasted value HIDDEN and stores
# it. macOS Developer-ID + Azure signing are deliberately out of scope (deferred).
#
# Encoding is verified against .github/workflows/release.yml:546
#   (`printf '%s' "$MINISIGN_SECRET_KEY" | base64 -d > minisign.key`), so
#   MINISIGN_SECRET_KEY must be the base64 of the raw minisign.key file.
set -euo pipefail

# ---- pretty output (no secret values are ever printed) ----------------------
bold() { printf '\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# ---- preflight --------------------------------------------------------------
bold "clw release-secret provisioning"
command -v gh  >/dev/null 2>&1 || die "gh (GitHub CLI) not found — install it first."
command -v base64 >/dev/null 2>&1 || die "base64 not found."
gh auth status >/dev/null 2>&1 || die "gh not authenticated — run: gh auth login"

# Secrets live on the repo where release.yml runs (the MAIN, private repo).
REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
[ "$REPO" = "HuGR-Labs/corelink-workspaces" ] || \
  warn "secret target repo resolved to '$REPO' (expected HuGR-Labs/corelink-workspaces)"
ok "secrets will be set on: $REPO"

# helper: set a secret from stdin (value never appears in argv or output)
set_secret() { gh secret set "$1" --repo "$REPO"; }   # reads the value from stdin

ask_yn() { # $1=prompt ; returns 0 for yes
  local a; read -r -p "$1 [y/N] " a; [ "$a" = "y" ] || [ "$a" = "Y" ]
}

# =============================================================================
bold $'\n1) minisign signing key'
# -----------------------------------------------------------------------------
if ask_yn "Provision the minisign key + password now?"; then
  command -v minisign >/dev/null 2>&1 || {
    warn "minisign not found — installing via brew"
    command -v brew >/dev/null 2>&1 || die "no brew; install minisign manually then re-run."
    brew install minisign
  }

  KEY_DEFAULT="${HOME}/.config/clw/clw-minisign.key"
  read -r -p "Path to STORE the secret key (kept OFFLINE, never committed) [${KEY_DEFAULT}]: " KEY
  KEY="${KEY:-$KEY_DEFAULT}"
  mkdir -p "$(dirname "$KEY")"

  if [ -f "$KEY" ]; then
    warn "key already exists at $KEY — reusing it (not regenerating)."
  else
    # minisign -G prompts (twice) for the encryption password on the tty.
    # Remember the password you type here — it IS MINISIGN_PASSWORD below.
    minisign -G -p docs/minisign.pub -s "$KEY"
    chmod 600 "$KEY"
    ok "generated $KEY + docs/minisign.pub"
  fi

  # Capture the password, VERIFY it actually unlocks the key by test-signing,
  # then store both secrets. A wrong password is caught here, not at release time.
  TMP_MSG="$(mktemp)"; TMP_SIG="${TMP_MSG}.minisig"
  printf 'clw-provision-selftest\n' > "$TMP_MSG"
  PW=""
  while :; do
    read -r -s -p "Re-enter the SAME minisign password: " PW; echo
    if printf '%s\n' "$PW" | minisign -S -s "$KEY" -m "$TMP_MSG" >/dev/null 2>&1; then
      ok "password verified against the key"
      break
    fi
    warn "that password did not unlock $KEY — try again (Ctrl-C to abort)."
  done
  rm -f "$TMP_MSG" "$TMP_SIG"

  printf '%s' "$PW" | set_secret MINISIGN_PASSWORD
  PW=""; unset PW
  base64 < "$KEY" | set_secret MINISIGN_SECRET_KEY
  ok "MINISIGN_PASSWORD + MINISIGN_SECRET_KEY set on $REPO"
  warn "BACK UP $KEY OFFLINE (password manager / hardware). Losing it = re-key everything."
  warn "docs/minisign.pub was (re)written — commit it on a branch (the PUBLIC half is safe to commit)."
else
  warn "skipped minisign."
fi

# =============================================================================
bold $'\n2) CLW_RELEASES_TOKEN  (publish binaries to the public mirror)'
# -----------------------------------------------------------------------------
provision_pat() { # $1=secret name  $2=repo it must be scoped to
  local name="$1" scope_repo="$2"
  if ask_yn "Provision $name now?"; then
    cat <<EOF
  Mint a FINE-GRAINED PAT (only you can — needs your GitHub login + 2FA):
    1. https://github.com/settings/personal-access-tokens/new
    2. Token name      : ${name}
    3. Resource owner  : HuGR-Labs
    4. Expiration      : 90 days (rotate per docs/go-live/RUNBOOK-key-rotation-revocation.md)
    5. Repository access: Only select repositories -> ${scope_repo}
    6. Permissions      : Repository permissions -> Contents -> Read and write
    7. Generate token  -> copy it
EOF
    read -r -s -p "  Paste the ${name} value (hidden), then Enter: " TOK; echo
    [ -n "$TOK" ] || { warn "empty — skipped $name."; return 0; }
    printf '%s' "$TOK" | set_secret "$name"
    TOK=""; unset TOK
    ok "$name set on $REPO"
  else
    warn "skipped $name."
  fi
}
provision_pat CLW_RELEASES_TOKEN HuGR-Labs/clw-releases

# =============================================================================
bold $'\n3) HOMEBREW_TAP_TOKEN  (bump the formula on real tags)'
# -----------------------------------------------------------------------------
provision_pat HOMEBREW_TAP_TOKEN HuGR-Labs/homebrew-clw

# =============================================================================
bold $'\nDone. Current secrets on '"$REPO"':'
gh secret list --repo "$REPO" || true
cat <<'EOF'

Next:
  • Commit docs/minisign.pub on a branch -> PR (the PUBLIC key is the release trust-root).
  • macOS Developer-ID + Azure signing remain deferred (not provisioned here).
  • With the 3 above set + docs/minisign.pub committed, the FIRST REAL tag (v0.1.0)
    will sign + publish to the mirror + bump the brew formula end-to-end.
EOF
