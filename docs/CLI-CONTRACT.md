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
| `CLW_ENDPOINT` | API base URL | [`main.rs`](../crates/clw-cli/src/main.rs) |
| `CLW_TENANT` | tenant id (carried in the URL path) | [`main.rs`](../crates/clw-cli/src/main.rs) |
| `CLW_TOKEN` | Static PAT for the developer/default and legacy runner paths (`Authorization: Bearer`) — redacted in all logs/Debug | [`main.rs`](../crates/clw-cli/src/main.rs) |
| `CLW_CACHE_DIR` | local cache dir | [`main.rs`](../crates/clw-cli/src/main.rs) |
| `CLW_REF_DOMAIN` | **FROZEN (2026-06-16):** ref keyspace selector: `user` (default) or `runner` | [`main.rs`](../crates/clw-cli/src/main.rs), [`clw-types`](../crates/clw-types/src/lib.rs) |
| `CLW_CRED_TICKET` | C2c runner-only, single-use credential-broker ticket — **not** a PAT | [`config.rs`](../crates/clw-cli/src/config.rs) |
| `CLW_LEASE_ID` | C2c lease binding; required with a non-empty ticket | [`config.rs`](../crates/clw-cli/src/config.rs) |
| `CLW_FABRIC_ENDPOINT` | C2c credential-broker base URL; required with a non-empty ticket | [`config.rs`](../crates/clw-cli/src/config.rs) |

A `CLW_*` var that is **set but EMPTY counts as ABSENT** (M12) — resolution falls through to the
config file and then the default, so a missing CI secret yields the actionable "missing token/tenant"
(exit 2, no request) instead of an empty `Bearer ` on the wire. Same rule as the `CORELINK_*` aliases.

The default path uses the static PAT. C2c redemption is **command-gated**, not a property of every
runner-domain invocation: its ordinary purpose is a command that will make a CAS/AC call. With a non-empty,
lease-bound `CLW_CRED_TICKET`, the gate redeems once at trusted boot for a scoped per-job CAS PAT; a missing
lease id/fabric endpoint or failed redemption is fail-closed and never falls back to `CLW_TOKEN`. The ticket
itself is not sent as the CAS/AC bearer token. The current
[`command_redeems_credential`](../crates/clw-cli/src/main.rs) classifier allows only the current CAS/AC
commands: `snapshot`, `hydrate`, `status`, `run`, `ls`, `rm`, `prune`, and `erase`. Local and auth commands,
including `init`, `auth login`, and `auth rotate`, do **not** redeem; `completions`, `--help`, and `--version`
exit before credential resolution. `clw` never sets `x-corelink-tenant-id` — the resolved bearer token plus
tenant-in-path is the sole client tenant authority. See [`config.rs`](../crates/clw-cli/src/config.rs).

## Auth + exit semantics
- `Authorization: Bearer <PAT>` + tenant-in-URL-path. Never `x-corelink-tenant-id` (server-internal).
- An expired/invalid PAT → a **non-retryable** auth error (clw does **not** self-re-mint; the
  dispatcher re-mints per D-9). For commands other than `clw run`, config/usage/transport/auth errors
  exit `2`; `clw run` has the distinct wrapper outcomes below.
- **`clw run` is exit-code-transparent** (load-bearing for a CI runner): it **propagates the wrapped
  command's exit code** (`std::process::exit(child_code)`). On Unix, a child killed by a known signal maps
  to `128 + signal` (SIGTERM → 143); an unavailable signal number, and every non-Unix no-code status, map
  to `255`. A **clw-internal failure where the child never ran** (config / transport / hydrate / cache
  error) exits the **reserved code `125`** (the
  docker / `env` / `timeout` "wrapper itself failed" convention). Child commands may also return 125,
  so a runner must not infer execution state from that number alone. (A clap *usage*
  error on `clw run` — malformed args, child never ran — exits `2` per clap; the runner builds the argv,
  so that is not expected in practice.) If `clw` kills a child for captured output over the CAS cap, the
  distinct internal outcome is `137` (`128 + SIGKILL`), not `125`. A non-zero run is **not** memoized — a
  failing job never poisons the cache. **Full frozen surface + exit table: `docs/CLI-SURFACE-EXIT-FREEZE.md`.**

