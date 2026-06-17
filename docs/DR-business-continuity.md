# Disaster Recovery & Business Continuity Policy — CoreLink Workspaces

**Product:** CoreLink Workspaces (CLI binary `clw`) and the hosted CoreLink API (`corelink-api.humangr.com`)
**Publisher:** Human Guardrail LTDA, a limited-liability company (sociedade empresária limitada) organized under the laws of Brazil ("HumanGR," "we," "us," or "our"), with registered office at Avenida Rouxinol, 857, apt. 113, Moema, São Paulo – SP, Brazil (CEP: 04516-001); company registration number (CNPJ): 66.103.400/0001-30
**Document owner:** Gustavo Schneiter — Head of Engineering / CEO (accountable owner)
**Classification:** Internal operational policy with customer-facing commitments (Sections 4, 9, and 11)
**Version:** 1.0
**Effective date:** June 14, 2026
**Last reviewed / last updated:** June 14, 2026
**Next scheduled review:** annually and after any major incident; next due June 14, 2027

> **Completion gate — not yet final for customer-facing use.** This policy is not final
> and must not be published or relied on as a customer-facing commitment until the
> following owner-only items are populated and cross-checked for mutual consistency:
> (1) every RTO/RPO target in Section 4, including confirmation of whether any target is
> a contractual SLA; (2) the Domain B backup mechanism, cadence, and retention in
> Section 5.2, confirmed to actually support the RPO targets entered in Section 4;
> (3) the testing-cadence frequencies in Section 11 (the recommended minimums are
> indicated inline); and (4) the sub-processor contingency / accepted-residual-risk
> determination in Section 8.3. The compliance qualifications in Sections 5.2, 8.1, 8.2,
> and 8.4 (regional-residency enforcement gated on roadmap M7.1) must remain consistent
> with `docs/DPA-subprocessors.md`; if M7.1 is in production at the effective date, update
> the corresponding sections to reflect that and remove the forward-looking qualifier.
> Full tenant erasure via `clw erase` is LIVE (M6.4 closed 2026-06-15) and §8.4 reflects that.

---

## 1. Purpose and scope

### 1.1 Purpose

This policy defines how HumanGR prepares for, responds to, and recovers from events
that disrupt the availability or integrity of (a) the release pipeline that builds,
signs, and distributes the proprietary `clw` binary, and (b) the hosted, multi-tenant
CoreLink API and the customer data it stores. It sets out our Recovery Time Objectives
(RTO) and Recovery Point Objectives (RPO), our backup and failover strategy, our
incident-response and communication procedures, and the cadence at which we test the
plan.

### 1.2 Scope

This policy covers two distinct continuity domains:

- **Domain A — Release pipeline.** The systems that produce and distribute the signed
  `clw` binaries for macOS (Intel and Apple Silicon), Windows, and Linux: source
  control, the build/sign/notarize pipeline, the code-signing trust roots, and the
  authenticated download surface.
- **Domain B — Hosted service and customer data.** The CoreLink API, its
  authentication subsystem (per-tenant Bearer Personal Access Tokens, "PATs"), the
  content-addressed store ("CAS") that holds snapshotted workspace content, and the
  associated manifests and metadata.

It also addresses the local CLI cache that resides on customer machines, to the extent
necessary to state what is and is not protected by this policy.

### 1.3 Out of scope

- Customer-side infrastructure, customer networks, and the integrity of customer
  machines on which `clw` is installed.
- Routine change management, capacity planning, and feature roadmap, except where they
  bear directly on recovery capability.
- Information-security controls generally, which are governed by `SECURITY.md` and the
  threat model in `docs/THREAT-MODEL.md`; this policy references but does not supersede
  them.

### 1.4 Related documents

This policy is read together with: `SECURITY.md` (vulnerability handling and incident
classification), `docs/THREAT-MODEL.md`, `docs/DISTRIBUTION.md` (release distribution
and trust-root rotation), `docs/DPA-subprocessors.md` (sub-processors and data
locations), and the Master Services Agreement / Terms of Service and Data Processing
Agreement between HumanGR and the customer.

---

## 2. Definitions

