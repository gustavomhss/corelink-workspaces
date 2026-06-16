# Windows PAT-file ACL — manual runtime verification

> **Why this doc exists.** `clw` stores the plaintext PAT in `%USERPROFILE%\.clw\config.toml`.
> On Unix it is forced to `0600`. On Windows it is restricted to the **current user only** via a
> protected DACL (`restrict_to_owner_windows` → `SetNamedSecurityInfoW`, see
> `crates/clw-cli/src/config.rs`).
>
> That Win32 FFI is **compile-verified** (the gate cross-compiles to `x86_64-pc-windows-gnu`) and
> **cold-reviewed** against the Win32 docs (SID-buffer lifetime, ACL sizing, `WIN32_ERROR` return,
> `PROTECTED_DACL`, 8-aligned buffers). What it has NOT had is a **runtime** check — the CI dev box
> is macOS and there is no Windows CI lane yet (M2.4, billing-gated).
>
> **You do not need to wait for M2.4.** Any real Windows machine (a VM, a friend's PC, a cloud
> Windows box) can confirm it in ~5 minutes with the script below. Run it once and report the
> verdict; that closes the runtime-verification gap.

---

## What "correct" looks like

After `clw init` writes `config.toml`, its ACL must:

1. **Grant the running user** Full control (or at least Read/Write).
2. Grant **no one else** — specifically **not** `Everyone`, **not** `Users`, **not** `Authenticated
   Users`. `SYSTEM` and `Administrators` may still appear (they can read anything regardless); the
   point is that an ordinary *peer* account on the machine cannot read the file.
3. Be **protected** (inheritance disabled) — so a permissive parent-folder ACL cannot re-widen it.

If `clw` could not set the ACL (e.g. the API failed), it prints a `warning:` to stderr and still
writes the file — so **also watch stderr** for an ACL warning during `clw init`.

---

## Verification script (PowerShell)

Run in an **ordinary** (non-elevated) PowerShell on Windows, from a checkout of this repo with a
Rust toolchain installed (`rustup`, MSVC or GNU target):

```powershell
# 1. Build the release binary.
cargo build --release -p clw-cli
$clw = ".\target\release\clw.exe"

# 2. Initialize config with a dummy token (writes %USERPROFILE%\.clw\config.toml).
#    Watch stderr: there must be NO "could not be ACL-restricted" warning.
& $clw init --endpoint "https://example.invalid" --tenant "verify-tenant" --token "dummy-pat-not-real"

$cfg = Join-Path $env:USERPROFILE ".clw\config.toml"
if (-not (Test-Path $cfg)) { Write-Error "config.toml was not created"; exit 1 }

# 3a. Human-readable ACL.
Write-Host "`n=== Get-Acl ===" -ForegroundColor Cyan
Get-Acl $cfg | Format-List

# 3b. icacls (compact).
Write-Host "`n=== icacls ===" -ForegroundColor Cyan
icacls $cfg

# 4. Automated assertions.
Write-Host "`n=== ASSERTIONS ===" -ForegroundColor Cyan
$acl   = Get-Acl $cfg
$me    = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$rules = $acl.Access

$bad = $rules | Where-Object {
    $_.IdentityReference.Value -match '(Everyone|\\Users$|Authenticated Users)'
}
$mine = $rules | Where-Object { $_.IdentityReference.Value -eq $me }

if ($acl.AreAccessRulesProtected) { Write-Host "PASS: DACL is protected (inheritance disabled)" -ForegroundColor Green }
else                              { Write-Host "FAIL: DACL is NOT protected — inherited ACEs may widen access" -ForegroundColor Red }

if ($mine)  { Write-Host "PASS: current user ($me) has an explicit grant" -ForegroundColor Green }
else        { Write-Host "FAIL: current user ($me) has no explicit grant" -ForegroundColor Red }

if (-not $bad) { Write-Host "PASS: no Everyone / Users / Authenticated-Users ACE" -ForegroundColor Green }
else { Write-Host "FAIL: over-broad ACE present:" -ForegroundColor Red; $bad | ForEach-Object { Write-Host "   $($_.IdentityReference.Value): $($_.FileSystemRights)" -ForegroundColor Red } }
```

### Stronger check (a second, peer user) — optional

The above proves the ACL *shape*. To prove a real peer *cannot read it*: create a second standard
user, log in as them, and attempt `Get-Content $env:USERPROFILE\..\<you>\.clw\config.toml` — it must
fail with **Access Denied**. (Skip if you only have one account; the ACE assertions above are the
practical proof.)

---

## Report back

Paste the `=== ASSERTIONS ===` block. Three `PASS` lines + no ACL warning on `clw init` = **runtime
verified**; we mark the Windows ACL closed for v0.1.0 instead of M2.4-tracked.

If anything says `FAIL`, paste the full `Get-Acl` + `icacls` output and the `clw init` stderr — that
is a real bug in `restrict_to_owner_windows` and we fix it immediately.
