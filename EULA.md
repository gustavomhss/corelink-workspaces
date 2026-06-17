# End User License Agreement

**CoreLink Workspaces (`clw`)**
HumanGR proprietary software

**Effective date:** June 14, 2026

THIS END USER LICENSE AGREEMENT ("**Agreement**" or "**EULA**") is a legal
agreement between you (either an individual or a single legal entity, "**Licensee**"
or "**you**") and Human Guardrail LTDA, a limited-liability company (sociedade
empresária limitada) organized under the laws of Brazil ("**Licensor**",
"**HumanGR**", "**we**", "**us**"), for the CoreLink Workspaces software,
including the `clw` command-line interface and all associated executable
components, libraries, configuration tooling, and accompanying documentation
distributed therewith, together with any updates, upgrades, patches, and
maintenance releases provided to Licensee (collectively, the "**Software**").

The Software is a client application that connects to and interoperates with the
hosted CoreLink Workspaces service operated by Licensor at
`corelink-api.humangr.com` (the "**Service**"). Licensee's access to and use of
the Service is governed by Licensor's Terms of Service and Data Processing
Addendum, as referenced in Section 5. This Agreement governs the Software itself.

**BY DOWNLOADING, INSTALLING, COPYING, ACCESSING, OR OTHERWISE USING THE
SOFTWARE, LICENSEE AGREES TO BE BOUND BY THE TERMS OF THIS AGREEMENT. IF LICENSEE
DOES NOT AGREE TO THESE TERMS, LICENSEE MUST NOT INSTALL OR USE THE SOFTWARE. IF
LICENSEE IS ENTERING INTO THIS AGREEMENT ON BEHALF OF AN ENTITY, LICENSEE
REPRESENTS THAT IT HAS THE AUTHORITY TO BIND THAT ENTITY, AND "LICENSEE" REFERS
TO THAT ENTITY.**

## 1. Definitions

1.1 "**Documentation**" means the user and technical documentation that Licensor
makes generally available for the Software.

1.2 "**Order**" means an order form, subscription, purchase, or evaluation
entitlement under which Licensor authorizes Licensee to use the Software,
including any free trial or evaluation registration.

1.3 "**Open-Source Components**" means the third-party software components
incorporated in or distributed with the Software that are licensed under their
own open-source license terms, as identified in the `NOTICE` file.

1.4 "**Update**" means any patch, bug fix, update, upgrade, enhancement, or new
version of the Software that Licensor makes available to Licensee under this
Agreement.

## 2. Proprietary Software; License, Not Sale

The Software is proprietary to and a trade secret of Licensor. **It is licensed,
not sold.** This Agreement grants Licensee only the limited rights expressly
stated below. All rights not expressly granted are reserved by Licensor. No
title to or ownership of the Software, or of any intellectual property embodied
in it, is transferred to Licensee. Licensee receives the Software in executable
(binary) form only and obtains no right to receive, and Licensor has no
obligation to provide, the source code of the Software.

## 3. Grant of License

3.1 **Grant.** Subject to Licensee's continuous compliance with this Agreement
and with the terms of the applicable Order, Licensor grants Licensee a
non-exclusive, non-transferable, non-sublicensable, revocable, limited license,
during the term of the applicable Order, to:

&nbsp;&nbsp;&nbsp;&nbsp;(a) install and execute the Software in executable
(binary) form on devices owned or controlled by Licensee, on the supported
platforms (macOS on Intel and Apple Silicon, Windows, and Linux); and

&nbsp;&nbsp;&nbsp;&nbsp;(b) use the Software solely for Licensee's internal
business purposes and solely to access and interoperate with the Service,

in each case only as authorized by, and within the scope, user count, and
duration of, the applicable Order.

3.2 **Evaluation use.** If the Software is provided to Licensee for evaluation,
trial, beta, or other no-charge use, the license is granted solely for the
purpose of internal evaluation, for the evaluation period designated by Licensor,
and the Software is provided without any warranty, indemnity, support, or
service commitment of any kind. Licensor may terminate evaluation use at any
time.

