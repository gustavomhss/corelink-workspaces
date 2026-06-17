# Privacy Policy — CoreLink Workspaces (`clw`)

**Effective date:** June 14, 2026
**Last updated:** June 14, 2026

This Privacy Policy explains how Human Guardrail LTDA ("**HumanGR**",
"**we**", "**us**", or "**our**"), operating the product **CoreLink Workspaces** — the
`clw` command-line client and the hosted CoreLink API at `corelink-api.humangr.com`
(together, the "**Service**") — collects, uses, discloses, retains, and protects
personal data, and the rights you have over that data.

It is written to satisfy the EU General Data Protection Regulation (Regulation (EU)
2016/679, "**GDPR**"), the UK GDPR and the UK Data Protection Act 2018, and the
California Consumer Privacy Act as amended by the California Privacy Rights Act
(together, "**CCPA/CPRA**"). As HumanGR is a Brazilian controller, this policy is also
written to satisfy Brazil's Lei Geral de Proteção de Dados (Lei nº 13.709/2018,
"**LGPD**"). Where those regimes use different terms for the same
concept (for example "personal data" vs. "personal information", or "data subject"
vs. "consumer"), we use them interchangeably.

This policy covers the Service only. It does not cover third-party products you may
connect to, your own customers' data-handling practices, or websites we link to.

> **Status — operative for the open beta; full counsel review tracked before GA.**
> This Privacy Policy is operative and in effect for the current open-beta /
> incubation period. A full review by external counsel has **not** yet been
> completed and is a tracked obligation to close **before general availability** —
> a known, time-limited gap, consistent with the EU/UK Article 27 representative
> note below. Any material revisions from that review will be published here.

---

## 1. Roles: who is the controller and who is the processor

CoreLink Workspaces is a multi-tenant, business-to-business developer tool. Two
distinct processing roles apply:

1. **Customer Content** — the workspace bytes, file paths, tree structure, snapshot
   manifests, and ref names that you explicitly snapshot and upload through `clw`.
   With respect to Customer Content, **you (the customer/tenant) are the controller**
   and **HumanGR is the processor**, acting only on your documented instructions
   under our Data Processing Agreement (see `docs/DPA-subprocessors.md`). If Customer
   Content contains personal data of your own employees, end users, or third parties,
   you are responsible for the lawful basis, notices, and rights-handling owed to
   those individuals; we process it on your behalf.

2. **Account & Operational Data** — the data we need to provide, secure, bill for,
   and support the Service (for example, the identity behind a tenant account, the
   Personal Access Token used to authenticate, billing details, and support
   correspondence). With respect to this data, **HumanGR is the controller**.

This Privacy Policy describes both roles. Where a processing activity is governed by
the Data Processing Agreement rather than this policy, we say so.

**Controller of record:** Human Guardrail LTDA, a limited-liability company (sociedade
empresária limitada) with registered office at Avenida Rouxinol, 857, apt. 113, Moema, São Paulo
– SP, Brazil (CEP: 04516-001), enrolled under CNPJ 66.103.400/0001-30, incorporated
in Brazil.

---

## 2. The data we process

### 2.1 Customer Content (you are the controller; we are the processor)

When you run `clw` to snapshot or hydrate a workspace, the client transmits **only the
content you explicitly select**, plus the credential needed to authenticate. We do not
scan your machine, index your filesystem, or collect anything you did not ask the
client to snapshot.

| Data | Where it lives | Transmitted to | Purpose | Safeguards |
|---|---|---|---|---|
| **File contents** — your source / working-tree bytes | Chunked (FastCDC), BLAKE3 content-addressed, uploaded to the content-addressed store (CAS); cached locally at `~/.clw/cache` | CoreLink API only | The core product function: snapshot / hydrate workspaces | TLS in transit; every chunk BLAKE3-verified on fetch; local cache directory created `0700`. Content is **not** encrypted client-side before upload — confidentiality in transit relies on TLS, and at rest on storage-layer controls and server-enforced tenant isolation |
| **File paths / tree structure** | Inside snapshot manifests (canonical JSON, Merkle-style) uploaded with each snapshot | CoreLink API | Reconstruct the workspace tree on hydrate | TLS in transit. Paths are metadata and **may themselves contain sensitive names** — see §7 |
| **Ref names** (cache pointers) | Set/read via the CoreLink API | CoreLink API | Name a snapshot root | TLS in transit; scoped to your tenant |

