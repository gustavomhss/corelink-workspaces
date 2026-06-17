# Export-Control & Sanctions Notice — CoreLink Workspaces (`clw`)

**Product:** CoreLink Workspaces, command-line client `clw` (the "**Software**") and
the hosted CoreLink API at `corelink-api.humangr.com` (the "**Service**").
**Publisher:** HumanGR (the "**Company**", "**we**", "**us**").
**Document type:** Export-control and economic-sanctions notice and end-user
compliance obligations.
**Effective date / last updated:** June 14, 2026

This Notice forms part of, and is incorporated by reference into, the
Terms of Service (`docs/TERMS-OF-SERVICE.md`) and the End-User License Agreement
(`EULA.md`, at the repository root) (the "**EULA**"). Capitalized terms not
defined here have the meaning given in those documents. In the event of a conflict
between this Notice and the Terms of Service or EULA on an export-control matter,
this Notice controls to the extent of that conflict.

> **Plain-language summary (not a substitute for the full text below).** The
> Software contains encryption (TLS) and cryptographic hashing and is therefore
> subject to export-control law. You may not download, install, use, export, or
> re-export the Software or Service in or to embargoed/sanctioned destinations,
> to denied or restricted parties, or for prohibited end-uses. By using the
> Software or Service you make the representations in Section 7. The Company's
> own classification and filing posture is set out in Sections 3–5.

---

## 1. Scope and applicability

This Notice applies to your acquisition, download, installation, access, use,
export, re-export, transfer, and onward distribution of the Software and the
Service. Because the Software is distributed as binaries for macOS (Intel
and Apple Silicon), Windows, and Linux (which the Company intends to code-sign and
notarize), and because the Service is a multi-tenant hosted offering,
export-control and sanctions law can be implicated by (a) the
geographic location from which you download or use the Software, (b) the
geographic location of the CoreLink data-residency region (currently the United States; customer selection is not yet available), (c) your
nationality or that of your personnel, and (d) the parties with whom and
end-uses for which you use the Software or Service.

The Company distributes the Software through an authenticated download surface
(`docs/DISTRIBUTION.md`); the Software is not published to a public package
registry or anonymous mirror. This does not relieve you of the obligations in
Section 6.

---

## 2. Cryptographic functionality of the Software

The Software incorporates and ships the following cryptographic functionality,
which is the basis for its classification under encryption controls:

- **Transport Layer Security (TLS).** The client requires `https://` for every
  call to the CoreLink API (`SECURITY.md`), implemented via the `rustls` TLS
  stack with `webpki-roots`. This provides confidentiality and integrity in
  transit using standard, published TLS primitives (AEAD ciphersuites; X25519 /
  ECDHE key agreement; standard certificate validation).
- **Cryptographic hashing (BLAKE3).** Used for content-addressing and chunk
  integrity verification (`docs/THREAT-MODEL.md`). BLAKE3 is a cryptographic
  **hash** function, not an encryption (confidentiality) primitive.

The Software performs **no proprietary or non-standard cryptography** and **no
client-side encryption of file contents at rest before upload**. Its only
confidentiality control is standard TLS in transit; at rest, customer data is
protected by tenant isolation and sub-processor controls on the Service side
(see the Privacy Policy and DPA). The underlying cryptographic libraries
(`rustls`, the `ring` / `aws-lc-rs` backends, and BLAKE3) are widely available,
published open-source primitives; the relevant classification question is the
classification of `clw` **as the shipped commercial product**, not of each
upstream dependency.

The Software does not provide an open cryptographic interface, end-user-modifiable
cryptography, or a software development kit enabling third parties to add or
substitute cryptographic functionality.

---

## 3. Export classification posture (U.S. EAR)

The Software is "information security" software within Category 5, Part 2 of the
U.S. Export Administration Regulations (EAR) because it uses encryption (TLS).

- **Export Control Classification Number (ECCN):**
  `5D992.c` (mass-market encryption software), self-classified by the Company. This
  classification is self-determined and remains subject to confirmation by counsel.
- **Applicable license exception (if any):**
  None required. The Software is treated as mass-market under ECCN `5D992.c`, so no
  License Exception ENC (§740.17) authorization is required for its export. (Self-classified;
  subject to counsel confirmation.)
- **CCATS / self-classification status:**
  No CCATS classification request was filed; the Company relies on self-classification
  (self-classified June 14, 2026, subject to counsel confirmation).

### Classification rationale (engineering view, subject to the confirmations above)

The Software is a commercial, closed-source binary distributed broadly to end
users, using only standard/published cryptography (TLS via `rustls`) that is not
customer-customizable. That profile is consistent with **mass-market** treatment
under the Cryptography Note (Note 3 to Category 5, Part 2), pointing toward
**`5D992.c`**. The principal open questions for legal confirmation are:

1. **Mass-market eligibility vs. the authenticated channel.** The Software's
   download surface is authenticated and commercial rather than anonymous retail
   (`docs/DISTRIBUTION.md`). Legal must confirm whether the authenticated
   commercial channel still satisfies the "generally available to the public"
   mass-market criteria, or whether the stricter `5D002` + License Exception ENC
   (§740.17) analysis and its reporting obligations apply.
