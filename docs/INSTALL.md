# Installing and verifying `clw`

This is the consumer-facing install + verification runbook for the `clw` client. It
covers the five supported targets, how to verify a signature/checksum on each
platform, where to put the binary on `$PATH`, and how to upgrade or uninstall.

`clw` is **proprietary**, commercial-licensed software (see `EULA.md`). It is **not**
published to a public GitHub Release, crates.io, Homebrew, or any package manager.
Binaries ship from an **authenticated download surface** (Cloudflare R2 behind a
CoreLink auth route — `docs/DISTRIBUTION.md` decision (a)). You must present a
download credential to fetch them; see [Authenticated download](#authenticated-download).

> Verifying the signature is **not optional**. `clw` carries a tenant Personal Access
> Token and snapshots your workspace — run only a binary whose signature and checksum
> you have verified against the published trust root.

---

## 1. Supported targets (the install matrix)

| OS | Arch | Target triple | Asset name | Signing / verify |
|----|------|---------------|------------|------------------|
| macOS (Apple Silicon) | arm64 | `aarch64-apple-darwin` | `clw-<version>-aarch64-apple-darwin.zip` | Developer ID + notarized zip → `spctl` |
| macOS (Intel) | x86_64 | `x86_64-apple-darwin` | `clw-<version>-x86_64-apple-darwin.zip` | Developer ID + notarized zip → `spctl` |
| Windows | x86_64 | `x86_64-pc-windows-gnu` | `clw-<version>-x86_64-pc-windows-gnu.exe` | Azure Trusted Signing (Authenticode) → `Get-AuthenticodeSignature` |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `clw-<version>-x86_64-unknown-linux-gnu` | minisign detached sig → `minisign -V` |
| Linux | arm64 | `aarch64-unknown-linux-gnu` | `clw-<version>-aarch64-unknown-linux-gnu` | minisign detached sig → `minisign -V` |

Notes:
- The Windows and both Linux legs are **cross-compiled on the macOS build host** via
  `cargo-zigbuild`; the `aarch64-unknown-linux-gnu` leg is build-only on that host
  (it cannot be runtime-tested there). See `docs/DISTRIBUTION.md`.
- macOS ships a **notarized `.zip`** (a bare Mach-O CLI cannot have a notarization
  ticket stapled). Windows and Linux ship the bare binary plus a detached signature.
- Every release also publishes a single **signed checksum manifest**: `SHA256SUMS`
  and its minisign signature `SHA256SUMS.minisig`, covering all assets.

---

## 2. Authenticated download

The download surface is private. Obtain a **`CLW_INSTALL_TOKEN`** from the
signup/dashboard flow (a narrowly-scoped, download-only token — `docs/DISTRIBUTION.md`
decision (e)), then fetch the asset for your target plus its signature and the
checksum manifest. Replace `<version>` and `<target>` from the matrix above:

```sh
export CLW_INSTALL_TOKEN=...   # from the dashboard / signup flow

BASE="https://<authenticated-download-host>/releases/clw/<version>"
ASSET="clw-<version>-<target>"          # add .zip for macOS, .exe for Windows

# Download the artifact, its signature, and the signed checksum manifest:
curl -fSL -H "Authorization: Bearer $CLW_INSTALL_TOKEN" -o "$ASSET"            "$BASE/$ASSET"
curl -fSL -H "Authorization: Bearer $CLW_INSTALL_TOKEN" -o "SHA256SUMS"        "$BASE/SHA256SUMS"
curl -fSL -H "Authorization: Bearer $CLW_INSTALL_TOKEN" -o "SHA256SUMS.minisig" "$BASE/SHA256SUMS.minisig"
# Linux only also fetch the per-asset minisign signature:
curl -fSL -H "Authorization: Bearer $CLW_INSTALL_TOKEN" -o "$ASSET.minisig"    "$BASE/$ASSET.minisig"
```

> The concrete download host is filled in once the R2-behind-CoreLink route (server
> dependency **D-12**) is provisioned; see `docs/go-live/LAUNCH-CHECKLIST.md` §A.

---

## 3. Verify the download (per platform)

Always verify **both** the platform signature **and** the checksum manifest.

### 3a. The minisign public key (Linux + the checksum manifest)

The Linux signatures and the `SHA256SUMS` manifest are signed with **minisign**. The
public key is published as `docs/minisign.pub` in the source tree and alongside every
release on the download surface. Pin its fingerprint out of band:

```sh
# Inspect the published key; compare its fingerprint to the one in this repo / README
# out-of-band before trusting it.
cat minisign.pub
```

> **Trust-root fingerprint:** _(filled in when the owner provisions the minisign key —
> `docs/go-live/LAUNCH-CHECKLIST.md` §A and `docs/DISTRIBUTION.md` decision (d). Until
> then `docs/minisign.pub` is not yet committed.)_ On key rotation, the old and new
> public keys are dual-published for one release cycle; see
> `docs/go-live/RUNBOOK-key-rotation-revocation.md`.

### 3b. Verify the signed checksum manifest (all platforms)

```sh
# 1. Verify the manifest signature against the published public key:
minisign -V -p minisign.pub -m SHA256SUMS

# 2. Verify your asset's checksum is the one the signed manifest attests:
#    (Linux: sha256sum; macOS: shasum -a 256)
sha256sum --ignore-missing -c SHA256SUMS    # Linux
shasum -a 256 -c SHA256SUMS                 # macOS (drop entries you didn't download)
```

A failure on either step means **do not run the binary** — treat it as a potential
compromise and follow `docs/go-live/RUNBOOK-key-rotation-revocation.md`.

### 3c. macOS — notarization

```sh
unzip "clw-<version>-<target>.zip"
# Gatekeeper assessment (online, first-run). 'accepted'/'Notarized Developer ID' = good:
spctl -a -t open --context context:primary-signature -vv "clw-<version>-<target>"
codesign --verify --deep --strict --verbose=2 "clw-<version>-<target>"
```

### 3d. Windows — Authenticode

```powershell
# 'Valid' status, signed via the Azure Trusted Signing certificate, is required:
Get-AuthenticodeSignature .\clw-<version>-x86_64-pc-windows-gnu.exe | Format-List
```

### 3e. Linux — minisign detached signature

```sh
# Verify the binary itself against the published public key:
minisign -V -p minisign.pub -m "clw-<version>-<target>"
```

---

## 4. Install on `$PATH`

`clw` is a single self-contained binary. Put it on `$PATH` and make it executable.

### macOS / Linux

```sh
chmod +x "clw-<version>-<target>"
sudo install -m 0755 "clw-<version>-<target>" /usr/local/bin/clw
# Or, no sudo (per-user) — ensure ~/.local/bin is on PATH:
install -m 0755 "clw-<version>-<target>" "$HOME/.local/bin/clw"
```

Make the per-user dir persistent on `$PATH`, per shell:

```sh
# bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
# zsh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
# fish
fish_add_path "$HOME/.local/bin"
```

Then restart the shell (or `source` the rc file).

### Windows (PowerShell)

```powershell
$dest = "$env:LOCALAPPDATA\Programs\clw"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item .\clw-<version>-x86_64-pc-windows-gnu.exe "$dest\clw.exe"
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

## 5. Upgrade

Upgrading is "verify the new asset, then replace the binary in place" — there is no
in-binary self-update. Repeat sections 2–3 for the new `<version>`, then overwrite the
installed binary:

```sh
# macOS / Linux
sudo install -m 0755 "clw-<new-version>-<target>" /usr/local/bin/clw
clw --version    # confirm the new version
```

```powershell
# Windows
Copy-Item .\clw-<new-version>-x86_64-pc-windows-gnu.exe "$env:LOCALAPPDATA\Programs\clw\clw.exe" -Force
```

Your `~/.clw` config + PAT + cache are preserved across an upgrade.

---

## 6. Uninstall

`clw` removes its own local footprint, then tells you where the binary is so you can
delete it (a running process cannot delete its own executable):

```sh
clw uninstall            # interactive; or --yes to skip the prompt, --dry-run to preview
```

This deletes the `~/.clw` directory (config + PAT + cache) and prints the binary path
to remove manually. See `docs/usage.md` (`clw uninstall`) and the DPA's local-client-data
clause (`docs/DPA-subprocessors.md` §9.3). Then remove the binary:

```sh
# macOS / Linux
sudo rm /usr/local/bin/clw      # or: rm ~/.local/bin/clw
```

```powershell
# Windows
Remove-Item "$env:LOCALAPPDATA\Programs\clw\clw.exe"
```

To erase your **server-side** data (GDPR Art. 17 / LGPD Art. 18), run `clw erase`
**before** uninstalling — uninstall removes only the local footprint, not stored
Customer Data. See `docs/usage.md` (`clw erase`).

---

## 7. Build from source (internal / development only)

If you have authenticated access to the source tree you can build directly instead of
downloading a signed artifact:

```sh
cargo build --release -p clw-cli
./target/release/clw --version
```

The toolchain is pinned in `rust-toolchain.toml` (rustfmt + clippy components install
automatically on first build). Source builds are **unsigned** and intended for
development only — production installs should use the signed, verified release assets
above. See the README "Building from source" section for details.
