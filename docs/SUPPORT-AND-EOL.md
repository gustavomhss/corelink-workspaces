# clw — support & end-of-life policy

> Satisfies exit-gate #10 ("support/EOL policy published"). Defines which versions are supported, the
> security-fix window, the EOL timeline, and how to get help. This is the **v0.1.0** policy; it
> tightens toward a stricter SLA at `1.0` GA.

---

## Versioning

clw follows **Semantic Versioning** (`MAJOR.MINOR.PATCH`).

- **Pre-1.0 (`0.x`):** per SemVer, the public surface may change between **minor** versions. A `0.x.y`
  bump is a breaking-allowed minor; `0.x.(y+1)` is bug-fix only. We will document every breaking
  change in `CHANGELOG.md` and provide a migration note.
- **Post-1.0:** breaking changes only on a **major** bump; minors are additive; patches are fixes.
- The **cache/manifest format** carries its own version (`MANIFEST_VERSION`); a format-incompatible
  change is treated as breaking and ships with an invalidate-or-migrate path (see `clw` cache
  versioning in `docs/usage.md`).

## Supported versions

| Version line | Status | Receives |
|---|---|---|
| latest `0.x` minor | **Supported** | bug fixes + **security fixes** |
| previous `0.x` minor | **Security-only** for 90 days after the next minor ships | security fixes only |
| anything older | **End-of-life** | nothing — upgrade required |

At `1.0`, this becomes: latest major fully supported; previous major security-only for 6 months.

## Security-fix window & disclosure

- **Report** a vulnerability per **`SECURITY.md`** (private channel — do **not** open a public issue).
  We acknowledge within **2 business days**.
- We aim to ship a fix for a confirmed **critical/high** issue within **7 days** on the latest
  supported line, and to backport to the security-only line within the same window.
- Coordinated disclosure: we credit reporters and publish an advisory (affected versions + checksums +
  remediation) once a fix is available. Key-compromise advisories follow
  `docs/go-live/RUNBOOK-key-rotation-revocation.md`.

## End-of-life process

1. When a version line goes EOL, we announce it in `CHANGELOG.md` and the release notes **at least one
   minor in advance**.
2. EOL means: no further releases, security or otherwise. The last build remains downloadable from the
   release surface but is **unsupported**.
3. EOL of the **product** (should it ever happen) triggers the data-portability + erasure commitments
   in the DPA / Privacy Policy: users retain `clw erase` (GDPR Art. 17 self-service) and an export
   path for their content for the wind-down window stated there.

## Getting help

| Need | Where |
|---|---|
| A bug report / feature request | the project issue tracker (private, per the proprietary-distribution model) |
| A security vulnerability | **`SECURITY.md`** (private) — never the public tracker |
| "Is my setup OK?" | run **`clw doctor`** first — it diagnoses config/auth/cache/permission problems and prints a redacted, paste-safe report |
| Account / billing / tenant | the CoreLink tenant console (server-side, not clw) |

## Platform support matrix (v0.1.0)

| Platform | Status |
|---|---|
| macOS (x86_64, aarch64) | supported |
| Linux (x86_64) | supported |
| Linux (aarch64) | build provided; tested as the M2.4 lane lands |
| Windows (x86_64) | supported; PAT-file protected by an owner-only ACL (see `docs/go-live/WINDOWS-ACL-VERIFICATION.md`). Symlinks use a copy-fallback unless Developer Mode/admin is available. |

> **Telemetry:** clw has **no** phone-home mechanism — there is nothing to opt out of. Diagnostics are
> local (`clw doctor`, `-v`/`RUST_LOG` → stderr) and never transmit your data or PAT.
