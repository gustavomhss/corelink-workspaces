# Data Processing Addendum (outline) & sub-processor list — CoreLink / `clw`

> **DRAFT — owner/legal review required.** Engineering draft for roadmap M6.3 to
> de-risk legal lead time. **Not** a binding DPA and **not** reviewed by counsel.
> The controlling-entity names, SCC module selection, governing law, and the
> liability/indemnity terms are **owner/legal to supply**. Do not execute or rely
> on this until the owner and legal confirm.

## 1. Purpose & roles

This addendum supplements the Terms of Service (`docs/TERMS-OF-SERVICE.md`) /
EULA and governs the processing of personal data that the CoreLink Workspaces
service processes on behalf of the customer.

- **Controller:** the customer (tenant) — determines what file contents/paths are
  uploaded via `clw`.
- **Processor:** CoreLink (the service operator / contracting entity — owner to
  name).
- **Sub-processors:** listed in §4.

`clw` itself is the customer-operated **client**; it runs on the customer's
machine, holds the PAT, and uploads content to the CoreLink API (see
`docs/PRIVACY-POLICY.md` for the full data inventory). The processing performed by
CoreLink (and its sub-processors) is server-side.

## 2. Subject matter, nature & categories of data

- **Subject matter / nature:** content-addressed storage and retrieval of customer
  source/working-tree data (snapshot / hydrate) plus reference (AC) metadata.
- **Categories of data:** file **contents**, file **paths / tree structure**, ref
  names, and the tenant PAT (credential). Any personal data is **incidental** —
  whatever the customer chooses to store. CoreLink does not require personal data
  to provide the service.
- **Data subjects:** determined by the customer's content.
- **Duration:** for the term of the agreement; see retention (§6) and the
  right-to-erasure status (§7).

## 3. Processor obligations (outline)

1. Process personal data only on documented customer instructions.
2. Ensure confidentiality commitments from personnel.
3. Implement appropriate technical & organizational measures — the **client-side**
   measures are in `SECURITY.md` / `docs/THREAT-MODEL.md` (PAT `0600`, https-only,
   token redaction, BLAKE3 integrity); **server-side and sub-processor** measures
   are owner to document.
4. Engage sub-processors only under §4/§5 terms and flow down equivalent
   obligations.
5. Assist the controller with data-subject requests, security, breach
   notification, and DPIAs.
6. Delete or return personal data at end of term — subject to the erasure gap (§7).
7. Make available information to demonstrate compliance and allow audits.

## 4. Sub-processor list

| Sub-processor | Service provided | Data processed | Location / residency |
|---|---|---|---|
| **Cloudflare R2** | Object storage backing content-addressed storage (CAS) — stores uploaded file **contents** (chunks) and manifests | File contents, paths (in manifests) | Regional residency is a product promise; region routing/correctness is roadmap M7.1. Confirm the configured R2 jurisdiction(s). |
| **Cloudflare Durable Objects** | Coordination / per-tenant state and write-path control for the CoreLink API | Tenant-scoped coordination metadata; mediates CAS writes | Cloudflare edge; confirm region binding |
| **The CoreLink API** | First-party service operator — auth (PAT), tenant scoping, AC ref store, CAS gateway | PAT (in transit), refs, content (in transit), request metadata | Operated by CoreLink; confirm hosting region |

Notes:

- Cloudflare (R2 + Durable Objects) is the underlying infrastructure
  sub-processor; the CoreLink API is the first-party processing layer in front of
  it. The CoreLink **server** is a separate system from `clw` (`SECURITY.md`
  scope).
- `clw` transmits data **only** to the CoreLink API endpoint; it does not contact
  Cloudflare directly or any other third party (telemetry, when/if shipped, is
  opt-in and off by default — `docs/PRIVACY-POLICY.md`).
- Cloudflare's own DPA / sub-processor terms and SCCs should be attached/referenced
  by the owner.

## 5. New sub-processors & objection

The owner/legal to define the notice mechanism for adding/replacing
sub-processors and the customer's right to object.

## 6. Retention & deletion

Retention periods are **owner/legal to specify**. Local client data (cache) is
ephemeral and reconstructible; `clw uninstall` (planned, roadmap M3.5) clears the
local PAT + cache. Server-side deletion is subject to §7.

## 7. Right to erasure (GDPR Art. 17) — known limitation

**Disclosed gap.** Today `clw rm` deletes only the AC ref pointer and **cannot
garbage-collect the underlying CAS blobs**, so a tenant cannot yet fully delete
uploaded content. Full erasure requires client `prune`/CAS-GC plus server
**ref-enumeration LIST (dep D-7)** and **CAS DELETE/LIST (dep D-8)** — tracked as
**go-live-blocking roadmap M6.4** (`docs/ROADMAP-go-live.md`). Until M6.4 lands,
this DPA must not represent that full erasure is available; the residency promise
is not yet erasure-compliant.

## 8. International transfers, breach, liability

SCC module selection, transfer-impact assessment, breach-notification timelines,
and liability/indemnity are **owner/legal to supply**. Security-incident reporting
to the customer ties to the disclosure process in `SECURITY.md`.
