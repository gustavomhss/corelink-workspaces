# Disaster recovery & business continuity — `clw` release pipeline + client data

> **DRAFT — owner/legal review required.** Engineering draft for roadmap M6.7 to
> de-risk lead time. **Not** a ratified runbook; the contact tree, RTO/RPO
> targets, and the customer-notification legal language are **owner to confirm**.
> Cross-references: `docs/ROADMAP-go-live.md` (M3 release pipeline, M6.6 key
> lifecycle), `docs/DISTRIBUTION.md`, `SECURITY.md`, `docs/THREAT-MODEL.md`.

This covers two distinct continuity domains: (A) the **release pipeline** that
produces and distributes the signed `clw` binary, and (B) **client data
durability** expectations.

---

## A. Release-pipeline disaster recovery

The release pipeline is `.github/workflows/release.yml` (M3): tag → build all
targets → **sign** (macOS Developer ID + notarize; Windows via cloud/HSM signing;
Linux minisign/sigstore) → signed SHA-256 checksums → publish to the authenticated
download surface (`docs/DISTRIBUTION.md`).

### A.1 Loss or compromise of the signing key(s)

This is the highest-severity pipeline event: signing keys are the trust root for
every distributed binary. Keys in scope: macOS Developer ID, Windows
Authenticode (cloud/HSM — **not** an exportable `.pfx`), and the Linux
minisign/sigstore key. Key rotation/revocation has a runbook owner in roadmap
**M6.6**; this is the **DR** half (loss/compromise response).

**On suspected compromise:**

1. **Contain.** Revoke the affected certificate/key with the issuing authority
   (Apple / Authenticode CA / minisign keyring). For HSM/cloud keys, rotate the
   credential and lock the signing identity.
2. **Re-key.** Generate a new key in the HSM/cloud signing service (no exportable
   private material). Publish the **new minisign public key / trust root** through
   the documented out-of-band channel (`docs/DISTRIBUTION.md` M3.2 trust-root +
   rotation).
3. **Re-sign.** Rebuild reproducibly (`SOURCE_DATE_EPOCH` + pinned image, M3.4) and
   re-sign all supported, still-current releases with the new key; publish new
   **signed** checksums.
4. **Customer notification.** Notify customers that prior-key signatures are
   revoked, provide the new trust root + fingerprint, and instruct re-verification
   / re-install. Treat as a security incident under `SECURITY.md` (coordinated,
   private first if a vuln is implicated). **Notification content/timing is owner
   to confirm.**
5. **Post-incident.** Record root cause; tighten key custody (CI secret scope, HSM
   access policy).

**On loss (not compromise):** same re-key / re-sign / re-publish-trust-root path,
without revoking for compromise reasons, but old artifacts become unverifiable
once the key is gone — so re-sign current releases and notify.

**Preventive posture:** keys live in HSM/cloud signing (non-exportable); CI holds
only scoped credentials, never raw private keys; the `SIGNUP_TOKEN_KEY`
write-only-secret **backup rule** is a tracked dep (**D-2**, server-owned) — the
same "back up write-only secrets" discipline applies to any recoverable signing
material.

### A.2 Loss of the authenticated download surface

The download surface is an owner decision (R2-behind-CoreLink-auth → possible dep
**D-12**, private GitHub Release behind SSO, or a vendor — `docs/DISTRIBUTION.md`).

- **Mitigation:** keep release artifacts + signed checksums reproducible and
  re-publishable from CI (rebuild from tag) so the surface can be restored or
  **failed over** to an alternate host without losing artifacts.
- If R2-behind-CoreLink (D-12): a CoreLink/Cloudflare outage takes downloads
  offline but **not** existing installs (the CLI keeps working). Document an
  alternate publish target.
- Verification stays intact across a surface move because trust rests on the
  signature + checksum, not the host.

### A.3 CI / org continuity

- **No single-founder-machine dependency.** The pipeline must run on
  GitHub-hosted (or redundant) runners, not a self-hosted founder Mac (roadmap
  exit-gate #11 / M2.4). A self-hosted dependency is itself a continuity risk.
- **Source/tag recovery:** the repo + tags are the source of truth; reproducible
  builds (M3.4) mean any release can be regenerated from a tag on fresh infra.
- **Account/org loss:** document recovery for the GitHub org, the signing
  accounts (Apple Developer / Authenticode CA), and the Cloudflare account
  (owner: maintain a break-glass access list + recovery contacts — to confirm).
- **CI secrets:** re-provisionable from the secret manager; rotate on any
  suspected exposure.

---

## B. Client data-durability expectations

### B.1 Local cache — ephemeral & reconstructible

`~/.clw/cache` holds plaintext chunk contents at rest (`0700`,
`docs/THREAT-MODEL.md`). It is a **performance cache**, not a system of record:

- **Loss is non-fatal.** Any cache miss is re-fetched from CAS (every chunk is
  BLAKE3-verified on fetch — `SECURITY.md`). Deleting the cache only costs
  re-download.
- Cache-format versioning + safe invalidation/migration (never silent stale
  replay) is roadmap M5.4.
- **No DR backup of the cache is needed or promised.**

### B.2 Server-side (R2) durability — a server-side commitment to confirm

Uploaded contents and manifests live in **Cloudflare R2** (CAS) via the CoreLink
API (`docs/DPA-subprocessors.md`). The durability/availability of that data is a
**server-side responsibility**, not the client's:

- `clw` relies on R2/CoreLink for the persistence of snapshotted data. The
  **snapshot durability / backup SLA is a server-side commitment that must be
  confirmed by the CoreLink/server owner** — and, if a specific backup/restore SLA
  is promised to customers, **scheduled as a cross-repo dependency** (server-owned,
  via the dependency ledger in `docs/ROADMAP-go-live.md`).
- Cloudflare's own R2 durability guarantees and any cross-region replication are
  owner to document and attach to the DPA.
- Until that confirmation exists, this document must not state a customer-facing
  data-durability SLA. The client guarantees **integrity** (BLAKE3) but not
  server-side **durability**.

---

## C. Targets (placeholders — owner to set)

| Event | RTO (target) | RPO (target) |
|---|---|---|
| Signing-key compromise → re-key + re-sign + notify | owner | n/a (no data loss) |
| Download-surface outage → failover/restore | owner | n/a (artifacts reproducible) |
| CI/org loss → rebuild from tag on fresh infra | owner | n/a (tags are source of truth) |
| Server-side (R2) data loss | server-owned | server-owned — **confirm** |