3.3 **Reservation of rights.** Except for the limited license expressly granted
in this Section 3, no other license, right, or interest is granted to Licensee,
whether by implication, estoppel, or otherwise.

## 4. Restrictions

Except, and only to the extent, that an applicable mandatory law expressly
permits the following despite this limitation, Licensee shall NOT, and shall not
permit or enable any third party to:

(a) copy the Software except as strictly necessary to install and run it under
    the license granted above, or otherwise make it available to any third
    party;

(b) sell, rent, lease, lend, distribute, publish, host, sublicense, assign,
    transfer, or otherwise **redistribute or make available** the Software, in
    whole or in part, to any third party;

(c) **reverse engineer**, decompile, disassemble, or otherwise attempt to derive
    or reconstruct the source code, underlying ideas, algorithms, data
    structures, file formats, wire or cache formats, or interface protocols of
    the Software, except to the limited extent that this restriction is
    prohibited by applicable mandatory law and, in such case, only after
    Licensee has requested the relevant interoperability information from
    Licensor and Licensor has failed to provide it within a reasonable time;

(d) modify, adapt, translate, or create derivative works of the Software;

(e) remove, alter, or obscure any copyright, trademark, proprietary, or other
    notice contained in or on the Software (including the `NOTICE` file or any
    license key, watermark, or attribution);

(f) use the Software to build, train, or improve a competing product or service,
    or for the purpose of monitoring the availability, performance, or
    functionality of the Software for competitive purposes;

(g) use the Software to provide a commercial hosting, time-sharing,
    service-bureau, managed-service, or "as-a-service" offering to third
    parties, except as expressly authorized by a separate written agreement with
    Licensor;

(h) circumvent, disable, or interfere with any license-enforcement,
    authentication, security, rate-limiting, or technical protection mechanism of
    the Software or the Service, or access the Software or Service other than
    through the interfaces and credentials provided by Licensor; or

(i) use the Software in violation of any applicable law, regulation, or
    third-party right, or in any manner not expressly permitted by this
    Agreement.

## 5. The Service; Authentication and Data

5.1 **Connection to the Service.** The Software functions by connecting to the
Service to snapshot developer workspaces and to memoize command outputs into a
content-addressed remote cache. The Software authenticates to the Service using
a per-tenant Bearer Personal Access Token ("**PAT**"). Licensee is responsible
for keeping its PATs confidential and for all activity occurring under its PATs.
Tenant isolation is enforced server-side by the Service.

5.2 **Data the Software transmits.** When operated by Licensee, the Software
uploads to the Service only (i) the workspace content that Licensee explicitly
snapshots or that is required to populate or retrieve from the content-addressed
cache, and (ii) the authentication token needed to communicate with the Service.
**The Software does not collect usage telemetry by default, and version 1 of the
Software ships with no telemetry network backend.** Licensee is responsible for
ensuring that it has the rights necessary to upload any workspace content it
snapshots.

5.3 **Service terms govern the Service.** Licensee's access to and use of the
Service, including data residency, data retention and erasure, security, support,
service levels, and the processing of personal data, are governed by the Terms of
Service and Data Processing Addendum referenced at
https://humangr.com/legal/terms and https://humangr.com/legal/dpa, and not by this
Agreement. In the event of a conflict between this Agreement and such Service
terms with respect to the Service, the Service terms control as to the Service.

## 6. Third-Party Components

The Software incorporates Open-Source Components that are licensed to Licensee
under their own respective license terms. The applicable notices and
attributions are provided in the accompanying `NOTICE` file and in the
third-party attribution bundle generated for each release (see `NOTICE`). To the
extent the terms of an Open-Source Component grant Licensee rights or impose
obligations that conflict with this Agreement, the terms of that Open-Source
Component govern solely with respect to that component. Nothing in this Agreement
limits Licensee's rights, or expands Licensee's obligations, under those
third-party licenses with respect to the components they govern.

