# clw distribution — model + remaining owner decisions

This document records the `clw` distribution model and the owner-provisioned secrets
the release scaffold (`.github/workflows/release.yml`) needs to run end-to-end.

> **Decision (owner, 2026-06-17) — SUPERSEDES the prior posture.** `clw` is now
> distributed **publicly**: binaries are hosted on a **public mirror repo**
> (`HuGR-Labs/clw-releases`) as GitHub Release assets, installed primarily via a
> **public Homebrew tap** (`brew install hugr-labs/clw/clw`), with the `curl`
> `scripts/install.sh` one-liner as the fallback. This **replaces** the earlier
> "authenticated Cloudflare-R2-only, NOT a public GitHub Release, dedicated
> `CLW_INSTALL_TOKEN`" model recorded in the previous decision (a)/(e).
>
> `clw` remains **proprietary-licensed** (the `EULA.md` governs *use*); it is simply
> publicly *downloadable* — the two are independent (many proprietary CLIs are
> `brew install`-able without being open-source). The private source tree stays in the
> private `corelink-workspaces` repo; only the built release assets are mirrored
> publicly. Full design + risk register: `docs/go-live/DISTRIBUTION-REWORK-PLAN-public-brew.md`.

---

## (a) Distribution model — public mirror + Homebrew tap + install.sh fallback

**Decision (RESOLVED, supersedes the old R2-authenticated model).**

```
  private corelink-workspaces (source) ── release.yml ── build×5 (signed) ──┐
                                                                            ▼
                              publish job → PUBLIC mirror HuGR-Labs/clw-releases
                                ├─ GitHub Release per tag v<version>  (PUBLIC binary host)
                                │    • 5 binaries (mac arm/intel, linux arm/intel, windows .exe)
                                │    • SHA256SUMS + SHA256SUMS.minisig + minisign.pub
                                │    • SBOM + gh attestation (provenance)
                                │    • rc tags ⇒ --prerelease (no formula bump)
                                └─ bump Homebrew formula on REAL tags only
                                     → HuGR-Labs/homebrew-clw : Formula/clw.rb
  consumers:
    • macOS/Linux : brew install hugr-labs/clw/clw        (brew formula → not quarantined)
    • any *nix    : curl … install.sh | sh              (fallback; strips macOS quarantine)
    • Windows     : direct .exe download from the Release (no brew on Windows)
```

- **Binary host = public GitHub Releases on `HuGR-Labs/clw-releases`.** The main
  `corelink-workspaces` repo is **private**, so its own Release assets are not public;
  the mirror repo exists solely to host the public, immutable, per-tag release assets.
  Asset URL: `https://github.com/HuGR-Labs/clw-releases/releases/download/v<version>/<asset>`.
- **Primary install = public Homebrew tap** `HuGR-Labs/homebrew-clw` holding
  `Formula/clw.rb` (4 platforms; Windows is not brew). The release pipeline bumps the
  formula on real tags; each per-binary `sha256` is parsed from the SAME `SHA256SUMS`
  the pipeline computes (single source of truth — never recomputed).
- **Fallback = `scripts/install.sh`** (anonymous `curl | sh`), and **Windows = direct
  `.exe` download**. See `docs/INSTALL.md` for the consumer-facing recipes.
- **No download credential.** The old dedicated `CLW_INSTALL_TOKEN` is **removed** —
  fetching a public release asset needs no auth. (`install.sh` no longer sets or
  requires it.)

**Why this supersedes the old model.** The previous decision (a) chose
"Cloudflare R2 behind a CoreLink auth route (D-12)" with a dedicated download token —
a private/authenticated surface. The owner decided open beta should distribute via a
public Homebrew tap. Public Homebrew requires a public binary host, so D-12 (the R2
auth route) is **no longer on the critical path** for distribution.

---

## (b) Windows code signing — cloud/HSM, not an exportable `.pfx`

**Problem.** "Authenticode" signing — post-2023 CA/Browser-Forum baseline requirements
**forbid exportable code-signing private keys**, so a raw `.pfx` in a GitHub secret is
not obtainable from a compliant CA. Signing must go through a cloud/HSM service.

**Decision: Azure Trusted Signing** (`azure/trusted-signing-action`). Microsoft's
managed signing service; OIDC-friendly, low per-signature cost, integrates with
`signtool` via a dlib. The scaffold's Windows step is written against it and gated on
the Azure credentials.
**Secrets (owner-provisioned):** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
`AZURE_SUBSCRIPTION_ID`, `AZURE_CODE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` (OIDC
preferred over a client secret). This stays last in the signing rollout.

---

## (c) macOS signing — Developer-ID (near-term)

