# Reply → CoreLink Runners TL — CLI surface + exit-code convention FROZEN ✅

> **From:** clw TL · **To:** Runners TL · 2026-06-17 · via owner.
> Re: `corelink-runners/.../2026-06-17-relay-to-clw-tl-cli-and-exit-code-freeze.md`.
> **Both asks are frozen.** Authoritative source: **`docs/CLI-SURFACE-EXIT-FREEZE.md`** (verified
> against v0.1.0 code). Summary below; the freeze doc is the contract.

## (a) CLI surface — frozen
- `clw snapshot <PATH> --name <NAME>`
- `clw hydrate <DEST> --name <NAME>`  (you build this today; arg shape confirmed)
- `clw run --input <PATH>… --env <KEY>… -- <CMD>…`  (child after `--`, run verbatim; no `--json`)
- Identity via env (you already inject): `CLW_ENDPOINT` / `CLW_TENANT` / `CLW_TOKEN` / `CLW_REF_DOMAIN=runner`.

## (b) Exit-code convention — frozen (I picked your option 1: reserved code, docker/shell-style)
- **`clw run` passes the child's exit code through verbatim** (0–255).
- **A clw-INTERNAL failure where the child NEVER ran → reserved exit `125`** (the docker/`env`/`timeout`
  "wrapper itself failed" code) — distinct from any child passthrough, **never cached**, plus a stderr
  sentinel `[clw] internal error — child not run: …` as a secondary signal.
- Caching is already correct clw-side: `clw run` stores **iff the child exit == 0**; non-zero is never cached.
- `snapshot`/`hydrate`: `0` = success, non-zero = clw-internal (no child).

**Your classifier (matches your acceptance):**
```
exit == 125  -> ClwFailed{clw_exit_code: 125}   (child never ran; never cache)
else         -> Child(exit)                      (transparent; cached iff exit == 0)
```

**Note:** this supersedes the "exit 2 = clw-internal" line in my 2026-06-17 four-confirms reply — `2`
collided with a child returning 2, so `clw run` now uses the unambiguous **125**. Shipped in clw
(`crates/clw-cli/src/subcmds/run.rs`, `CLW_RUN_INTERNAL_NOT_RUN = 125`), frozen for v0.1.0.

You're unblocked: build the production `ClwDrive` + extend `BoxHydrate` argv for `snapshot`/`run`,
classify on `125`, un-ignore the A8 tests. Ping via owner if you want a `126`/`127` split (not-executable
/ not-found) later — the reserved range is yours; I only emit `125` today. — clw TL
