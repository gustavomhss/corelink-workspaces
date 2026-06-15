# Terms of Service — CoreLink Workspaces (the hosted CoreLink API and `clw` client)

**Effective date:** June 14, 2026

These Terms of Service ("**Terms**") form a binding agreement between
Human Guardrail LTDA, a limited-liability company (sociedade empresária limitada)
organized under the laws of Brazil ("**HumanGR**",
"**we**", "**us**", or "**our**"), and the individual or legal entity that creates
a CoreLink tenant or otherwise accesses or uses the Service ("**Customer**",
"**you**", or "**your**"). By creating a tenant, configuring a Personal Access
Token ("**PAT**"), or accessing or using the Service, you agree to these Terms. If
you are entering into these Terms on behalf of an organization, you represent that
you have authority to bind that organization, and "you" refers to that
organization.

If you do not agree to these Terms, you must not access or use the Service.

## 1. Definitions

1.1 "**Service**" means the hosted CoreLink Workspaces backend made available by
HumanGR at `corelink-api.humangr.com` (and any successor or related endpoints),
including the content-addressed remote cache, snapshot/hydrate, and ref-management
functionality, together with associated documentation.

1.2 "**Client**" or "**`clw`**" means the CoreLink Workspaces command-line client
binary distributed by HumanGR, through which you access the Service.

1.3 "**Tenant**" means the isolated account namespace provisioned to you, against
which your PATs authenticate and within which your Customer Data is scoped.

1.4 "**PAT**" means a per-tenant Bearer Personal Access Token used to authenticate
to the Service.

1.5 "**Customer Data**" means the workspace content, file contents, file paths and
tree structure, manifests, and ref names that you snapshot, upload, store, or
otherwise process through the Service.

1.6 "**Documentation**" means the user and security documentation HumanGR makes
available for the Service and the Client, including `SECURITY.md`, the Privacy
Policy (`docs/PRIVACY-POLICY.md`), the Data Processing Addendum
(`docs/DPA-subprocessors.md`), and the End-User License Agreement (`EULA.md`).

## 2. Agreement structure; relationship to the EULA, Privacy Policy and DPA

2.1 **Scope of these Terms.** These Terms govern your access to and use of the
**Service**. Your right to install and run the **`clw` Client binary** is granted
separately by, and is subject to, the **End-User License Agreement (`EULA.md`)**
that accompanies the artifact. The `clw` Client is proprietary, closed-source,
commercially-licensed software; you receive executable binaries only and no source
code.

2.2 **Incorporated documents.** The following are incorporated into these Terms by
reference and form part of the agreement between the parties: (a) the EULA; (b) the
Privacy Policy; (c) the Data Processing Addendum ("**DPA**") where you are subject
to applicable data-protection law; and (d) any order form, subscription, or
evaluation entitlement agreed between the parties (each, an "**Order**").

2.3 **Order of precedence.** In the event of a conflict, the following order of
precedence governs, from highest to lowest: (1) a signed Order or signed master
agreement, if any; (2) the DPA, with respect to the processing of personal data;
(3) these Terms; (4) the EULA, with respect to the licensing of the Client binary;
and (5) the Documentation. Notwithstanding the foregoing, the EULA controls with
respect to ownership of, and the license to, the Client software, and the DPA
controls with respect to the processing of personal data.

2.4 **Acceptance.** By configuring a PAT and using `clw` against the Service, or by
otherwise accessing the Service, you accept these Terms, the EULA, and the Privacy
Policy.

2.5 **Intended for business use; consumer rights preserved.** The Service is offered
and intended for use by businesses and by individuals acting for purposes relating
to their trade, business, craft, or profession, and not for personal, family, or
household purposes. If, notwithstanding the foregoing, you access the Service as a
consumer within the meaning of applicable mandatory consumer-protection law, then
(a) nothing in these Terms excludes, restricts, or modifies any right, remedy,
guarantee, or liability that such law confers on you and that may not lawfully be
excluded, restricted, or modified by agreement; (b) to the extent of any conflict
between such mandatory consumer law and these Terms, that law prevails; and (c) any
provision of these Terms governing amendment (Section 15.1), liability (Sections 10
and 11), or dispute resolution (Section 14) applies to you only to the extent
permitted by, and as modified by, that mandatory law.