## 7. Ownership

The Software is owned by Licensor and is protected by copyright, trade-secret,
and other intellectual-property laws and international treaty provisions.
Licensor and its licensors retain all right, title, and interest in and to the
Software and all intellectual-property rights therein, including all
modifications, enhancements, and derivative works thereof, and including the
"CoreLink", "CoreLink Workspaces", "HumanGR", and `clw` names, logos, and other
trademarks (the "**Marks**"). This Agreement does not grant Licensee any right to
use the Marks except as necessary to identify the Software in accordance with
the Documentation. If Licensee provides Licensor with any suggestions,
feedback, or recommendations regarding the Software ("**Feedback**"), Licensor
may freely use and incorporate such Feedback without restriction or obligation
to Licensee.

## 8. Updates

Licensor may, but is not obligated to, make Updates available to Licensee. Any
Update provided to Licensee is part of the Software and is governed by this
Agreement, unless the Update is accompanied by separate license terms, in which
case those terms govern that Update. Licensor may modify, discontinue, or stop
supporting any version, feature, or component of the Software at any time.
Licensee is responsible for installing Updates that Licensor designates as
required for continued operation, security, or compatibility.

## 9. Support and Maintenance

Except as expressly set forth in a separate written agreement or Order, Licensor
has no obligation under this Agreement to provide support, maintenance, training,
or professional services for the Software. Any support actually provided is
governed by the terms of the applicable Order or support plan. Support and abuse
matters may be directed to support@humangr.com and
security@humangr.com, respectively.

## 10. Term and Termination

10.1 **Term.** This Agreement is effective on the date Licensee first downloads,
installs, or uses the Software and continues until terminated as provided in this
Section 10 or until expiration of the applicable Order.

10.2 **Termination for breach.** This Agreement, and the license granted herein,
terminate automatically and immediately, without notice, if Licensee breaches
any of its terms, including any restriction in Section 4. Licensor may also
terminate this Agreement for convenience upon expiration or termination of the
applicable Order.

10.3 **Effect of termination.** Upon termination or expiration, the license
granted under Section 3 ceases immediately, and Licensee must cease all use of
the Software and permanently delete or destroy all copies of the Software in its
possession or control. Termination of this Agreement does not, by itself, delete
data stored by Licensee in the Service; deletion of, and erasure rights with
respect to, data stored in the Service are governed by the Service terms
referenced in Section 5.3.

10.4 **Survival.** Sections 1, 2, 4, 5.2, 5.3, 6, 7, 9, 10.3, 10.4, 11, 12, 13,
14, 15, 16, 17, and 18 survive any termination or expiration of this Agreement.

## 11. Disclaimer of Warranty

THE SOFTWARE IS PROVIDED "AS IS" AND "AS AVAILABLE", WITHOUT WARRANTY OF ANY
KIND. TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, LICENSOR DISCLAIMS ALL
WARRANTIES, WHETHER EXPRESS, IMPLIED, STATUTORY, OR OTHERWISE, INCLUDING WITHOUT
LIMITATION ANY IMPLIED WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE, TITLE, AND NON-INFRINGEMENT, AND ANY WARRANTIES ARISING FROM COURSE OF
DEALING OR USAGE OF TRADE. LICENSOR DOES NOT WARRANT THAT THE SOFTWARE WILL BE
UNINTERRUPTED, ERROR-FREE, OR SECURE, THAT DEFECTS WILL BE CORRECTED, OR THAT THE
SOFTWARE WILL MEET LICENSEE'S REQUIREMENTS. LICENSEE BEARS THE ENTIRE RISK AS TO
THE QUALITY, PERFORMANCE, AND RESULTS OBTAINED FROM THE SOFTWARE. SOME
JURISDICTIONS DO NOT ALLOW THE EXCLUSION OF CERTAIN WARRANTIES, SO SOME OF THE
ABOVE EXCLUSIONS MAY NOT APPLY TO LICENSEE.

## 12. Limitation of Liability

TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, IN NO EVENT SHALL LICENSOR OR
ITS LICENSORS BE LIABLE FOR ANY INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL,
EXEMPLARY, OR PUNITIVE DAMAGES, OR FOR ANY LOSS OF PROFITS, REVENUE, DATA, USE,
OR GOODWILL, OR FOR BUSINESS INTERRUPTION OR COST OF SUBSTITUTE PRODUCTS OR
SERVICES, ARISING OUT OF OR RELATED TO THIS AGREEMENT OR THE SOFTWARE, HOWEVER
CAUSED AND UNDER ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, TORT (INCLUDING
NEGLIGENCE), STRICT LIABILITY, OR OTHERWISE, AND WHETHER OR NOT LICENSOR HAS BEEN
ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, LICENSOR'S TOTAL AGGREGATE
LIABILITY ARISING OUT OF OR RELATED TO THIS AGREEMENT OR THE SOFTWARE SHALL NOT
EXCEED THE AMOUNTS PAID BY LICENSEE TO LICENSOR FOR THE SOFTWARE IN THE TWELVE
(12) MONTHS PRECEDING THE EVENT GIVING RISE TO THE CLAIM. FOR SOFTWARE PROVIDED ON
A NO-CHARGE, EVALUATION, OR BETA BASIS, OR WHERE NO FEES WERE PAID BY LICENSEE FOR
THE SOFTWARE IN THAT TWELVE (12) MONTH PERIOD, LICENSOR'S TOTAL AGGREGATE
LIABILITY SHALL NOT EXCEED ONE HUNDRED U.S. DOLLARS (USD $100).

THE FOREGOING LIMITATIONS AND EXCLUSIONS APPLY TO THE FULLEST EXTENT PERMITTED BY
LAW AND ARE A FUNDAMENTAL BASIS OF THE BARGAIN BETWEEN THE PARTIES. NOTHING IN
THIS AGREEMENT EXCLUDES OR LIMITS EITHER PARTY'S LIABILITY FOR ANY MATTER FOR
WHICH LIABILITY CANNOT BE EXCLUDED OR LIMITED UNDER APPLICABLE MANDATORY LAW.
SOME JURISDICTIONS DO NOT ALLOW THE EXCLUSION OR LIMITATION OF CERTAIN DAMAGES,
SO SOME OF THE ABOVE MAY NOT APPLY TO LICENSEE.

## 13. Indemnification