### Out-of-band execution receipt (additive)

Wrappers that must decide whether an optional cold retry is safe may set
`CLW_RUN_STATE_FILE` to a fresh, private file path. `clw run` consumes this variable and strips it
from the child environment. The file contains exactly one ASCII state line: `NOT_STARTED`,
`DISPATCHING`, or `EXECUTED`. `NOT_STARTED` is written before setup; immediately before spawn the
state advances to `DISPATCHING`; a known OS spawn failure restores `NOT_STARTED`; successful spawn
advances it to `EXECUTED`. State updates use a temporary sibling and atomic rename. Failure to
advance after dispatch leaves an ambiguous state and cannot authorize a retry. Missing, malformed,
or unknown receipt content is also ambiguous. A wrapper may retry only when the command failed and
the final receipt is exactly `NOT_STARTED`; exit numbers and output text never prove execution
state. This additive variable leaves ordinary `clw run` invocation and exit-code behavior unchanged.

## `clw run` memoization scope (v0.1)
The run cache key is derived **only** from: the declared `--input` path digests (and Unix mode bits),
the declared `--env` variable **names** (and their current presence/value), the command vector + the
resolved identity of `command[0]`, and the **canonicalized working directory**. Two runs that differ
only in cwd get different keys (no cross-workspace stale replay). On a successful zero-exit miss,
`clw` makes a bounded best-effort cache write: the tenant's AC record contains only the captured
stdout/stderr blob digests plus exit code. A hit fetches and replays those streams — it neither captures
nor reconstructs output files. This behavior is defined by [`clw-run`](../crates/clw-run/src/lib.rs).

**Undeclared non-secret environment variables are the caller's responsibility:** `clw` does not call
`env_clear`, so they remain ambient and any one that affects output **must** be declared via `--env` or
it is invisible to the key and may replay stale output. Before spawning the untrusted child, `clw` removes
its credential-bearing variables (`CLW_TOKEN`/`CORELINK_TOKEN`, credential ticket, lease id, and fabric
endpoint aliases); non-secret `CLW_*` configuration remains available. A complete child-environment
allowlist is a tracked post-beta hardening, deliberately out of scope for the `v0.1.0` line.

## JSON output
The family-relevant verbs support `--json` for machine parsing (content-address root, cache
hit/miss, byte counts). Pin against the documented fields in `docs/usage.md`; fields are additive
within the `v0.1.0` line.

### G2 additive compatibility policy

Top-level keys in an existing JSON object are additive for the current deprecation
cycle: a release may add a key, but must not remove or rename an existing one. The versioned
`json_contract_v1_top_level_keys` golden verifies the real `snapshot`, `ls --name`, and
`ls --all` builders changed in the current compatibility slice (including `ls --all`'s structured
capability failure). It deliberately does not impose an artificial shared schema on unrelated
commands; expanding the inventory is tracked as a separate, low-risk refactor. `run` is excluded
because it intentionally proxies the child's streams and has no JSON document; `completions` is
excluded because its result is a shell script, not JSON.

## Ref namespacing — **FROZEN (2026-06-16)**

The frozen runners seam selects one of two domain separators in
[`ref_key_in`](../crates/clw-types/src/lib.rs): `user` (the default) uses
`BLAKE3("clw/ref/v1/" ‖ name)` and `runner` uses
`BLAKE3("clw/ref/runner/v1/" ‖ name)`. `CLW_REF_DOMAIN=runner` (or
`--ref-domain runner`) therefore selects a runner-only AC keyspace that is disjoint by construction;
a client using the default domain cannot produce a runner-domain key for any name. Only `user` and
`runner` are accepted; another value is a configuration error (exit 2). The runners fabric sets the
runner selector at dispatch time.
