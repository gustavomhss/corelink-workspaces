# Disaster Recovery & Business Continuity Policy — CoreLink Workspaces

**Product:** CoreLink Workspaces (CLI binary `clw`) and the hosted CoreLink API (`corelink-api.humangr.com`)
**Publisher:** [[OWNER: registered legal entity name]] ("HumanGR," "we," "us," or "our")
**Document owner:** [[OWNER: role/title of the accountable owner, e.g. Head of Engineering / CTO]]
**Classification:** Internal operational policy with customer-facing commitments (Sections 4, 9, and 11)
**Version:** 1.0
**Effective date:** [[OWNER: effective date]]
**Last reviewed / last updated:** [[OWNER: last-updated date]]
**Next scheduled review:** [[OWNER: next review date — recommended at least annually and after any major incident]]

> **Completion gate — not yet final for customer-facing use.** This policy is not final
> and must not be published or relied on as a customer-facing commitment until the
> following owner-only items are populated and cross-checked for mutual consistency:
> (1) every RTO/RPO target in Section 4, including confirmation of whether any target is
> a contractual SLA; (2) the Domain B backup mechanism, cadence, and retention in
> Section 5.2, confirmed to actually support the RPO targets entered in Section 4;
> (3) the testing-cadence frequencies in Section 11 (the recommended minimums are
> indicated inline); and (4) the sub-processor contingency / accepted-residual-risk
> determination in Section 8.3. The compliance qualifications in Sections 5.2, 8.1, 8.2,
> and 8.4 (regional-residency enforcement gated on roadmap M7.1, and full tenant erasure
> gated on roadmap M6.4) must remain consistent with `docs/DPA-subprocessors.md`; if M6.4
> or M7.1 is in production at the effective date, update the corresponding sections to
> reflect that and remove the forward-looking qualifier.

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
| **Incident Commander (IC)** | Declares a disaster, owns the response, authorizes failover and recovery actions, and approves customer communications. | [[OWNER: named role/individual and deputy]] |
| **Communications Lead** | Drafts and issues customer, sub-processor, and (where required) regulator notifications; maintains the status channel. | [[OWNER: named role/individual]] |
| **Engineering / Recovery Lead** | Executes technical recovery (rebuild, re-sign, failover, restore). | [[OWNER: named role/individual]] |
| **Security Lead** | Handles compromise containment, key revocation/rotation, and forensic preservation. | [[OWNER: named role/individual]] |
| **Data Protection Officer / Privacy contact** | Assesses personal-data impact and breach-notification obligations. | [[OWNER: DPO / privacy contact name and email]] |
| **Executive sponsor** | Final escalation, authorizes extraordinary spend and break-glass access. | [[OWNER: named individual]] |

A current contact tree, including out-of-hours contacts, escalation order, and
break-glass access list, is maintained at [[OWNER: location of the live contact tree /
on-call rotation]] and reviewed on the cadence in Section 11. No role above may be held
by a single individual without a named deputy.

---

## 4. Recovery objectives (RTO / RPO)

The following targets are the commitments governed by this policy. Where a target is
also offered contractually as a Service Level, the controlling figures are those in the
applicable SLA; this table must not be read to enlarge or reduce a contractual SLA.

| Event | RTO (target) | RPO (target) |
|---|---|---|
| Code-signing key compromise → revoke, re-key, re-sign, re-publish trust root, notify | [[OWNER: RTO target]] | Not applicable — no customer data loss; existing installs continue to function. |
| Download-surface outage → failover to alternate host / restore | [[OWNER: RTO target]] | Not applicable — release artifacts and signed checksums are reproducible from tags. |
| CI / source-control / org loss → rebuild from tag on fresh infrastructure | [[OWNER: RTO target]] | Not applicable — Git tags are the source of truth. |
| CoreLink API regional outage (no data loss) → restore service in-region | [[OWNER: RTO target]] | [[OWNER: RPO target]] |
| Loss of CAS data in a region → restore from backup/replica | [[OWNER: RTO target]] | [[OWNER: RPO target]] |
| Loss of auth/metadata datastore → restore from backup | [[OWNER: RTO target]] | [[OWNER: RPO target]] |

[[OWNER: confirm each RTO/RPO target above, and confirm whether any of these are
expressed as contractual SLA percentages or response times in the customer agreement.]]

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
  from CI. Retention period: [[OWNER: artifact retention period]].
- **Repository backup.** A mirror of the repository and tags is maintained independently
  of the primary host at [[OWNER: location/provider of the off-host repository mirror]],
  refreshed at least [[OWNER: mirror frequency]].
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
- **Backup scope and frequency.** [[OWNER: confirm backup mechanism, frequency, and
  retention for CAS content/manifests and for the auth/metadata datastore — e.g. R2
  versioning/replication settings, snapshot cadence, and retention window. The RPO
  targets in Section 4 must be supported by this configuration.]]
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
  CoreLink API in a Region exceeding [[OWNER: SEV-1 duration threshold]].
- **SEV-2 (Major):** loss of the authenticated download surface; partial CoreLink API
  unavailability or degradation in a Region exceeding [[OWNER: SEV-2 duration
  threshold]]; loss of CI / source-control org access.
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
  alternate host without loss of artifacts. The alternate publish target is maintained at
  [[OWNER: alternate download host / failover target]].
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
  accounts, including a break-glass access list and named recovery contacts: [[OWNER:
  break-glass / account-recovery contacts and procedure location]].
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
alternate-provider plan exists or the concentration risk is accepted:
[[OWNER: confirm, per critical sub-processor, alternate-provider plan OR explicitly
accepted residual risk; if accepted, confirm this single-vendor concentration is
disclosed to customers in procurement/security review consistent with the DPA]].