13.1 **Indemnification by Licensee.** Licensee shall defend, indemnify, and hold
harmless Licensor and its affiliates, and their respective officers, directors,
employees, agents, licensors, and suppliers, from and against any and all
third-party claims, demands, suits, or proceedings, and all resulting losses,
damages, liabilities, settlements, costs, and expenses (including reasonable
attorneys' fees), arising out of or relating to: (a) any workspace content or
other data that Licensee uploads, snapshots, transmits, caches, or otherwise
makes available through the Software or the Service, including any claim that
such content infringes, misappropriates, or violates any intellectual-property,
privacy, publicity, or other right of a third party, or that Licensee lacked the
rights required under Section 5.2 to upload it; (b) Licensee's breach of the
restrictions in Section 4 or of any other term of this Agreement; or (c)
Licensee's use of the Software in violation of any applicable law, regulation, or
third-party right. Allocation of liability for claims relating to the processing
of personal data is governed by the Data Processing Addendum referenced in
Sections 5.3 and 14.

13.2 **No Licensor IP-infringement indemnity.** EXCEPT TO THE EXTENT, IF ANY,
EXPRESSLY SET FORTH IN A SEPARATE WRITTEN AGREEMENT OR ORDER SIGNED BY LICENSOR,
LICENSOR PROVIDES NO INDEMNITY, DEFENSE, OR OTHER PROTECTION TO LICENSEE AGAINST
ANY CLAIM THAT THE SOFTWARE OR ITS USE INFRINGES OR MISAPPROPRIATES ANY
INTELLECTUAL-PROPERTY OR OTHER RIGHT OF A THIRD PARTY. Where an applicable Order
expressly grants Licensee an IP-infringement indemnity, that indemnity, together
with its stated exclusions and its remedy-or-refund cap, is Licensee's sole and
exclusive remedy and Licensor's entire liability for any such claim.

13.3 **Procedure.** The indemnifying party's obligations are conditioned on the
indemnified party (a) promptly notifying the indemnifying party in writing of the
claim, (b) granting the indemnifying party sole control of the defense and
settlement of the claim (provided that no settlement imposing any liability or
obligation on the indemnified party may be made without its prior written
consent, not to be unreasonably withheld), and (c) providing reasonable
cooperation at the indemnifying party's expense.

## 14. Data Protection

14.1 **Roles and applicable terms.** With respect to any personal data processed
in connection with the Software and the Service, Licensee acts as the controller
(or equivalent) and Licensor acts as a processor (or equivalent) processing such
personal data on Licensee's documented instructions, except where Licensor acts
as a controller for limited operational purposes as described in the Data
Processing Addendum. The processing of personal data, the parties' respective
roles and obligations, and the rights of data subjects are governed by Licensor's
Data Processing Addendum referenced in Section 5.3 (the "**DPA**"), which is
incorporated into this Agreement by reference and the acceptance of which is a
condition of Licensee's use of the Software and the Service.

14.2 **Data-subject and statutory rights.** The DPA addresses, and Licensee's and
data subjects' rights with respect to, among other things: (a) regional data
residency, under which customer data is stored in a customer-selected region; (b)
the right to erasure under Article 17 of the EU General Data Protection
Regulation (and the corresponding right under Article 18 of Brazil's Lei Geral
de Proteção de Dados, Lei nº 13.709/2018 ("LGPD")), under which a tenant may
delete all of its stored data; (c) the rights of consumers under the California
Consumer Privacy Act and California Privacy Rights Act, including limitations on
the "sale" or "sharing" of personal information; and (d) the allocation of
controller and processor responsibilities. As Licensor is a Brazilian
controller, the LGPD applies in parallel with the foregoing laws, which continue
to serve Licensee's own end users abroad.
In the event of a conflict between this Agreement and the DPA with respect to the
processing of personal data, the DPA controls. Privacy inquiries and requests to
exercise data-protection rights may be directed to the Data Protection Officer /
privacy contact identified at the end of this Agreement.

## 15. Equitable Relief

Licensee acknowledges and agrees that any actual or threatened breach of Section
4 (Restrictions), Section 7 (Ownership), or Section 16 (Confidentiality) would
cause Licensor immediate and irreparable harm for which monetary damages would be
an inadequate remedy. Accordingly, in addition to any other remedies available at
law or in equity, Licensor is entitled to seek injunctive and other equitable
relief to prevent or restrain any such breach, without the necessity of proving
actual damages and without being required to post any bond or other security. The
limitations and exclusions in Section 12 do not limit or apply to Licensor's
right to obtain injunctive or other equitable relief.

## 16. Confidentiality

The Software, its source and object code, its performance and security
characteristics, and any non-public information disclosed by Licensor in
connection with it are the confidential information and trade secrets of
Licensor ("**Confidential Information**"). Licensee shall protect such
Confidential Information using at least the same degree of care it uses to
protect its own confidential information of like importance, and in no event less
than reasonable care, shall use it only as necessary to exercise its rights under
this Agreement, and shall not disclose it to any third party except as expressly
permitted by Licensor in writing. This obligation does not apply to information
that is or becomes publicly available through no fault of Licensee, or that
Licensee is required to disclose by law, provided Licensee gives Licensor
reasonable prior notice where legally permitted.

## 17. Compliance with Laws; Export and Government Use

17.1 **Export controls.** The Software may be subject to export-control and
sanctions laws and regulations. Licensee shall comply with all such applicable
laws and shall not export, re-export, or transfer the Software to any prohibited
country, entity, or person, or use the Software for any prohibited end use.
Licensee represents that it is not located in, and is not a national of or
controlled by, any country or person subject to applicable embargoes or
sanctions.

17.2 **U.S. Government end users.** If Licensee is an agency or instrumentality
of the U.S. Government, the Software is "commercial computer software" and
"commercial computer software documentation", and is provided with only those
rights granted to all other end users under this Agreement, consistent with FAR
12.212 and DFARS 227.7202, as applicable.

## 18. General

18.1 **Governing law and venue.** This Agreement is governed by and construed in
accordance with the laws of the Federative Republic of Brazil, without regard
to its conflict-of-laws principles. The United Nations Convention on Contracts
for the International Sale of Goods does not apply. The parties consent to the
exclusive jurisdiction and venue of the courts of the Judicial District
(Comarca) of São Paulo, State of São Paulo, Brazil, for any dispute arising out of or relating
to this Agreement, and each party waives any objection to such jurisdiction and
venue. Notwithstanding the foregoing, where Licensee is an individual acting as a
consumer and mandatory consumer-protection law of Licensee's country of habitual
residence grants Licensee a non-waivable right to the protection of provisions of
that law or to bring or defend proceedings in the courts of that country, this
Section 18.1 does not deprive Licensee of, and is subject to, those non-waivable
rights; this carve-out does not apply to Licensee where Licensee is a business,
governmental, or other non-consumer entity.

18.2 **Assignment.** Licensee may not assign or transfer this Agreement or any
rights or obligations under it, in whole or in part, by operation of law or
otherwise, without Licensor's prior written consent. Any attempted assignment in
violation of this Section is void. Licensor may freely assign this Agreement.
This Agreement binds and inures to the benefit of the parties' permitted
successors and assigns.

18.3 **Notices.** Legal notices to Licensor must be sent in writing to Human Guardrail
LTDA, Avenida Rouxinol, 857, apt. 113, Moema, São Paulo – SP, Brazil (CEP:
04516-001), with a copy to legal@humangr.com. Notices are deemed given upon
receipt.

18.4 **Audit.** Upon reasonable prior written notice and no more than once per
twelve (12) months, Licensor may verify Licensee's compliance with this
Agreement and the scope of the applicable Order. Licensee shall reasonably
cooperate with such verification.

18.5 **Waiver and severability.** No failure or delay in exercising any right
under this Agreement operates as a waiver of that right. If any provision of this
Agreement is held unenforceable, that provision shall be modified to the minimum
extent necessary to make it enforceable, or if it cannot be so modified, severed,
and the remaining provisions shall continue in full force and effect.

18.6 **Force majeure.** Licensor is not liable for any delay or failure to
perform resulting from causes beyond its reasonable control.

18.7 **Entire agreement; order of precedence.** This Agreement, together with the
applicable Order and the `NOTICE` file, constitutes the entire agreement between
the parties with respect to the Software and supersedes all prior or
contemporaneous understandings regarding its subject matter. In the event of a
conflict, the applicable Order controls over this Agreement with respect to its
subject matter, and this Agreement controls over the `NOTICE` file except as
provided in Section 6. No modification of this Agreement is binding unless in
writing and signed by, or expressly accepted by, an authorized representative of
Licensor. Any pre-printed or conflicting terms in a Licensee purchase order are
rejected and of no effect.

18.8 **Independent contractors.** The parties are independent contractors. This
Agreement creates no partnership, joint venture, agency, or employment
relationship.

18.9 **No third-party beneficiaries.** Except for Licensor's licensors and
suppliers as beneficiaries of the disclaimers and limitations in Sections 11 and
12, this Agreement confers no rights on any third party.

---

For licensing inquiries, contact sales@humangr.com.
For privacy inquiries, contact the Data Protection Officer / privacy contact
(LGPD Encarregado), Gustavo Schneiter, at privacy@humangr.com.

Copyright © 2026 Human Guardrail LTDA. All rights reserved.
"CoreLink", "CoreLink Workspaces", "HumanGR", and `clw` are trademarks of
Licensor.
