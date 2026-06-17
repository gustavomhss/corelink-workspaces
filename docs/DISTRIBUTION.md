# clw distribution — open OWNER DECISIONS

This document records the open **owner decisions** that the M3 release scaffold
(`.github/workflows/release.yml`) needs before it can run end-to-end. The
workflow is a correct skeleton; the steps that depend on these decisions (and the
secrets they imply) are marked `TODO(OWNER)` in the YAML and are guarded so the
pipeline is dry-runnable without them.

Each decision below states the problem, the options considered, and a
**recommended default**. The default is what the scaffold assumes today; an owner
can override it. Once a decision is made, fill the matching `TODO(OWNER)` block in
`release.yml`, provision the secret(s), and delete the dry-run guard.

Locked context (from `docs/go-live/WP-BRIEFS.md`, M2.3 / M2.5):

- Proprietary product: the download surface is **not** a public GitHub Release.
- Five locked targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-pc-windows-gnu` (cross-compiled via `cargo-zigbuild` on the Mac host),
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` (the Linux and Windows
  legs are cross-compiled; `aarch64-unknown-linux-gnu` is build-only).
- Signed releases with signed SHA-256 checksums.

---

## (a) Authenticated download surface host

**Problem.** The roadmap says "authenticated proprietary download surface" but
names no concrete host. The publish job and the M2.5 installer both need a fixed
URL + auth mechanism + asset-naming convention. This is the single biggest
unblocker for the pipeline.

**Options.**

1. **Cloudflare R2 behind a CoreLink auth route.** Artifacts live in a private R2
   bucket under `releases/clw/<version>/`; a new authenticated `corelink-server`
   route (call it **D-12**, not yet in the D-1..D-6 ledger) issues short-lived
   signed download URLs (or proxies the download) after validating a token.
   Consistent with the existing CoreLink/R2 stack. Cost: one new server route +
   R2 bucket. Adds a cross-repo dependency the roadmap did not track.
2. **Private GitHub Release behind org SSO.** Zero new infra; relies on GitHub
   org membership + SSO for access control. But it couples distribution to every
   consumer having a GitHub account in the org, which is awkward for paying
   customers and weak as a "proprietary, licensed" boundary.
3. **A distribution vendor (Cloudsmith / JFrog Artifactory).** Turnkey
   authenticated artifact hosting with tokens and per-OS layouts. Fastest to
   stand up; recurring vendor cost and a new third party in the supply chain.

**Recommended default: (1) R2 behind a CoreLink auth route (D-12).** Stack
consistency and a clean licensed-download boundary outweigh the new-route cost.
The scaffold's publish step defaults to an R2 S3-compatible upload under
`releases/clw/<version>/` and is gated on R2 credentials.
**Tracked dependency:** D-12 (new `corelink-server` authenticated download
route). This must be filed; it is not in the existing D-1..D-6 ledger.

---

## (b) Windows code signing — cloud/HSM, not an exportable `.pfx`

**Problem.** The roadmap says "Authenticode," but post-2023 CA/Browser-Forum
baseline requirements **forbid exportable code-signing private keys**. A raw
`.pfx` in a GitHub secret is no longer obtainable from a reputable CA. Signing
must therefore go through a cloud/HSM-backed signing service. This is a
procurement/cost item, not just a config choice.

**Options.**

1. **Azure Trusted Signing** (`azure/trusted-signing-action`). Microsoft's
   managed signing service; OIDC-friendly, low per-signature cost, integrates
   with `signtool` via a dlib. Requires an Azure tenant + a Trusted Signing
   account + identity validation.