### 2.2 Account & Operational Data (we are the controller)

| Data | Source | Purpose | Lawful basis (GDPR) |
|---|---|---|---|
| **Personal Access Token (PAT)** and the tenant identity it authenticates | You, at provisioning / sign-up | Authenticate you to the Service; enforce tenant isolation server-side | Performance of contract (Art. 6(1)(b)) |
| **Account / billing identity and contact details** | You / your organization | Create and administer the account, invoice, prevent fraud | Performance of contract (Art. 6(1)(b)); legal obligation (Art. 6(1)(c)) for tax/accounting records |
| **Request metadata** — for each API call: the bearer PAT, the tenant identifier in the URL path, source IP, timestamp, and a client `User-Agent` of the form `clw/<version> (<os>; <arch>)` | Generated by the request | Routing, authentication, security monitoring, abuse prevention, and compatibility/support | Legitimate interests (Art. 6(1)(f)). **Specific interest:** securing the Service and preventing abuse, fraud, and unauthorized access (source IP and request metadata), and diagnosing faults and ensuring client compatibility (`User-Agent`, timestamps). See §5 for the balancing test and how to obtain the assessment |
| **Server operational logs** (e.g., error/security/audit logs) | Generated server-side | Operate, secure, and troubleshoot the Service; detect and investigate abuse | Legitimate interests (Art. 6(1)(f)). **Specific interest:** detecting, investigating, and preventing security incidents and abuse, and operating and debugging the Service. Legal obligation (Art. 6(1)(c)) where applicable. See §5 |
| **Support correspondence** | You, when you contact us | Respond to and resolve your requests | Legitimate interests (Art. 6(1)(f)). **Specific interest:** responding to and resolving the requests you send us. Performance of contract (Art. 6(1)(b)). See §5 |

The PAT is the **single** credential the client sends. The internal
`x-corelink-tenant-id` header is set server-side and is never set by `clw`.

Under the LGPD, the corresponding legal bases (Lei nº 13.709/2018, Art. 7) are the
execution of a contract (Art. 7, V), compliance with a legal or regulatory obligation
(Art. 7, II), and the controller's legitimate interests (Art. 7, IX), mirroring the
GDPR bases identified above.

### 2.3 Telemetry — none

**CoreLink Workspaces collects no usage telemetry.** The Service is privacy-respecting
by design: version 1 of the `clw` client ships with **no telemetry network backend at
all** and transmits no analytics, usage metrics, or crash reports. The client contacts
**only** the CoreLink API endpoint you configure, and only to perform the snapshot /
hydrate operations you request.

If we ever introduce telemetry or analytics, it will be **off by default**, will be
described in an updated version of this policy **before** it is enabled, and — where
consent is the appropriate basis — will be collected only with your prior opt-in.

### 2.4 Data we do **not** collect

We do not collect special-category / sensitive personal data (GDPR Art. 9) for our own
purposes, we do not knowingly collect data from children (see §13), we do not sell or
"share" personal information for cross-context behavioral advertising (see §11), and
we do not profile you or make decisions about you by solely automated means producing
legal or similarly significant effects (Art. 22).

---

## 3. How the client transmits data (data flow)

```
your files ──FastCDC chunk + BLAKE3──> [clw] ──TLS, Bearer PAT──> CoreLink API ──> regional content store
     ▲                                   │
     └──────── hydrate ◄─────────────────┘   local cache: ~/.clw/cache (0700, plaintext, reconstructible)
   PAT: ~/.clw/config.toml (0600), never logged, https-only
```

`clw` transmits data **only** to the CoreLink API endpoint you configure. It contacts
no other host. There is no telemetry, analytics, or crash-reporting backend.

---

## 4. Why we process your data (purposes)

We process personal data to:

- provide the snapshot / hydrate functionality you request (Customer Content);
- authenticate you and enforce tenant isolation;
- operate, maintain, secure, and improve the Service;
- detect, investigate, and prevent fraud, abuse, and security incidents;
- bill for the Service and keep required financial records;
- provide customer support; and
- comply with our legal obligations and enforce our agreements.

---

## 5. Lawful bases (GDPR / UK GDPR)

For **Account & Operational Data**, the lawful bases are those identified in the table
in §2.2: principally performance of our contract with you (Art. 6(1)(b)), our
legitimate interests (Art. 6(1)(f)), and compliance with legal obligations (Art. 6(1)(c)).

Where we rely on **legitimate interests** (Art. 6(1)(f)), the specific interests we pursue
are:

- **Security and abuse prevention** — for request metadata (including source IP) and
  server security/audit logs, our interest is in protecting the Service and our customers
  by detecting, investigating, and preventing unauthorized access, fraud, and abuse;
- **Service operation and debugging** — for request metadata (including `User-Agent` and
  timestamps) and operational/error logs, our interest is in routing requests, ensuring
  client compatibility, and diagnosing and fixing faults; and
- **Responding to support requests** — for support correspondence, our interest is in
  answering and resolving the requests you choose to send us.

For each of these activities we have carried out a legitimate-interests assessment
balancing our interest against your rights, freedoms, and reasonable expectations, and we
have concluded that our interest is not overridden — in particular because the data
involved is limited to what the operation generates, is used only for the stated purpose,
and (in the case of IP and request metadata) is retained for no longer than the period
stated in §9. You may obtain a summary of the relevant legitimate-interests assessment by
contacting us at the address in §15, and you may object to this processing as described in
§10.

For **Customer Content**, you are the controller and are responsible for establishing
the lawful basis for any personal data it contains; we process it as your processor on
your documented instructions under the Data Processing Agreement.

---

## 6. Data residency, storage location, and international transfers

**Regional data residency.** At launch, Customer Content is stored **only in the United
States** (Cloudflare's `enam` region). We do not move stored Customer Content out of that
region except as necessary to provide the Service you requested or as you instruct.

Customer-selectable region routing is a roadmap item (tracked as M7.1 in
`docs/ROADMAP-go-live.md`, consistent with `docs/DPA-subprocessors.md` §11) and is **not yet
available**. **At launch there is no region selection: the US (`enam`) is the only region
served, and a sign-up that resolves to any other region (including an EEA/UK location) is
refused at provisioning rather than stored elsewhere.** The regions in which Customer
Content is stored today, and the region(s) we make available, are:

- **Today — United States only.** All Customer Content is stored in the **United States**, in
  Cloudflare's **"enam" (Eastern North America)** region (CF colo `iad`). This is the **only**
  region available at launch.
- **Other regions (roadmap M7.1) — not yet served, fail-closed.** No serving infrastructure is
  deployed outside the US region at launch. A sign-up whose region resolves to a region we do not
  yet serve (currently every region except US `enam` — e.g. an EEA/UK location) is **refused at
  provisioning** rather than silently stored in the US; the Service does **not** provision EEA/UK
  tenants today. **EEA/UK in-region data residency is not available**, and an EU/UK Article 27
  representative has not yet been appointed — a known, time-limited compliance gap (see the EU/UK
  representative note in §15 and the DPA). Customer-selectable regional residency (roadmap M7.1)
  will change this once it ships and is verified end to end.

If you have a contractual or regulatory residency requirement (for example, EEA or UK
residency under the GDPR), contact us at the address in §15 to confirm that the required
region is available **before** you rely on residency for compliance purposes. **At launch only
the US (`enam`) region is available; no EEA/UK in-region residency is offered.**

**Sub-processors.** We use the third-party infrastructure and service providers listed
in `docs/DPA-subprocessors.md`, namely:

- **Cloudflare, Inc.** — R2 object storage (content-addressed store) and Durable
  Objects (compute/state), provided over Cloudflare's global edge. Customer Content is
  currently processed and stored in the **"enam" (Eastern North America)** region.
  Cloudflare is the **sole** infrastructure sub-processor; there is no separate
  underlying cloud or CDN provider beneath it. Cloudflare's data-protection terms and
  Standard Contractual Clauses are available at
  <https://www.cloudflare.com/cloudflare-customer-dpa/>.

At v1 there is **no** payment, billing/CRM, transactional-email/notification,
crash-reporting, or telemetry sub-processor: billing is not yet live, the client ships
no email backend or telemetry, and change/breach notices are given via a published
sub-processor page together with in-product/account-admin notice. We give at least
**30 days' advance notice** of any new or replacement sub-processor via that page and an
email/in-product notice.

We impose data-protection obligations on each sub-processor by written contract that
are no less protective than those in this policy and our Data Processing Agreement, and
we remain responsible for their performance. The current list, and the process for
advance notice of changes, is maintained in `docs/DPA-subprocessors.md`.

**International transfers.** Where personal data is transferred outside the
EEA, the UK, or another jurisdiction whose laws require a transfer mechanism, we rely
on an appropriate safeguard under GDPR Chapter V — an adequacy decision where one
applies, or otherwise the European Commission's Standard Contractual Clauses (and the
UK International Data Transfer Addendum for UK transfers), together with any
supplementary measures required following a transfer-impact assessment. For
international transfers of personal data subject to Brazil's LGPD, we likewise rely on
an appropriate transfer mechanism under LGPD Arts. 33–36 (such as standard contractual
clauses or adequacy, as recognized by the ANPD). You may request a copy of the relevant
safeguard via the contact in §15.

