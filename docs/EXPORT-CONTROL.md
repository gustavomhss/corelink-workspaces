# Export-control self-classification — `clw` (CoreLink Workspaces CLI)

> **DRAFT — owner/legal review required.** Engineering self-classification draft
> for roadmap M6.2. **Not** legal advice and **not** a filed classification. The
> actual ECCN, the applicable license exception, and any CCATS/self-classification
> report or **encryption-registration / annual-self-classification-report** filing
> obligations are **OWNER/LEGAL TO CONFIRM BEFORE any international distribution**
> of the signed binary under the M3 release pipeline
> (`.github/workflows/release.yml`, `docs/DISTRIBUTION.md`). This note exists to
> start that process early because it has lead time and **gates the M3.1
> international release**.

## 1. What cryptography the binary ships

`clw` is a commercial, closed-source binary that links and ships:

- **TLS via `rustls`** (with `webpki-roots`) — the client requires `https://` for
  every CoreLink API call (`SECURITY.md`); this provides confidentiality and
  integrity in transit using standard TLS (AEAD ciphersuites, X25519/ECDHE key
  agreement, etc.).
- **BLAKE3 hashing** — used for content-addressing and chunk integrity
  verification (`docs/THREAT-MODEL.md`). BLAKE3 is a cryptographic **hash**, not
  an encryption primitive.

`clw` performs **no client-side encryption of file contents at rest before
upload**; its cryptography is (a) TLS for transport and (b) a hash for integrity.
This matters for classification: the controlled functionality is the **TLS /
"information security" symmetric+asymmetric crypto**, not a bespoke encryption
feature.

## 2. Likely classification (engineering view — to be confirmed)

Under the U.S. Export Administration Regulations (EAR), Category 5 Part 2
("Information Security"):

- **Most likely ECCN: `5D992.c`** — "mass-market" encryption software that does
  not meet the `5D002` decontrol-excluded criteria, i.e. software that performs or
  uses cryptography but qualifies for mass-market treatment. A commercial CLI
  distributed broadly to end users, using **standard/published** crypto
  (TLS/rustls) and not customer-customizable cryptography, is the textbook
  mass-market profile.
- The **components themselves** (rustls, the underlying ring/aws-lc-rs, BLAKE3) are
  widely available open-source primitives; the relevant question is the
  classification of **`clw` as the shipped commercial product**, not of each
  dependency.

### EAR considerations to evaluate

1. **The Cryptography Note (Note 3 to Cat. 5 Part 2).** Mass-market items "generally
   available to the public" via retail/over-the-counter-equivalent channels, where
   the crypto functionality cannot be easily changed by the user, are eligible for
   mass-market treatment (`5D992` / `5A992` family). `clw`'s download surface is
   **authenticated** (`docs/DISTRIBUTION.md`), not anonymous retail — **legal must
   assess whether the authenticated commercial channel still qualifies as
   "mass-market"** or whether the stricter `5D002` + License Exception ENC
   (§740.17) analysis and reporting applies.
2. **"Publicly available" / open-source decontrol (§742.15(b), §734.3(b)(3)).** The
   *source* of the crypto libraries is publicly available, but `clw` ships as a
   **proprietary closed-source binary** sold commercially — so the open-source/TSU
   self-classification-email path likely does **not** apply to the product even
   though it applies to the upstream libraries. Confirm.
3. **ENC reporting / self-classification report.** If the item is `5A992`/`5D992`
   mass-market or `5D002` under License Exception ENC, an **annual
   self-classification report** (and possibly a one-time email to BIS/NSA) may be
   required for certain crypto items. Owner/legal to determine whether any
   `clw` distribution triggers this.
4. **Sanctions / embargoes (EAR + OFAC).** Independent of ECCN, distribution to
   embargoed destinations and denied/blocked persons is prohibited. The Terms of
   Service (`docs/TERMS-OF-SERVICE.md` §7) carries the end-user export
   representation; the **release/download surface must enforce destination
   screening** if required — owner to wire into M3.3.
5. **Non-US controls.** If distributed from or into the EU/UK or other
   jurisdictions, the EU Dual-Use Regulation (2021/821) and local crypto rules may
   impose parallel obligations. Owner/legal scope per target market.

## 3. Action items (owner/legal, BEFORE international release)

- [ ] **Confirm the ECCN** (expected `5D992.c`; rule out `5D002` + ENC).
- [ ] Determine whether mass-market treatment applies given the **authenticated**
      (non-retail) download surface.
- [ ] Determine filing obligations: CCATS vs self-classification report vs none;
      any one-time BIS/NSA email; annual self-classification report cadence.
- [ ] Decide destination-screening / denied-party enforcement at the download
      surface (M3.3) and the export representation in the ToS/EULA.
- [ ] Record the confirmed classification + license exception in this file and in
      the release artifact metadata (SBOM/attribution bundle, roadmap M4.3).

## 4. Status

**BLOCKING for international distribution under M3.1.** Domestic-only or
internal-testing distribution may proceed at owner discretion, but **no signed
binary should be distributed internationally until the boxes above are checked and
this file records a legal-confirmed classification.**
