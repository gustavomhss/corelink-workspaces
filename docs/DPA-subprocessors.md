# Data Processing Addendum & Sub-Processor List — CoreLink Workspaces (`clw`)

> **DRAFT — owner/legal review and execution required.** This Data Processing
> Addendum ("DPA") is an engineering draft prepared to de-risk legal lead time
> (roadmap M6.3). It is **not** binding and has **not** been reviewed by counsel.
> Each `[[OWNER: …]]` placeholder must be completed, and the DPA reviewed and
> executed by an authorized signatory, before it is relied upon or presented to a
> customer. Do not execute or rely on this until the owner and legal confirm.

This DPA forms part of, and is incorporated by reference into, the agreement
between the Processor and the Customer governing the Customer's use of the
CoreLink Workspaces service (the **"Agreement"**) — namely the Terms of Service
(`docs/TERMS-OF-SERVICE.md`) and the End-User License Agreement (`EULA.md`). It
governs the Processing of Customer Personal Data by the Processor on behalf of the
Customer in connection with the service. Where this DPA conflicts with the
Agreement with respect to the Processing of Personal Data, this DPA controls.

**The Parties:**

- **Customer / Controller:** the legal entity that has accepted the Agreement and
  operates a CoreLink tenant (the **"Customer"**).
- **Processor:** [[OWNER: registered legal entity name (e.g. "HumanGR Labs Inc.") + jurisdiction/country of incorporation + registered address]], operator of the
  CoreLink Workspaces service and the hosted CoreLink API at
  `corelink-api.humangr.com` (the **"Processor"** or **"CoreLink"**).

**Effective Date:** [[OWNER: effective date of this DPA]].

---

## 1. Definitions

1.1. Capitalized terms not defined here have the meaning given in the Agreement.

1.2. **"GDPR"** means Regulation (EU) 2016/679. **"UK GDPR"** means the GDPR as
incorporated into the law of the United Kingdom by the Data Protection Act 2018
and the European Union (Withdrawal) Act 2018. **"Data Protection Law"** means the
GDPR, the UK GDPR, the US State Privacy Laws (defined in §1.9), and any other
data-protection or privacy law applicable to the Processing under this DPA, in each
case as amended or replaced.

1.3. **"Controller"**, **"Processor"**, **"Data Subject"**, **"Personal Data"**,
**"Processing"**, **"Personal Data Breach"**, and **"Supervisory Authority"** have
the meanings given in the GDPR.

1.4. **"Customer Personal Data"** means Personal Data contained within the Customer
Data that the Processor Processes on behalf of the Customer under the Agreement.

1.5. **"Customer Data"** means all data the Customer (or its users) uploads to or
generates within the service via the `clw` client — principally file contents,
file paths and tree structure, reference (AC) names, and the associated tenant
Personal Access Token (PAT). The PAT is a security **credential** and is subject to
the credential carve-out in §1.10 and §9.4 rather than to the "return" mechanics
applicable to substantive Customer content.

1.6. **"Sub-Processor"** means any third party engaged by the Processor to Process
Customer Personal Data, as listed in Annex II (§13).

1.7. **"Standard Contractual Clauses"** or **"SCCs"** means (a) for transfers
subject to the GDPR, the standard contractual clauses approved by Commission
Implementing Decision (EU) 2021/914; and (b) for transfers subject to the UK GDPR,
the UK International Data Transfer Addendum to the EU SCCs (the **"UK Addendum"**)
issued by the Information Commissioner.

1.8. **"clw"** means the proprietary CoreLink Workspaces command-line client,
distributed to the Customer as a binary and operated by the Customer on the
Customer's own machines.

1.9. **"US State Privacy Laws"** means the California Consumer Privacy Act of 2018
as amended by the California Privacy Rights Act (collectively, the **"CCPA/CPRA"**)
and any other US state privacy or data-protection law applicable to the Processing,
in each case as amended or replaced. **"Business"**, **"Service Provider"**,
**"Sell"**, **"Share"**, **"Personal Information"**, and **"Business Purpose"** have
the meanings given in the CCPA/CPRA. Where the Processing is subject to the
CCPA/CPRA, the Customer is the **Business** and the Processor is the **Service
Provider**, and §6A applies.