2. **DigiCert KeyLocker.** HSM-backed cloud signing via a `signtool` dlib (the
   `smctl`/PKCS#11 path). Mature, vendor-managed; higher cost; another vendor
   relationship.
3. *(Non-option)* Exportable `.pfx` in a secret — **rejected**: not obtainable
   from a compliant CA post-2023.

**Recommended default: (1) Azure Trusted Signing.** Lowest cost, OIDC fits GitHub
Actions cleanly, and avoids a second signing vendor. The scaffold's Windows step
is written against Azure Trusted Signing and is gated on the Azure credentials.
**Secrets required (owner-provisioned):** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
`AZURE_SUBSCRIPTION_ID`, `AZURE_CODE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`
(OIDC preferred over a client secret).

---

## (c) macOS notarization artifact format — `.pkg`/`.dmg` or notarized zip

**Problem.** A bare Mach-O CLI binary **cannot be `xcrun stapler staple`d** — only
bundles (`.app`), `.dmg`, and `.pkg` accept a stapled notarization ticket. So
"sign + notarize" hides a real format decision: what do we submit to notarization
and what do consumers verify?

**Options.**

1. **Notarized zip.** `codesign` the bare binary, zip it, submit the zip to
   `notarytool`. The ticket is **not** stapled to the binary; consumers rely on
   an online Gatekeeper assessment (`spctl`) the first time they run it. Simplest;
   works for a curl-pipe install; offline machines can't re-verify the staple.
2. **Notarized + stapled `.pkg`.** Wrap the binary in a signed `.pkg` installer,
   notarize it, and `stapler staple` the ticket onto the `.pkg`. Cleanest install
   story (ties into M2.5), staple works offline, but adds an installer-build step
   and changes the install UX from "drop a binary on PATH" to "run a pkg."

**Recommended default: (1) notarized zip** for this scaffold, since a bare-binary,
PATH-based install is the current model. Revisit `.pkg` if M2.5 decides on a
guided installer — the scaffold's macOS step notes the `.pkg` branch as an
alternative. **Couple this decision with M2.5** (install story) so the consumer
verification recipe in `docs/INSTALL.md` matches the chosen format.

---

## (d) minisign public-key trust root + rotation

**Problem.** Signed checksums and Linux signatures only protect consumers if they
obtain the **public key over a trusted channel**. Without a pinned, documented
trust root, a signature is theater. Key **rotation** is also unaddressed.

**Decision (trust root).**

- Commit the minisign **public** key to the repo (e.g. `minisign.pub` at repo
  root) and publish it on the authenticated download surface alongside every
  release. Document its raw fingerprint in `docs/INSTALL.md` and the README so a
  consumer can pin it out-of-band. The signing **secret** key + passphrase live
  only as GitHub secrets (`MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD`) and are
  never on a runner after the signing step.
- sigstore/cosign keyless is the documented alternative (no key to distribute,
  but adds an OIDC/transparency-log dependency). **Default: minisign** —
  deterministic and dependency-light.

**Decision (rotation).**

- **Recommended default:** a single active signing key with a documented
  successor procedure. On rotation, publish the new public key on the surface and
  in-repo for at least one release cycle **alongside** the old one (dual-publish),
  sign new releases with the new key, and announce the fingerprint change.
  Never silently swap keys. Treat a suspected key compromise as an incident:
  rotate immediately, re-sign current artifacts, and revoke trust in the old
  fingerprint in the docs.

---

## (e) install-token vs tenant-PAT bootstrap

**Problem.** The download surface is authenticated, so the M2.5 installer must
present a credential — but the roadmap never says **which** credential, nor how a
brand-new user obtains it **before** they have `clw` configured. This blocks a
clean first-run story.

**Options.**

1. **Reuse the tenant PAT.** The installer sends the same Personal Access Token
   the CLI already uses (`Authorization: Bearer`). Zero new credential type, but a
   long-lived high-scope token now travels on every download request and must
   exist before install — chicken-and-egg for a first-time user.
2. **Dedicated install/download token.** A separate, narrowly-scoped,
   short-lived token issued by the signup/dashboard flow specifically for
   downloading binaries (`CLW_INSTALL_TOKEN`). Decouples "download the software"
   from "use the API," limits blast radius, and the signup flow can mint it at
   account creation so a new user has it before first run.

**Recommended default: (2) a dedicated `CLW_INSTALL_TOKEN`** minted by the
signup/dashboard flow, scoped to release downloads only. The M2.5 installer reads
it from the `CLW_INSTALL_TOKEN` env var and fails with an actionable error if
absent. **Dependency:** the signup/dashboard flow must issue this token at
account creation — coordinate with whoever owns signup so the bootstrap UX is
defined (this is the M2.5 auth-bootstrap loose end).

---

## Summary — decisions and their secrets

| # | Decision | Recommended default | Owner-provisioned secrets / deps |
|---|----------|---------------------|----------------------------------|
| (a) | Download surface host | R2 behind a CoreLink auth route (**D-12**) | `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_ENDPOINT`, `R2_BUCKET`; new server route |
| (b) | Windows signing | Azure Trusted Signing | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_SUBSCRIPTION_ID`, `AZURE_CODE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` |
| (c) | macOS notarize format | Notarized zip (revisit `.pkg` with M2.5) | `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY`, `MACOS_NOTARY_KEY_BASE64`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER_ID` |
| (d) | minisign trust root + rotation | Commit `minisign.pub`, dual-publish on rotation | `MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD` |
| (e) | Install credential | Dedicated `CLW_INSTALL_TOKEN` from signup | signup/dashboard must mint the token |

These are deployment decisions and secrets the **owner** must make/provision; the
release scaffold references them but cannot create them.
