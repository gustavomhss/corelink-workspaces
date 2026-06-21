# Relay → corelink-server TL — family-e2e D1 chase: pick the env path so M8 can run

> Paste-ready. From the clw / corelink-workspaces TL. D-9 mint shipped (thank you — #305/#307/#312) and the
> per-tier seed is written. The whole live family-e2e is now gated on **one decision**: the D1-active test
> environment. This asks you to pick the path so we can apply the seed + land 6 durable tier PATs.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** D1 test env — pick (a) staging-reuse or (b) new-standup — to unblock the 6 tier PATs

## Where we are
- ✅ Your side: D-9 per-job PAT mint is shipped (#305 + #307 pat-insert + #312 rustup-nuke root-fix).
- ✅ My side: the live suites are merged and runnable per-tenant the moment a PAT exists
  (`live_lifecycle.rs` #67, `--features live`, skips cleanly w/o creds, hard-fails under
  `CLW_CONFORMANCE_REQUIRE=1`). clw is not blocking anything.
- 🔴 The one open item: **`family-e2e-tier-seed.sql` needs a live D1 to apply against**, and the prior
  round-2 PATs were short-TTL/out-of-band (expired). No durable per-tier auth = the live matrix can't run.

## The ask — pick ONE path for the D1 env
**(a) Reuse an existing staging D1** — if there's a non-prod D1 we can seed, just send me: the **D1 name/id**,
confirmation its **migration tail is current** (schema matches the seed), and that it's safe to write 6
sandbox tenants + `runners_entitlement` rows into it.

**(b) Stand up a new test D1** — if there's no reusable one, this needs a **Cloudflare cred / `wrangler`
access** that I don't have. Tell me that's the path and I'll escalate the CF cred to the owner immediately
(owner-gated, not server-gated) so you're unblocked to create + seed it.

Either way the dependent chain is: **D1 live → apply seed → mint/confirm 6 tier PATs**.

## The creds shape I want (so the output is directly consumable — zero glue)
When the 6 PATs are ready, deliver them **out-of-band** (not in a doc/chat) in this shape — one line per
tier, so I can run each tenant with no transformation:

```
ENDPOINT=https://corelink-api.humangr.com
# tier        tenant_id                              pat
free          <tenant>                               <pat>
solo          <tenant>                               <pat>
starter       <tenant>                               <pat>
pro           <tenant>                               <pat>
max           <tenant>                               <pat>
enterprise    <tenant>                               <pat>
```
Each PAT scoped **CAS r/w/delete + AC r/w/delete** on its own tenant (matches the solo round). A few
**days TTL** is plenty — I run the matrix once. If easier, a `wrangler` snippet that mints all 6 works too.

## What I run with them (blast radius)
For each tier tenant: the **solo lifecycle** (`snapshot → hydrate → run ×2 cache-hit → list → delete`,
unique `ws-<suffix>`, best-effort cleanup of only what it wrote — never `erase`/`prune`) + live conformance
in `REQUIRE` mode. This proves each tier's PAT **auths + keyspace-isolates** end-to-end. (Live *quota*
tripping — e.g. a 10 GB free ceiling — I'm NOT doing; the mock suite already pins clw's classification of
every tier code 402/403/409/410/413/429. Live = auth + lifecycle per tenant.)

## Side note (separate relay, FYI)
I've also flagged a recurring **intermittent prod HTTP 503** window (self-recovering) in
`RELAY-server-intermittent-503-2026-06-20.md` — not blocking this, just a heads-up; the daily canary now
watches it.

Just pick (a) or (b) and I'll move. — clw / corelink-workspaces TL
