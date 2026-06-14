# Threat model — `clw` (CoreLink Workspaces CLI)

> Concise threat model for the `clw` client. Companion to `SECURITY.md`.
> Findings reference the 2026-06-09 audit (`docs/AUDIT-clw-2026-06-09.md`) and
> its M0.5 follow-up (`CHANGELOG.md`). Tracked / out-of-scope items reference
> `docs/ROADMAP-go-live.md`.

`clw` is a **pure client** of the CoreLink API. It does not run a server, open a
listening socket, or process input from untrusted local users — its untrusted
inputs are (a) the data it receives from the CoreLink API (manifests, chunks)
and (b) the local environment in which it stores its credential and cache.

## Assets

| Asset | Where it lives | Why it matters |
|---|---|---|
| **The PAT** (tenant Personal Access Token) | `~/.clw/config.toml`, optionally env / `--token`; sent as `Authorization: Bearer` | The single credential. Leaking it gives full tenant access to CAS + AC. |
| **Tenant data / file contents** | Uploaded to content-addressed storage (CAS); referenced by AC refs | The product's payload — source/working-tree contents. Confidentiality + integrity required; regional residency is a product promise. |
| **The local cache** | `~/.clw/cache` (chunk store) | Holds plaintext file contents at rest on the user's machine; integrity-sensitive (a tampered cache could yield a corrupt hydrate). |

## Trust boundaries

- **Client ↔ CoreLink API**, over **TLS**. The PAT is the **only** credential
  the client holds and sends. The client trusts the TLS endpoint to be CoreLink
  (`https://` is required; see PAT-leakage mitigations).
- **CoreLink API → client (untrusted payload).** Manifests and chunks returned
  by the API are treated as **untrusted input**: a compromised/MITM'd server, a
  tampered manifest, or a malicious tenant's data must not be able to escape the
  hydrate destination or forge content past integrity checks.
- **Client ↔ local filesystem.** Config + cache live under `~/.clw` with
  restrictive permissions; the client trusts its own machine but minimizes
  blast radius if other local users exist (perms) or if a tool changes
  underneath it (run-key identity).
- **`x-corelink-tenant-id` is server-internal** and is **never** set by `clw`
  — tenant scoping is carried by the PAT + tenant-in-URL-path.

## Threats considered & mitigations

| Threat | Vector | Mitigation | Ref |
|---|---|---|---|
| **Malicious / tampered manifest → path traversal** | A manifest entry path or symlink target that is absolute or contains `..`/`.` causes `hydrate` to write **outside** the destination (zip-slip / symlink-slip) | `validate_rel_path` + `validate_symlink_target` reject such entries at `Manifest::from_bytes`; `hydrate` re-validates (defense-in-depth) | audit H1 |
| **PAT leakage — at rest** | World-readable config on a multi-user machine | `config.toml` `0600`; `~/.clw` + cache root `0700` | audit H2 |
| **PAT leakage — in transit** | PAT sent over cleartext `http://`, incl. a loopback allow-list bypass (`http://127.0.0.1.evil.com`) | `https://`-only client; loopback exception uses an **exact-host** check, not a prefix match (regression-tested) | audit M2 / M0.5 CRIT |
| **PAT leakage — in output / argv** | Token printed in logs/diagnostics; `--token` visible in the process list | Token **redaction** in all `clw` output; `--token` help **warns** of process-list visibility (config/env preferred) | audit LOWs |
| **Integrity forgery** | Server/MITM returns wrong bytes for a content address | Every chunk BLAKE3-verified on fetch; canonical-JSON Merkle-style manifests | (baseline) |
| **Stale / incorrect replay** | `clw run` replays a cached result after the underlying toolchain changed, yielding wrong output | Run key folds the **tool-identity** (content digest of `command[0]`, resolved against the run cwd); key domain bumped `v1→v2` | audit M3 / M0.5 HIGH |
| **Memory-unsafety** | `unsafe` raw-pointer borrow | Removed; **zero `unsafe`** in shipped crates | audit M1 |

## Out of scope / residual risks

These are **known and tracked**, not silently accepted. See
`docs/ROADMAP-go-live.md` for owners and sequencing.

- **Resource exhaustion from a hostile manifest.** Path traversal is closed, but
  a manifest with enormous entry counts / chunk counts / total download size
  could still drive OOM or disk-fill, and symlink **cycles** are not yet bounded.
  Resource-safety / abuse bounds are tracked as **roadmap M1.6** (complements H1
  with anti-OOM / anti-disk-fill / symlink-cycle guards). Hydrate-side hardening
  (re-verify fetched-chunk digests, mask setuid/setgid/sticky/world-writable
  mode bits, materialize files before symlinks) is **M1.7**.
- **Windows path validation.** `validate_rel_path` / `validate_symlink_target`
  currently reason about POSIX paths; Windows backslash / drive-letter / UNC
  forms are **not yet rejected** (traversal could be possible on Windows). The
  Windows-correct path & symlink semantics are tracked under the cross-platform
  port, **roadmap M2** (specifically M2.2). `clw` is unix-only until that lands.
- **Right-to-erasure (GDPR Art. 17) gap.** `clw rm` deletes only the AC ref
  pointer; it **cannot enumerate or garbage-collect the CAS blobs**, so a tenant
  cannot today fully delete their uploaded data. Closing this needs client
  `prune`/CAS-GC **plus** server capabilities: ref-enumeration LIST
  (**dependency D-7**) and CAS DELETE/LIST (**dependency D-8**), tracked under
  the erasure milestone **M6/M6.4**.
- **Operational secret backup.** The server-side `SIGNUP_TOKEN_KEY` backup rule
  is tracked as dependency **D-2** (server-owned, coordinated via the ledger).

## Non-goals

- `clw` is not a server and exposes no network listener; remote-code-execution
  surface is limited to the data it fetches (covered above) and the commands a
  user explicitly runs via `clw run`.
- Defending the user's own machine against a local root attacker is out of
  scope; the `0600/0700` perms reduce blast radius among **non-root** local
  users only.
