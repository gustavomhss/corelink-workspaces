# To: hugit TL — family e2e FYI + setuid confirm-back (✅ RESOLVED)

> **From:** clw / corelink-workspaces TL · 2026-06-16. **Relay-ready, self-contained.** Light touch.
> **UPDATE: the §2 confirm-back is RESOLVED** — the hugit TL replied (cold-verified):
> `~/Documents/HuGR/hugit/docs/handoff/2026-06-16-reply-clw-tl-memo-key-folds-only-exec-bits.md`.
> hugit's memo key folds **only `mode & 0o111` (exec bits)** (`hugit-cli/src/checks/run.rs:678`), and
> does NOT fold the setuid/setgid/sticky high bits clw strips (nor the rw bits). So clw's M1.7 strip
> and hugit's `0o111`-only fold are **disjoint → no silent mis-hit**: the only bit hugit's soundness
> needs (exec) is the one clw preserves. **No code change either side.** Nothing left to relay here —
> kept for the record. (§1 engine-lane FYI also acknowledged by hugit.)

## 1. FYI — the family-e2e compute path rides your engine lane
In corelink-runners, the only `clw` invocation (`clw hydrate`, `BoxHydrate` at
`crates/corelink-runner/src/boot/mod.rs:290`) sits on the **`HUGIT_RUNNER_HOST` box/exec lane** — i.e.
the engine lane you own drives the runner→clw cache-warm. When the family e2e goes live (after server
D-9 + the runner builds the clw-invocation step), it exercises your lane. No action now; just so it's
on your radar. Full plan: `corelink-workspaces/docs/FAMILY-E2E-integration-plan-2026-06-16.md`.

## 2. Open confirm-back — the memo-key / `clw hydrate` mode contract (from your seam reply)
You ratified: hugit consumes `clw hydrate <ref>` and the hydrated tree must be byte+mode identical or
the memo key (which folds the POSIX mode bit) mis-hits. **clw confirms** hydrate is byte+mode+symlink
identical and regression-tested (`crates/clw-cli/tests/e2e.rs` + `clw-hydrate` mode tests).

**The one thing still to nail (raised in `REQUEST-hugit-TL-2026-06-16.md`, not yet answered):** clw
hydrate **masks to `0o777` and strips setuid/setgid/sticky** (security M1.7). So a source file with a
setuid (or sticky) bit hydrates **without** it. **Confirm hugit's memo key folds only the `0o777`
permission bits (esp. the exec bit), not the high bits** — otherwise a setuid/sticky source tree would
memo-key one way at snapshot and hydrate to a different mode: a silent mis-hit. (Expected fine — "the
POSIX mode bit" reads as the exec bit — but memo soundness depends on it.)

> Cross-ref: `REQUEST-hugit-TL-2026-06-16.md` (the original seam + this confirm).
