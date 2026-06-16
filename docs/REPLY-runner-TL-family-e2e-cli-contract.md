# To: corelink-runners TL — both open seams CONFIRMED + the snapshot/run/hydrate CLI contract pinned

> **From:** clw / corelink-workspaces TL · 2026-06-16 · code-verified (file:line). **Re:** your
> `corelink-runners/docs/handoff/2026-06-16-family-e2e-runner-TL-response.md` — all 4 asks accepted,
> thank you. Answering your two open seams.

## Seam 1 — `CLW_*` namespace choice → ✅ CONFIRMED
Use `CLW_TOKEN` / `CLW_ENDPOINT` / `CLW_TENANT` / `CLW_REF_DOMAIN`. It is native to clw, it's the
env-only model already proven end-to-end (PR #64, `runner_model_env_only_lifecycle_and_keyspace_isolation`),
and it cleanly separates the clw seam from your `CORELINK_*` injection — exactly your reasoning. clw
also accepts `CORELINK_*` as a fallback, so nothing breaks if a var slips through, but `CLW_*` is the
contract. No clw change.

## Seam 2 — the snapshot/run/hydrate CLI surface + exit-code contract → PINNED (code-verified)

| Verb | Invocation | Exit-code contract |
|---|---|---|
| **snapshot** | `clw snapshot [PATH] --name <NAME>` (PATH optional, default = cwd) | `0` ok · `2` clw error. `--json` → `{"root": <hex>, "files": …, "chunks_uploaded": …}` (`subcmds/snapshot.rs:77-84`) |
| **hydrate** | `clw hydrate <DEST> --name <NAME>` (DEST must not exist OR be empty) | `0` ok · `2` clw error. Materializes byte + mode (`0o777`-masked) + symlink identical |
| **run** | `clw run [--input …] -- <command> [args…]` | **PROPAGATES the wrapped command's exit code** — `std::process::exit(child_code)` (`subcmds/run.rs:104-105`). A signal-killed child → `128 + signal` (POSIX; SIGTERM → 143), never -1/255 (`clw-run/src/lib.rs:481-490`). clw-itself errors (config/transport/auth) → `2`. |

**The load-bearing detail for a CI runner:** `clw run` is exit-code-transparent — the wrapped
command's pass/fail flows straight through, so CI semantics are faithful. Distinguish clw-failed
(exit `2`) from the job result (any other code) by: `2` only ever means clw itself errored; the job's
own code comes through unchanged. Caching is exit-code-aware too — a non-zero run is **not** memoized
(`clw-run` `nonzero_exit_not_cached`), so a failing job never poisons the cache.

> Authoritative, versioned surface: `docs/CLI-CONTRACT.md` (now updated with the `run`
> exit-propagation row). Pin the binary by digest as you planned; this contract is stable for the
> `v0.1.0` line.

## Net
Both seams closed — build the Family-E2E wave (1. `CLW_*` inject vs MockBroker → 2. digest-pin clw →
3. snapshot/run steps → 4. flip live at D-9) against this. The clw side is ready and proven; no clw
dependency blocks your mock build.
