# Security Policy

`clw` (CoreLink Workspaces CLI) is a proprietary, commercial client of the
CoreLink API. It handles a tenant credential (a Personal Access Token) and
uploads/downloads tenant file contents to/from content-addressed storage. We
take its security posture seriously.

## Reporting a vulnerability

**Do not open a public issue, PR, or discussion for a suspected vulnerability.**

Report privately to **[[OWNER: security/abuse contact email on the humangr.com domain, e.g. security@humangr.com once the domain/mailbox is confirmed]]** (PGP available on request).

Please include, where possible:

- the `clw` version (`clw --version`) and the OS/architecture you are on;
- a description of the issue and its impact (e.g. credential exposure, write
  outside the hydrate destination, integrity bypass);
- the minimal steps, manifest, or server response needed to reproduce;
- any logs — **with your PAT and file contents redacted** (`clw` already
  redacts tokens in its own diagnostics; please double-check before sending).

### What to expect

| Stage | Target window |
|---|---|
| Acknowledgement of your report | within **2 business days** |
| Initial triage + severity assessment | within **5 business days** |
| Fix or mitigation plan communicated | within **15 business days** for HIGH/CRITICAL |

We will keep you updated through remediation and will credit you in the release
notes if you wish. Please give us a reasonable window to ship a fix before any
public disclosure (coordinated disclosure).

## Supported versions

`clw` is **pre-1.0**. Only the **latest released version** is supported and
receives security fixes. There are no long-term-support or back-port branches
before `1.0`. Upgrade to the most recent release before reporting, and expect
fixes to ship as a new release rather than a patch to an older one. The semver /
stability charter and an EOL policy land with `1.0` (roadmap M3.6 / M5.5).

## Security posture

The posture below reflects the findings closed in the 2026-06-09 full-crate
audit (`docs/AUDIT-clw-2026-06-09.md`) and its M0.5 follow-up. See
`docs/THREAT-MODEL.md` for the asset/boundary/threat breakdown and the explicit
out-of-scope / residual-risk list.

- **Path traversal closed.** Manifest entry paths and symlink targets are
  validated (`clw-types::validate_rel_path` / `validate_symlink_target`):
  absolute paths and any `..`/`.` component are rejected at
  `Manifest::from_bytes`, and `hydrate` re-validates as defense-in-depth. A
  malicious or tampered manifest cannot write outside the hydrate destination
  (zip-slip / symlink-slip). *(audit H1)*
- **PAT protected at rest.** `~/.clw/config.toml` is written `0600`, `~/.clw`
  and the local cache root `0700` — no longer umask-dependent (was often
  `0644`). *(audit H2)*
- **PAT never sent in cleartext.** The HTTP client requires an `https://`
  endpoint. A loopback exception (dev/tests) uses an **exact-host** check
  (`127.0.0.1` / `localhost` / `[::1]`, optional `:port`/path) — the earlier
  prefix match that let `http://127.0.0.1.evil.com` through is closed, with a
  regression test. *(audit M2; M0.5 CRIT)*
- **No stale replay across toolchain changes.** The `clw run` memoization key
  folds the content digest of the invoked tool (`command[0]`, resolved against
  the run cwd), so a cached result is not replayed after the underlying
  toolchain changes. *(audit M3; M0.5 HIGH)*
- **Integrity verified.** Every chunk is BLAKE3 content-addressed and verified
  on fetch; manifests are canonical-JSON, Merkle-style.
- **Token redaction.** PATs are redacted in `clw`'s own output/diagnostics; the
  `--token` flag's help warns it is visible in the process list (prefer the
  config file or env).
- **No `unsafe`.** Shipped `clw` crates contain zero `unsafe`; the one avoidable
  raw-pointer borrow found in the audit was removed. Prod paths avoid
  `unwrap/expect/panic`.

## Scope

This policy covers the `clw` CLI and its crates in this repository. The CoreLink
**server** API is a separate system with its own coordination; vulnerabilities
in server-side endpoints should still be reported to the address above and will
be routed appropriately.
