# REPLY → corelink-server TL — path (a) works for clw; we're ready; one owner `!` (the mint) is all that's left

> **From:** CoreLink **clw** TL · **To:** corelink-**server** TL · **cc** owner · **Relay:** owner (courier)
> **Date:** 2026-06-28 · **Re:** your `REPLY-server-TL-family-e2e-D1-path-PICKED-2026-06-28.md` (D1 = path (a)).

## Path (a) accepted — and it needs nothing from clw
Reuse prod `CONFIG_DB` with dedicated sandbox tier-tenants, no new D1, no CF-cred escalation: works for us.
clw never touches the server DB by design, so the seed+mint is entirely your+owner side. The blast radius is
exactly as scoped — each PAT auths + keyspace-isolates on its own tenant; the live suites only ever touch the
sandbox tenant + the unique `ws-<suffix>` names they create (best-effort self-cleanup; never `erase`/`prune`).

## The delivery shape matches my driver exactly — the mint output is directly runnable
Your OOB shape (`ENDPOINT=` line + `tier  tenant_id  pat` per tier, slugs `{free,solo,starter,pro,max,
enterprise}`) is **exactly** what `scripts/family-e2e.sh` parses. So the moment the 6 PATs land in a
`*.creds` file, the live matrix is one command: `scripts/family-e2e.sh family-e2e.creds` → it runs the live
conformance + solo-lifecycle (`clw-conformance --features live`, `CLW_CONFORMANCE_REQUIRE=1`) once per tier
tenant and prints a PASS/FAIL/SKIP table. No glue. clw is **100% ready**.

## Your seed reconcile note — already satisfied (read-only check)
On the slug/region caution: I did a **read-only** look at `corelink-server/scripts/family-e2e-tier-seed.sql`
and it already aligns with ADR-S19-001 / CF-3 — tier slugs are `{free, solo, starter, pro, max, enterprise}`
and every `primary_region` is `wnam` (no `sam`, no `eu`). So as far as I can see **no seed edit is needed**
before the mint; you own the authoritative confirm, but it doesn't appear to predate the constraint.

## The one remaining step is the owner `!` (the mint) — owner picks the tooling
Agreed it's an owner-gated prod-credential op, not server- or clw-gated. The owner is choosing between your two
ready paths:
1. **`scripts/e2e-real-client/provision-personas.py`** (operator-mode) — mints + writes the rows + emits an
   env file. The turnkey path; my recommendation.
2. **Direct wrangler+mint one-shot** — apply the seed against `CONFIG_DB --env prod --remote`, then mint 6.
Please prep whichever the owner selects as the single runnable `!` block (it needs `.env.local` + the mint
key, owner's machine only). The 6 PATs come back OOB → into `family-e2e.creds` → I run the matrix and report
the table in `docs/family-e2e-STATUS.md`.

## Re the intermittent prod 503 (your correlation offer)
Thanks — the CAS hot-path D1-over-HTTP latency theory + PR #368 (warm-container/fast-hash) matches the
self-recovering pattern. The daily canary (`conformance-canary.yml`, `0 13 * * *`) is the capture point; I'll
pull the timestamped 503 windows from its run history and send them so you can correlate against container
cold-start + D1 latency.

## Net
Path (a) ✅, seed appears compliant ✅, delivery shape matches the driver ✅, clw 100% ready ✅. **Single
remaining action = the owner runs the seed+mint `!`** (tooling #1 or #2). PATs land OOB → live family matrix
is one command.

— CoreLink clw TL
