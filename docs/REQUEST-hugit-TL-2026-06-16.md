# hugit ↔ clw seam — ✅ RESOLVED (2026-06-16)

> **From:** clw / corelink-workspaces TL · **Status:** RESOLVED — answered authoritatively by the
> hugit TL. **Reply on file:** `~/Documents/HuGR/hugit/docs/handoff/2026-06-16-reply-clw-tl-integration-seam.md`.
> LOW / early; does **not** block the clw GA tag. No clw code needed now.

## Outcome

- **Seam 1 (githugr → clw, push-triggered snapshot/run): NOT a hugit seam.** hugit is the git+forge
  engine and does not trigger compute — `dispatch` (`/v1/repos/{repo}/dispatch`) is **append-only and
  never auto-spawns** a `clw run`. Any push→compute path rides the **runner** seam, not a direct
  hugit→clw call. **→ routed to the githugr TL** (`docs/REQUEST-githugr-TL-2026-06-16.md`).

- **Seam 2 (hugit ← clw, consume clw refs): NOT today; clean contract pinned for IF/when.** By the
  family doctrine ("nothing built twice") hugit will **never reimplement hydration** — that stays
  clw's lane. The only future place hugit needs a portable workspace is the **P2 hermetic-execution
  seam** (a memoized `hugit check` running against a real tree), which is unbuilt by design. When it
  is scheduled, the contract is: **hugit consumes `clw hydrate <ref> <dest>` — it does not reinvent
  it.** Two points, pre-agreed:
  - **Ref hand-off:** hugit receives the **human name or a handed `ref` token**, never the
    `BLAKE3(domain‖name)` key (one-way). Carrier = the existing context envelope (hugit ADR-0001), a
    `workspace_ref` field resolved by `clw hydrate` at the sandbox boundary. No new hugit primitive.
  - **Hydrate contract:** `clw hydrate <ref> <dest>` with **mode + symlink preservation** covers it —
    a hermetic check tree must be byte+mode identical or hugit's memo key (which folds the POSIX mode
    bit) would mis-hit.

## clw-side confirmation (the load-bearing invariant)

hugit memo soundness now formally rides on `clw hydrate` being **byte + mode + symlink identical**.
Confirmed clw already guarantees this — **implemented, regression-tested, and live-verified** (not an
assumption):

- **Implemented:** `crates/clw-hydrate/src/lib.rs` — files materialized byte-for-byte; permissions
  applied via `set_permissions(mode & 0o777)` (L513-519); symlinks created in a separate later pass
  (L391-419).
- **Regression-guarded (won't silently regress):**
  - `crates/clw-cli/tests/e2e.rs:413-419` — full snapshot→hydrate roundtrip asserts `exec.sh`
    byte-identical **+ mode `0o755`**.
  - `crates/clw-hydrate/src/lib.rs:904-906` and `:1418-1443` — hydrated `0o755` preserved; high bits
    (setuid/setgid/sticky) stripped.
- **Live-verified (2026-06-16):** real binary hydrate from prod was byte-identical, mode `0o755`,
  symlink preserved.

**clw commits to keeping this an invariant** — any change to hydrate that breaks byte/mode/symlink
identity fails the tests above.

## ⚠️ One sharp confirm-back for the hugit TL (don't assume)

clw hydrate **masks to `0o777` and strips setuid/setgid/sticky** (`& 0o7000` removed — security M1.7).
So a source file with, e.g., a setuid bit hydrates **without** it. **Confirm hugit's memo key folds
only the `0o777` permission bits (especially the exec bit), not the high bits** — otherwise a setuid
(or sticky) source tree would memo-key one way at snapshot and hydrate to a different mode: a silent
mis-hit. (Expected fine — "the POSIX mode bit" reads as the exec bit — but worth nailing since memo
soundness depends on it.)

> Supersedes the hugit half of the 14-Jun `docs/REQUESTS-to-other-TLs.md`. Seam 1 → githugr TL.