These Chapter V safeguards apply to **both** categories of data: processor-side **Customer
Content** (covered by the Data Processing Agreement) and controller-side **Account &
Operational Data** for which HumanGR is the controller (for example, billing identity,
support correspondence, and request metadata including source IP, where these are handled
by a provider located outside your jurisdiction). Because Customer Content is stored **only
in the United States** (`enam`) at launch, EEA/UK Customer Content **is** stored and
processed outside the EEA/UK; the transfer safeguards in this paragraph (the Standard
Contractual Clauses) apply. The sub-processor list above
(once completed) identifies which providers receive controller-side personal data and the
corresponding transfer mechanism.

This policy and the Service are governed by the laws of the Federative Republic of
Brazil, and any dispute arising from them is subject to the exclusive jurisdiction of
the courts of the Judicial District (Comarca) of São Paulo, State of São Paulo, Brazil.

---

## 7. Your responsibilities as controller of Customer Content

Because the client uploads exactly what you ask it to snapshot:

- **Paths and contents are uploaded as-is.** Do not snapshot secrets, credentials, or
  personal data you do not intend to store. File and directory **names** are uploaded
  as metadata and may themselves be sensitive.
- **Establish your own lawful basis and notices** for any personal data contained in
  Customer Content, and respond to the rights requests of the individuals it concerns,
  for which we will assist you as your processor.
- **Protect your machine and your PAT.** Treat the PAT as a secret; rotate it promptly
  if you believe it has been exposed.

---

## 8. Security

We apply technical and organizational measures appropriate to the risk, including:

- **Encryption in transit:** the client requires `https://`; the development loopback
  exception uses an exact-host check, not a prefix match.
- **Credential handling at rest:** `~/.clw/config.toml` (which holds the PAT) is
  written `0600`; `~/.clw` and the local cache root are created `0700`. This reduces
  the blast radius among non-root local users on a shared machine; it is not a defense
  against a local root attacker (see `docs/THREAT-MODEL.md`).
- **Credentials never logged:** PATs are redacted from all `clw` output and
  diagnostics.
- **Integrity:** every chunk is BLAKE3 content-addressed and verified on fetch;
  manifests are canonical JSON, Merkle-style.
- **Tenant isolation** is enforced server-side; the PAT scopes every request to your
  tenant.
- **Local cache:** the cache at `~/.clw/cache` holds plaintext file contents and is
  reconstructible from the server; it is created `0700`.