1.10. **"PAT"** or **"Personal Access Token"** means the per-tenant Bearer
authentication credential the `clw` client uses to authenticate to the CoreLink
API. The PAT is a security **credential**; the deletion-or-return mechanics in §4.7
and §9.2 apply to it as set out in §9.4 (the Processor **destroys** credentials and
may retain credential hashes for security/audit), and it is not subject to the
"return" obligation in the manner of substantive Customer content.

---

## 2. Roles of the Parties & scope

2.1. **Roles.** With respect to Customer Personal Data, the Customer is the
Controller (or, where the Customer itself acts as a processor for a third party,
the Customer is a processor and the Processor is a sub-processor), and the
Processor acts as a Processor. The Customer determines the purposes and means of
the Processing by deciding what file contents and paths it uploads via `clw`; the
Processor Processes Customer Personal Data only to provide the service. Where the
Processing is subject to the CCPA/CPRA, the Customer is the **Business** and the
Processor is the **Service Provider** (§1.9, §6A). The Customer's role per this §2.1
determines the applicable SCC Module under §11.2(a): Module Two where the Customer
is a controller, and Module Three where the Customer is itself a processor.

2.2. **Client vs. service.** `clw` is the Customer-operated **client**: it runs on
the Customer's machine, holds the PAT, and uploads only the content the Customer
explicitly snapshots, plus the auth token needed to talk to the CoreLink API (see
`docs/PRIVACY-POLICY.md` for the full data inventory). The Processing performed by
the Processor and its Sub-Processors is **server-side**, in the hosted CoreLink
API and its backing infrastructure.

2.3. **Compliance with law.** Each Party shall comply with its respective
obligations under Data Protection Law. The Customer is responsible for the lawful
basis, notices, and (where required) consents for the Customer Data it uploads,
and for not uploading content it has no right to store. The Processor is
responsible for Processing such data only as instructed and as set out in this
DPA.

2.4. **No telemetry / minimal data.** The service is privacy-respecting by design.
`clw` collects **no usage telemetry by default**, and v1 ships with **no telemetry
network backend at all**. The only data `clw` transmits is the workspace content
the Customer explicitly snapshots and the PAT required to authenticate. If
telemetry is later introduced, it will be off by default, consent-gated, and this
DPA and the Privacy Policy will be updated to enumerate the exact payload before
release.

---

## 3. Subject matter, nature, purpose, categories & duration (Annex I)

3.1. **Subject matter & nature of Processing.** Content-addressed storage and
retrieval of Customer source / working-tree data (snapshot and hydrate operations)
together with reference (AC) metadata, using BLAKE3 content digests and FastCDC
chunking, performed to provide the CoreLink Workspaces service.

3.2. **Purpose.** To provide, maintain, secure, and support the service in
accordance with the Agreement and the Customer's documented instructions.

3.3. **Categories of Customer Data and Personal Data.** File **contents**; file
**paths / tree structure**; reference (AC) names; and the tenant **PAT**
(an authentication **credential**, handled under the carve-out in §1.10 / §9.4 —
destroyed rather than returned, with security-retention of credential hashes
permitted). Any Personal Data is **incidental** — it consists of whatever the
Customer chooses to store; the service does not require Personal Data to function.
Paths and contents may, depending on the Customer's choices, contain personal or
sensitive information. The Customer must not upload special-category data (GDPR
Art. 9) or other highly sensitive data unless it has independently determined the
service is appropriate for it and has implemented appropriate safeguards on its
side.

3.4. **Categories of Data Subjects.** Determined solely by the Customer's content
(e.g. the Customer's own personnel, contractors, or any individuals named in the
uploaded files).

3.5. **Frequency of Processing.** Continuous, for the duration of the Agreement,
on each snapshot, hydrate, or reference operation initiated by the Customer.

3.6. **Duration.** For the term of the Agreement, subject to the retention and
deletion terms in §9 and the erasure terms in §10.

3.7. **Annex I roles.** For the purposes of the SCCs, the data exporter is the
Customer and the data importer is the Processor; the competent Supervisory
Authority is determined per §11.4.

---

## 4. Processor obligations

The Processor shall:

