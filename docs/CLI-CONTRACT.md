# clw CLI contract — the stable surface consumers may pin against (v0.1.0)

> **Audience:** the corelink-runners fabric (shell-out, digest-pinned binary) and any engine/CI
> consumer. **Promise:** the surface below is the **stable contract** for the `v0.1.0` line. Additive
> changes (new verbs/flags/JSON fields) are minor; removing or changing the meaning of anything here
> is a **breaking change** (major bump) and will be announced per `docs/SUPPORT-AND-EOL.md`.
> **Authoritative detail:** `clw --help` / `clw <verb> --help` and `docs/usage.md`.

## Invocation model
Consumers **shell out** to the `clw` binary (`clw <verb> …`) and **pin it by digest** (bake it into
the consumer image). No library/ABI coupling. This is the model the runners TL ratified
(`corelink-runners` runner-contract response, 2026-06-16).

## Stable verbs (the family-path subset in **bold**)
| Verb | Purpose |
|---|---|
| **`snapshot [PATH] --name <NAME>`** | snapshot a workspace dir → content-addressed ref |
| **`hydrate <DEST> --name <NAME>`** | materialize a ref into DEST — byte+mode+symlink identical (or `--manifest-digest <HEX>` to hydrate directly by content digest, skipping the named-ref lookup) |
| **`run …`** | content-addressed, memoized execution (cache miss→hit) |
| `ls [--all]` | list refs (`--all` enumerates the tenant) |
| `rm --name <NAME> --yes` | delete a ref (idempotent; or `--ref-key <KEY>` for a dangling ref) |
| `prune` | CAS GC (mark-and-sweep; fail-safe on incomplete live set) |
| `erase --yes` | GDPR Art.17 — wipe the tenant, verify-empty |
| `doctor` / `status` | environment + auth health |
| `auth` / `init` / `completions` / `uninstall` | setup / lifecycle |

## Configuration contract — **env-var driven (no config file required)**
A disposable runner needs **no `~/.clw/config.toml`**; every value falls back to an env var (clap),
and `Config::resolve()` succeeds from env alone:

| Env var | Meaning | Source |
|---|---|---|
| `CLW_ENDPOINT` | API base URL | `crates/clw-cli/src/main.rs:31` |
| `CLW_TENANT` | tenant id (carried in the URL path) | `:40` |
| `CLW_TOKEN` | PAT (`Authorization: Bearer`) — redacted in all logs/Debug | `:49` |
| `CLW_CACHE_DIR` | local cache dir | `:58` |
| `CLW_REF_DOMAIN` | **(PROPOSED, see below)** ref keyspace selector: `user` (default) or `runner` | — |

The per-job, short-TTL, CAS+AC-scoped PAT is **injected at dispatch time** (server **D-9**, FROZEN).
clw never sets `x-corelink-tenant-id` — PAT + tenant-in-path is the sole client tenant authority.

## Auth + exit semantics
- `Authorization: Bearer <PAT>` + tenant-in-URL-path. Never `x-corelink-tenant-id` (server-internal).
- An expired/invalid PAT → a **non-retryable** auth error (clw does **not** self-re-mint; the
  dispatcher re-mints per D-9). Exit code is non-zero; config/usage/transport/auth errors exit `2`.
- **`clw run` is exit-code-transparent** (load-bearing for a CI runner): it **propagates the wrapped
  command's exit code** (`std::process::exit(child_code)`), with a signal-killed child mapped to
  `128 + signal` (POSIX; SIGTERM → 143), never -1/255. A **clw-internal failure where the child never
  ran** (config / transport / hydrate / cache error) exits the **reserved code `125`** (the
  docker / `env` / `timeout` "wrapper itself failed" convention) — distinct from any child passthrough —
  so a runner classifies `125 → clw-failed`, any other code → the child's own result. (A clap *usage*
  error on `clw run` — malformed args, child never ran — exits `2` per clap; the runner builds the argv,
  so that is not expected in practice.) A non-zero run is **not** memoized — a failing job never poisons
  the cache. **Full frozen surface + exit table: `docs/CLI-SURFACE-EXIT-FREEZE.md`.**

## `clw run` memoization scope (v0.1)
The run cache key is derived **only** from: the declared `--input` path digests (+ unix mode bits), the
declared `--env` variable **names** (and their current presence/value), the command vector + the resolved
identity of `command[0]`, and the **canonicalized working directory**. Two runs that differ only in cwd get
different keys (no cross-workspace stale replay). **Undeclared environment variables are the caller's
responsibility:** clw does **not** sandbox the child environment in v0.1 (the child inherits the full
ambient env — no `env_clear`), so any env var that affects the command's output **must** be declared via
`--env` or it is invisible to the key and may replay a stale result. Child-env sandboxing (`env_clear` +
allowlist) is a tracked post-beta hardening, deliberately out of scope for the `v0.1.0` line.

## JSON output
The family-relevant verbs support `--json` for machine parsing (content-address root, cache
hit/miss, byte counts). Pin against the documented fields in `docs/usage.md`; fields are additive
within the `v0.1.0` line.

## ⚠️ Ref namespacing — PROPOSED contract (see `docs/REPLY-to-runners-TL-2026-06-16.md`)
clw's ref key is `BLAKE3(REF_KEY_DOMAIN ‖ name)` with a **single fixed** `REF_KEY_DOMAIN =
"clw/ref/v1/"` (a protocol-version separator, **not** a per-producer field). To give runner refs a
**collision-proof-by-construction** namespace, clw will reserve a second domain
`"clw/ref/runner/v1/"`, selected via `CLW_REF_DOMAIN=runner`. A consumer without that selector can
never produce a runner-domain key regardless of `name`. **Frozen contract; implementation lands as a
gated change.**