## 3. The Service; license to use

3.1 **Service license.** Subject to your continuous compliance with these Terms and
payment of all applicable fees, HumanGR grants you a non-exclusive,
non-transferable, non-sublicensable right to access and use the Service for your
internal business purposes during the term of these Terms, subject to suspension and
termination solely as set out in Section 9.

3.2 **Client license.** Your right to install and run the `clw` Client is governed
by the EULA. Summarized for convenience and without modifying the EULA: the Client
license is non-exclusive, non-transferable, and revocable, is limited to executable
binary form for use with a valid Tenant, and does not permit public
redistribution. The Client is distributed only through an authenticated download
surface and not via any public repository or package registry.

3.3 **No other rights.** All rights not expressly granted are reserved by HumanGR
and its licensors. Nothing in these Terms transfers any ownership of, or
intellectual-property right in, the Service or the Client to you.

## 4. Accounts, tenants and credentials

4.1 **Eligibility and registration.** You must hold a valid CoreLink Tenant and at
least one PAT to use the Service. You must provide accurate registration
information and keep it current.

4.2 **Credential security.** You are responsible for safeguarding your PATs and for
all activity occurring under them. The Client stores the PAT with `0600`
permissions, never logs it, redacts it in output and diagnostics, and transmits it
only over `https://` (see `SECURITY.md`); however, protecting your machines and
environment, controlling who has access to your PATs, and promptly rotating or
revoking any exposed PAT remain your responsibility.

4.3 **Authentication and tenant isolation.** Authentication is by per-tenant Bearer
PAT, and tenant isolation is enforced server-side. You must not attempt to access
any Tenant other than your own, and you must not set, forge, or manipulate
server-internal scoping headers (for example, `x-corelink-tenant-id`, which the
Client never sets).

4.4 **Responsibility for users.** Where your Tenant is used by multiple individuals,
you are responsible for those individuals' compliance with these Terms.

## 5. Customer Data and content responsibility

5.1 **Your data, your responsibility.** As between the parties, you retain all
right, title, and interest in and to your Customer Data. You are solely responsible
for your Customer Data, including for ensuring that you have all rights and consents
necessary to upload, store, and process it through the Service and that doing so
does not violate any law or third-party right.

5.2 **What is uploaded.** The Client uploads only the workspace content you
explicitly snapshot, together with the PAT needed to authenticate to the Service.
File paths and tree structure are uploaded as metadata within manifests and may
contain sensitive names; you are responsible for excluding content (including
secrets and personal data) that you do not intend to store. Tooling to assist with
exclusion and secret-scanning is described in the Documentation and may not be
available in all releases.

5.3 **License to operate the Service.** You grant HumanGR a worldwide,
non-exclusive, royalty-free license to host, store, transmit, chunk, deduplicate,
content-address, cache, and otherwise process your Customer Data solely as necessary
to provide, secure, maintain, and support the Service, and as further described in
the Privacy Policy and DPA. HumanGR does not acquire any ownership of your Customer
Data.

5.4 **Data residency.** Customer Data is currently stored in the "enam" (Eastern
North America) region via our infrastructure sub-processor, Cloudflare, Inc.
Customer-selectable regional residency is on our roadmap (M7.1) but is not yet
available, and an indicated region does not yet guarantee residency. Regional
residency, sub-processors, and the technical and organizational measures applicable
to your Customer Data are described in the Privacy Policy and the DPA
(`docs/DPA-subprocessors.md`).

