# clw CLI surface + exit-code convention — FROZEN (v0.1.0)

> **Authoritative freeze** of the `clw` verbs/flags and exit-code convention the CoreLink **runner**
> drives. Requested by the Runners TL (2026-06-17 `relay-to-clw-tl-cli-and-exit-code-freeze.md`).
> Every line below is verified against the v0.1.0 code (cited). **Frozen for v0.1.0** — changes are a
> semver/contract event, announced via this file.

## 1. The driven verbs (exact surface)

### `clw snapshot [PATH] --name <NAME>`
- `PATH` — positional, the workspace directory to snapshot; **defaults to `.`** if omitted (`crates/clw-cli/src/subcmds/snapshot.rs`).
- `--name <NAME>` — **required**, the workspace name to store the snapshot under.
- Effect: chunk + content-address + upload; writes the AC ref for `<NAME>` (in the active ref domain).
- **ADDITIVE (post-freeze, PR #143 — new flag + softened error, not a contract change):**
  re-snapshotting a name with **identical** content is a **no-op success** (exit 0; prints
  `already up to date`) — it does **not** error. Re-snapshotting **differing** content still
  rejects (HTTP 409) unless the new `--force` flag is passed, which does an rm-then-write.
  Both are additive (new flag / a former error path now succeeding) and so are permitted under
  this freeze; no frozen exit code or arg meaning changed.

### `clw hydrate <DEST> --name <NAME>`
- `DEST` — positional, destination directory (must not exist or be empty) (`…/subcmds/hydrate.rs`).
- `--name <NAME>` — **required**, the workspace name to materialize.

### `clw run --input <PATH>… --env <KEY>… -- <CMD>…`
- `--input <PATH>` — repeatable; the file/dir paths that form the memoization key (`…/subcmds/run.rs`).
- `--env <KEY>` — repeatable; env var **names** folded into the key.
- `-- <CMD>…` — everything after `--` is the child command + args, run verbatim.
- `clw run` proxies the child's stdout/stderr and has **no `--json` form** (the child owns stdout).

### Identity / domain (all verbs) — set by the runner via env, NOT flags
- `CLW_ENDPOINT`, `CLW_TENANT`, `CLW_TOKEN` (Bearer PAT; tenant-in-URL-path; never `x-corelink-tenant-id`).
- `CLW_REF_DOMAIN=runner` → keyspace `clw/ref/runner/v1/` (disjoint from the user domain).
- (CLI flags `--endpoint/--tenant/--token/--ref-domain` exist but the runner uses env.)

## 2. Exit-code convention (the load-bearing freeze)

### `clw snapshot` / `clw hydrate` (no child process)
| Exit | Meaning |
|---|---|
| `0` | success |
| non-zero (`2`) | **clw-internal failure** (no child involved) — substrate/config/IO error or bad args (`main.rs::exit_err` → 2; clap usage errors also 2) |

### `clw run` (wraps a child) — docker/`env`/`timeout`-style reserved-code convention
| Exit | Meaning | Cache | Runner classification |
|---|---|---|---|
| `N` (the child's code, **0–255**, **passed through verbatim**) | the child ran and returned `N` (`run.rs` → `process::exit(outcome.exit_code)`) | cached **iff `N == 0`** (`clw-run` only stores on exit 0) | `Child(N)` |
| **`125`** (RESERVED) | **clw-INTERNAL failure: the child NEVER ran** — substrate unreachable, bad config, hydrate/cache error (`run.rs` → `process::exit(CLW_RUN_INTERNAL_NOT_RUN=125)`) | **never** | `ClwFailed{clw_exit_code: 125}` |
| `2` (clap usage error) | **malformed `clw run` invocation** — bad/missing flags; clap exits before the child runs (child NEVER ran). The runner builds the argv, so this is not expected in production. | **never** | treat as clw-failed (child never ran) |

- **125** is the industry "the wrapper itself failed" code (docker run, `env`, `timeout`, `nice`). A
  child SHOULD avoid emitting 125–127; clw reserves them for "child did not run."
- A clw-internal `clw run` failure also prints a stderr sentinel `[clw] internal error — child not run: …`
  as a secondary signal (the exit code is the contract; the line is for humans/logs).
- **This supersedes** the earlier "exit 2 = clw-internal" framing in the 2026-06-17 four-confirms reply:
  exit `2` collided with a child that returns 2, so `clw run` now uses the unambiguous reserved **125**.

### Runner rule (matches the relay's acceptance)
```
exit = run `clw run … -- <cmd>`
if exit == 125  -> ClwFailed{clw_exit_code: 125}   # child never ran; never cache
else            -> Child(exit)                      # transparent; clw already caches iff exit==0
```
(`snapshot`/`hydrate`: `0` = ok, non-zero = `ClwFailed`.)

## 3. Stability
Frozen for v0.1.0. The verbs, flag names, arg order, the `125` reserved code, and the
child-passthrough + cache-iff-0 semantics will not change without a new contract version announced here.
