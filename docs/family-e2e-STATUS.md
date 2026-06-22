# family-e2e — cross-team status ledger (driven by clw TL)

> Single tracker for the live end-to-end "family" run: **clw ↔ corelink-server ↔ corelink-runners**
> (with hugit/githugr as producers). Updated by the clw TL as milestones move. Last update: 2026-06-21.
>
> **What a family run proves:** a disposable runner gets a per-job scoped PAT (server D-9 mint) → injects
> `CLW_*` → drives the real `clw` binary `snapshot → hydrate → run` in the isolated runner keyspace
> (`clw/ref/runner/v1/`) → correct memoization (hit/miss) + verbatim child exit code, across the 6 tiers
> (free/solo/starter/pro/max/enterprise) with real per-tier enforcement.

## State at a glance

| # | Milestone | Owner | Status | Blocks |
|---|---|---|---|---|
| M1 | clw CLI contract + exit-code freeze | clw | ✅ DONE (`docs/CLI-SURFACE-EXIT-FREEZE.md`, `CLI-CONTRACT.md`) | — |
| M2 | clw runner-model E2E (env-only, ref-domain isolation) | clw | ✅ DONE (PR #64) | — |
| M3 | clw per-tier RESPONSE classification (mock) | clw | ✅ DONE (PR #66, `tier_responses.rs`) | — |
| M4 | clw solo live-lifecycle suite (`--features live`) | clw | ✅ DONE (PR #67, `live_lifecycle.rs`) | — |
| M5 | clw binary published + digests handed to runner | clw | ✅ DONE (v0.1.3 on mirror; digests in the runner relay) | M9 |
| M6 | server D-9 per-job PAT mint shipped | server | ✅ DONE (#305/#307/#312, per REPLY 2026-06-16) | M8 |
| M7 | **D1-active test env decision + standup** | server (+owner CF cred) | 🔴 OPEN — the single blocker | M8 |
| M8 | apply `family-e2e-tier-seed.sql` + mint/confirm 6 tier PATs | server | ⛔ BLOCKED on M7 | M10 |
| M9 | runner: clw-on-image + `CLW_*` inject + invocation step (mock-first) | runner | 🟡 CAN-START-NOW (no clw blocker) | M10 |
| M10 | live family run (per-tier mint → cache-hit, cross-repo) | all | ⛔ BLOCKED on M8 + M9 | — |
| M11 | prod Worker redeploy picks up server #307 (live mint auths) | owner/launch | 🟡 OPEN — confirm | M10 (prod) |

## The critical path
**M7 (D1 env) is the one decision that unblocks the chain.** Until D1 is live, the seed can't be applied
(M8), so there are no durable per-tier PATs to run M4 against the 6 tenants, and M10 can't happen. Two
paths for M7: **(a)** reuse an existing **staging** D1 (server-TL confirms the name + migration tail), or
**(b)** stand up a **new** D1 (needs a Cloudflare cred → escalate to owner). Chasing server-TL to pick one:
`docs/RELAY-server-TL-family-e2e-D1-CHASE-2026-06-21.md`.

## What clw is doing UNILATERALLY (no external wait)
- ✅ Suites #66/#67 merged + green; #67 skips cleanly without creds and hard-fails under
  `CLW_CONFORMANCE_REQUIRE=1` — so it's directly runnable per-tenant the moment a PAT lands.
- ✅ v0.1.3 Linux binary digests + download URL handed to runner-TL so M9 can start in parallel
  (`docs/RELAY-runner-TL-family-e2e-binary-ready-2026-06-21.md`).
- ▶ Defined the creds-delivery shape clw wants for the 6 tier PATs (in the D1 chase relay) so M8's output
  is directly consumable — `cargo test -p clw-conformance --features live` per tenant, no glue.
- ✅ **M10 driver ready** (`scripts/family-e2e.sh`, PR #152): give it the creds file (the shape above) and
  it runs the live conformance + solo-lifecycle once per tier tenant in `REQUIRE` mode and prints a
  PASS/FAIL/SKIP table — absent tiers report SKIP (never PASS). PAT never echoed; `*.creds` gitignored.
  So the live run is **one command** the moment the 6 PATs land. Cold-reviewed APPROVE (leak-safe).
- ⏭ When the 6 PATs land: `scripts/family-e2e.sh <creds-file>` → report the table here as M10 progress.
  (Live per-tier *quota* enforcement — tripping a 10 GB free ceiling — is not cheaply CI-testable; the
  mock suite #66 already pins clw's classification of every tier code, so live validates auth+lifecycle
  per tenant, not quota-tripping.)

## Inbound/outbound correspondence (index)
- OUT: `RELAY-family-e2e-to-{runner,server,hugit}-TL.md`, `RELAY-family-e2e-to-server-TL-ROUND2.md`,
  `RELAY-server-TL-family-e2e-D1-CHASE-2026-06-21.md`, `RELAY-runner-TL-family-e2e-binary-ready-2026-06-21.md`
- IN: `REPLY-from-server-TL-family-e2e-2026-06-16.md`, `REPLY-runner-TL-family-e2e-cli-contract.md`,
  `RESPONSE-from-server-TL-family-e2e-plan-2026-06-17.md`, hugit setuid-contract CONFIRM (resolved).