No method of transmission or storage is perfectly secure, and we cannot guarantee
absolute security. Our detailed security posture is described in `SECURITY.md` and
`docs/THREAT-MODEL.md`.

**Personal data breaches.** Where we are the controller and a breach is likely to
result in a risk to individuals' rights and freedoms, we will notify the competent
supervisory authority without undue delay and, where required, within 72 hours of
becoming aware of it, and will notify affected individuals where the law requires.
Under the LGPD (Art. 48), we will likewise notify Brazil's Autoridade Nacional de
Proteção de Dados (ANPD) and affected data subjects within a reasonable period where a
breach may create risk or relevant harm. Where we are your processor, we will notify you
without undue delay after becoming aware of a breach affecting your Customer Content.

---

## 9. Retention and deletion

**Customer Content.** We retain Customer Content for as long as your tenant maintains
it, and delete it on your instruction or following termination of your account, subject
to the erasure rights in §10. Following termination, you have a **30-day** window to export your Customer Content;
thereafter we delete it, with deletion completed within **30 days** of the end of that
export window. Deletion propagates to backups and replicas within a further **35 days**.

**Account & Operational Data.** We retain account, billing, and operational data for as
long as your account is active and thereafter only as long as needed for the purposes
described in this policy or as required by law (for example, tax and accounting records).
Specific retention periods, by category, are: **account data** — for the life of the
account plus **30 days**; **server operational/security/audit logs and request metadata
(including source IP)** — **90 days**; and **billing records** — **5 years** (the
Brazilian fiscal standard). In particular, request metadata (including source IP) and
server operational/security/audit logs are retained for **90 days**, after which they
are deleted or aggregated/anonymized.

**Right to erasure (GDPR Art. 17; LGPD Art. 18).** We honor erasure requests as
described here and in §10. Full self-service erasure of your stored content is
**available today** through the client:

- **Reference deletion (self-service).** The client `clw rm` operation removes the
  reference (cache pointer) to a single snapshot, making it unreachable through
  normal use.
- **Full content erasure (self-service).** The client `clw erase` operation
  **irreversibly deletes all of your tenant's data** — every reference and every
  underlying content-addressed (CAS) blob — directly through the client (it is the
  GDPR Art. 17 path; the server-side reference-enumeration and CAS delete/list
  capabilities it relies on are deployed and live). `clw prune` additionally
  garbage-collects unreferenced CAS blobs. These operations carry safety rails
  (dry-run by default; `clw erase` also requires explicit `--yes` and a typed
  tenant confirmation) because they cannot be undone.
- **Operator-assisted erasure (alternative).** If you prefer, or cannot run the
  client, we will effect full erasure of your stored Customer Content on a verified
  written request to the contact in §15, within **30 days** of the request.

On account closure, and on a valid erasure request, we delete the affected Customer
Content through the process above and we delete Account & Operational Data subject to the
limited exceptions the law permits (for example, records we must retain for legal, tax, or
dispute-resolution purposes). This section is consistent with `docs/DPA-subprocessors.md`
§10 (right to erasure — commitment and current limitation). See §10 for how to make a
request and our response timelines.

**Local data on your machine.** Removing the `clw` binary and the `~/.clw` directory
deletes your local configuration, your PAT, and the local cache. The local cache is
ephemeral and reconstructible from the server; deleting it does not delete data stored
server-side, which is handled through the erasure process above.

**Backups.** Residual copies may persist in routine, time-limited backups after
deletion from production systems; such copies are isolated, are not used for any other
purpose, and are overwritten or deleted on the ordinary backup-rotation cycle, within
**35 days** of deletion from production.

---

## 10. Your data-protection rights

Subject to the conditions and exceptions in applicable law, you have the rights below.
Where we act as **processor** for Customer Content, we will forward a request we receive
to the relevant controller, or assist that controller in responding, rather than act on
it directly.

### 10.1 Rights under GDPR / UK GDPR

- **Access** — obtain confirmation of, and a copy of, the personal data we hold about
  you (Art. 15).