- **RTO (Recovery Time Objective):** the maximum targeted period between a disruptive
  event being declared and restoration of the affected service or capability.
- **RPO (Recovery Point Objective):** the maximum targeted period of data loss measured
  as the age of the last recoverable copy preceding an event.
- **Disaster:** an unplanned event that causes, or is reasonably likely to cause, an
  outage or data-integrity impact exceeding the thresholds in Section 6.
- **CAS:** the content-addressed store of snapshotted workspace content and manifests,
  using BLAKE3 content digests and FastCDC chunking.
- **Trust root:** the code-signing keys and published public keys/fingerprints by which
  customers verify the authenticity of a distributed `clw` binary.
- **Region:** the customer-selected geographic region in which that customer's stored
  data resides.
- **Sub-processor:** a third party engaged by HumanGR to process customer data on our
  behalf, as enumerated in `docs/DPA-subprocessors.md`.

---

## 3. Roles and responsibilities

| Role | Responsibility | Holder |
|---|---|---|
| **Incident Commander (IC)** | Declares a disaster, owns the response, authorizes failover and recovery actions, and approves customer communications. | Gustavo Schneiter — Head of Engineering / CEO (sole operator during incubation; deputy: same) |
| **Communications Lead** | Drafts and issues customer, sub-processor, and (where required) regulator notifications; maintains the status channel. | Gustavo Schneiter — Head of Engineering / CEO |
| **Engineering / Recovery Lead** | Executes technical recovery (rebuild, re-sign, failover, restore). | Gustavo Schneiter — Head of Engineering / CEO |
| **Security Lead** | Handles compromise containment, key revocation/rotation, and forensic preservation. | Gustavo Schneiter — Head of Engineering / CEO (security/abuse: security@humangr.com) |
| **Data Protection Officer / Privacy contact** | Assesses personal-data impact and breach-notification obligations. | Gustavo Schneiter — LGPD Encarregado / DPO — privacy@humangr.com |
| **Executive sponsor** | Final escalation, authorizes extraordinary spend and break-glass access. | Gustavo Schneiter — Head of Engineering / CEO |

A current contact tree, including out-of-hours contacts, escalation order, and
break-glass access list, is maintained by the accountable owner (Gustavo Schneiter —
legal@humangr.com), who is the sole operator during incubation, and reviewed on the
cadence in Section 11. No role above may be held by a single individual without a named
deputy; during incubation a single operator holds all roles and is his own deputy, a
posture to be revisited as the team grows.

---

## 4. Recovery objectives (RTO / RPO)

The following targets are the commitments governed by this policy. Where a target is
also offered contractually as a Service Level, the controlling figures are those in the
applicable SLA; this table must not be read to enlarge or reduce a contractual SLA.

| Event | RTO (target) | RPO (target) |
|---|---|---|
| Code-signing key compromise → revoke, re-key, re-sign, re-publish trust root, notify | 24 hours | Not applicable — no customer data loss; existing installs continue to function. |
| Download-surface outage → failover to alternate host / restore | 24 hours | Not applicable — release artifacts and signed checksums are reproducible from tags. |
| CI / source-control / org loss → rebuild from tag on fresh infrastructure | 24 hours | Not applicable — Git tags are the source of truth. |
| CoreLink API regional outage (no data loss) → restore service in-region | 24 hours | 24 hours |
| Loss of CAS data in a region → restore from backup/replica | 24 hours | 24 hours |
| Loss of auth/metadata datastore → restore from backup | 24 hours | 24 hours |

These RTO/RPO targets are internal recovery objectives. None of them is expressed as a
contractual SLA percentage or response time in the customer agreement; any binding
service level, if agreed, is set out separately in writing per Section 12.

These objectives are **targets**, not warranties, except to the extent a binding SLA is
separately agreed in writing with a customer. Nothing in this policy creates a service
level beyond what the applicable customer agreement provides.

---

## 5. Backup strategy

### 5.1 Domain A — release artifacts and source

- **Source and tags are the system of record.** The Git repository and its release tags
  are the authoritative source. Builds are reproducible (pinned toolchain image and
  `SOURCE_DATE_EPOCH`), so any released binary can be regenerated bit-for-bit from a
  tag on clean infrastructure.
