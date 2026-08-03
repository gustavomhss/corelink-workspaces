# Installing and verifying `clw`

This is the consumer-facing install + verification runbook for the `clw` client. It
covers the supported targets, the preferred install path (Homebrew), the `curl`
fallback, the direct Windows download, how to verify a signature/checksum on each
platform, and how to upgrade, pin, or uninstall.

`clw` is **proprietary**, commercial-licensed software (see `EULA.md`) — but it is
**publicly downloadable**: binaries are published as GitHub Release assets on the
public mirror repo **`HuGR-Labs/clw-releases`**, and macOS/Linux install via a
public Homebrew tap. The license governs *use*; the download itself needs no
credential. (See `docs/DISTRIBUTION.md`.)

> Verifying the signature is **not optional**. `clw` carries a tenant Personal Access
> Token and snapshots your workspace — run only a binary whose signature and checksum
> you have verified against the published trust root. Homebrew verifies the per-asset
> SHA-256 for you; for the `curl` and Windows paths, verify per
> [§4](#4-verify-the-download-per-platform).

---

## 1. Quick start

### macOS / Linux — Homebrew (preferred)

```sh
brew install hugr-labs/clw/clw
clw --version
```

> **macOS — a raw browser download is Gatekeeper-blocked (the binary is not yet
> Apple-notarized).** `brew` and the `curl | sh` installer below both strip the
> quarantine attribute for you, so prefer those. If you download the `*-apple-darwin`
> asset directly from the Releases page and macOS says *"cannot be opened because the
> developer cannot be verified"*, clear the quarantine after verifying the signature
> ([§4](#4-verify-the-download-per-platform)):
>
> ```sh
> xattr -d com.apple.quarantine ./clw-*-apple-darwin   # or: right-click → Open
> ```
> (Developer-ID signing + notarization is planned post-launch — this recovery is only
> needed for the direct-download path.)

That's it — brew resolves the right binary for your OS/arch, verifies its SHA-256,
and (on macOS) installs a **formula** binary that is **not** Gatekeeper-quarantined,
so it runs with no "unidentified developer" prompt. **Prefer brew on macOS** for
exactly this reason (the `curl` path below downloads a quarantined binary and has to
strip the attribute itself).

### macOS / Linux — `curl` one-liner (fallback)

Use this only where Homebrew is unavailable. `CLW_VERSION` is required (the mirror has
no `latest` alias):

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  "https://raw.githubusercontent.com/HuGR-Labs/clw-releases/main/install.sh" \
  | CLW_VERSION=<version> sh
```

Homebrew is the preferred path; this `curl` fallback verifies the minisign signature
against the embedded trust-root (§4a) before installing. `install.sh` downloads the
binary + the signed `SHA256SUMS` (+ `.minisig`) + `minisign.pub`, verifies the minisign
signature over the checksum manifest, then the binary's SHA-256 (**never skippable**),
strips the macOS quarantine attribute, and installs to `$HOME/.local/bin/clw`. See the
[macOS quarantine note](#3-macos-quarantine-curl-path-only).

### Windows — direct `.exe` download

There is no Homebrew on Windows; download the `.exe` directly from the mirror release
and verify it (see [§4d](#4d-windows--authenticode)):

```powershell
$ver = "<version>"
$base = "https://github.com/HuGR-Labs/clw-releases/releases/download/v$ver"
Invoke-WebRequest "$base/clw-$ver-x86_64-pc-windows-gnu.exe" -OutFile "clw.exe"
Invoke-WebRequest "$base/SHA256SUMS"          -OutFile "SHA256SUMS"
Invoke-WebRequest "$base/SHA256SUMS.minisig"  -OutFile "SHA256SUMS.minisig"
Invoke-WebRequest "$base/minisign.pub"        -OutFile "minisign.pub"
```

Then install it on `PATH` per [§5 Windows](#windows-powershell-1).

---

## 2. Supported targets (the install matrix)

| OS | Arch | Target triple | Asset name | Signing / verify |
|----|------|---------------|------------|------------------|
| macOS (Apple Silicon) | arm64 | `aarch64-apple-darwin` | `clw-<version>-aarch64-apple-darwin` | minisign over `SHA256SUMS` (Developer-ID near-term) |
| macOS (Intel) | x86_64 | `x86_64-apple-darwin` | `clw-<version>-x86_64-apple-darwin` | minisign over `SHA256SUMS` (Developer-ID near-term) |
| Windows | x86_64 | `x86_64-pc-windows-gnu` | `clw-<version>-x86_64-pc-windows-gnu.exe` | Azure Trusted Signing (Authenticode) → `Get-AuthenticodeSignature` |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `clw-<version>-x86_64-unknown-linux-gnu` | minisign over `SHA256SUMS` |
| Linux | arm64 | `aarch64-unknown-linux-gnu` | `clw-<version>-aarch64-unknown-linux-gnu` | minisign over `SHA256SUMS` |

All assets live at:

```
https://github.com/HuGR-Labs/clw-releases/releases/download/v<version>/<asset>
```

Notes:
- The Windows and both Linux legs are **cross-compiled on the macOS build host** via
  `cargo-zigbuild`; the `aarch64-unknown-linux-gnu` leg is build-only on that host
  (it cannot be runtime-tested there). See `docs/DISTRIBUTION.md`.
- Every release publishes a single **signed checksum manifest** — `SHA256SUMS` and its
  minisign signature `SHA256SUMS.minisig` — plus the trust-root key `minisign.pub`,
  covering all assets. (A `gh attestation` provenance statement rides along too.)
- The macOS and Linux binaries ship as bare binaries (no `.zip`); Homebrew is the
  recommended way to get them on macOS/Linux.

---

## 3. macOS quarantine (curl path only)

A binary fetched with `curl`/`wget`/a browser is tagged `com.apple.quarantine` by
macOS, so Gatekeeper blocks it on first run. **A Homebrew *formula* install is not
quarantined** — which is why `brew install hugr-labs/clw/clw` is the preferred
macOS path and needs no extra step.

The `curl` `install.sh` handles this for you: after verifying the download it runs
`xattr -d com.apple.quarantine` on the binary. If you download a macOS binary manually
(not via brew, not via `install.sh`), strip it yourself before running:

```sh
xattr -d com.apple.quarantine ./clw 2>/dev/null || true
```

(Full Developer-ID signing/notarization, which closes this gap for manual downloads
too, is near-term — see `docs/DISTRIBUTION.md`.)

---

## 4. Verify the download (per platform)

Homebrew already verifies the per-asset SHA-256 from the formula, and `install.sh`
verifies automatically. For the **Windows** and any **manual** download, verify both
the checksum manifest **and**, where applicable, the platform signature.

### 4a. The minisign public key (trust root)

`SHA256SUMS` is signed with **minisign**. The public key **is committed** to the
source tree as `docs/minisign.pub` **and** published alongside every release on the
public mirror (`.../v<version>/minisign.pub`) as the trust root — v0.1.0 and later are
signed against it. Pin its fingerprint out of band:

```sh
# Inspect the published key; compare its fingerprint to the one below / in docs/minisign.pub
# out-of-band before trusting it.
cat minisign.pub
```

> **Trust-root fingerprint (the committed `docs/minisign.pub`):**
>
> ```
> minisign key id: 4B57B8B54A0E396D
> RWRtOQ5KtbhXSxt24bM/T+ndWfjUDUm8c7gr8194pxnyWEUS/CTDIkm0
> ```
>
> On key rotation, the old and new public keys are dual-published for one release
> cycle; see `docs/go-live/RUNBOOK-key-rotation-revocation.md`.

Verify `SHA256SUMS` directly against this trust-root key:

```sh
minisign -V -P RWRtOQ5KtbhXSxt24bM/T+ndWfjUDUm8c7gr8194pxnyWEUS/CTDIkm0 \
  -m SHA256SUMS
# (equivalently: minisign -V -p minisign.pub -m SHA256SUMS — see §4b)
```

### 4b. Verify the signed checksum manifest (all platforms)

```sh
# 1. Verify the manifest signature against the published public key:
minisign -V -p minisign.pub -m SHA256SUMS

# 2. Verify your asset's checksum is the one the signed manifest attests:
#    (Linux: sha256sum; macOS: shasum -a 256)
sha256sum --ignore-missing -c SHA256SUMS      # Linux
shasum -a 256 --ignore-missing -c SHA256SUMS  # macOS
```

A failure on either step means **do not run the binary** — treat it as a potential
compromise and follow `docs/go-live/RUNBOOK-key-rotation-revocation.md`.

> Optional supply-chain check (any platform, needs the `gh` CLI): verify the build
> provenance attestation published with the release:
>
> ```sh
> gh attestation verify clw-<version>-<target> --repo HuGR-Labs/clw-releases
> ```

### 4c. macOS — Gatekeeper / signature

Once Developer-ID signing is live you can assess the binary with Gatekeeper:

```sh
spctl -a -t open --context context:primary-signature -vv ./clw   # 'accepted' when signed
codesign --verify --deep --strict --verbose=2 ./clw
```

Until then the macOS binaries are verified via minisign-over-`SHA256SUMS` (§4b); the
brew path runs them unquarantined, and the `curl`/manual path strips quarantine (§3).

### 4d. Windows — Authenticode

```powershell
# 'Valid' status, signed via the Azure Trusted Signing certificate, is required:
Get-AuthenticodeSignature .\clw.exe | Format-List
```

Also verify the checksum manifest (PowerShell):

```powershell
(Get-FileHash .\clw.exe -Algorithm SHA256).Hash.ToLower()
# compare against the clw-<version>-x86_64-pc-windows-gnu.exe line in SHA256SUMS
```

---

## 5. Install on `$PATH` (manual / Windows)

Homebrew and `install.sh` put `clw` on `PATH` for you. For a manual download:

### macOS / Linux

```sh
chmod +x "clw-<version>-<target>"
# macOS manual download only — strip quarantine first (see §3):
xattr -d com.apple.quarantine "clw-<version>-<target>" 2>/dev/null || true
install -m 0755 "clw-<version>-<target>" "$HOME/.local/bin/clw"   # no sudo
# or system-wide: sudo install -m 0755 "clw-<version>-<target>" /usr/local/bin/clw
```

Make `~/.local/bin` persistent on `$PATH`, per shell:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc   # bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc    # zsh
fish_add_path "$HOME/.local/bin"                           # fish
```

Then restart the shell (or `source` the rc file).

### Windows (PowerShell)

```powershell
$dest = "$env:LOCALAPPDATA\Programs\clw"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item .\clw.exe "$dest\clw.exe"
# Add to the per-user PATH (persists across sessions):
[Environment]::SetEnvironmentVariable(
  "Path", [Environment]::GetEnvironmentVariable("Path","User") + ";$dest", "User")
```

Open a new terminal so the updated `PATH` takes effect.

### Confirm + first-run

```sh
clw --version
clw doctor            # redacted diagnostics; confirm green (see docs/usage.md)
clw auth login        # configure the tenant PAT (see docs/usage.md)
```

`clw doctor` printing a green, redacted diagnostics bundle confirms a healthy install.

> **Shell completions** (optional) are generated per shell — `bash`, `zsh`, `fish`,
> `powershell`, `elvish`. Install them per your shell's convention (see `docs/usage.md`).

---

## 6. Upgrade, pin, and uninstall (Homebrew)

### Upgrade

```sh
brew update
brew upgrade clw
clw --version
```

### Install latest and pin

The tap publishes a single formula (`Formula/clw.rb`) tracking the latest release —
there are no versioned `clw@<version>` formulae. Install the latest, then pin to stop
a future `brew upgrade` from moving it:

```sh
brew install hugr-labs/clw/clw   # installs the latest release
brew pin clw                        # hold this version; `brew unpin clw` to allow upgrades
```

To roll back / move off a pinned version and re-track the latest:

```sh
brew unpin clw 2>/dev/null || true
brew uninstall clw
brew install hugr-labs/clw/clw
```

### Uninstall

```sh
brew uninstall clw
```

For the `curl`/manual install, remove the binary directly:

```sh
rm ~/.local/bin/clw        # or: sudo rm /usr/local/bin/clw
```

```powershell
Remove-Item "$env:LOCALAPPDATA\Programs\clw\clw.exe"   # Windows
```

### Manual / non-brew upgrade

Re-run `brew upgrade`, or for the `curl` path re-run `install.sh` with the new
`CLW_VERSION` (it verifies and replaces the binary in place). There is no in-binary
self-update.

> Removing the binary leaves your local `~/.clw` config + PAT + cache in place. To
> clear the **local** footprint, run `clw uninstall` first (deletes `~/.clw`; see
> `docs/usage.md` and the DPA local-client-data clause, `docs/DPA-subprocessors.md`
> §9.3). To erase **server-side** data (GDPR Art. 17 / LGPD Art. 18), run `clw erase`
> **before** uninstalling — uninstall removes only the local footprint, not stored
> Customer Data.

---

## 7. Build from source (internal / development only)

If you have access to the (private) source tree you can build directly instead of
downloading a signed artifact:

```sh
cargo build --release -p clw-cli
./target/release/clw --version
```

The toolchain is pinned in `rust-toolchain.toml` (rustfmt + clippy components install
automatically on first build). Source builds are **unsigned** and intended for
development only — production installs should use Homebrew or the signed, verified
release assets above. See the README "Building from source" section for details.
