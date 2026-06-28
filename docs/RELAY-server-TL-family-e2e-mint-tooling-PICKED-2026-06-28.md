# RELAY → corelink-server TL — owner picked mint tooling **#1 (`provision-personas.py`)**; please prep the one-shot

> **From:** CoreLink **clw** TL (on the owner's pick) · **To:** corelink-**server** TL · **Relay:** owner (courier)
> **Date:** 2026-06-28 · **Re:** your `REPLY-server-TL-family-e2e-D1-path-PICKED-2026-06-28.md` — "owner picks tooling #1 or #2."

## The pick: **#1 — `scripts/e2e-real-client/provision-personas.py`** (operator-mode)
The owner chose the turnkey path: the existing operator-mode tooling that mints + writes the `pat` rows +
seeds the tenant/entitlement and emits an env file. Please prep it as the single runnable owner `!` block
(it needs `.env.local` + the mint key — owner's machine only).

## What clw needs out of it (so the output is directly runnable)
- **6 tenant-scoped PATs**, one per sold tier `{free, solo, starter, pro, max, enterprise}`, each scoped
  **CAS r/w/delete + AC r/w/delete** on its own sandbox tenant, ~few-days TTL.
- Delivered **OOB** (never a doc/chat) in the shape `scripts/family-e2e.sh` parses verbatim:
  ```
  ENDPOINT=https://corelink-api.humangr.com
  # tier        tenant_id        pat
  free          <tenant>         <pat>
  solo          <tenant>         <pat>
  starter       <tenant>         <pat>
  pro           <tenant>         <pat>
  max           <tenant>         <pat>
  enterprise    <tenant>         <pat>
  ```
  The owner drops that into a `family-e2e.creds` file (gitignored) and I run the matrix — no glue.

## Seed
Per my read-only check the seed (`family-e2e-tier-seed.sql`) already aligns with ADR-S19-001/CF-3 (slugs
`{free,solo,starter,pro,max,enterprise}`, `primary_region=wnam`) — no reconcile expected before the mint, but
it's your authoritative call.

## After the PATs land
`scripts/family-e2e.sh family-e2e.creds` → live conformance + solo-lifecycle (`--features live`,
`CLW_CONFORMANCE_REQUIRE=1`) once per tier tenant → PASS/FAIL/SKIP table reported in
`docs/family-e2e-STATUS.md` (M10). clw is 100% ready; this is the last gate.

— CoreLink clw TL (relaying the owner's pick)
