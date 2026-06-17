#!/bin/sh
# install.sh — clw one-line authenticated installer (macOS / Linux).
#
# Usage (one-liner):
#   curl --proto '=https' --tlsv1.2 -fsSL \
#     -H "Authorization: Bearer $CLW_INSTALL_TOKEN" \
#     "https://<download-surface>/install.sh" | CLW_VERSION=<version> sh
#
# The download surface is PROPRIETARY and AUTHENTICATED (see docs/DISTRIBUTION.md
# decisions (a) and (e)). This script presents a dedicated, narrowly-scoped
# download token (CLW_INSTALL_TOKEN) on every request; it is NEVER echoed.
#
# What it does:
#   - detects OS (uname -s) + arch (uname -m) and maps to the release target
#     triple naming from .github/workflows/release.yml (clw-<version>-<target>);
#   - downloads the binary, the signed SHA256SUMS, its minisign signature, and
#     the minisign trust-root public key;
#   - verifies the minisign detached signature over SHA256SUMS FIRST, then the
#     binary's SHA-256 checksum (hard fail on mismatch; NEVER skippable);
#   - installs to a no-sudo per-user prefix ($HOME/.local/bin by default),
#     idempotently (re-running upgrades in place);
#   - prints a PATH + `clw --version` next-step message.
#
# Windows is OUT OF SCOPE for this POSIX shell installer — see docs/INSTALL.md
# for the manual Windows recipe.
#
# Contract (kept in lockstep with release.yml / docs/DISTRIBUTION.md):
#   asset basename : clw-<version>-<target>           (no ext on macOS/Linux)
#   checksums      : SHA256SUMS            (one `<hex>  <file>` line per artifact)
#   signature      : SHA256SUMS.minisig    (minisign detached over SHA256SUMS)
#   trust root     : minisign.pub          (committed + published; pin its fingerprint)
#   layout         : <base-url>/releases/clw/<version>/<file>
#
# NOTE on versioning: the release pipeline publishes ONLY under
#   releases/clw/<concrete-version>/  (the version from preflight.outputs.version).
# There is NO `latest/` alias on the surface, so CLW_VERSION is REQUIRED — there
# is no safe default to fall back to. (Verify: .github/workflows/release.yml
# publish step + docs/DISTRIBUTION.md.)
#
# Overridable via env (CLW_VERSION + CLW_INSTALL_TOKEN are REQUIRED):
#   CLW_INSTALL_TOKEN  REQUIRED — dedicated download token (never printed).
#   CLW_VERSION        REQUIRED — concrete version to install (e.g. 0.1.0). No
#                      default: the surface has no `latest` alias to resolve.
#   CLW_BASE_URL       download surface base URL (no trailing slash).
#   CLW_INSTALL_DIR    install prefix (default: $HOME/.local/bin).
#   CLW_SKIP_SIGNATURE if "1", proceed when minisign is unavailable instead of
#                      hard-failing. Checksum verification is NEVER skippable.

set -eu

# --- constants ---------------------------------------------------------------
# [OWNER: confirm surface — DISTRIBUTION.md (a) / D-12]
# The download surface is an OPEN owner decision (it may become a Homebrew tap
# with this install.sh as the fallback). This is a PLACEHOLDER, not a real URL —
# override it with CLW_BASE_URL until the surface is confirmed.
DEFAULT_BASE_URL="https://downloads.example.invalid"   # [OWNER: confirm surface — D-12]