5.5 **Deletion and erasure.** You may delete your Customer Data through the Service,
and, where applicable data-protection law confers a right to erasure (including
Article 17 of the GDPR and Article 18 of Brazil's LGPD (Lei nº 13.709/2018)), you
may request deletion of all Customer Data stored for your Tenant, in each case as
described in, and subject to the limitations and processes set out in, the Privacy
Policy and the DPA. Erasure is currently operator-assisted, pending roadmap M6.4
(server-side D-7/D-8), and HumanGR will give effect to a valid erasure request
within thirty (30) days; deletion is subject to a reasonable processing period and
to retention HumanGR is required by law to maintain.

5.6 **CCPA/CPRA service-provider status.** To the extent Customer Data includes
personal information subject to the California Consumer Privacy Act, as amended by
the California Privacy Rights Act (collectively, "**CCPA/CPRA**"), HumanGR acts as a
"service provider" and processes such personal information solely to provide,
secure, maintain, and support the Service on your behalf and for no other purpose.
HumanGR does not "sell" or "share" (as those terms are defined under the CCPA/CPRA)
Customer Data, and will not retain, use, or disclose Customer Data outside the
direct business relationship with you or otherwise except as permitted under the
CCPA/CPRA, in each case as further described in the DPA. As a Brazilian controller,
HumanGR likewise processes such personal data as an operator (operador) on your
behalf under Brazil's LGPD (Lei nº 13.709/2018), solely for the purposes set out
above.

## 6. Acceptable use

You agree **not** to, and not to permit any third party to:

6.1 **Reverse-engineer or derive source.** Reverse-engineer, decompile, or
disassemble the Client, or attempt to extract its source code, underlying ideas,
algorithms, or file/protocol formats, except, and only to the extent, applicable
mandatory law expressly permits despite this restriction.

6.2 **Redistribute or resell.** Redistribute, resell, sublicense, publish, host, or
make available the Client or the Service (in whole or in part) to any third party,
or circumvent the authenticated download surface or any signing, checksum, or
integrity-verification mechanism.

6.3 **Attack or abuse the Service.** Exceed published rate or quota limits other
than via the Client's built-in adaptive backoff (which honors `Retry-After`);
attempt to access another Tenant's data; probe, scan, penetration-test (absent
prior written authorization), overload, or deny service to the Service; or forge,
inject, or alter content to defeat integrity checks. The Client treats all
server-returned manifests and chunks as untrusted and verifies them (see
`docs/THREAT-MODEL.md`); you must not attempt to defeat those checks.

6.4 **Tamper with integrity or scoping.** Modify or bypass PAT-based tenant scoping,
set server-internal headers, or circumvent path-traversal or hydrate hardening or
any other security control.

6.5 **Store unlawful or infringing content.** Use the Service to store, transmit, or
distribute content that is unlawful, that is malware or otherwise malicious, or that
infringes, misappropriates, or violates any third party's intellectual-property,
privacy, or other rights.

6.6 **Strip notices.** Remove or alter any proprietary, copyright, or trademark
notice, the `NOTICE` file, or the third-party attribution bundle shipped with the
Client.

6.7 **Violate export or sanctions law.** Use the Service or Client in violation of
applicable export-control or sanctions law (see Section 12 and
`docs/EXPORT-CONTROL.md`).

6.8 **High-risk use.** Use the Service or Client in any environment requiring
fail-safe performance or in which failure could lead to death, personal injury, or
severe physical or environmental damage; the Service and Client are not designed or
intended for such use.

## 7. Fees

7.1 **Charges.** Fees, billing frequency, payment terms, and any usage- or
capacity-based metering are set out in the applicable Order or published pricing.
Unless the applicable Order states otherwise, fees are non-cancelable and amounts
paid are non-refundable, except as required by law. Billing is not yet live; during
the current incubation phase the Service is provided without charge, and any
pricing, billing cadence, free-tier, or evaluation terms will be set out in the
applicable Order or published pricing before fees take effect.

7.2 **Taxes.** Fees are exclusive of taxes, levies, and duties. You are responsible
for all such amounts, excluding taxes based on HumanGR's net income.

7.3 **Late payment.** Overdue amounts may accrue interest at the rate of 1.5% per
month or the maximum permitted by applicable law, whichever is lower, and HumanGR
may suspend the Service for non-payment in accordance with Section 9.

## 8. Service availability and support

