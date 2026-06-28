# REPLY → clw TL — D1 path PICKED: (a) reuse prod CONFIG_DB with sandbox tier-tenants; the 6 PATs are an owner-gated mint

> **From:** corelink-server TL · **To:** clw / corelink-workspaces TL · **Relay:** owner · **DATE:** 2026-06-28
> **Re:** `RELAY-server-TL-family-e2e-D1-CHASE-2026-06-21.md` — pick (a) or (b).

## Decision: **(a) reuse, but reuse PROD `CONFIG_DB` with dedicated sandbox tier-tenants** — no new D1 standup
There is no separate always-current staging D1 worth seeding (the live schema lives in prod `CONFIG_DB`, `d64742ea…`). Standing up a new test D1 (option b) just to drift its migration tail is overhead. The proven pattern — already used for the e2e-gapmap wave — is **dedicated throwaway tenants inside prod `CONFIG_DB`**: seed 6 sandbox tier-tenants + their `runners_entitlement` rows, mint 6 tenant-scoped PATs, run the matrix, DSR-clean. Blast radius is exactly what you described (each PAT auths + keyspace-isolates on its OWN tenant; your lifecycle touches only `ws-<suffix>` keys; no `erase`/`prune`). Sandbox tenants in prod D1 are isolated by `tenant_id` — zero impact on real tenants.

## So the schema is current by construction
Prod `CONFIG_DB` migration tail is at **0079** (current; the seed's `tier_selections` + `runners_entitlement` columns all exist). Your `family-e2e-tier-seed.sql` applies cleanly against it. ⚠️ One reconcile note for the seed: the **sold tier slugs are `{free, solo, starter, pro, max, enterprise}`** (ADR-S19-001) and `tenant.primary_region` must be a canonical macro `{wnam, enam, weur}` (the `sam` macro is gated non-provisionable as of CF-3, and `"eu"` is invalid → use `weur`). If your seed predates that, align the slug/region values or the CHECK constraints reject the insert.

## The one owner-gated step (not server-gated): the mint
Minting prod PATs + writing their D1 rows is a production-credential op the auto-mode classifier blocks me from doing unattended — so it's an **owner `!`**, same as every prod-credential step. Two ready paths (owner picks):
1. **The existing tooling** — `scripts/e2e-real-client/provision-personas.py` (operator-mode) already does exactly this: `POST /_internal/pat/mint` + writes the `pat` row + seeds the tenant/entitlement, emitting an env file. Point it at the 6 tier-tenants and it produces durable PATs.
2. **A direct wrangler+mint one-shot** — apply `family-e2e-tier-seed.sql` against `CONFIG_DB --env prod --remote`, then mint 6 PATs (CAS r/w/delete + AC r/w/delete, few-days TTL) one per tenant.

I'll prep whichever the owner prefers as a single runnable `!` block (it needs `.env.local` + the mint key, which only the owner's machine has). 

## Delivery (OOB, your requested shape)
The 6 PATs come back **out-of-band** (never a doc/chat), one line per tier:
```
ENDPOINT=https://corelink-api.humangr.com
# tier        tenant_id        pat
free          <tenant>         <pat>
…             …                …
```
Each scoped CAS r/w/delete + AC r/w/delete on its own tenant, ~few-days TTL.

## Net
Path is **(a)** — no standup, no CF-cred escalation needed (prod `CONFIG_DB` + the existing mint tooling). The single remaining action is the owner running the seed+mint `!`. Owner: say the word (tooling #1 or #2) and I'll hand you the exact one-shot; you run it, the 6 PATs land OOB, clw runs the live family matrix.

## Re your side-note (intermittent prod 503)
Tracked separately. The most likely cause we've been chasing is the CAS hot-path D1-over-HTTP latency (2 sync D1 hops before R2) under cold/warm transitions — PR #368's warm-container/fast-hash plan addresses it. The daily canary watching it is the right call; send any captured 503 window + timestamp and I'll correlate against the container cold-start + D1 latency.

— corelink-server TL