4.1. **Process only on instructions.** Process Customer Personal Data only on the
Customer's documented instructions, including with regard to international
transfers, unless required to do otherwise by Union or Member State law to which
the Processor is subject; in that case the Processor shall inform the Customer of
that legal requirement before Processing, unless the law prohibits such
information on important grounds of public interest. The Agreement, this DPA, and
the Customer's configured use of `clw` (which paths/contents it snapshots, which
region it selects) constitute the Customer's complete and final documented
instructions. The Processor shall inform the Customer if, in its opinion, an
instruction infringes Data Protection Law.

4.2. **Confidentiality.** Ensure that persons authorized to Process Customer
Personal Data are bound by an appropriate obligation of confidentiality and
process the data only as necessary to perform their duties.

4.3. **Security.** Implement and maintain the technical and organizational measures
set out in §5 (Annex III).

4.4. **Sub-Processors.** Engage Sub-Processors only in accordance with §6 and flow
down data-protection obligations no less protective than those in this DPA.

4.5. **Assistance — Data Subject rights.** Taking into account the nature of the
Processing, assist the Customer by appropriate technical and organizational
measures, insofar as possible, in fulfilling the Customer's obligation to respond
to requests to exercise Data Subject rights (access, rectification, erasure,
restriction, portability, objection) under §8.

4.6. **Assistance — compliance.** Assist the Customer in ensuring compliance with
its obligations under GDPR Arts. 32–36 (security, breach notification, data
protection impact assessments, and prior consultation), taking into account the
nature of the Processing and the information available to the Processor.

4.7. **Deletion or return.** At the Customer's choice, delete or return all
Customer Personal Data at the end of the provision of services, and delete
existing copies, unless Union or Member State law requires storage — as set out in
§9, **subject to §10.2 and effected by the interim operator-side process in §10.3**
until roadmap item M6.4 lands, and subject to the credential carve-out in §9.4 (the
PAT is destroyed, not returned).

4.8. **Records & demonstration of compliance.** Maintain records of Processing
activities to the extent required by GDPR Art. 30(2), and make available to the
Customer the information necessary to demonstrate compliance with this DPA and
GDPR Art. 28, and allow for and contribute to audits in accordance with §7.

4.9. **Breach notification.** Notify the Customer of Personal Data Breaches in
accordance with §12.

---

## 5. Technical & organizational security measures (Annex III)

5.1. Taking into account the state of the art, the costs of implementation, and
the nature, scope, context, and purposes of Processing, as well as the risk to
Data Subjects, the Processor implements appropriate technical and organizational
measures to ensure a level of security appropriate to the risk, including as
appropriate:

(a) **Encryption in transit.** All client-server communication requires `https://`
(TLS); the `clw` HTTP client refuses plaintext, with an exact-host loopback
exception for local development only (`SECURITY.md`, `docs/THREAT-MODEL.md`).

(b) **Encryption / protection at rest.** Object storage and infrastructure
at-rest protection are provided by the Sub-Processors identified in Annex II;
the specific at-rest encryption posture and key management are
[[OWNER: server-side at-rest encryption and key-management description (e.g. provider-managed AES-256, key custody)]].

(c) **Credential protection.** The tenant PAT is the single credential
authenticating a tenant. On the client it is stored with `0600` permissions under
a `0700` directory, never logged, redacted in all output and diagnostics, and sent
only over TLS (`docs/PRIVACY-POLICY.md`). Server-side PAT storage, hashing, and
rotation are [[OWNER: server-side credential-storage and rotation description]].

(d) **Tenant isolation.** Tenant isolation is enforced server-side via per-tenant
Bearer PAT authentication and tenant scoping; the `x-corelink-tenant-id` header is
server-internal and never set by `clw`.

(e) **Integrity.** All content is BLAKE3 content-addressed and chunk-verified on
fetch; manifests are canonical-JSON, Merkle-style. The client treats all
server-returned manifests and chunks as untrusted and re-verifies them.

(f) **Data residency.** Customer Data is stored in a Customer-selected region; see
§11.

(g) **Access control, logging & monitoring.** Server-side access controls, audit
logging, monitoring, and the personnel and operational controls listed in
[[OWNER: server-side access-control, logging, retention-of-logs, and monitoring measures]].

