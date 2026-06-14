# Terms of Service — `clw` (CoreLink Workspaces CLI)

> **DRAFT — owner/legal review required.** Engineering draft for roadmap M6.1 to
> de-risk legal lead time. **Not** final legal text; **not** reviewed by counsel.
> Governing law, venue, limitation-of-liability figures, warranty disclaimers, and
> the contracting entity are **owner/legal to supply**. Do not publish or rely on
> this until the owner and legal confirm.

## 1. Agreement & relationship to the EULA

These Terms govern access to and use of the **`clw` command-line client** and the
CoreLink Workspaces service it connects to. `clw` is **proprietary, commercial
software**. Your right to install and run the binary is granted by, and subject
to, the **End-User License Agreement (`EULA.md`)** that ships with the artifact;
these Terms and the EULA are intended to be read together. **Where they conflict,
the order of precedence is owner/legal to set.** (The EULA replaces the
placeholder `UNLICENSED` SPDX field per roadmap M4.2; if `EULA.md` is not yet
present in this tree, this cross-reference is forward-looking.)

By configuring a PAT and using `clw` against the CoreLink API, you accept these
Terms and the EULA.

## 2. License grant (summary — EULA controls)

Subject to these Terms and the EULA, you receive a non-exclusive,
non-transferable, revocable license to install and run `clw` for your internal
use with a valid CoreLink tenant. No public redistribution: the product ships via
an authenticated download surface, not a public repo or crates.io
(`docs/DISTRIBUTION.md`, roadmap M3.3). All crates are `publish = false`.

## 3. Accounts, credentials & responsibility

- You must hold a valid CoreLink tenant and a **Personal Access Token (PAT)**. You
  are responsible for keeping the PAT confidential. `clw` stores it `0600`, never
  logs it, and sends it only over `https://` (`SECURITY.md`), but protecting your
  machine and rotating an exposed token is your responsibility.
- You are responsible for the data you snapshot/upload, including ensuring you have
  the right to store it and that you do not upload secrets/PII you do not intend to
  (a `.clwignore`/secret-scan warning is planned, roadmap M6.5, not yet shipped —
  see the Privacy Policy §5).

## 4. Acceptable use

You agree **not** to:

1. **Reverse-engineer, decompile, or disassemble** `clw`, or attempt to extract
   source, except to the extent applicable law expressly permits despite this
   restriction.
2. **Redistribute, resell, sublicense, or publish** the binary or its components,
   or circumvent the authenticated download surface or signing/checksum
   verification.
3. **Attack or abuse the service:** exceed published rate/quota limits other than
   via `clw`'s built-in adaptive backoff (the client honors `Retry-After` + AIMD,
   roadmap M1.3); attempt to access another tenant's data; probe, scan, or DoS the
   CoreLink API; or forge content past integrity checks. `clw` treats all
   server-returned manifests/chunks as untrusted and verifies them
   (`docs/THREAT-MODEL.md`); do not rely on or attempt to defeat those checks.
4. **Tamper with integrity or scoping:** modify the PAT-based tenant scoping, set
   server-internal headers (e.g. `x-corelink-tenant-id`, which `clw` never sets),
   or bypass path-traversal / hydrate hardening.
5. Use `clw` to store or transmit **unlawful content**, malware, or content that
   infringes third-party rights.
6. Remove or alter proprietary notices, the shipped attribution bundle (SBOM /
   `cargo-about`, roadmap M4.3), or license files.
7. Use the software in violation of **export-control law** — see
   `docs/EXPORT-CONTROL.md` and §7 below.

## 5. Service availability, security & vulnerability reporting

The service is provided on an as-available basis; SLAs (if any) are owner/legal to
state. **Do not publicly disclose suspected vulnerabilities** — report privately
to **security@corelink.dev** per `SECURITY.md` (coordinated disclosure). Only the
latest released `clw` version is supported pre-1.0.

## 6. Disclaimers & limitation of liability

**Placeholder — owner/legal to supply.** Standard "AS IS" warranty disclaimer and
a limitation-of-liability cap, consistent with the EULA, go here. `clw` is
**pre-1.0**; no stability/semver guarantee exists before 1.0 (`SECURITY.md`).

## 7. Export compliance

You represent that you are not barred from receiving the software under applicable
export-control or sanctions law, and you will not export, re-export, or use it in
violation of such law. The binary ships cryptography (TLS via rustls, BLAKE3); its
self-classification is in `docs/EXPORT-CONTROL.md` and is **owner/legal to confirm
before international distribution**.

## 8. Termination

The owner may suspend or terminate access for breach (including any §4 violation).
On termination, stop using `clw` and remove it (`clw uninstall`, planned roadmap
M3.5). Sections that by nature survive (IP, disclaimers, liability limits, export,
governing law) survive termination.

## 9. Changes & governing law

The owner may update these Terms; the change-notification mechanism and the
governing law / venue are **owner/legal to supply**.