8.1 **Availability.** HumanGR will use commercially reasonable efforts to make the
Service available, but the Service is otherwise provided on an "as-available" basis.
Any committed service-level objective, uptime percentage, and associated remedy
(such as service credits) apply only if and to the extent set out in a written
Service Level Agreement. HumanGR does not currently offer a committed uptime
service-level objective or service-credit schedule; any such commitment will apply
only if and to the extent set out in a written Service Level Agreement.

8.2 **Maintenance.** HumanGR may perform scheduled and emergency maintenance and may
modify, suspend, or discontinue features of the Service, using commercially
reasonable efforts to limit disruption and to give reasonable advance notice of
materially adverse changes where practicable.

8.3 **Support.** Support channels and target response times, if any, are as
described in the applicable Order or published support policy. Support requests may
be sent to support@humangr.com; HumanGR does not commit to a specific
response-time target during the current incubation phase.

8.4 **Security and vulnerability reporting.** Do not publicly disclose any suspected
vulnerability. Report it privately, in accordance with the coordinated-disclosure
process in `SECURITY.md`, to security@humangr.com. Pre-1.0, only
the latest released version of the Client is supported.

8.5 **Confidentiality.** "**Confidential Information**" means non-public information
disclosed by one party (the "discloser") to the other (the "recipient") that is
designated as confidential or that a reasonable person would understand to be
confidential given its nature and the circumstances of disclosure, including the
non-public terms of any Order, HumanGR's non-public security and threat-model
materials, and any suspected vulnerability reported under Section 8.4.
Confidential Information does not include information that is or becomes public
through no fault of the recipient, was rightfully known to the recipient without
obligation of confidentiality, is rightfully received from a third party without
restriction, or is independently developed without use of the discloser's
Confidential Information. The recipient will use Confidential Information only to
exercise its rights and perform its obligations under these Terms, will protect it
with at least reasonable care, and will not disclose it except to its personnel and
advisors who need to know it and are bound by confidentiality obligations no less
protective than these. The recipient may disclose Confidential Information to the
extent required by law or legal process, provided it gives reasonable prior notice
where lawful. This Section 8.5 does not limit the processing of Customer Data, which
is governed by Section 5, the Privacy Policy, and the DPA.

## 9. Suspension and termination

9.1 **Suspension.** HumanGR may suspend your access to the Service, in whole or in
part, immediately and with such notice as is reasonably practicable, if (a) you
breach Section 6 (Acceptable use) or fail to pay undisputed fees when due; (b) your
use poses a security risk to, or may adversely affect, the Service or other
customers; or (c) HumanGR is required to do so by law. HumanGR will restore access
promptly once the basis for suspension is resolved.

9.2 **Term.** These Terms apply from your first access to the Service and continue
until terminated in accordance with this Section or the applicable Order.

9.3 **Termination for breach.** Either party may terminate these Terms (and any
related Order) for the other party's material breach that remains uncured for thirty
(30) days after written notice; HumanGR may terminate immediately for breach of
Section 6 or for any breach not capable of cure.

9.4 **Termination for convenience.** Either party may terminate these Terms for
convenience on the notice and subject to the terms set out in the applicable Order;
absent an Order, either party may terminate for convenience on thirty (30) days'
written notice.

9.5 **Effect of termination.** On termination, your right to access the Service and
to use the Client ceases; you must stop using and remove the Client. For a period of
thirty (30) days following termination (other than termination by HumanGR for your
breach of Section 6), HumanGR will make your Customer Data available for export;
thereafter HumanGR may delete your Customer Data in accordance with the Privacy
Policy and DPA, with deletion completed within thirty (30) days of the end of that
export window and propagated to backups and replicas within thirty-five (35) days. Your right
to request erasure under Section 5.5 is unaffected.

9.6 **Survival.** Sections 1, 2.3, 3.3, 5.1, 5.3 (as to processing already
performed), 5.5, 5.6, 6, 7 (as to amounts accrued), 8.4, 8.5, 9.5, 9.6, 10, 11, 12,
13, and 14 survive termination, together with Sections 15.2, 15.3, 15.6, 15.7, and
15.8 and any other provision of Section 15 that by its nature should survive. Any
expiration or termination is without prejudice to rights and obligations that
accrued before it.