- **Release artifacts and signed SHA-256 checksums** are retained and re-publishable
  from CI. Retention period: 35 days (test records of recovery exercises are retained for
  1 year per Section 11).
- **Repository backup.** An off-host mirror of the repository and tags, maintained on a
  secondary region/provider independent of the primary host, is **to be established**: a
  secondary region/provider is planned but not yet in place during incubation. Until it is
  in place, the authoritative source of truth remains the Git repository and its release
  tags, from which any released binary is reproducible bit-for-bit. Once established, the
  mirror is refreshed at least daily.
- **Signing material.** Signing private keys are non-exportable and held in an HSM /
  cloud signing service; CI holds only scoped credentials, never raw private keys. Any
  recoverable signing material follows the "back up write-only secrets" discipline so
  that a credential can be re-provisioned without reconstructing a private key.

### 5.2 Domain B — customer data

- **Regional residency — target architecture and current limitation.** The intended
  guarantee is that backups and any replicas of a customer's stored data are kept within
  that customer's selected Region, and are not moved across Regions except where the
  customer has selected a multi-region configuration. **This is the at-launch target
  architecture, delivered by roadmap item M7.1, and is not yet fully enforced.** As
  disclosed in `docs/DPA-subprocessors.md` §4 and the region-routing handoff, regional
  residency is presently a product commitment whose routing/correctness is still being
  implemented; until M7.1 is in production the selected-Region guarantee — including for
  backups, replicas, and in-Region recovery — is not yet fully enforced for every tenant.
  This policy must not be read to assert selected-Region enforcement as an in-force
  guarantee before M7.1 lands.
- **Encryption.** Backups are encrypted at rest and in transit.
- **Backup scope and frequency.** CAS content/manifests and the auth/metadata datastore
  are backed up daily (continuous on Cloudflare R2 where available), with backups retained
  for 35 days. This cadence supports the 24-hour RPO targets in Section 4. The configured
  data region for R2/Durable Objects is currently "enam" (Eastern North America) via
  Cloudflare; customer-selectable regional residency is on the roadmap (M7.1) and an
  indicated region does not yet guarantee residency (see the residency limitation above).
- **Restore-from-backup ownership.** Durability and restoration of server-side data is
  HumanGR's operational responsibility, exercised through the provider configuration in
  `docs/DPA-subprocessors.md` (storage provider durability guarantees are documented and
  attached to the Data Processing Agreement).
- **Integrity on restore.** All restored CAS content is content-addressed and
  BLAKE3-verified on fetch; a restore cannot silently substitute altered content
  without detection.

### 5.3 Backup verification

Backups are tested by restore, not assumed. Restore tests are performed on the cadence
in Section 11; an untested backup is treated as no backup.

### 5.4 Local CLI cache — not backed up

`~/.clw/cache` is a content-addressed performance cache (`0700` permissions). It is not
a system of record. Any cache miss is re-fetched from the CAS and BLAKE3-verified;
deleting it costs only re-download. **No backup of the local cache is performed,
required, or promised.** Cache-format versioning and safe invalidation prevent silent
stale replay across client upgrades.

---

## 6. Incident classification and declaration

A disaster is declared by the Incident Commander when an event meets or exceeds any of
the following thresholds:

- **SEV-1 (Critical):** suspected or confirmed compromise of a code-signing trust root;
  confirmed loss or unauthorized disclosure of customer data; full unavailability of the
  CoreLink API in a Region exceeding 1 hour of downtime.
- **SEV-2 (Major):** loss of the authenticated download surface; partial CoreLink API
  unavailability or degradation in a Region that persists but does not amount to full
  unavailability exceeding the SEV-1 1-hour downtime threshold; loss of CI /
  source-control org access.
- **SEV-3 (Minor):** degraded but available service; single-component failure with no
  customer-data impact and within RTO.

Any event involving personal data is additionally and immediately routed to the Data
Protection Officer for assessment of notification obligations under Section 9.3,
regardless of severity rating.

---

## 7. Domain A — release-pipeline disaster recovery

