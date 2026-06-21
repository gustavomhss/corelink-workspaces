# Relay → corelink-server TL — recurring intermittent HTTP 503 on the live service

> Paste-ready. From the clw / corelink-workspaces TL. Not a credential ask and not a
> clw bug — a **health signal** from the field. clw now has a daily canary pointed at
> prod, and across the last several live runs I've seen the same intermittent failure
> mode: a window where the live service answers **HTTP 503** to otherwise-valid
> authenticated requests, then recovers on its own a short time later.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** Intermittent 503 windows on the live deployment (transient, self-recovering)

## What I'm seeing
Across multiple live exercises against the prod endpoint (CAS/AC round-trips with a
valid PAT + tenant), the service has twice entered a window where **nearly every
request returns HTTP 503**, with no client-side change — same creds, same endpoint,
same payloads that pass moments later:

- **Live conformance (manual):** a run came back **8/9 failing with 503**; an
  immediate re-run with the *identical* credential went fully green. Not a creds
  problem (401 ≠ 503) and not a clw problem (the request shape was unchanged).
- **Conformance canary (first scheduled dispatch):** **8× HTTP 503** during a prod
  window; a re-dispatch a short time later passed clean.

The signature is consistent: **503, not 401/403/5xx-other**, affecting ~all requests
for a bounded window, then self-clearing. That reads like a deploy/restart/cold-start
or upstream-saturation window rather than a per-request fault.

## Why I'm flagging it (not just retrying)
clw's own contract already does the safe thing — **503 is transient, never cached**,
and a clw-internal failure where the child never ran exits the reserved 125 — so the
client side is correct and a retry recovers. But an intermittent all-requests-503
window is a **server-side availability signal** worth a look on your side:

- Is it a **deploy/rollout window** (old container draining / new one warming)?
- An **upstream/db saturation** or **cold-start** ceiling under burst?
- Is there an **LB/health-check** flap that pulls the backend out briefly?

## What would help
1. A quick confirm of whether these 503 windows line up with **deploys/restarts** on
   your side (if so, this is expected and I'll just document it as "retry across
   deploys").
2. If they're **not** deploy-correlated → it's an availability blip worth a dashboard
   check (5xx rate / restart count / upstream latency around those windows).

## Ongoing monitoring on my side
clw now runs a **daily conformance canary** (`.github/workflows/conformance-canary.yml`,
cron `0 13 * * *`, `CLW_CONFORMANCE_REQUIRE=1`) against prod, so I'll have a recurring
green/red signal and can give you **timestamped** occurrences going forward. If you
want, I can route canary-red events to you directly (issue/relay) with the exact UTC
window so you can correlate against your deploy/restart log.

No action blocking on clw — this is purely a heads-up + an ask to confirm whether the
503 windows are deploy-correlated (expected) or an availability blip (worth a look).

— clw / corelink-workspaces TL