**Problem.** A bare Mach-O CLI binary cannot have a notarization ticket stapled (only
`.app`/`.dmg`/`.pkg` bundles can). For the brew path, a **formula**-installed binary
is **not** Gatekeeper-quarantined, so it currently runs unsigned; but arm64 ad-hoc
binaries can be killed on launch and Homebrew is tightening Gatekeeper handling.

**Decision: Developer-ID signing is near-term** (full notarization can follow). The
brew path is not blocked on it; the **`curl`/manual** path is the gap — a
curl-downloaded binary IS quarantined, so `install.sh` strips `com.apple.quarantine`
after verifying the download, and `docs/INSTALL.md` pushes brew as the primary macOS
path. Developer-ID + notarization closes the manual-download gap.
**Secrets (owner-provisioned, when pulled in):** `MACOS_CERT_P12_BASE64`,
`MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY`, and (for notarization)
`MACOS_NOTARY_KEY_BASE64`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER_ID`.

---

## (d) minisign public-key trust root + rotation

**Problem.** Signed checksums protect consumers only if they obtain the **public key
over a trusted channel** and key **rotation** is defined.

**Decision (trust root).** Commit the minisign **public** key to the repo
(`docs/minisign.pub`) and publish it on the public mirror alongside every release
(`.../v<version>/minisign.pub`). Document its fingerprint in `docs/INSTALL.md` + the
README so a consumer can pin it out-of-band. The signing **secret** key + passphrase
live only as GitHub secrets (`MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD`) and never
persist on a runner after the signing step. The pipeline also publishes a
`gh attestation` provenance statement (SLSA) as the SOTA supply-chain bar, with
minisign as the offline signature-of-record.

**What `brew install` actually verifies — and what the real backstop is.**
`brew install` does **not** itself run minisign. It trusts the tap + mirror: it
downloads the asset and checks it against the `sha256` baked into `Formula/clw.rb`,
where that `sha256` was derived **at formula-bump time** from the minisign-verified
`SHA256SUMS` (single source of truth, never recomputed). So the brew path's integrity
rests on (1) branch-protection rulesets on the mirror + tap repos and (2) that
bump-time chain back to the signed manifest — not on any per-machine signature check.
Independent cryptographic verification is available to the diligent via the documented
`minisign -V` / `gh attestation` path (`docs/INSTALL.md` §4), and **that** is the real
backstop. Do not represent `brew install` as auto-verifying the minisign signature.

**Decision (rotation).** A single active signing key with a documented successor
procedure. On rotation, dual-publish the new public key (on the mirror and in-repo)
**alongside** the old one for at least one release cycle, sign new releases with the
new key, and announce the fingerprint change — never silently swap keys. Treat a
suspected compromise as an incident: rotate, re-sign, revoke trust in the old
fingerprint. See `docs/go-live/RUNBOOK-key-rotation-revocation.md`.

---

## (e) Cross-repo publish credential

**Problem.** The release pipeline runs in the private `corelink-workspaces` repo but
must push assets to the public `clw-releases` mirror and bump the formula in the
public `homebrew-clw` tap. The default `GITHUB_TOKEN` cannot cross repos.

**Decision.** A **deploy key** (repo-bound by construction) on `clw-releases` +
`homebrew-clw` is preferred; a fine-grained PAT scoped to ONLY those repos
(`contents:write`) is acceptable. Stored as a release secret. Provision branch
protection on both public repos **before** wiring the push credential. The formula
bump happens **only after** the Release + all 5 binaries upload and verify
successfully, and **only on real `v<version>` tags** (rc tags publish a `--prerelease`
and do NOT bump the formula).

> The dedicated `CLW_INSTALL_TOKEN` from the old authenticated model is **gone** — the
> download surface is public, so no consumer-side credential exists anymore.

---

## Summary — model + secrets

| # | Topic | Decision | Owner-provisioned secrets / deps |
|---|-------|----------|----------------------------------|
| (a) | Distribution model | **Public mirror `clw-releases` + Homebrew tap `homebrew-clw` + `install.sh` fallback** (supersedes R2-authenticated) | the two public repos + branch protection |
| (b) | Windows signing | Azure Trusted Signing | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_SUBSCRIPTION_ID`, `AZURE_CODE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` |
| (c) | macOS signing | Developer-ID (near-term); brew path unblocked, curl path strips quarantine | `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY` (+ notary keys) |
| (d) | minisign trust root + rotation | Commit `minisign.pub`, dual-publish on rotation; `gh attestation` provenance | `MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD` |
| (e) | Cross-repo publish credential | Deploy key (preferred) / scoped fine-grained PAT to `clw-releases` + `homebrew-clw` | `HOMEBREW_TAP_TOKEN` (or deploy keys) |

These are deployment decisions + secrets the **owner** provisions; the release
scaffold references them but cannot create them. The full work-package breakdown,
paranoid risk register, and dry-run verification strategy live in
`docs/go-live/DISTRIBUTION-REWORK-PLAN-public-brew.md`.