The release pipeline (`.github/workflows/release.yml`): tag → build all targets → sign
(macOS Developer ID + notarization; Windows Authenticode via cloud/HSM signing; Linux
minisign / sigstore) → produce signed SHA-256 checksums → publish to the authenticated
download surface (`docs/DISTRIBUTION.md`).

### 7.1 Loss or compromise of a signing key (SEV-1)

Signing keys are the trust root for every distributed binary; their loss or compromise
is the highest-severity pipeline event. Keys in scope: macOS Developer ID, Windows
Authenticode (cloud/HSM, non-exportable), and the Linux minisign / sigstore key. Routine
rotation is governed by `docs/DISTRIBUTION.md`; the loss/compromise response is governed
here and is coordinated with `SECURITY.md`.

**On suspected compromise:**

1. **Contain.** Revoke the affected certificate/key with the issuing authority (Apple /
   Authenticode CA / minisign keyring). For HSM/cloud keys, rotate the credential and
   lock the signing identity. Preserve forensic evidence.
2. **Re-key.** Generate a new key in the HSM/cloud signing service (no exportable private
   material). Publish the new public key / trust root and fingerprint through the
   documented out-of-band channel (`docs/DISTRIBUTION.md`).
3. **Re-sign.** Rebuild reproducibly (pinned image + `SOURCE_DATE_EPOCH`) and re-sign all
   supported, still-current releases with the new key; publish new signed checksums.
4. **Notify.** Notify affected customers that prior-key signatures are revoked, provide
   the new trust root and fingerprint, and instruct re-verification / re-install, per
   Section 9. Coordinate disclosure timing with `SECURITY.md` where a vulnerability is
   implicated.
5. **Post-incident.** Record root cause; tighten key custody (CI secret scope, HSM access
   policy); complete the review in Section 10.

**On loss without compromise:** follow the same re-key / re-sign / re-publish-trust-root
path without compromise-based revocation. Because artifacts signed with a lost key can no
longer be re-issued under that key, re-sign all current releases under the new key and
notify.

### 7.2 Loss of the authenticated download surface (SEV-2)

- **Existing installs are unaffected.** An outage of the download surface prevents new
  downloads but does not affect already-installed CLIs or the CoreLink API.
- **Failover.** Release artifacts and signed checksums are reproducible and
  re-publishable from CI, so the surface can be restored or failed over to a documented
  alternate host without loss of artifacts. A secondary region/provider to serve as the
  alternate publish target is **to be established** — planned but not yet in place during
  incubation; until it is, recovery relies on re-publishing the reproducible artifacts and
  signed checksums from CI.
- **Verification is host-independent.** Trust rests on the signature and checksum, not on
  the host, so a surface move does not weaken verification.

### 7.3 CI, source-control, and organization continuity

- **No single-machine dependency.** The pipeline runs on hosted or redundant runners,
  never a single self-hosted maintainer machine.
- **Source/tag recovery.** Repository and tags are the source of truth; reproducible
  builds mean any release regenerates from a tag on fresh infrastructure (see Section
  5.1 mirror).
- **Account/org recovery.** Recovery procedures are maintained for the GitHub
  organization, the signing accounts (Apple Developer / Authenticode CA), and the cloud
  accounts, including a break-glass access list and named recovery contacts. During
  incubation the sole named recovery contact and break-glass holder is the accountable
  owner, Gustavo Schneiter (legal@humangr.com).
- **CI secrets** are re-provisionable from the secret manager and are rotated on any
  suspected exposure.

---

## 8. Domain B — hosted service and customer-data recovery

### 8.1 Regional outage of the CoreLink API (no data loss)

- The CoreLink API is a multi-tenant SaaS backend with server-side tenant isolation
  enforced via per-tenant Bearer PATs.
- On a Region becoming unavailable, the Recovery Lead restores service in-Region per the
  provider failover configuration. Under the target residency architecture (M7.1, see
  Section 5.2), customer data is not relocated outside its selected Region as part of
  recovery. Until M7.1 is in production, selected-Region placement is not yet fully
  enforced (Section 5.2); recovery is performed within the Region in which the data
  actually resides, and this policy does not assert selected-Region recovery as an
  in-force guarantee before M7.1 lands.