2. **Open-source / publicly-available decontrol does not transfer to the
   product.** The *source* of the upstream crypto libraries is publicly available
   and may itself qualify under §742.15(b) / §734.3(b)(3) (TSU). The Software,
   however, ships as a **proprietary closed-source binary sold commercially**, so
   the open-source self-classification path does **not** apply to the product as
   shipped, even though it applies to the upstream libraries.

---

## 4. Encryption reporting obligations (U.S.)

Depending on the confirmed ECCN and license-exception basis, U.S. encryption
controls may impose reporting obligations on the Company:

- **Annual self-classification report.** Certain `5A992` / `5D992` mass-market and
  `5D002` (License Exception ENC) items require an annual self-classification
  report to BIS/NSA.
- **One-time / supplemental reporting.** Certain ENC items require a one-time
  report or a supplemental email at first export.

Whether any `clw` distribution triggers these obligations, and the cadence and
content of any filing, is as follows: based on the Company's self-classification of
`clw` as mass-market under ECCN `5D992.c`, no annual self-classification report and no
one-time or supplemental BIS/NSA reporting obligation applies to the Software. (This
posture is self-classified and subject to counsel confirmation.)

This is a Company obligation and is stated here for transparency; it does not
create an obligation on you.

---

## 5. Non-U.S. export controls

Distribution from, or into, jurisdictions other than the United States may engage
parallel controls, including the EU Dual-Use Regulation (Regulation (EU)
2021/821), UK export-control law, and local cryptography-import/use rules in the
destination country. The Company's posture under non-U.S. controls is as follows: the
Software's cryptography is standard, published mass-market cryptography, and the Company
treats it as not requiring an additional export license under the EU Dual-Use Regulation
(Regulation (EU) 2021/821) or UK export-control law for the Company's target markets. (This
posture is self-classified and subject to counsel confirmation; it does not address every
country-specific cryptography import/use restriction, which remains your responsibility under
Sections 5 and 7.)

You remain responsible for compliance with the export- and import-control laws of
your own jurisdiction and of any jurisdiction into or from which you use, export,
or re-export the Software or Service (Section 6).

Because the Software contains encryption (TLS), certain destinations impose
cryptography **import or use** restrictions that can require a permit, license, or
registration merely to download, install, or use cryptography-bearing software
locally — independently of any U.S. export control. Determining whether such an
import or use authorization is required, and obtaining and maintaining it, is your
responsibility (Section 7.5).

---

## 6. Embargoed destinations, restricted parties, and prohibited end-uses

The following prohibitions apply regardless of the Software's ECCN and regardless
of any license exception. They derive from U.S. sanctions and export-control law
(administered by the U.S. Office of Foreign Assets Control (OFAC) and the Bureau
of Industry and Security (BIS)) and, where applicable, from EU, UK, and United
Nations measures.

### 6.1 Embargoed and sanctioned destinations

You may not download, install, access, use, export, re-export, or transfer the
Software or Service, and you may not request, select, or use any data-residency
region offered by the Service in order to cause customer data to be stored or
processed, in or for the benefit of any country or territory subject to
comprehensive U.S. or other applicable embargoes or comprehensive sanctions. As of
the effective date these comprehensively sanctioned destinations include
**Cuba, Iran, North Korea, Syria, and the Crimea, Donetsk, Luhansk, Zaporizhzhia, and
Kherson regions of Ukraine**, and any other destination later designated as
comprehensively embargoed. For the avoidance of doubt, this prohibition applies to
the data-residency regions the Service actually makes available and honors; it does
not represent that the Service offers a residency region in, or screens residency
selections by reference to, any particular country. This list was confirmed against the
OFAC/BIS designations in force as of June 14, 2026; in addition to the U.S. measures, the
Company applies the comprehensive sanctions and restricted-party measures of the EU, the UK,
and the United Nations to the extent they apply. The list is subject to change and should be
re-confirmed against the then-current OFAC/BIS, EU, UK, and UN designations at each
publication.

### 6.2 Denied, blocked, and restricted parties

You may not provide the Software or Service to, or use it on behalf of or for the
benefit of, any person or entity that is:

- listed on OFAC's Specially Designated Nationals and Blocked Persons (SDN) List
  or any other OFAC sanctions list, or owned 50% or more (individually or in the
  aggregate) by one or more such persons;
- listed on the BIS Entity List, Denied Persons List, or Unverified List, or
  otherwise denied U.S. export privileges; or
- listed on any analogous EU, UK, UN, or other applicable consolidated sanctions
  or restricted-party list that the Company is required to screen against.

### 6.3 Prohibited end-uses