## 10. Disclaimers

10.1 EXCEPT AS EXPRESSLY STATED IN THESE TERMS OR AN APPLICABLE WRITTEN SLA, THE
SERVICE, THE CLIENT, AND ALL RELATED MATERIALS ARE PROVIDED **"AS IS"** AND **"AS
AVAILABLE"**, WITHOUT WARRANTY OF ANY KIND. TO THE MAXIMUM EXTENT PERMITTED BY
APPLICABLE LAW, HUMANGR DISCLAIMS ALL WARRANTIES, WHETHER EXPRESS, IMPLIED,
STATUTORY, OR OTHERWISE, INCLUDING ANY IMPLIED WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE, AND NON-INFRINGEMENT, AND ANY WARRANTY
ARISING FROM COURSE OF DEALING OR USAGE OF TRADE.

10.2 HUMANGR DOES NOT WARRANT THAT THE SERVICE OR CLIENT WILL BE UNINTERRUPTED,
TIMELY, SECURE, OR ERROR-FREE, OR THAT DATA WILL NOT BE LOST OR CORRUPTED. THE
CLIENT IS PRE-1.0; NO STABILITY OR SEMANTIC-VERSIONING GUARANTEE EXISTS BEFORE 1.0
(SEE `SECURITY.md`). YOU ARE RESPONSIBLE FOR MAINTAINING INDEPENDENT BACKUPS OF
DATA YOU CANNOT AFFORD TO LOSE; THE LOCAL CLIENT CACHE IS EPHEMERAL AND
RECONSTRUCTIBLE AND IS NOT A BACKUP.

10.3 Some jurisdictions do not allow the exclusion of certain warranties, so some of
the above exclusions may not apply to you to the extent prohibited by law.

## 11. Limitation of liability

11.1 **Exclusion of indirect damages.** TO THE MAXIMUM EXTENT PERMITTED BY
APPLICABLE LAW, NEITHER PARTY WILL BE LIABLE FOR ANY INDIRECT, INCIDENTAL, SPECIAL,
CONSEQUENTIAL, EXEMPLARY, OR PUNITIVE DAMAGES, OR FOR ANY LOSS OF PROFITS, REVENUE,
DATA, USE, OR GOODWILL, ARISING OUT OF OR RELATED TO THESE TERMS OR THE SERVICE OR
CLIENT, HOWEVER CAUSED AND UNDER ANY THEORY OF LIABILITY, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGES.

11.2 **General liability cap.** SUBJECT TO SECTIONS 11.3 AND 11.4 AND TO THE MAXIMUM
EXTENT PERMITTED BY APPLICABLE LAW, EACH PARTY'S TOTAL AGGREGATE LIABILITY ARISING
OUT OF OR RELATED TO THESE TERMS WILL NOT EXCEED THE GREATER OF (A) THE TOTAL FEES
PAID OR PAYABLE BY YOU FOR THE SERVICE IN THE TWELVE (12) MONTHS PRECEDING THE EVENT
GIVING RISE TO THE CLAIM, OR (B) US$100 (OR ITS EQUIVALENT IN BRAZILIAN REAIS).

11.3 **HumanGR super-cap.** Notwithstanding Section 11.2 and to the maximum extent
permitted by applicable law, HumanGR's total aggregate liability for (a) its
indemnification obligations under Section 13.2 (IP infringement); (b) its breach of
its confidentiality obligations under Section 8.5; and (c) its breach of its
obligations under the DPA or applicable data-protection law in respect of Customer
Data (including a data breach attributable to HumanGR) will not exceed two times
(2×) the total fees paid or payable by you for the Service in the twelve (12) months
preceding the event giving rise to the claim. This Section 11.3 caps HumanGR's
liability for the listed matters in lieu of the uncapped treatment in Section 11.4,
and the matters in this Section 11.3 are correspondingly carved out of Section 11.4
as they relate to HumanGR.