- Target RTO/RPO: Section 4.

### 8.2 Loss of CAS or metadata data in a Region (SEV-1)

- Restore CAS content/manifests and the auth/metadata datastore from the
  backups/replicas described in Section 5.2, in the Region in which the data resides
  (in-Region under the M7.1 target architecture; see the Section 5.2 residency
  limitation).
- All restored content is BLAKE3-verified on fetch; restoration cannot silently
  introduce altered content.
- Target RTO/RPO: Section 4.

### 8.3 Sub-processor failure

Where a disruption originates with a sub-processor (e.g. the storage or compute provider
listed in `docs/DPA-subprocessors.md`), HumanGR (a) executes provider-level failover or
restore where available, and (b) where a sub-processor is materially and durably
impaired, invokes the documented contingency posture for that sub-processor.

**Concentration risk — single-vendor dependency (disclosed).** The hosted service
currently relies on a single critical infrastructure vendor, Cloudflare, for both object
storage backing the CAS (R2) and write-path coordination (Durable Objects), as recorded
in `docs/DPA-subprocessors.md` §4/Annex II. There is no named alternate provider for
these functions today; the realistic contingency for a durable Cloudflare-level
impairment is therefore an **accepted residual risk** rather than a hot alternate-provider
failover, mitigated by the provider's own regional failover and durability controls and
by the in-Region restore path in Section 8.2. HumanGR must state, per critical
sub-processor and consistently with `docs/DPA-subprocessors.md`, whether a real
alternate-provider plan exists or the concentration risk is accepted: for the sole
critical infrastructure sub-processor, Cloudflare, Inc. (R2 object storage and Durable
Objects; the only infrastructure sub-processor, with no underlying cloud/CDN beneath it),
there is no named alternate-provider failover plan today, and the single-vendor
concentration is treated as an **explicitly accepted residual risk**. This concentration
is disclosed to customers in procurement / security review consistent with the Data
Processing Agreement (`docs/DPA-subprocessors.md`), and is mitigated by Cloudflare's own
regional failover and published R2 durability (eleven nines) and by the in-Region restore
path in Section 8.2.

### 8.4 Data-subject rights during recovery