- **Rectification** — have inaccurate or incomplete data corrected (Art. 16).
- **Erasure ("right to be forgotten")** — have your data deleted (Art. 17). We honor this
  right through the mechanism described in §9: both reference deletion (`clw rm`) and
  full self-service erasure of all stored content (`clw erase`) are available today
  through the client, with an operator-assisted process available as an alternative on
  verified request. See §9 for the mechanism and §10.3 for timelines.
- **Restriction** — restrict processing in certain circumstances (Art. 18).
- **Portability** — receive the data you provided in a structured, commonly used,
  machine-readable format, and have it transmitted to another controller where
  technically feasible (Art. 20).
- **Objection** — object to processing based on legitimate interests, and to direct
  marketing at any time (Art. 21).
- **Withdraw consent** — where processing is based on consent, withdraw it at any time,
  without affecting prior processing.
- **Automated decisions** — not be subject to a decision based solely on automated
  processing that produces legal or similarly significant effects (Art. 22); we do not
  carry out such processing.
- **Lodge a complaint** — with your local supervisory authority (in the EEA) or the UK
  Information Commissioner's Office (in the UK). We would, however, appreciate the
  chance to address your concern first.

Under Brazil's LGPD (Art. 18), data subjects hold equivalent rights — including
confirmation of processing, access, correction, anonymization/blocking/deletion,
portability, information about sharing, and revocation of consent — and may petition
Brazil's ANPD; the mechanisms described in this section and §10.3 apply equally to LGPD
requests.

### 10.2 Rights under CCPA/CPRA (California residents)

- **Right to know / access** the categories and specific pieces of personal information
  we have collected, the sources, the purposes, and the categories of recipients.
- **Right to delete** personal information we have collected, subject to exceptions.
- **Right to correct** inaccurate personal information.
- **Right to opt out of sale/sharing** — we do **not** sell personal information and do
  **not** "share" it for cross-context behavioral advertising, so there is nothing to
  opt out of; we will honor opt-out preference signals (e.g., Global Privacy Control)
  should that change.
- **Right to limit use of sensitive personal information** — we do not use sensitive
  personal information for purposes that trigger this right.
- **Right to non-discrimination** for exercising your rights.

#### CCPA/CPRA notice at collection — categories of personal information

The table below maps the data described functionally in §2 to the CCPA/CPRA statutory
categories, the categories of sources, the business or commercial purpose, the categories
of recipients, and the retention period (or the criteria used to determine it). We do
**not** collect personal information for the purpose of selling or sharing it, and we do
not sell or share any of the categories below.

| CCPA/CPRA category | Personal information (per §2) | Categories of sources | Business / commercial purpose | Categories of recipients | Retention period / criteria |
|---|---|---|---|---|---|
| **Identifiers** | Account / billing identity and contact details; tenant identifier; Personal Access Token (credential) | You / your organization; generated at provisioning | Provide, authenticate, administer, and bill for the Service; enforce tenant isolation | Sub-processors / service providers (§6, §11); professional advisers; legal/government recipients where required | While the account is active and as needed for the purposes in §9, then per the schedule in §9 (account data: life of account + 30 days; billing: 5 years) |
| **Internet or other electronic network activity information** | Request metadata: source IP, timestamp, `User-Agent`; server operational/security/audit logs | Generated by your use of the Service | Routing, authentication, security monitoring, abuse prevention, debugging, compatibility | Sub-processors / service providers (§6); legal/government recipients where required | 90 days |
| **Commercial information** | Billing / transaction records | You / your organization | Bill for the Service; keep required financial records | Payment / billing sub-processor (§6); professional advisers; tax/legal recipients | As required for tax and accounting law (5 years, Brazilian fiscal standard) and otherwise per §9 |
| **Customer Content** (file contents, paths/tree structure, ref names) | The workspace bytes and metadata you explicitly snapshot. For this category HumanGR is a **service provider/processor**, not a business collecting for its own purposes; any personal information it contains is determined by you (the controller) | You, via the `clw` client | Provide the snapshot / hydrate functionality you request | Object-storage / infrastructure sub-processors (§6, `docs/DPA-subprocessors.md`) | As your tenant maintains it, then per §9 and the erasure mechanism in §9 (30-day export window, then deleted within 30 days of its end) |