(h) **Business continuity.** Backup, durability, and disaster-recovery posture per
`docs/DR-business-continuity.md` and [[OWNER: R2/object-store durability and backup SLA, if any]].
The local `clw` cache is ephemeral and reconstructible from the server.

(i) **Vulnerability management & secure development.** [[OWNER: vulnerability-management, patching cadence, and secure-SDLC measures]].

5.2. The Processor may update its security measures from time to time provided the
updates do not materially reduce the overall level of protection.

---

## 6. Sub-Processors

6.1. **Authorization.** The Customer provides a general authorization for the
Processor to engage the Sub-Processors listed in Annex II (§13), and to engage new
Sub-Processors subject to this §6.

6.2. **Flow-down.** The Processor shall impose on each Sub-Processor, by written
contract, data-protection obligations that are no less protective than those in
this DPA, in particular sufficient guarantees to implement appropriate technical
and organizational measures so that the Processing meets the requirements of Data
Protection Law.

6.3. **Liability.** The Processor remains fully liable to the Customer for the
performance of each Sub-Processor's obligations.

6.4. **Change notice.** Before authorizing any new Sub-Processor or replacing an
existing one, the Processor shall give the Customer at least
[[OWNER: change-notice period, e.g. 30 days]] prior notice by
[[OWNER: change-notice mechanism (e.g. email to the tenant admin and/or a published sub-processor page URL)]],
so the Customer has the opportunity to object on reasonable data-protection
grounds.

6.5. **Objection.** If the Customer objects on reasonable, documented
data-protection grounds within [[OWNER: objection window, e.g. 30 days]] of the
notice, the Parties shall work together in good faith to resolve the objection,
including by the Processor offering a commercially reasonable alternative. If no
resolution is reached within a reasonable period, the Customer may, as its sole and
exclusive remedy, terminate the affected service (or, where every Sub-Processor in
Annex II is load-bearing for the content-addressed store such that no severable
portion exists, the Agreement) on written notice, and the Processor shall refund any
prepaid fees for the terminated service covering the period after the effective date
of termination, calculated on a pro-rata basis. This is reconciled with the
termination and refund provisions of the Agreement at
[[OWNER: termination/refund section reference in the Agreement]].

---

## 6A. US State Privacy Laws / CCPA-CPRA (Service-Provider terms)

This §6A applies to the extent the Processor Processes Personal Information subject
to the US State Privacy Laws on the Customer's behalf. In that case the Customer is
the **Business** and the Processor is the **Service Provider**, and the following
covenants apply (in addition to the rest of this DPA):

6A.1. **Limited purpose.** The Processor shall Process Personal Information only on
behalf of the Customer and solely for the specific **Business Purpose** of providing
the service under the Agreement and the Customer's documented instructions, and not
for any other purpose. The Processor shall not retain, use, or disclose Personal
Information for any purpose other than the specified Business Purpose, including not
retaining, using, or disclosing it outside the direct business relationship between
the Parties.

6A.2. **No Sale / No Share.** The Processor shall not **Sell** or **Share** Personal
Information, and the disclosure of Personal Information by the Customer to the
Processor is not a Sale or Share and is made only for the limited purpose in §6A.1.

6A.3. **No combining.** The Processor shall not combine Personal Information received
under the Agreement with Personal Information it receives from, or on behalf of, any
other person, or collects from its own interaction with any consumer, except as
permitted by §1798.140(ag)(1) of the CCPA/CPRA (e.g. to perform the Business
Purpose).

6A.4. **Certification.** The Processor **certifies** that it understands the
restrictions in this §6A and the CCPA/CPRA and will comply with them.

6A.5. **Onward transfer / sub-processors.** The Processor shall impose the
restrictions in this §6A on any Sub-Processor it engages to Process Personal
Information, by written contract, consistent with §6.2.

6A.6. **Customer rights to monitor and remediate.** The Customer may take
reasonable and appropriate steps to ensure that the Processor uses Personal
Information in a manner consistent with the Customer's obligations under the
US State Privacy Laws, including by exercising the audit rights in §7, and to stop
and remediate any unauthorized use of Personal Information.

6A.7. **Notice of inability to comply.** The Processor shall notify the Customer
without undue delay if it determines it can no longer meet its obligations under the
US State Privacy Laws.