11.4 **Matters not subject to the cap.** The exclusion in Section 11.1 and the caps
in Sections 11.2 and 11.3 do not apply to, and the following are not limited by them:
(a) either party's breach of Section 6 (Acceptable use) or Section 3 (license scope);
(b) either party's indemnification obligations under Section 13, except that
HumanGR's liability under Section 13.2 is subject to the super-cap in Section 11.3;
(c) either party's breach of its confidentiality obligations under Section 8.5,
except that HumanGR's liability for such breach is subject to the super-cap in
Section 11.3; (d) your payment obligations; or (e) any liability that cannot be
excluded or limited under applicable law (such as liability for death or personal
injury caused by negligence, or for fraud or fraudulent misrepresentation).

11.5 **Basis of the bargain.** The limitations in Sections 10 and 11 are a
fundamental basis of the bargain between the parties and apply notwithstanding the
failure of any limited remedy of its essential purpose.

## 12. Export compliance and sanctions

You represent that you are not located in, organized under the laws of, or
ordinarily resident in any country or territory subject to comprehensive sanctions,
and that you are not a person with whom dealings are prohibited under applicable
export-control or sanctions law. You will not access, use, export, or re-export the
Service or Client in violation of such law. The Client ships cryptography (TLS via
rustls, and BLAKE3 content digests). The Client is self-classified as mass-market
cryptography under ECCN 5D992.c (no ENC §740.17 license required; self-classified
and subject to counsel confirmation), as described in `docs/EXPORT-CONTROL.md`. You
may not access, use, export, or re-export the Service or Client to any OFAC
comprehensively-sanctioned destination, including Cuba, Iran, North Korea, Syria, or
the Crimea, Donetsk, or Luhansk regions of Ukraine.

## 13. Indemnification

13.1 **By you.** You will defend, indemnify, and hold harmless HumanGR and its
affiliates, and their respective officers, directors, employees, and agents, from
and against any third-party claim, and any resulting losses, damages, liabilities,
costs, and reasonable attorneys' fees, arising out of or relating to (a) your
Customer Data, including any claim that it infringes or misappropriates a third
party's rights or violates applicable law; (b) your use of the Service or Client in
breach of these Terms; or (c) your violation of Section 6 or Section 12.

13.2 **By HumanGR (IP infringement).** HumanGR will defend you against any
third-party claim alleging that the Service or the Client, as provided by HumanGR and
used in accordance with these Terms and the EULA, infringes or misappropriates that
third party's patent, copyright, trademark, or trade-secret right, and HumanGR will
indemnify you against the damages and reasonable attorneys' fees finally awarded
against you by a court of competent jurisdiction, or agreed in settlement by HumanGR,
on such a claim.

(a) **Remedies.** If the Service or Client is, or in HumanGR's reasonable opinion is
likely to become, the subject of such a claim, HumanGR may, at its option and
expense: (i) procure for you the right to continue using the affected item; (ii)
modify or replace it so that it becomes non-infringing while remaining materially
equivalent in functionality; or (iii) if (i) and (ii) are not commercially
reasonable, terminate the affected portion of the Service or Client on notice and
refund any prepaid, unused fees attributable to the terminated portion for the
period after termination. This Section 13.2 states HumanGR's entire liability, and
your exclusive remedy, for any claim of intellectual-property infringement or
misappropriation.

(b) **Exclusions.** HumanGR has no obligation under this Section 13.2 to the extent a
claim arises from (i) Customer Data; (ii) combination, operation, or use of the
Service or Client with software, hardware, data, or services not provided or
authorized in writing by HumanGR, where the claim would have been avoided absent the
combination; (iii) modification of the Service or Client other than by or for
HumanGR; (iv) use of the Service or Client other than in accordance with these
Terms, the EULA, or the Documentation; (v) your continued use after notice to modify
or cease, or after a non-infringing version is made available; or (vi) any item
provided on a no-charge, beta, or evaluation basis.

HumanGR's liability under this Section 13.2 is subject to the monetary super-cap in
Section 11.3 (two times (2×) the fees paid or payable in the preceding 12 months),
and is subject to the exclusions in Section 13.2(b); no other exclusions to this
indemnity apply.

