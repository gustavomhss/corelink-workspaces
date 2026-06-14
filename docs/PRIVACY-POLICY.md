# Privacy Policy — `clw` (CoreLink Workspaces CLI)

> **DRAFT — owner/legal review required.** This is an engineering draft authored
> to de-risk legal lead time (roadmap M6.1). It is **not** final legal text and
> has **not** been reviewed by counsel. Do not publish or rely on it until the
> owner and legal confirm. Effective date, controller identity, governing law,
> data-subject-request contact, and jurisdiction-specific clauses (GDPR / UK GDPR
> / CCPA) are **owner/legal to supply**.

## 1. Scope

This policy describes what data the **`clw` command-line client** transmits and
stores, and the guarantees around it. `clw` is a **proprietary, commercial pure
client** of the CoreLink API; it runs no server and opens no listener (see
`docs/THREAT-MODEL.md`). Server-side processing (storage, retention, regional
residency) is governed by the CoreLink service terms and the DPA
(`docs/DPA-subprocessors.md`); the relevant sub-processors are Cloudflare R2
(object storage), Cloudflare Durable Objects, and the CoreLink API itself.

## 2. Every datum `clw` transmits or stores

| Datum | Where | Transmitted to | Why | Protection |
|---|---|---|---|---|
| **Personal Access Token (PAT)** — the tenant credential | At rest in `~/.clw/config.toml`; optionally from env or `--token`; sent as `Authorization: Bearer` | CoreLink API only | The single credential authenticating the tenant to CAS + AC | Config file written `0600`, `~/.clw` + cache root `0700` (not umask-dependent); **never** sent in cleartext (`https://`-only client); **redacted** in all `clw` output/diagnostics; `--token` help warns it is visible in the process list (config/env preferred) — see `SECURITY.md` |
| **File contents** (your source / working-tree bytes) | Chunked, BLAKE3-content-addressed, uploaded to CAS; cached locally at `~/.clw/cache` | CoreLink API → Cloudflare R2 (CAS) | The product payload: snapshot / hydrate | TLS in transit; every chunk BLAKE3-verified on fetch (integrity); local cache under `0700`. **Contents are not encrypted client-side before upload** — confidentiality in transit relies on TLS, at rest on the sub-processor + tenant scoping |
| **File paths / tree structure** | Inside manifests (canonical-JSON, Merkle-style) uploaded with a snapshot | CoreLink API → R2 | Reconstruct the tree on hydrate | TLS in transit. **Paths are metadata and may contain sensitive names** — see §5 |
| **Ref names** (AC pointers) | Set/read via the CoreLink AC API | CoreLink API | Name a snapshot root | TLS in transit |
| **Hostname / endpoint** | The CoreLink API URL you configure | CoreLink API (it is the endpoint) | Routing | `https://`-only; loopback exception is exact-host checked (`SECURITY.md`) |
| **Request metadata** | Per-request: PAT (bearer), tenant-in-URL-path, and a planned version/user-agent header `clw/<ver> (<os>; <arch>)` | CoreLink API | Auth, routing, support/compat | The `x-corelink-tenant-id` header is **server-internal and never set by `clw`** |
| **Telemetry / crash reports** (optional, **not yet shipped**) | Local until sent | Telemetry destination (owner to name) | Diagnostics / product quality | **Off by default**, first-run consent, CI/non-interactive default OFF, honors `DO_NOT_TRACK` + `CLW_TELEMETRY` kill-switch; **test-proven redaction** — no PAT, no paths, no contents. Payload, destination, and retention are owner to finalize (roadmap M5.3). **If telemetry ships, this policy must be updated to enumerate the exact payload before release.** |

## 3. Data flow (summary)

```
your files ──chunk+BLAKE3──> [clw] ──TLS, Bearer PAT──> CoreLink API ──> Cloudflare R2 (CAS) / Durable Objects
     ▲                          │
     └───── hydrate ◄───────────┘   local cache: ~/.clw/cache (0700, plaintext, reconstructible)
   PAT: ~/.clw/config.toml (0600), never logged, https-only
```

`clw` transmits data **only** to the CoreLink API endpoint you configure. It does
not contact any other host (telemetry, when/if enabled, is the sole exception and
is opt-in).

## 4. At-rest & in-transit guarantees (from `SECURITY.md`)

- **PAT at rest:** `~/.clw/config.toml` is `0600`; `~/.clw` and the cache root are
  `0700` (reduces blast radius among non-root local users; not a defense against a
  local root attacker — see `docs/THREAT-MODEL.md` non-goals).
- **PAT in transit:** the HTTP client **requires `https://`**; the loopback dev
  exception uses an exact-host check, not a prefix match.
- **Token never logged:** PATs are redacted in all `clw` output and diagnostics.
- **Integrity:** every chunk is BLAKE3 content-addressed and verified on fetch;
  manifests are canonical-JSON, Merkle-style.
- **Local cache:** holds plaintext file contents; integrity-sensitive and
  reconstructible from the server (see `docs/DR-business-continuity.md`).

## 5. What you control / your responsibilities

- **Paths and contents are uploaded as-is.** Avoid snapshotting secrets/PII you do
  not intend to store. A `.clwignore`/exclude + secret-scanning warning is planned
  (roadmap M6.5) but is **not yet shipped** — until then, exclusion is the user's
  responsibility.
- Protect your machine and your PAT; rotate it if exposed (client `auth
  rotate`/`logout` is planned, roadmap M6.6).

## 6. Retention & deletion

- **Local:** `clw uninstall` (planned, roadmap M3.5) removes the binary, `~/.clw`
  (config + PAT), cache, and telemetry artifacts. The cache is ephemeral and
  reconstructible.
- **Server-side retention** is governed by the CoreLink service terms and the DPA
  (`docs/DPA-subprocessors.md`). **Retention periods and the controller's policy
  are owner/legal to specify here.**
- **Right to erasure (GDPR Art. 17) — known gap.** `clw rm` today deletes only the
  AC ref pointer and **cannot garbage-collect the underlying CAS blobs**, so a
  tenant cannot yet fully delete uploaded data. Closing this requires client
  `prune`/CAS-GC plus server ref-enumeration (dep **D-7**) and CAS DELETE/LIST
  (dep **D-8**) — tracked as **go-live-blocking roadmap M6.4**
  (`docs/ROADMAP-go-live.md`). This policy must not claim full erasure until M6.4
  lands.

## 7. Contact & changes

Security reports: **security@corelink.dev** (see `SECURITY.md`). Data-subject
requests, controller identity, and the change-notification process are
**owner/legal to supply** before publication.
