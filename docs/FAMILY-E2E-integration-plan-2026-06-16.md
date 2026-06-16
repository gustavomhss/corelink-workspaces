# Family e2e — integration test plan + cross-TL alignment (2026-06-16)

> **From:** clw / corelink-workspaces TL. **Purpose:** the user wants a real integration test —
> clw ↔ corelink-server ↔ corelink-runners, a solo test, and per-plan/tier tests. I recon'd both
> sibling repos (read-only) to ground this in evidence, not assumption. **Bottom line: the clw side
> is built and proven (mock); a LIVE cross-repo run is blocked on three cross-team deliverables that
> must be aligned with the server + runner TLs.** This doc is relay-ready — hand the relevant section
> to each TL.

> ✅ **UPDATE (2026-06-16, later) — the keystone SHIPPED.** corelink-server merged **D-9**:
> `POST /internal/v1/runner/mint` (+ `/revoke`) is in `main` (PR #305), and they delivered the
> per-tier **seed** (`corelink-server/scripts/family-e2e-tier-seed.sql`, #303 — 6 tenants
> free→enterprise, deterministic ids `…0f0001..0006`). "Finding 1 — D-9 doesn't exist" below is now
> HISTORICAL. Remaining blockers are **not clw code**: a D1-active test env + applying the seed +
> minting test-tenant PATs (**server TL**, see `RELAY-family-e2e-to-server-TL-ROUND2.md`); the runner's
> clw-invocation build (**runner TL** wave); the shared-host rustup-nuke root cause (owner/infra —
> clw's CI is now isolated, PR #68). clw's three test suites are merged (PR #64 runner-model, #66
> per-tier matrix, #67 solo-live).

---

## The three test layers (what the user asked for)

| Layer | Scope | Status |
|---|---|---|
| **Solo** | clw ↔ live corelink-server, full lifecycle | clw side ready; an **automated** solo live suite is the next clw build (runs once a fresh conformance PAT is dropped) |
| **Cross-repo (family)** | signup → runner mints/injects PAT → runner invokes `clw` → snapshot/hydrate/run → cache-hit | clw half **DONE + proven (mock)** — PR #64; **D-9 mint route SHIPPED** (server #305); live run now waits on the runner's clw-invocation build + a D1 test env |
| **Per-plan/tier** | behavior across the 6 tiers (free/solo/starter/pro/max/enterprise) | clw-side response-handling testable now; **live per-tier enforcement is server-side + fail-open without D1** |

### What clw has already done (build against this)
- **PR #64** `runner_model_env_only_lifecycle_and_keyspace_isolation` — proves clw runs exactly as a
  disposable runner drives it: env-only (`CLW_ENDPOINT/TENANT/TOKEN`, no config file, no flags),
  runner keyspace via `CLW_REF_DOMAIN=runner`, full lifecycle byte-identical, and keyspace isolation
  (a user-domain hydrate of a runner ref is NOT FOUND). This is the "mock-tested first" the runners
  TL committed to.
- `CLW_REF_DOMAIN=runner` → reserved keyspace `clw/ref/runner/v1/` (merged, collision-proof by
  construction).
- Env-injection ready (`CLW_*` + `CORELINK_*` aliases). Frozen CLI contract (`docs/CLI-CONTRACT.md`).

---

## TO: corelink-server TL — findings + asks (evidence from a read-only recon of corelink-server)

### ✅ Finding 1 — RESOLVED: D-9 runner-mint route SHIPPED (server #305). *(Was unbuilt at recon time; recorded below for history.)*
Grep of corelink-server finds **no** `/internal/v1/runner/mint`, `POST /v1/tokens`, or `mintScopedPat`.
What exists: `POST /_internal/pat/mint` (`crates/corelink-container/src/routes/internal_pat.rs:490` —
the **signup auto-provision** mint, scopes `admin|cas:rw`, long-TTL, **not** per-job) and
`POST /internal/v1/auth/introspect` (`crates/corelink-container/src/routes/auth_introspect.rs:434` —
the fabric's real touchpoint: validates an existing PAT → `{valid, tenant_id, plan, max_concurrency}`,
runner cap from `runners_entitlement`, `migrations/d1/0070`).

**This matches your "NEW WP, after cycle-2" framing — confirming D-9 is genuinely unbuilt.** Ask:
1. Confirm the runner→clw flow uses the **future per-job mint** (a scoped, short-TTL CAS+AC PAT the
   runner USES), not introspect (which only validates an existing PAT and returns a cap — it does not
   hand the runner a usable credential). The family e2e needs the **mint**.
2. ETA / shape freeze for the mint route, so the runner side + clw mock can flip to live with zero
   rework.

### Finding 2 — per-tier enforcement is server-side AND fail-open without D1
The 6 tiers (`crates/corelink-tier-selection/src/tier.rs`) gate different axes in different places:
storage bytes + monthly requests at the **edge Worker** (`worker/src/lib/quota.ts:84-93`), RPS ladder
(`corelink-ratelimit/src/tier.rs`), retention TTL (`corelink-eviction/src/tier.rs`), runner concurrency
(`runners_entitlement`, a **separate** axis). The container's quota/byte-accounting/PAT gates all return
`None`/fail-open in dev/CI without D1 env. **Ask for a per-tier live test:** a way to provision test
tenants on **non-free** tiers (signup hardcodes `free` + region `enam`), i.e. seeded
`tier_selections` / `runners_entitlement` / `tenant_quota` rows, with the D1 gates ACTIVE — otherwise a
per-tier test exercises no-ops. clw will assert its side (correct handling of 429/402/409/410/413).

### Finding 3 — the rustup-nuke is STILL recurring on the shared CI host
`docs/RELAY-server-ci-fuzz-nukes-rustup.md` reported this fixed ("remediation on main"), but it
recurred again today: a job nuked `~/.cargo/bin` (every shim → dangling symlink to a deleted `rustup`),
breaking the clw `gates` job with `cargo: command not found` (exit 127). I reinstalled rustup to unblock,
but **the remediation is incomplete** — some job is still deleting the shared `~/.cargo/bin`. Please
re-investigate (isolate `CARGO_HOME`/`RUSTUP_HOME` per runner, or stop the nuking job).

---

## TO: corelink-runners TL — findings + asks (read-only recon of corelink-runners)

The contract you froze (`docs/handoff/2026-06-16-clw-runner-contract-response.md`) is good; these are
the gaps between it and the current code, for the family e2e:

1. **`clw` is not on the runner image.** Only `clw hydrate` is shelled out, in `BoxHydrate`
   (`crates/corelink-runner/src/boot/mod.rs:290`), box-lane-only and untested without `HUGIT_RUNNER_HOST`.
   `snapshot`/`run` verbs are **not** invoked anywhere. **Ask:** bake the digest-pinned `clw` binary
   into `deploy/runner/` and build the snapshot/hydrate/run invocation step (per your point 1).
2. **No `CLW_*` env injection exists.** The injected vars today are all `CORELINK_*`
   (`CORELINK_RUNNER_JITCONFIG` `runner_inject.rs:39`, `CORELINK_ENVELOPE_INGEST_*` `envelope_inject.rs`).
   **Ask:** inject the per-job PAT as **`CLW_TOKEN`** (+ `CLW_ENDPOINT` / `CLW_TENANT` /
   `CLW_REF_DOMAIN=runner`). clw accepts `CLW_*` **and** `CORELINK_*` (so either works) — but pick one
   and the env-only model is proven (PR #64).
3. **`CLW_REF_DOMAIN` / ref-domain not in runner code.** The reserved keyspace exists only in the
   handoff doc. clw shipped it as `CLW_REF_DOMAIN=runner` → `clw/ref/runner/v1/`. **Ask:** set that env
   so runner refs land in the reserved, collision-proof keyspace (no name-prefix scheme needed).
4. **Build against the MockBroker first.** Your `MockBroker` (`runner_broker.rs:185`) +
   `acceptance_runner_lease.rs` is the exact pattern to stub the per-job PAT before server D-9 lands —
   so the runner↔clw seam goes green mock-first, then flips live.

---

## Sequenced path to a LIVE family e2e

```
[clw ✅ PR#64 runner-model proven]   +   server D-9 mint route   +   runner: clw-on-image + CLW_* inject + invocation step
                                              │                              │
                                              └──────────────┬───────────────┘
                                                             ▼
                                       live signup → mint per-job PAT → runner runs clw → cache-hit
                                                             ▼
                                          per-tier matrix (server seeds non-free tenants, D1 gates active)
```

**clw's remaining solo build (no cross-team dep):** an automated solo live-lifecycle suite (gated by a
fresh conformance PAT) + a per-tier response-handling matrix. Both land next on the clw side.

> Evidence: read-only recon of `~/Documents/HuGR/corelink-server` + `~/Documents/HuGR/corelink-runners`
> (2026-06-16). Cross-refs: `docs/REPLY-to-runners-TL-2026-06-16.md`, `docs/RESPONSE-from-server-TL-2026-06-16.md`,
> `docs/CLI-CONTRACT.md`.
