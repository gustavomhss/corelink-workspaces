# HANDOFF — per-job runner PAT minting (D-9)

> **For:** the corelink-server / CoreLink-API session owning token issuance (and
> whoever owns the Clerk PAT-scope boundary). **From:** the `clw` go-live
> hardening fleet's family-integration adversarial critic (ROADMAP v2, §M7.2 /
> §M7.5). **Severity:** HIGH — **blocks the family e2e (M7.5)** and the runner ↔
> clw contract freeze (M7.2). **Why a handoff, not a fix:** token minting lives
> in `corelink-server` / the CoreLink API / Clerk — separate repos `clw` never
> edits. This is the scoped contract to define before runner-side work starts.

---

## TL;DR

A disposable CoreLink runner **cannot ship a long-lived PAT** (a baked-in
credential on an ephemeral, possibly-untrusted compute box is a leak waiting to
happen). It needs a **per-job, narrowly-scoped, short-TTL token** minted at job
dispatch and discarded with the runner. Today the minting contract is
**undefined**: who mints it, what scope it carries, what TTL, and what triggers
issuance are all open. This handoff pins the **auth two-layer model** `clw` will
rely on and lists the exact minting contract the server/API session must define.

---

## Evidence / why this is needed

- **Disposable runners have no durable secret store.** ROADMAP M7.2: "a disposable
  runner has no `~/.clw/config.toml`, an injected secret — conflicts with the H2
  `$HOME`-config hardening." A long-lived PAT baked into a runner image, or
  injected once and reused across jobs, is exactly the credential-lifetime
  problem the audit's H2 hardening exists to prevent.
- **The runner authenticates exactly like any `clw` client** — so it needs a PAT
  in the same shape, just scoped tighter and living shorter.
- **The scope must grant BOTH CAS and AC.** A runner does the full snapshot →
  hydrate → run → snapshot-back loop, which touches CAS (blob PUT/GET) **and** AC
  (ref read/write). ROADMAP M7.5 flags the risk directly: "scoped PAT (**must
  grant CAS+AC** — the drifted D1 single-`scope` enum may not; **confirm**)." If
  the PAT carries a single `scope` enum that picks one of CAS *or* AC, the runner
  flow breaks halfway. This is the load-bearing open question.
- **No owner / no contract today.** M7.2 requires freezing an anchor contract
  before any runner-side or e2e work; D-9 is its auth half. M7.5 (family e2e)
  gates on D-9.

---

## The auth two-layer model (what clw relies on — pin this)

This is the model the runner and `clw` will implement; the server MUST honor it
exactly:

1. **Layer 1 — client-facing (the runner / `clw`):**
   - Sends `Authorization: Bearer <per-job tenant PAT>`.
   - Selects the tenant via **tenant-in-URL-path** (`:tenant`), identical to every
     other `clw` route (CAS, AC, ref-LIST D-7, CAS-LIST D-8).
   - **NEVER sets `x-corelink-tenant-id`.** That header is **server-INTERNAL**
     (service-to-service / gateway → backend). `clw` and the runner do not know
     about it and must never emit it. A request that arrives from a client with
     `x-corelink-tenant-id` set MUST be ignored or rejected — it must not be a
     way to escape the PAT's tenant binding.

2. **Layer 2 — server-internal:**
   - The server resolves the Bearer PAT, authorizes it against the path tenant,
     and derives `x-corelink-tenant-id` (or equivalent) for its own downstream
     fan-out. This is invisible to and untouchable by the client.

The single invariant `clw` depends on: **the PAT + tenant-in-path is the sole
client-side tenant authority; nothing the client sends as a header can broaden
or redirect it.**

---

## The minting contract to define (the open questions)

`clw`/runner cannot proceed until these are pinned. The server/API/Clerk session
owns the answers:

| Question | Options to decide between | Why it matters to clw |
|---|---|---|
| **Who mints?** | CoreLink API endpoint (e.g. `POST /v1/tokens` exchanging a runner identity for a job PAT) **vs** Clerk machine/short-lived token **vs** the dispatcher service that launches the runner | Determines where the runner gets its token and what bootstrap credential it presents to get one. |
| **Scope** | **MUST grant BOTH CAS + AC** (full snapshot↔hydrate↔run↔snapshot-back). The drifted **D-1 single-`scope` enum** may only allow one. | If scope is single-valued, the runner flow is impossible — this is the top blocker. Confirm the PAT can carry a CAS+AC compound scope. |
| **TTL** | Bounded to the job (minutes–low hours), with a hard cap; ideally renewable only by re-minting, not by the runner | Short TTL is the whole point — a leaked per-job token must expire fast. clw/runner must handle expiry mid-job (re-mint vs fail). |
| **Trigger** | Minted at **job dispatch** by the dispatcher and injected into the runner env, **vs** the runner self-requests on startup using a launch identity | Defines the runner's config-resolution path (M7.2) — the injected-secret vs self-fetch decision conflicts with H2 `$HOME`-config hardening; must be reconciled. |
| **Tenant binding** | The minted PAT is bound to exactly one tenant (the job's), enforced server-side; tenant-in-path must match | clw relies on the PAT being single-tenant so tenant-in-path is unambiguous. |
| **Revocation** | Can a job PAT be revoked before TTL (e.g. job aborted)? Is it auto-invalidated when the runner is torn down? | Affects the erasure/abuse story and whether a leaked token is containable before expiry. |

### Proposed minting-contract shape (for the owner to ratify or replace)

```
POST  <minting endpoint>           # CoreLink API or Clerk — TBD
Auth: <runner launch identity>     # NOT a tenant PAT — the bootstrap credential
Body: { "tenant": "<tenant>", "job_id": "<id>", "scopes": ["cas", "ac"] }
→ 200 { "pat": "<short-lived token>", "tenant": "<tenant>",
        "scopes": ["cas","ac"], "expires_at": "<RFC3339>" }
```

The runner then uses `pat` as `Authorization: Bearer` with tenant-in-path for the
whole job, and discards it on teardown.

---

## Confirm with owner

1. **Who mints** — CoreLink API endpoint, Clerk, or the dispatcher? Name the
   concrete bootstrap credential the runner presents to obtain the job PAT.
2. **Scope grants BOTH CAS + AC** — confirm the PAT scope model can express a
   CAS+AC compound. **This is the #1 blocker**; ties to the **D-1 single-`scope`
   enum drift** CONFIRM in the ledger (Clerk/signup own that enum).
3. **TTL + renewal** — the job-bounded lifetime, hard cap, and whether mid-job
   expiry means re-mint or fail.
4. **Trigger / injection** — dispatch-time env injection vs runner self-fetch;
   reconcile with the M7.2 runner config-resolution path and the H2
   `$HOME`-config hardening (a disposable runner has no `~/.clw/config.toml`).
5. **`x-corelink-tenant-id` is server-internal** — confirm the server rejects /
   ignores it when set by a client, so the PAT + tenant-in-path is the sole
   client-side tenant authority.
6. **Revocation** — pre-TTL revocation + teardown invalidation semantics.

This must be **frozen as an anchor contract before any runner-side (M7.3) or
family-e2e (M7.5) work** (ROADMAP §M7.2).

## Cross-references

- ROADMAP dependency ledger: `docs/ROADMAP-go-live.md` (D-9), §M7.2, §M7.5,
  and the two `CONFIRM` rows (Clerk PAT scope grants CAS+AC; region-routing).
- Sibling deps: `docs/HANDOFF-ref-enumeration.md` (D-7),
  `docs/HANDOFF-cas-delete-list.md` (D-8) — same Bearer-PAT + tenant-in-path
  two-layer auth model.