### 8.4 Data-subject rights during recovery

The right to erasure under GDPR Article 17 applies through and after recovery, subject to
the current product limitation disclosed in `docs/DPA-subprocessors.md` §10 and mirrored
here so this policy does not overstate the capability.

- **Current limitation (until roadmap item M6.4).** As of the effective date, the client
  `clw rm` operation deletes only the reference (AC) pointer and **cannot
  garbage-collect the underlying content-addressed (CAS) blobs**; un-naming content does
  not by itself reclaim the stored bytes. Full tenant erasure (CAS blob
  garbage-collection, requiring server CAS DELETE/LIST and ref-enumeration) is delivered
  by go-live-blocking roadmap item **M6.4**. Until M6.4 is in production, erasure of
  affected Customer Data is effected by the operator-side interim process described in
  the DPA, not by automated self-service deletion. This policy does not represent that
  full, propagated-to-backup erasure is available before M6.4 lands.
- **Backward-looking commitment (in force once erasure executes).** When an erasure is
  executed — whether by the interim operator-side process today or by the M6.4
  self-service path once live — backups and replicas are not used to resurrect data a
  tenant has erased; the erasure propagates to backups and replicas within
  [[OWNER: maximum time within which an executed deletion propagates to backups/replicas]]
  and superseded backup copies age out within their retention window.

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

On declaration, the Incident Commander convenes the response team via [[OWNER: incident
bridge / channel]] and opens an incident record. Status is updated at intervals
appropriate to severity (at least every [[OWNER: SEV-1 internal update interval]] for
SEV-1).

### 9.2 Customer notification

- **Channel:** the official status page / customer notification channel at [[OWNER:
  status page URL and notification channel]], supplemented by direct notice to affected
  customers at their administrative contact.
- **Timing:** initial customer notice for a SEV-1 or SEV-2 affecting availability or data
  is issued within [[OWNER: customer-notification target time]] of declaration, followed
  by periodic updates until resolution and a final summary.
- **Content:** nature and scope of the event, services and Regions affected, current
  status, expected restoration where known, required customer actions (e.g. trust-root
  re-verification per Section 7.1), and a point of contact.
- **Approval:** all customer-facing communications are approved by the Incident Commander;
  the exact legal language of any notice is approved by [[OWNER: legal review/approver for
  customer notices]].

### 9.3 Regulatory and personal-data breach notification

Where an event constitutes a personal-data breach, the Data Protection Officer assesses
and, where required, notifies the competent supervisory authority and affected data
subjects within the timelines required by applicable law (including, where GDPR applies,
the obligation to notify the supervisory authority without undue delay and, where
feasible, within 72 hours of becoming aware). HumanGR also notifies controllers/customers
of a personal-data breach without undue delay as required by the Data Processing
Agreement. Governing law and competent authority: [[OWNER: governing law and competent
supervisory authority / jurisdiction]]. Privacy contact: [[OWNER: DPO / privacy contact
email]]; security/abuse contact: [[OWNER: security/abuse email]]; general support:
[[OWNER: support email]].

---

## 10. Post-incident review

Within [[OWNER: post-incident review deadline, e.g. 5 business days]] of resolving a SEV-1
or SEV-2, the Incident Commander leads a blameless post-incident review producing: a
timeline, root-cause analysis, customer impact, whether RTO/RPO were met, and tracked
corrective actions with owners and due dates. Corrective actions are tracked to closure
and verified at the next test cycle.

---

## 11. Testing, maintenance, and review cadence

- **Backup restore tests:** at least [[OWNER: restore-test frequency, recommended
  quarterly]], including a CAS restore and a metadata restore, validated by integrity
  check.
- **Release-pipeline rebuild test:** at least [[OWNER: frequency]] — rebuild a released
  tag on clean infrastructure and confirm reproducible, verifiable output.
- **Trust-root rotation drill:** at least [[OWNER: frequency]] — exercise the re-key /
  re-publish path in a non-production context.
- **Failover exercise:** at least [[OWNER: frequency]] — exercise download-surface
  failover and a CoreLink API Regional failover.
- **Tabletop incident exercise:** at least annually, covering a SEV-1 scenario end to
  end, including the communication plan.
- **Plan review:** this policy is reviewed at least annually and after any SEV-1
  incident, any material architecture change, or any change to sub-processors. The
  contact tree and break-glass list are reviewed at least [[OWNER: contact-tree review
  frequency]].

Each test produces a dated record of scope, result, and any corrective actions, retained
for [[OWNER: test-record retention period]].

---

## 12. Limitations and relationship to the customer agreement

This policy describes HumanGR's internal preparedness and operational targets. Except for
any service levels expressly stated in a signed customer agreement or SLA, the objectives
and procedures here are targets and do not constitute warranties or guarantees, and do not
create third-party rights. In the event of any conflict between this policy and the
customer agreement, Data Processing Agreement, or a binding SLA, those documents control.
This policy is governed by, and construed under, [[OWNER: governing law]], with disputes
subject to [[OWNER: dispute venue]].
