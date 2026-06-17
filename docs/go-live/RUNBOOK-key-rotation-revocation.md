# Runbook — signing-key & credential rotation / revocation

> Satisfies exit-gate #10 ("signing keys have a documented rotation+revocation runbook"). Covers every
> long-lived secret clw's release + operation depends on: the code-signing keys, the Linux release
> key, the release-mirror + Homebrew-tap push tokens, and the CI conformance PAT. Written to be runnable by whoever
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
| minisign secret key | Linux artifact signature | GitHub secret `MINISIGN_SECRET_KEY` (passphrase as `MINISIGN_PASSWORD`) | 24 mo or on compromise | publish new pubkey + revocation notice |
| `CLW_RELEASES_TOKEN` | Publish the GitHub Release on the public mirror `clw-releases` | GitHub secret (deploy-key / fine-grained PAT, scoped to that repo, contents:write) | 6 mo or on compromise | regenerate the deploy key / PAT on `clw-releases` |
| `HOMEBREW_TAP_TOKEN` | Push the formula bump to the tap `homebrew-clw` | GitHub secret (deploy-key / fine-grained PAT, scoped to that repo, contents:write) | 6 mo or on compromise | regenerate the deploy key / PAT on `homebrew-clw` |
| `CLW_CI_TOKEN` (conformance PAT) | pre-tag conformance gate | GitHub secret | 90 days | corelink-server → revoke the PAT |
| `CLW_CI_TENANT` (conformance tenant id) | pre-tag conformance gate (identifies the CI tenant the PAT is scoped to) | GitHub secret | with `CLW_CI_TOKEN` (90 days) | corelink-server → retire the CI tenant / re-scope the PAT |
| `CLW_CI_ENDPOINT` (conformance API endpoint) | pre-tag conformance gate (target host) | GitHub secret | on endpoint/region change | update to the new host; no consumer trust-root |

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

### Release-mirror / tap tokens / conformance PAT
Roll in place: create the new credential (regenerate the deploy key / fine-grained PAT for
`clw-releases` or `homebrew-clw`, or the conformance PAT), update the GitHub secret, confirm one
pipeline run, delete the old credential. No consumer-facing trust-root, so no hand-off note needed.

---

## Emergency revocation (the compromise path)

**Trigger:** a signing secret leaked (committed, logged, an ex-employee retained access) or a malicious
signed artifact is reported.

Execute in this order — **revoke first, explain second:**

1. **Revoke the key at the source NOW** (the "Revoke via" column): Azure cert profile, Apple cert,
   minisign pubkey retraction, regenerate the release-mirror / tap push tokens, corelink-server PAT
   revoke. This invalidates the attacker's ability to sign *new* artifacts or push a malicious formula.
2. **Pull the affected artifacts:** draft/delete the bad GitHub Release on the public mirror
   `clw-releases` and revert the formula bump on `homebrew-clw` so nobody downloads a suspect binary
   (see *Bad-release rollback* below).
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

---

## Bad-release rollback (Homebrew tap + public mirror)

A published release asset is **immutable by URL** and the Homebrew formula may already be bumped + users
may have cached the (now-bad) `sha256` in `~/Library/Caches/Homebrew`. **Never reuse a tag or asset URL**
to "fix" a bad release — a different binary at the same URL is a `sha256` mismatch for everyone who cached
it. Procedure:

1. **Stop the bleed.** On `humangr-labs/clw-releases`, mark the bad GitHub Release as a **draft** (or
   delete it) so new `brew install`s can't fetch it. `gh release edit vX.Y.Z --repo humangr-labs/clw-releases --draft`.
2. **Yank the formula.** On `humangr-labs/homebrew-clw`, **revert** the formula-bump commit (point
   `Formula/clw.rb` back at the last-good `vX.Y.(Z-1)`). `brew update` then serves the good version again.
3. **Re-release forward.** Fix the defect, **bump to a NEW patch** `vX.Y.(Z+1)` (never re-cut the bad
   tag), tag → the pipeline publishes the new Release + re-bumps the formula. Users get the fix via
   `brew upgrade clw`.
4. **Caches.** Users who already installed the bad version recover with `brew update && brew upgrade clw`;
   a `sha256` mismatch on the *withdrawn* version is expected and harmless once the formula points forward.
5. **If the cause is key/credential compromise** (signing key, `CLW_RELEASES_TOKEN`, `HOMEBREW_TAP_TOKEN`),
   ALSO run the rotation/revocation path above before re-releasing.
