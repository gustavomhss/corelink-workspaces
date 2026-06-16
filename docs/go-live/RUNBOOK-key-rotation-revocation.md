# Runbook — signing-key & credential rotation / revocation

> Satisfies exit-gate #10 ("signing keys have a documented rotation+revocation runbook"). Covers every
> long-lived secret clw's release + operation depends on: the code-signing keys, the Linux release
> key, the R2 download credentials, and the CI conformance PAT. Written to be runnable by whoever
> holds the owner role — key-agnostic (it documents the *procedure*, valid before and after the keys
> exist).
>
> **Principle.** Every signing/credential secret has (1) a named owner, (2) a scheduled rotation
> cadence, (3) an emergency-revocation path that can be executed in minutes, and (4) a public,
> versioned trust-root so consumers can distinguish a rotation from a compromise.

---

## Inventory — what we hold and where

| Secret | Used for | Stored in | Cadence | Revoke via |
|---|---|---|---|---|
| Azure Trusted Signing identity | Windows Authenticode | GitHub secrets (OIDC) | 12 mo (cert profile auto-rolls) | Azure portal → revoke cert profile |
| macOS Developer ID cert (`.p12`) | macOS codesign | GitHub secret (base64) | 12 mo or on staff change | Apple Developer → revoke certificate |
| App Store Connect API key | macOS notarization | GitHub secret | 12 mo | App Store Connect → revoke key |
| minisign secret key | Linux artifact signature | GitHub secret (+ passphrase) | 24 mo or on compromise | publish new pubkey + revocation notice |
| R2 access keys | Download-surface upload | GitHub secrets | 6 mo | Cloudflare → roll the access key |
| `CLW_CI_TOKEN` (conformance PAT) | pre-tag conformance gate | GitHub secret | 90 days | corelink-server → revoke the PAT |

> The user **PAT** that an end user configures (`~/.clw/config.toml`) is NOT in this table — its
> rotation is the user-facing `clw auth login` flow (see `docs/usage.md`); revocation is the
> corelink-server tenant console. This runbook is the *operator's* keys.

---

## Scheduled rotation (the calm path)

Do this on the cadence above, or whenever a person with access leaves.

### Code-signing keys (Windows / macOS)
1. Mint the **new** identity alongside the old (Azure cert profile auto-rolls; for Apple, issue a 2nd
   Developer ID cert — Apple allows two active).
2. Update the GitHub secret(s) to the new identity.
3. Cut a throwaway pre-release tag (or run the `release.yml` `build+sign` job on a branch) and
   confirm an artifact signs + verifies under the **new** key.
4. Leave the **old** identity valid for one release cycle (so already-downloaded binaries still
   verify against a still-trusted chain), then revoke it (below).
5. Record the rotation in `docs/DISTRIBUTION.md` trust-root section (date, new key fingerprint).

### minisign (Linux)
minisign has no CA — the trust root **is** the published public key, so a rotation is a coordinated
publish:
1. Generate the new keypair (`minisign -G`).
2. Sign the new **public** key with the old secret key (a signed hand-off) and publish both, plus a
   one-line `MINISIGN-ROTATION.txt` note (old fingerprint → new fingerprint, date, reason).
3. Update the `MINISIGN_SECRET_KEY` secret; sign the next release with the new key.
4. Keep verifying-instructions for BOTH keys in `DISTRIBUTION.md` for one release cycle.

### R2 / conformance PAT
Roll in place: create the new credential, update the GitHub secret, confirm one pipeline run, delete
the old credential. No consumer-facing trust-root, so no hand-off note needed.

---

## Emergency revocation (the compromise path)

**Trigger:** a signing secret leaked (committed, logged, an ex-employee retained access) or a malicious
signed artifact is reported.

Execute in this order — **revoke first, explain second:**

1. **Revoke the key at the source NOW** (the "Revoke via" column): Azure cert profile, Apple cert,
   minisign pubkey retraction, Cloudflare key roll, corelink-server PAT revoke. This invalidates the
   attacker's ability to sign *new* artifacts.
2. **Pull the affected artifacts** from the R2 surface (`releases/clw/<version>/`) so nobody downloads
   a suspect binary.
3. **Publish a security advisory** per `SECURITY.md`: which version(s)/checksums are affected, the
   compromised key fingerprint, and the new trust-root. For minisign, publish a `REVOKED.txt` listing
   the retired public key so a verifier can hard-fail it.
4. **Rotate the key** (the scheduled path above) and **re-sign + re-publish** the last known-good
   release under the new key.
5. **Post-mortem:** how the secret leaked, close the vector (the clw codebase already redacts PATs in
   logs/Debug/errors — confirm the leak wasn't in a place we missed), and shorten that secret's
   cadence if warranted.

**Time budget:** steps 1–2 are minutes (portal + bucket delete); the goal is to make new-artifact
signing impossible before the post is even written.

---

## Verifying a release after any rotation

A consumer must be able to tell a legitimate rotation from an attack:
- Windows: the Authenticode chain validates to the Azure Trusted Signing root.
- macOS: `spctl -a -vv` / notarization ticket validates.
- Linux: `minisign -V` against the **currently published** public key (and NOT against any key listed
  in `REVOKED.txt`).
- All: the **signed** SHA-256 checksums file matches, and its signature verifies under the same trust
  root as the binary.

If any of these fail on a freshly-downloaded artifact, treat it as a potential compromise and follow
the emergency path.