# --- cleanup trap (runs on any exit, incl. set -e abort) ---------------------
TMPDIR_INSTALL=""
cleanup() {
  [ -n "$TMPDIR_INSTALL" ] && rm -rf "$TMPDIR_INSTALL" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- helpers -----------------------------------------------------------------
err() { printf 'clw-install: error: %s\n' "$*" >&2; }
die() { err "$*"; exit 1; }
info() { printf 'clw-install: %s\n' "$*"; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

# sha256sums_hex_for <asset> <sumsfile>
# Print the leading hex digest for <asset> from a `<hex>  <file>` SHA256SUMS
# file, matched LITERALLY (a shell `case` glob, not a regex) so a version's '.'
# chars cannot act as wildcards and an unrelated entry cannot mask a mismatch.
# Defined at top level (not inline in a `$()`), which keeps the `case` arm's `)`
# out of a command substitution — some POSIX shells (e.g. bash 3.2) misparse it
# there. Prints nothing (empty) when there is no exact match.
sha256sums_hex_for() {
  _asset="$1"; _sums="$2"
  while IFS= read -r _line || [ -n "$_line" ]; do
    case "$_line" in
      *"  ${_asset}")
        printf '%s' "${_line%% *}"   # leading hex digest, up to the first space
        return 0
        ;;
    esac
  done < "$_sums"
  return 0
}

# --- preflight: required tooling --------------------------------------------
need_cmd uname
need_cmd mkdir
need_cmd chmod
need_cmd mv

# A downloader: prefer curl, fall back to wget. Auth is passed via header so it
# never appears in the URL (and never in `ps` output).
DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then
  DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then
  DOWNLOADER="wget"
else
  die "need curl or wget to download the binary"
fi

# A SHA-256 checker: sha256sum (Linux) or shasum (macOS) — both emit the same
# `<hex>  <file>` format that SHA256SUMS uses.
SHA_TOOL=""
if command -v sha256sum >/dev/null 2>&1; then
  SHA_TOOL="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  SHA_TOOL="shasum -a 256"
else
  die "need sha256sum or shasum to verify the download"
fi

# --- auth token (REQUIRED; never echoed) ------------------------------------
# Validate presence WITHOUT ever printing the value.
if [ -z "${CLW_INSTALL_TOKEN:-}" ]; then
  die "CLW_INSTALL_TOKEN is not set.
  The clw download surface is authenticated. Set a dedicated download token
  (minted by signup/dashboard — see docs/DISTRIBUTION.md (e)) and re-run:
      export CLW_INSTALL_TOKEN=<your-token>
  The token is sent only as an Authorization header and is never printed."
fi

# --- version (REQUIRED; no `latest` alias exists on the surface) ------------
# The release pipeline publishes ONLY under releases/clw/<concrete-version>/.
# There is no `latest/` pointer to resolve, so we refuse to guess a default.
if [ -z "${CLW_VERSION:-}" ]; then
  die "CLW_VERSION is not set.
  The clw download surface publishes each release under a CONCRETE version
  (releases/clw/<version>/) and has NO 'latest' alias, so a version is required.
  Set the version you want and re-run, e.g.:
      export CLW_VERSION=0.1.0
  (See the project release notes / docs/INSTALL.md for the current version.)"
fi

# --- detect OS + arch, map to the release target triple ----------------------
UNAME_S="$(uname -s)"
UNAME_M="$(uname -m)"

case "$UNAME_S" in
  Darwin) os="apple-darwin" ;;
  Linux)  os="unknown-linux-gnu" ;;
  *)
    die "unsupported OS: '$UNAME_S'. This installer supports macOS and Linux only;
  see docs/INSTALL.md for the manual Windows recipe."
    ;;
esac

case "$UNAME_M" in
  arm64|aarch64) arch="aarch64" ;;
  x86_64|amd64)  arch="x86_64" ;;
  *)
    die "unsupported architecture: '$UNAME_M'. Supported: arm64/aarch64, x86_64."
    ;;
esac

TARGET="${arch}-${os}"

# --- resolve config ----------------------------------------------------------
BASE_URL="${CLW_BASE_URL:-$DEFAULT_BASE_URL}"
BASE_URL="${BASE_URL%/}"   # strip any trailing slash
VERSION="$CLW_VERSION"     # required; validated above
INSTALL_DIR="${CLW_INSTALL_DIR:-$HOME/.local/bin}"

ASSET="clw-${VERSION}-${TARGET}"
REL_DIR="${BASE_URL}/releases/clw/${VERSION}"

info "target:   ${TARGET}"
info "version:  ${VERSION}"
info "asset:    ${ASSET}"
info "install:  ${INSTALL_DIR}/clw"

# --- workspace ---------------------------------------------------------------
TMPDIR_INSTALL="$(mktemp -d 2>/dev/null || mktemp -d -t clw-install)"
[ -d "$TMPDIR_INSTALL" ] || die "could not create a temp directory"

# --- download (auth header; token never on the command line as a URL) --------
# fetch <url> <dest>
fetch() {
  _url="$1"; _dest="$2"
  if [ "$DOWNLOADER" = "curl" ]; then
    # --proto '=https' --tlsv1.2: refuse plaintext/downgrade. -f: fail on HTTP errors.
    curl --proto '=https' --tlsv1.2 -fsSL \
      -H "Authorization: Bearer ${CLW_INSTALL_TOKEN}" \
      -H "User-Agent: clw-install/${VERSION}" \
      -o "$_dest" "$_url"
  else
    # wget passes the header the same way; --https-only refuses plaintext.
    wget -q --https-only \
      --header="Authorization: Bearer ${CLW_INSTALL_TOKEN}" \
      --header="User-Agent: clw-install/${VERSION}" \
      -O "$_dest" "$_url"
  fi
}