---

## 7. Audit

7.1. The Processor shall make available to the Customer all information necessary
to demonstrate compliance with GDPR Art. 28 and this DPA, and shall allow for and
contribute to audits, including inspections, conducted by the Customer or an
auditor mandated by the Customer.

7.2. To satisfy 7.1, the Processor may, in the first instance, provide
[[OWNER: available audit artifacts — e.g. SOC 2 / ISO 27001 reports, penetration-test summaries, or the Sub-Processors' equivalent certifications]].

7.3. Where such artifacts are insufficient to address a specific, reasonable
concern, the Customer may request an audit, on at least
[[OWNER: audit notice period, e.g. 30 days]] prior written notice, no more than
[[OWNER: audit frequency cap, e.g. once per 12-month period]] (except where
required by a Supervisory Authority or following a Personal Data Breach), during
business hours, subject to confidentiality, and conducted so as not to disrupt the
Processor's operations or the security or confidentiality of other customers'
data. The Customer bears its own audit costs.

---

## 8. Data Subject rights

8.1. The Processor shall, to the extent legally permitted, promptly notify the
Customer if it receives a request from a Data Subject to exercise rights under Data
Protection Law in respect of Customer Personal Data, and shall not respond to that
request itself except on the Customer's documented instructions or as required by
law.

8.2. Taking into account the nature of the Processing, the Processor shall assist
the Customer by appropriate technical and organizational measures, insofar as
possible, to fulfil the Customer's obligation to respond to such requests,
including erasure requests as governed by §10.

---

## 9. Retention, return & deletion

9.1. The Processor Processes and retains Customer Personal Data for the term of the
Agreement. Default server-side retention periods are
[[OWNER: server-side retention period(s) for Customer Data after last access / after account closure]].

9.2. On termination or expiry of the Agreement, the Processor shall, at the
Customer's choice, delete or return all Customer Personal Data and delete existing
copies within [[OWNER: post-termination deletion window, e.g. 30 days]], unless
Union or Member State law requires continued storage, in which case the Processor
shall protect the data and Process it only as required by that law. Because the
content-addressed (CAS) blobs cannot yet be erased by self-service means (§10.2),
deletion of server-side Customer Data — including the "delete existing copies"
obligation in this §9.2 — is performed by, and subject to, the interim
operator-side process and SLA in §10.3 until roadmap item M6.4 lands. The PAT is
handled under the credential carve-out in §9.4, not "returned".

9.4. **Credential carve-out (PAT).** Notwithstanding §4.7 and §9.2, the tenant PAT
is a security credential, not substantive Customer content. On termination or on a
verified deletion request, the Processor shall **destroy** (revoke and delete) the
PAT and the Customer's stored copy is destroyed locally by `clw uninstall` (§9.3);
the Processor does **not** "return" the credential. The Processor may retain a
salted hash or non-reversible derivative of the PAT, and associated security/audit
logs, for as long as necessary for security, fraud-prevention, and audit purposes
and as permitted or required by law; such retention is an express exception to the
erasure and return obligations of this DPA.

9.3. **Local client data.** The `clw` local cache is ephemeral and reconstructible;
`clw uninstall` (roadmap M3.5) removes the binary, the PAT, the cache, and any
local artifacts.

---

## 10. Right to erasure (GDPR Art. 17) — commitment and current limitation

10.1. **Commitment.** The service is being built so that a tenant can delete all of
its stored Customer Data, and the Processor will give effect to Data Subject and
Customer erasure requests in accordance with §8 and Data Protection Law. Until the
self-service capability in §10.2 is complete, the Processor effects erasure of all
stored Customer Data — including the content-addressed (CAS) blobs — through the
interim operator-side process and SLA in §10.3. The Processor does not represent
that fully automated, verifiable self-service erasure of CAS content is available
before M6.4 lands (§10.4).

10.2. **Disclosed current limitation.** As of the Effective Date, the client `clw
rm` operation deletes only the reference (AC) pointer and **cannot
garbage-collect the underlying content-addressed (CAS) blobs**; consequently a
tenant cannot yet fully and verifiably erase all uploaded content through
self-service alone. Full self-service erasure requires client `prune` / CAS
garbage-collection together with server reference-enumeration LIST (dependency
D-7) and server CAS DELETE / LIST (dependency D-8), tracked as go-live-blocking
roadmap item **M6.4** (`docs/ROADMAP-go-live.md`).

10.3. **Interim erasure process.** Until M6.4 lands, the Processor shall, on a
Customer's verified written erasure request to the contact in §15, effect deletion
of the affected Customer Data — **including the underlying CAS blobs** — by an
operator-side process, and confirm completion **without undue delay and in any
event within [[OWNER: interim erasure SLA — a committed number of days; must be filled before launch (an Art. 17 obligation cannot be left open) and the operator-side process must in fact delete CAS blobs and be verifiable]]**. This interim
operator-side process is the means by which the deletion-or-return obligations in
§4.7 and §9.2 are performed for server-side Customer Data until M6.4 lands.

10.4. **No misrepresentation.** This DPA does not represent that fully automated,
self-service erasure of CAS content is available before M6.4 lands. The Processor
shall update §10 upon closure of M6.4.

---

## 11. International data transfers

11.1. **Residency.** Customer Data is stored in a Customer-selected region. Where
the selected region keeps Customer Personal Data within the EEA or the UK as
applicable, no transfer mechanism under this §11 is required for storage.

11.2. **Transfer mechanism.** To the extent the Processing involves a transfer of
Customer Personal Data from the EEA, the UK, or Switzerland to a country not
benefiting from an adequacy decision, the Parties agree that the **Standard
Contractual Clauses** apply and are incorporated into this DPA by reference, as
follows:

(a) For GDPR transfers, the EU SCCs (Decision (EU) 2021/914) apply, with the
applicable **Module determined conditionally by the Customer's role under §2.1**:
**Module Two (Controller→Processor)** applies where the Customer is a controller,
and **Module Three (Processor→Processor)** applies where the Customer is itself a
processor. The Parties incorporate whichever Module corresponds to the Customer's
actual role for the relevant transfer; the document is not limited to a single
fixed Module. [[OWNER: confirm both Modules Two and Three are populated/annexed so the correct one binds per the Customer's §2.1 role, rather than a single pre-selected Module]];

(b) For UK GDPR transfers, the UK Addendum applies on top of the EU SCCs;

(c) For Swiss transfers, the EU SCCs apply as adapted by the Swiss Federal Data
Protection and Information Commissioner (FDPIC) — [[OWNER: confirm Switzerland is an in-scope export jurisdiction for the chosen regions and that the FDPIC is a relevant authority before retaining this paragraph; delete if no Swiss transfers occur]].

11.3. **SCC operative selections.** Where the SCCs apply:

(a) the optional docking clause (Clause 7) [[OWNER: applies / does not apply]];

(b) under Clause 9, **Option [[OWNER: 1 (specific) or 2 (general)]]** for
sub-processor authorization applies, with the change-notice period in §6.4;

(c) under Clause 11, the independent-dispute-resolution option
[[OWNER: applies / does not apply]];

(d) under Clause 17, the governing law is [[OWNER: governing law of the SCCs, e.g. the law of the EU Member State of the data exporter]];

(e) under Clause 18, the forum for disputes is [[OWNER: courts/forum for SCC disputes]];

(f) Annexes I, II, and III to the SCCs are populated by §3 (Annex I), §13 (Annex
II — sub-processors), and §5 (Annex III — security measures) of this DPA, with the
data exporter, data importer, and competent Supervisory Authority as set out in
this DPA.

11.4. **Competent Supervisory Authority.** The competent Supervisory Authority is
[[OWNER: competent supervisory authority for the SCCs]]. This designation depends on
first resolving the Art. 27 representative (§15.6): where the Processor is **not**
established in the EEA/UK, an Art. 27 representative is **required, not optional**,
and the competent authority is generally that of the Member State where the EU
representative is established (and, for UK transfers, the ICO). The supervisory
authority designation **cannot be completed until the Art. 27 representative
determination in §15.6 is made**.

11.5. **Transfer-impact assessment.** The Processor shall, on the Customer's
reasonable request, provide information to assist the Customer's transfer-impact
assessment, including the Sub-Processor locations in Annex II and the safeguards in
place.

11.6. **Precedence.** In the event of a conflict between the SCCs and this DPA, the
SCCs prevail with respect to the relevant restricted transfer.

---

## 12. Personal Data Breach notification

12.1. The Processor shall notify the Customer without undue delay, and in any event
within [[OWNER: breach-notification deadline, e.g. 72 hours]] after becoming aware
of a Personal Data Breach affecting Customer Personal Data.

12.2. The notification shall, to the extent known and as it becomes available,
describe the nature of the breach (including categories and approximate numbers of
Data Subjects and records concerned), the likely consequences, and the measures
taken or proposed to address it and mitigate its effects, and shall provide the
name and contact details of a point of contact.

12.3. The Processor shall cooperate with the Customer and take reasonable steps to
assist in the investigation, mitigation, and remediation of the breach. Security
incidents are handled in coordination with the disclosure process in `SECURITY.md`.

12.4. The Processor's notification is not an acknowledgment of fault or liability.

---

## 13. Annex II — Sub-Processor list

The Processor engages the following Sub-Processors to Process Customer Personal
Data:

| Sub-Processor | Service provided | Data Processed | Location / residency |
|---|---|---|---|
| **Cloudflare R2** | Object storage backing the content-addressed store (CAS) — stores uploaded file **contents** (chunks) and manifests | File contents; file paths (within manifests) | Customer-selected region; specific R2 jurisdiction(s) configured per tenant region — [[OWNER: configured R2 region/jurisdiction(s)]] |
| **Cloudflare Durable Objects** | Coordination and per-tenant write-path control for the CoreLink API | Tenant-scoped coordination metadata; mediates CAS writes | Cloudflare network; region binding per Customer-selected region — [[OWNER: configured DO region binding]] |
| [[OWNER: payment processor — name, service (billing / card-token handling), data (billing contact, card token, billing email), location/jurisdiction; or state "none — no payment sub-processor at v1" if billing is not yet live]] | | | |
| [[OWNER: billing / CRM / account-portal provider — name, service, data, location; or "none at v1"]] | | | |
| [[OWNER: transactional-email / notification provider used for the §6.4 change-notice and §12 breach-notice mechanisms — name, service, data, location; or state "none — v1 ships no email backend and §6.4/§12 notices are delivered by <the actual mechanism, e.g. published sub-processor page + in-product/account-admin notice>"]] | | | |
| [[OWNER: error / crash-reporting provider, if any — name, service, data, location; or "none — clw ships no telemetry/crash-reporting network backend at v1"]] | | | |
| [[OWNER: underlying cloud provider / CDN beneath the Cloudflare edge, if any distinct sub-processor exists — name, service, data, location; or "none — Cloudflare is the sole infrastructure sub-processor"]] | | | |

**Notes:**

13.1. The hosted **CoreLink API** (`corelink-api.humangr.com`) is the Processor's
own first-party processing layer (auth via PAT, tenant scoping, AC reference store,
CAS gateway), operated by the Processor; it is not a Sub-Processor. The Cloudflare
services above are the underlying infrastructure Sub-Processors behind it. The
CoreLink server is a separate system from `clw`.

13.2. `clw` transmits Customer Data **only** to the CoreLink API endpoint; it does
not contact Cloudflare or any other third party directly. `clw` collects no usage
telemetry by default and v1 ships with no telemetry network backend.

13.3. The Processor shall maintain Cloudflare's then-current data-protection terms,
sub-processor disclosures, and transfer safeguards as part of the flow-down under
§6.2; the current Cloudflare DPA reference is
[[OWNER: link/reference to the executed Cloudflare DPA and its SCCs]].

13.4. **Notice-delivery mechanism.** The change-notice (§6.4), Sub-Processor
objection (§6.5), breach-notice (§12), and erasure-confirmation (§10.3) mechanisms
require a means of contacting the Customer. The Processor must ensure the mechanism
referenced in §6.4 actually exists at launch and lists any email/notification
provider used as a Sub-Processor in the Annex II table above. [[OWNER: confirm the actual notice-delivery mechanism for §6.4/§6.5/§10.3/§12 (e.g. email to the tenant admin via a transactional-email sub-processor, a published sub-processor page, and/or in-product/account-admin notice); if v1 has no email backend, the table and §6.4 must reference the mechanism that does exist so the DPA does not promise a capability it lacks]].

---

## 14. Liability

14.1. Each Party's liability arising out of or related to this DPA, whether in
contract, tort, or otherwise, is subject to the limitations and exclusions of
liability set out in the Agreement, and any reference to liability of a Party
means the aggregate liability of that Party under the Agreement and this DPA
combined, **except as provided in §14.2 and §14.3**. [[OWNER: (1) confirm the Agreement in fact contains the liability cap incorporated here, and its amount, before relying on this incorporation by reference; and (2) decide whether data-protection and confidentiality breaches sit OUTSIDE the general liability cap — a separate higher "super-cap" or an uncapped carve-out for such breaches is market-standard for processor liability and a generic SaaS cap may be commercially unacceptable or, in some jurisdictions, unenforceable against GDPR statutory liability]].

14.2. Nothing in this DPA limits or excludes either Party's liability to Data
Subjects under the SCCs, or to the extent such limitation or exclusion is not
permitted by Data Protection Law (including a Party's liability for administrative
fines or compensation that cannot lawfully be capped).

14.3. **Data-protection indemnity.** To the extent permitted by law, the Processor
shall indemnify the Customer against regulatory fines, Data-Subject compensation
claims, and reasonable associated costs to the extent directly caused by the
Processor's breach of this DPA or its obligations under Data Protection Law as a
Processor, subject to the cap and carve-out treatment to be fixed under §14.1.
[[OWNER: confirm scope, cap treatment, and reciprocity of this data-protection indemnity (e.g. whether the Customer gives a reciprocal indemnity for unlawful instructions/content it uploads), or replace with an express statement that no indemnity is given and the reason why]].

---

## 15. General

15.1. **Term.** This DPA takes effect on the Effective Date and continues for as
long as the Processor Processes Customer Personal Data under the Agreement.

15.2. **Order of precedence.** In case of conflict regarding the Processing of
Personal Data: (1) the SCCs (for restricted transfers), then (2) this DPA, then
(3) the Agreement.

15.3. **Changes.** The Processor may amend this DPA where required to comply with
Data Protection Law or a Supervisory Authority, or to reflect a change of
Sub-Processor under §6, provided such amendments do not materially reduce the
protection of Customer Personal Data.

15.4. **Governing law & venue.** Except where the SCCs require otherwise, this DPA
is governed by the law and subject to the venue stated in the Agreement, namely
[[OWNER: governing law and dispute venue]].

15.5. **Severability.** If any provision of this DPA is held invalid or
unenforceable, the remaining provisions remain in full force, and the Parties
shall replace the invalid provision with a valid one that achieves its purpose as
closely as possible.

15.6. **Contacts.**

- Data Protection Officer / privacy contact: [[OWNER: DPO / privacy contact email on the humangr.com domain]].
- Security / incident contact: [[OWNER: security/abuse contact email on the humangr.com domain (e.g. security@humangr.com once confirmed); this must be the same address used in SECURITY.md]] (`SECURITY.md`). Breach notifications under §12 and Data Subject communications routed to the Processor must use this address; it must be confirmed and live before launch so notice is validly given and not mis-routed.
- Legal notices to the Processor: [[OWNER: legal-notice postal address for the Processor]].
- EU/UK representative (GDPR Art. 27): [[OWNER: EU and/or UK Art. 27 representative name and contact, or "not applicable" only if the Processor is established in the EEA/UK]]. **Note:** if the Processor is **not** established in the EEA/UK and offers the service to EU/UK Data Subjects, an Art. 27 representative is **mandatory, not optional**, and the §11.4 Supervisory-Authority designation cannot be completed until it is resolved (see §11.4).

15.7. **Signatures.** Where this DPA is executed as a stand-alone document, it is
signed by authorized representatives of each Party; otherwise, acceptance of the
Agreement constitutes acceptance of this DPA.

| Processor | Customer |
|---|---|
| Name: [[OWNER: signatory name]] | Name: ____________________ |
| Title: [[OWNER: signatory title]] | Title: ____________________ |
| Date: ____________________ | Date: ____________________ |