The right to erasure under GDPR Article 17 (and, for the Brazilian controller, the
corresponding right under Brazil's LGPD — Lei nº 13.709/2018, Art. 18) applies through and
after recovery, consistent with the erasure capability disclosed in
`docs/DPA-subprocessors.md` §10 and mirrored here. Self-service erasure is **live**: the
`clw erase` operation is the primary Article 17 path, with an operator-assisted alternative
(30-day SLA) for Customers who prefer it or cannot run the client.

- **Self-service erasure (live).** As of the effective date, the client `clw erase`
  operation **irreversibly deletes all of a tenant's stored Customer Data — every
  reference (Action Cache / AC) pointer and every underlying content-addressed (CAS)
  blob**. It relies on server reference-enumeration LIST (dependency D-7) and server CAS
  DELETE/LIST (dependency D-8), which are **deployed and live** (roadmap item **M6.4**,
  closed; verified by the live conformance suite). The complementary `clw prune` operation
  garbage-collects unreferenced CAS blobs. These operations carry safety rails (dry-run by
  default; `clw erase` additionally requires explicit confirmation and a typed tenant
  identifier). An operator-assisted alternative (DPA §10.3, 30-day SLA) remains available
  for Customers who prefer it or cannot run the client.
- **Backward-looking commitment (in force once erasure executes).** When an erasure is
  executed — whether by the self-service `clw erase` path or by the operator-assisted
  process — backups and replicas are not used to resurrect data a tenant has erased; the
  erasure propagates to backups and replicas within 35 days and superseded backup copies
  age out within their retention window.

Any erasure request received during an incident is routed to the Data Protection Officer
and tracked to completion under the DPA timelines; an active incident does not suspend
the erasure obligation, only its execution timing where recovery is in progress.

### 8.5 Telemetry posture during incidents

`clw` collects no usage telemetry by default, and v1 ships with no telemetry network
backend. Incident diagnosis relies on server-side operational logs of the CoreLink API
and on customer-provided reports; it does not depend on client telemetry. No additional
client data is collected during an incident beyond the workspace content a user
explicitly snapshots and the auth token required to reach the API.

---

## 9. Communication plan

### 9.1 Internal

On declaration, the Incident Commander convenes the response team via the internal
incident channel maintained by the accountable owner (Gustavo Schneiter,
legal@humangr.com) and opens an incident record. Status is updated at intervals
appropriate to severity (at least every 30 minutes for SEV-1).

### 9.2 Customer notification

- **Channel:** because v1 ships no email backend, customer notices are given via an
  in-product / account-admin notice and the published sub-processor / status page,
  supplemented by direct notice to affected customers at their administrative contact.
- **Timing:** initial customer notice for a SEV-1 or SEV-2 affecting availability or data
  is issued within 24 hours of confirmation, followed by periodic updates until resolution
  and a final summary.
- **Content:** nature and scope of the event, services and Regions affected, current
  status, expected restoration where known, required customer actions (e.g. trust-root
  re-verification per Section 7.1), and a point of contact.
- **Approval:** all customer-facing communications are approved by the Incident Commander;
  the exact legal language of any notice is approved by HumanGR Legal
  (legal@humangr.com).

### 9.3 Regulatory and personal-data breach notification

Where an event constitutes a personal-data breach, the Data Protection Officer assesses
and, where required, notifies the competent supervisory authority and affected data
subjects within the timelines required by applicable law (including, where GDPR applies,
the obligation to notify the supervisory authority without undue delay and, where
feasible, within 72 hours of becoming aware). As HumanGR is a Brazilian controller, the
parallel obligation under Brazil's LGPD (Lei nº 13.709/2018) to communicate a security
incident to the Autoridade Nacional de Proteção de Dados (ANPD) and affected data subjects
within a reasonable time also applies. HumanGR also notifies controllers/customers of a
personal-data breach without undue delay as required by the Data Processing Agreement.
Governing law and competent authority: this policy is governed by the laws of the
Federative Republic of Brazil, with the ANPD as the competent supervisory authority for
LGPD matters; where EU personal data is in scope, the relevant EU Member State supervisory
authority of the data exporter (e.g., Ireland's Data Protection Commission) is also
competent. Privacy contact: privacy@humangr.com; security/abuse contact:
security@humangr.com; general support: support@humangr.com.

---

## 10. Post-incident review

Within 5 business days of resolving a SEV-1
or SEV-2, the Incident Commander leads a blameless post-incident review producing: a
timeline, root-cause analysis, customer impact, whether RTO/RPO were met, and tracked
corrective actions with owners and due dates. Corrective actions are tracked to closure
and verified at the next test cycle.

---

## 11. Testing, maintenance, and review cadence

- **Backup restore tests:** at least annually, including a CAS restore and a metadata
  restore, validated by integrity check.
- **Release-pipeline rebuild test:** at least annually — rebuild a released
  tag on clean infrastructure and confirm reproducible, verifiable output.
- **Trust-root rotation drill:** at least annually — exercise the re-key /
  re-publish path in a non-production context.
- **Failover exercise:** at least annually — exercise download-surface
  failover and a CoreLink API Regional failover.
- **Tabletop incident exercise:** at least annually, covering a SEV-1 scenario end to
  end, including the communication plan.
- **Plan review:** this policy is reviewed at least annually and after any SEV-1
  incident, any material architecture change, or any change to sub-processors. The
  contact tree and break-glass list are reviewed at least annually.

Each test produces a dated record of scope, result, and any corrective actions, retained
for 1 year.

---

## 12. Limitations and relationship to the customer agreement

This policy describes HumanGR's internal preparedness and operational targets. Except for
any service levels expressly stated in a signed customer agreement or SLA, the objectives
and procedures here are targets and do not constitute warranties or guarantees, and do not
create third-party rights. In the event of any conflict between this policy and the
customer agreement, Data Processing Agreement, or a binding SLA, those documents control.
This policy is governed by, and construed under, the laws of the Federative Republic of
Brazil, with disputes subject to the courts of the Judicial District (Comarca) of São
Paulo, State of São Paulo, Brazil.
