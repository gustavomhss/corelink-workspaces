# Reply → CoreLink Runners TL — clw v0.1.1 binary digest + fetch URL (the pin you needed)

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Runners** TL
> **Date:** 2026-06-19 · **Forward via:** owner
> **Re:** the open item from `REPLY-runner-TL-four-confirms-2026-06-17.md §1` — "binary digest to pin (X4)",
> which was ⏳ NOT-YET-AVAILABLE (no published release). **It is now available.** This closes it.

## TL;DR
`clw` shipped a real, signed, publicly-downloadable release. **You can now pin the binary by digest.**
For the standard Linux x86_64 runner image, pin:
- **digest (sha256):** `be8733781a1d19a96d88528df3fe67c9d50723e7bed95cb35d1e9ab712685610`
- **fetch URL:** `https://github.com/HumanGuardrail/clw-releases/releases/download/v0.1.1/clw-0.1.1-x86_64-unknown-linux-gnu`
- **version:** `v0.1.1` (latest). `clw --version` → `clw 0.1.1`.

## All five target digests (pin the one matching your runner arch)
From the **minisign-signed** `SHA256SUMS` of the v0.1.1 GitHub Release on the public mirror
`HumanGuardrail/clw-releases` (sha256, lowercase hex):

| target | sha256 | asset |
|---|---|---|
| **x86_64-unknown-linux-gnu** | `be8733781a1d19a96d88528df3fe67c9d50723e7bed95cb35d1e9ab712685610` | `clw-0.1.1-x86_64-unknown-linux-gnu` |
| aarch64-unknown-linux-gnu | `fdb46cb45dea9fd272fba7239d4d08c4886246b7ab77410fb7ff68066194704e` | `clw-0.1.1-aarch64-unknown-linux-gnu` |
| x86_64-apple-darwin | `6d9ff8336023ab6bffa3f82dc5139eaa29effa0c1974857fc0cca16e44ff7aad` | `clw-0.1.1-x86_64-apple-darwin` |
| aarch64-apple-darwin | `ee4a4f283f57ac283ffce820d47c90fea0e3a224e6bb03c9b99373ad78e66f60` | `clw-0.1.1-aarch64-apple-darwin` |
| x86_64-pc-windows-gnu | `b2af90b3d4cccd61e578af4f418708a26a0575584962dc682386b573300faeab` | `clw-0.1.1-x86_64-pc-windows-gnu.exe` |

Asset URL pattern: `https://github.com/HumanGuardrail/clw-releases/releases/download/v0.1.1/<asset>`.
Binaries are **bare executables** (not archived) — download, `chmod +x`, pin by the digest above.

## Verify before baking into the image (signature-of-record)
The binaries are signed with **minisign** (the cross-platform signature-of-record). Verify the checksum
manifest, then the binary against it — no trust in GitHub/the CDN required:
```sh
BASE=https://github.com/HumanGuardrail/clw-releases/releases/download/v0.1.1
curl -fsSLO "$BASE/SHA256SUMS"
curl -fsSLO "$BASE/SHA256SUMS.minisig"
curl -fsSLO "$BASE/clw-0.1.1-x86_64-unknown-linux-gnu"
# minisign trust root (committed at docs/minisign.pub, key id 4B57B8B54A0E396D):
minisign -Vm SHA256SUMS -P RWRtOQ5KtbhXSxt24bM/T+ndWfjUDUm8c7gr8194pxnyWEUS/CTDIkm0
sha256sum -c --ignore-missing SHA256SUMS    # confirms the binary matches the signed manifest
```

## The CLI/exit contract you drive (unchanged, frozen)
No change since the freeze — restating so the pin + the invocation seam land together:
- **Verbs:** `clw snapshot [PATH] --name <NAME>`, `clw hydrate <DEST> --name <NAME>`,
  `clw run --input <PATH>… --env <KEY>… -- <CMD>…`. Drive these; never reimplement.
- **Exit (load-bearing):** `clw run` exits the child's code **verbatim**; a clw-INTERNAL failure where the
  **child never ran** exits the **reserved 125** (classify `125 → ClwFailed`, else `Child(N)`); a non-zero
  run is **never cached**. Authoritative: `docs/CLI-SURFACE-EXIT-FREEZE.md` + `docs/CLI-CONTRACT.md`.
- **Identity via env:** inject `CLW_ENDPOINT` / `CLW_TENANT` / `CLW_TOKEN` (Bearer PAT, tenant-in-path,
  never `x-corelink-tenant-id`) + `CLW_REF_DOMAIN=runner` (→ keyspace `clw/ref/runner/v1/`, disjoint from user).

## Net / what this unblocks
This was the **only** thing `clw` owed you (digest + URL) — now delivered. You can bake the digest-pinned
clw into the Phase-2 runner image and build the clw-invocation drive. Once that's up, we can run the
**family-e2e** (suites #66 per-tier + #67 solo) end-to-end — owner routes the scheduling between us +
the server TL. Ping me (via owner) when the image is built or if any digest/URL/contract detail needs more.

— CoreLink Workspaces / `clw` TL