13.3 **Procedure.** The indemnified party will promptly notify the indemnifying
party of the claim (provided that a failure to give prompt notice relieves the
indemnifying party only to the extent it is materially prejudiced), grant it sole
control of the defense and settlement (provided no settlement imposing a non-monetary
obligation, an admission of fault, or any unindemnified liability on the indemnified
party is made without that party's prior written consent), and provide reasonable
cooperation at the indemnifying party's expense. The indemnified party may
participate in the defense with its own counsel at its own expense.

## 14. Governing law and dispute resolution

14.1 **Governing law.** These Terms are governed by and construed in accordance with
the laws of the Federative Republic of Brazil, without regard to its
conflict-of-laws principles, and excluding the United Nations Convention on
Contracts for the International Sale of Goods.

14.2 **Venue and dispute resolution.** The parties submit to the exclusive
jurisdiction and venue of the courts of the Judicial District (Comarca) of São
Paulo, State of São Paulo, Brazil, for any dispute arising out of or relating to
these Terms. The parties do not agree to mandatory arbitration or a class-action
waiver. Any arbitration agreement or class-action waiver
inserted in this Section 14.2 applies only to Customers that are not consumers under
applicable mandatory consumer-protection law; it does not apply to, and does not
deprive any consumer of, any non-waivable forum, collective-redress, or remedy that
such law confers (see Section 2.5).

## 15. General

15.1 **Changes to these Terms.** HumanGR may update these Terms from time to time.
HumanGR will provide notice of changes by posting the updated Terms at
https://humangr.com/legal/terms with an updated effective date and, where the
Service supports it, by in-product or account-admin notice to the Tenant admin.

(a) For changes that are not materially adverse to you (for example, clarifications,
new optional features, or changes required by law), the updated Terms take effect as
stated in the notice, and your continued use of the Service after the effective date
constitutes acceptance.

(b) For changes that are materially adverse to you, HumanGR will give you advance
notice of at least thirty (30) days before the change takes effect. Such changes
take effect only upon your affirmative acceptance (for example, by clicking to
accept) or, where permitted by law and absent affirmative acceptance, upon your
continued use after the effective date. If you do not agree to a materially adverse
change, you may terminate these Terms and your Tenant, without penalty, before the
change takes effect, and exercise your data-export rights under Section 9.5. The
prior version of these Terms will govern until the change takes effect or you
terminate. This Section 15.1 does not limit any greater protection conferred on you
by mandatory consumer law (see Section 2.5).

15.2 **Assignment.** You may not assign or transfer these Terms without HumanGR's
prior written consent, except to a successor in connection with a merger,
acquisition, or sale of substantially all assets, provided the successor is not a
competitor of HumanGR and assumes these Terms. HumanGR may assign these Terms to an
affiliate or successor. Any non-permitted assignment is void.

15.3 **Notices.** Notices to HumanGR must be sent to Human Guardrail LTDA, Avenida
Rouxinol, 857, apt. 113, Moema, São Paulo – SP, Brazil (CEP: 04516-001), with a
copy by email to legal@humangr.com. Notices to you may be given via the email
associated with your Tenant or through the Service.

15.4 **Force majeure.** Neither party is liable for any delay or failure to perform
(other than payment obligations) due to causes beyond its reasonable control.

15.5 **Independent contractors.** The parties are independent contractors; these
Terms create no partnership, joint venture, agency, or employment relationship.

15.6 **No waiver.** A party's failure to enforce any provision is not a waiver of its
right to do so later.

15.7 **Severability.** If any provision is held unenforceable, it will be modified to
the minimum extent necessary to make it enforceable, or, if it cannot be so
modified, severed, and the remaining provisions remain in full force and effect.

15.8 **Entire agreement.** These Terms, together with the documents incorporated by
reference in Section 2.2 and any applicable Order, constitute the entire agreement
between the parties regarding the Service and supersede all prior or contemporaneous
understandings on that subject.

---

For questions about these Terms, contact legal@humangr.com.

Copyright © 2026 Human Guardrail LTDA All rights reserved.
