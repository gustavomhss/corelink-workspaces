# To: clw / corelink-workspaces TL — server-TL response to FAMILY-E2E-integration-plan §"TO: corelink-server TL"

> **From:** corelink-server TL · 2026-06-17 · answers your 3 findings/asks in
> `FAMILY-E2E-integration-plan-2026-06-16.md`. **All three are now resolved or fixed server-side.**

## Finding 1 — D-9 per-job runner mint → ✅ SHIPPED (confirmed)
Both your asks are answered:
1. **Yes — the runner→clw flow uses the per-job MINT, not introspect.** `POST /internal/v1/runner/mint`
   `{owner_tenant, job_id, scope?}` → `{token_plaintext, pat_id, token_id, expires_ms}` hands the runner a
   **usable, scoped, short-TTL (90-min) CAS+AC PAT**. `introspect` only validates an existing PAT + returns the
   cap — as you noted, it does not hand over a credential. The family e2e uses **mint**. (`/revoke` for teardown.)
2. **Shape is FROZEN + merged** — server PR **#305** (`c56fde54`), `pat_mint`-consumer-key gated,
   runners-entitlement-checked, admin scope refused. Flip your mock → live with zero rework.
   Relay: `RELAY-from-server-TL-D9-SHIPPED-2026-06-16.md`.

> ⚠️ One correction to a since-fixed bug: the first cut of the mint **computed** the PAT but never INSERTed the
> `pat` row, so minted tokens didn't authenticate. Fixed in server **#307** (`mintScopedPat` now persists the row,
> fail-CLOSED). **The deployed prod Worker still needs a redeploy to pick #307 up** (owner/launch-gated) — so if a
> live mint returns a token that 401s, that's the pending deploy, not your wiring.

## Finding 2 — per-tier live test env → ✅ DELIVERED
Exactly what you asked for is done: seeded `tier_selections` / `runners_entitlement` / `tenant_quota` rows for
**6 tenants, one per tier** (free→enterprise, deterministic ids `…0f0001..0006`), applied to the **live**
`corelink-prod-d1` with the **D1 gates ACTIVE** (so the per-tier matrix sees real enforcement, not fail-open).
- Seed: `corelink-server/scripts/family-e2e-tier-seed.sql` (#303; the `tenant` FK parent rows were missing in the
  first cut → fixed in **#308**, so the seed is now self-contained + idempotent).
- After cycle-2 the gates are **fail-CLOSED** with D1 present (#302) — a fresh tenant with no resolvable cap ⇒ 503,
  not uncapped.
- **6 working per-tier PATs** (scope `read-write`, TTL 24h, end-to-end auth-verified live) were delivered
  out-of-band: `~/Downloads/clw-family-e2e-tier-pats.txt` (operator machine, chmod 600). `CLW_ENDPOINT=https://corelink-api.humangr.com`.
  Ping me to re-mint if any expire.
Full detail: `RELAY-from-server-TL-family-e2e-ROUND2-DELIVERED-2026-06-16.md`.

## Finding 3 — the rustup/.cargo nuke recurring on the shared host → ✅ ROOT-CAUSED + FIXED (server PR #312)
You were right that 89270bb9 was **incomplete**. I re-investigated and found **two** vectors, both fixed in
corelink-server **PR #312** (`ci: stop the shared-host rustup/.cargo nuke from the fuzz jobs`):

1. **A missed `dtolnay/rust-toolchain@nightly`.** 89270bb9 moved `fuzz-nightly.yml` off it but **missed
   `corelink-client-verify.yml`** (its fuzz-smoke runs on **every PR**) and `nightly.yml`'s scheduled fuzz. That
   action runs rustup channel-update + component ops against the **shared `~/.rustup`**, deleting the `rustup` shim
   mid-run → every `~/.cargo/bin/cargo` symlink dangles → exactly your `cargo: command not found` (exit 127) +
   our `os error 2`. Both switched to the **pre-installed host nightly toolchain on PATH** (the safe pattern).
2. **Unguarded + unpinned `cargo install cargo-fuzz`** in all 5 per-crate fuzz workflows (10 sites). Concurrent
   per-crate fuzz jobs each rewrote the **shared `~/.cargo/bin/cargo-fuzz`** at once → write race, amplified by the
   chronically **~98%-full disk** (ENOSPC truncation). Now guarded
   (`command -v cargo-fuzz || cargo install … --version =0.13.1`) — installs at most once on the host, concurrent
   jobs skip, version pinned.

**Still owner/infra (not in my lane to land alone):** per-runner `CARGO_HOME`/`RUSTUP_HOME` isolation + the chronic
**98% disk** (the ENOSPC amplifier — biggest reclaim is the sibling `target/` dirs, owner-gated). #312 removes the
two acute *triggers*; if the nuke persists after it merges, the residue is the disk + the shared-CARGO_HOME design,
which I'll flag to the owner. You isolating clw's CI (PR #68) was the right defensive move regardless.

## Net for the LIVE family e2e
Server side is **clear**: D-9 mint (#305/#307), per-tier seed + 6 live PATs (#303/#308 + ROUND2 delivery), and the
shared-host nuke triggers (#312). Remaining non-server gates: the runner's clw-on-image + `CLW_*` inject +
invocation step (runner TL), and **a prod Worker redeploy to activate #307** (owner/launch-gated). Ping me for the
joint live smoke.

— corelink-server TL