info "downloading binary + checksums + signature..."
fetch "${REL_DIR}/${ASSET}"            "${TMPDIR_INSTALL}/${ASSET}" \
  || die "failed to download ${ASSET} (check token, version, and network)"
fetch "${REL_DIR}/SHA256SUMS"          "${TMPDIR_INSTALL}/SHA256SUMS" \
  || die "failed to download SHA256SUMS"
fetch "${REL_DIR}/SHA256SUMS.minisig"  "${TMPDIR_INSTALL}/SHA256SUMS.minisig" \
  || die "failed to download SHA256SUMS.minisig"
fetch "${REL_DIR}/minisign.pub"        "${TMPDIR_INSTALL}/minisign.pub" \
  || die "failed to download minisign.pub (trust root)"

# --- verify signature over the checksum file (before trusting the checksums) -
if command -v minisign >/dev/null 2>&1; then
  info "verifying minisign signature over SHA256SUMS..."
  ( cd "$TMPDIR_INSTALL" && minisign -V -p minisign.pub -m SHA256SUMS ) \
    || die "minisign signature verification FAILED — refusing to install."
  info "signature OK (trust root: minisign.pub)"
else
  if [ "${CLW_SKIP_SIGNATURE:-}" = "1" ]; then
    info "WARNING: minisign not installed and CLW_SKIP_SIGNATURE=1 — proceeding on checksum only."
    info "         Install minisign and re-run to verify the signed trust chain."
  else
    die "minisign is not installed, so the signed checksum file cannot be verified.
  Install minisign (brew install minisign / apt-get install minisign) and re-run,
  or set CLW_SKIP_SIGNATURE=1 to proceed on the SHA-256 checksum alone (NOT recommended)."
  fi
fi

# --- verify the SHA-256 checksum of the binary (NEVER skippable) -------------
info "verifying SHA-256 checksum..."
# SHA256SUMS lists every published file by its basename in `<hex>  <file>` form.
# Pull our asset's digest with a LITERAL match (see sha256sums_hex_for) so a
# version's '.' chars cannot act as regex wildcards and an unrelated entry cannot
# mask a mismatch, then rebuild a clean one-entry checksum file. (Defense-in-depth:
# the minisign signature over SHA256SUMS is already verified above; this guards
# the basename match itself.)
EXPECTED_HEX="$(sha256sums_hex_for "$ASSET" "${TMPDIR_INSTALL}/SHA256SUMS")"
[ -n "$EXPECTED_HEX" ] || die "SHA256SUMS has no entry for ${ASSET} — refusing to install."

# Run the checker from the temp dir so the bare basename resolves. Rebuild the
# checksum line ourselves (clean hex + two spaces + literal basename).
printf '%s  %s\n' "$EXPECTED_HEX" "$ASSET" > "${TMPDIR_INSTALL}/.expected.sha256"
( cd "$TMPDIR_INSTALL" && $SHA_TOOL -c .expected.sha256 >/dev/null 2>&1 ) \
  || die "SHA-256 checksum MISMATCH for ${ASSET} — the download is corrupt or tampered. NOT installing."
info "checksum OK"

# --- install (idempotent, no sudo) ------------------------------------------
mkdir -p "$INSTALL_DIR" || die "could not create install dir: $INSTALL_DIR"
chmod 0755 "${TMPDIR_INSTALL}/${ASSET}"
# Atomic-ish replace: mv over any existing binary upgrades in place.
mv -f "${TMPDIR_INSTALL}/${ASSET}" "${INSTALL_DIR}/clw" \
  || die "could not install to ${INSTALL_DIR}/clw"

info "installed clw to ${INSTALL_DIR}/clw"

# --- post-install message ----------------------------------------------------
printf '\n'
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    info "${INSTALL_DIR} is already on your PATH."
    ;;
  *)
    info "NOTE: ${INSTALL_DIR} is not on your PATH. Add it, e.g.:"
    # The literal $PATH is intentional — we print a line the user pastes verbatim.
    # shellcheck disable=SC2016
    printf '    export PATH="%s:$PATH"\n' "$INSTALL_DIR"
    info "(put that line in your shell profile — see docs/INSTALL.md for per-shell detail)."
    ;;
esac
printf '\n'
info "verify with:  clw --version"