You may not use, and you may not permit any third party to use, the Software or
Service in connection with any prohibited end-use under the EAR or applicable law,
including the design, development, production, stockpiling, or use of nuclear,
chemical, or biological weapons or missiles, or the unauthorized development or
production of certain military, military-intelligence, or advanced-computing or
semiconductor end-uses subject to control. The same prohibition applies to the
equivalent dual-use and weapons-of-mass-destruction end-use controls under the EU
Dual-Use Regulation (Regulation (EU) 2021/821), UK export-control law, and any
other applicable measures, to the extent they govern your use, export, or
re-export of the Software or Service. The Company has not identified any country-specific
end-use control in its target markets that requires an explicit overlay beyond the U.S. EAR
and the EU/UK measures named here; this determination is subject to counsel confirmation and
to re-assessment as the Company's target markets change.

### 6.4 Screening and enforcement

The Company may screen tenants, account registrations, and the download surface
against applicable sanctions and restricted-party lists and may refuse, suspend,
or terminate access where screening or other information indicates a violation or
material risk of a violation of this Notice or applicable law. If and when the Service
offers a customer-selected data-residency region (roadmap M7.1), the Company may treat that
selection as a screening signal. The current state of automated destination,
denied-party, and residency-region screening at the download and registration
surface is as follows: during the Company's incubation phase, automated
destination, denied-party, and residency-region screening at the download and
tenant-registration surface is not yet fully in place; the Company relies on the
contractual representations in Section 7, manual review where information indicates
risk, and the screening rights reserved above, and is building out automated
screening as the Service matures. The Company does not represent that automated
screening currently blocks every prohibited destination, party, or residency
selection.

---

## 7. Your representations and obligations

By downloading, installing, accessing, or using the Software or Service, you
represent, warrant, and covenant, on your own behalf and on behalf of any
organization you represent, that:

1. you are not located in, organized under the laws of, ordinarily resident in,
   or a national or government of, any destination identified in Section 6.1;
2. you are not, and are not owned or controlled by or acting on behalf of, any
   party identified in Section 6.2;
3. you will not export, re-export, transfer, or otherwise make the Software or
   Service available, directly or indirectly, to any destination, party, or
   end-use prohibited by Section 6 or by applicable export-control or sanctions
   law;
4. you will not request or use any data-residency region made available by the
   Service, or otherwise route or store customer data, in a manner that violates
   Section 6;
5. you have obtained, and will maintain, any license, authorization, or
   registration required under the export- and import-control and cryptography
   laws applicable to you and to your use of the Software or Service; and
6. you will not use the Software or Service for any prohibited end-use described
   in Section 6.3.

These representations are made each time you access or use the Software or
Service. The export-compliance representation in the Terms of Service
(`docs/TERMS-OF-SERVICE.md` §12, "Export compliance and sanctions") is
incorporated here, and this Notice is the detailed statement of those obligations.

---

## 8. Suspension, termination, and survival

A breach of this Notice is a material breach of the Terms of Service and the
EULA. Without limiting any other remedy, the Company may immediately suspend or
terminate your access to the Software and Service, and decline to provide
support, where it reasonably determines that continued provision would violate, or
create a material risk of violating, applicable export-control or sanctions law.
Your obligations and representations under Sections 6 and 7 survive termination,
together with any provision that by its nature should survive.

---

## 9. No facilitation; cooperation

You will not take any action that would cause the Company, or any of its
affiliates, personnel, or sub-processors, to violate applicable export-control or
sanctions law, and you will not use the Software or Service to facilitate any
transaction that you could not lawfully conduct directly. You will reasonably
cooperate with the Company in connection with any export-control or sanctions
inquiry, audit, or screening relating to your use of the Software or Service.

---

## 10. Changes to this Notice

The Company may update this Notice to reflect changes in applicable law, in the
Software's cryptographic functionality, or in the Company's classification or
filing posture. Material changes will be notified through the mechanism stated in
the Terms of Service. Your continued use of the Software or Service after a change
takes effect constitutes acceptance of the updated Notice.

---

## 11. No legal advice; relationship to government determinations

This Notice describes the Company's export-control posture and your obligations;
it is **not legal advice** to you and is **not** a substitute for your own
determination of the export-, import-, and sanctions-law requirements applicable
to you. Export-control classifications and sanctions designations change over
time; in the event of any conflict between this Notice and applicable law or a
binding government determination, applicable law and that determination control.

---

## 12. Contact

Export-control and sanctions questions regarding the Software or Service may be
directed to:

- **Export-control / compliance contact:** legal@humangr.com
- **Legal notices:** Human Guardrail LTDA, Avenida Rouxinol, 857, apt. 113, Moema, São Paulo – SP, Brazil (CEP: 04516-001); legal@humangr.com
- **Contracting entity:** Human Guardrail LTDA, a limited-liability company (sociedade empresária limitada) organized under the laws of Brazil (CNPJ: 66.103.400/0001-30)
- **Governing law and venue (for disputes under this Notice):** This Notice is governed by the laws of the Federative Republic of Brazil, and any dispute arising under it is subject to the exclusive jurisdiction of the courts of the Judicial District (Comarca) of São Paulo, State of São Paulo, Brazil, consistent with the Terms of Service §14.