We do not collect special-category or sensitive personal information for our own purposes
(see §2.4), and we do not use personal information for cross-context behavioral
advertising. The purposes and disclosures above are described further in §2, §4, §6, §9,
and §11.

### 10.3 How to exercise your rights

Submit a request to privacy@humangr.com. We will
verify your identity (and an authorized agent's authority, where applicable) before
acting. We respond to GDPR/UK GDPR requests within **one month**, extendable by up to
two further months for complex or numerous requests with notice to you; and to
CCPA/CPRA requests within **45 days**, extendable by a further 45 days with notice; and
to LGPD requests within the timeframes set by Brazilian law (in general, without undue
delay, and immediately or within 15 days for confirmation-of-processing and access
requests). We do not charge a fee unless a request is manifestly unfounded or excessive,
as permitted by law.

---

## 11. Disclosure of personal data

We disclose personal data only:

- to **sub-processors and service providers** that process data on our behalf under
  contract, as listed in §6 / `docs/DPA-subprocessors.md`;
- to **professional advisers** (lawyers, auditors, accountants) under duties of
  confidentiality;
- in connection with a **merger, acquisition, financing, or sale of assets**, in which
  case we will require the recipient to honor this policy or notify you of any material
  change;
- where required by **law, legal process, or a lawful government request**, or to
  establish, exercise, or defend legal claims; and
- to **protect the rights, safety, and security** of HumanGR, our users, or the public,
  including to prevent fraud and abuse.

We do **not** sell personal information, and we do **not** share it for cross-context
behavioral advertising.

---

## 12. Cookies and tracking

The `clw` command-line client uses no cookies and performs no tracking. At v1 there is
no hosted account portal, and humangr.com uses only the strictly necessary cookies (if
any) required to serve the site; we do not use cookies for analytics, advertising, or
cross-site tracking. If this changes, we will describe the cookies used here or in a
separate cookie notice before any such use begins.

---

## 13. Children

The Service is a business tool not directed to children and is not intended for use by
anyone under the age of 16 (or the minimum age required by your jurisdiction). We do not
knowingly collect personal data from children. If you believe a child has provided us
personal data, contact us (see §15) and we will delete it.

---

## 14. Changes to this policy

We may update this policy from time to time. When we make material changes, we will
update the "Last updated" date above and notify you by a reasonable means before the
change takes effect. We give such notice through a published policy/sub-processor page
together with an in-product or account-admin notice, with at least **30 days'** advance
notice of material changes.
Where a change relies on a lawful basis of **consent**, the change will not apply to you
until you have given that consent; we will not treat continued use of the Service as
consent. For changes that do **not** require consent as their lawful basis, your continued
use of the Service after the change becomes effective indicates your awareness of the
updated policy, without prejudice to any objection or other right you may exercise under
§10.

---

## 15. Contact

| Purpose | Contact |
|---|---|
| Privacy / data-subject requests / Data Protection Officer (LGPD Encarregado: Gustavo Schneiter) | privacy@humangr.com |
| Customer support | support@humangr.com |
| Security and abuse reports | security@humangr.com (see `SECURITY.md`) |
| Legal notices (postal) | Human Guardrail LTDA, Avenida Rouxinol, 857, apt. 113, Moema, São Paulo – SP, Brazil (CEP: 04516-001) |

**Controller:** Human Guardrail LTDA, a sociedade empresária limitada incorporated in Brazil
(CNPJ 66.103.400/0001-30).

**EU / UK representative (GDPR Art. 27).** HumanGR is established in Brazil, outside the
EEA and the UK. During the current open beta the Service may be accessed by EU/UK data
subjects, but HumanGR has **not yet appointed** an Art. 27 representative. We acknowledge
that, where a service is offered to EU/UK data subjects, Art. 27 makes appointing a
representative **mandatory**; until one is appointed this is an **outstanding obligation
and a known, time-limited compliance gap** during the beta. HumanGR will appoint a
representative — and publish that representative's name and contact here — **before general
availability (the end of the open beta)**.
