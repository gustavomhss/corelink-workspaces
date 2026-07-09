# FROZEN CONTRACT — GDPR1 account-erasure verb (githugr ⟷ hugit)

> **Frozen by:** clw coordinator (go-live lead) · **Date:** 2026-07-03 · authorized wave: no-waiver bar.
> The single cross-repo seam both sides transcribe. Additive-only (hugit-http-contracts rule).

## The verb
```
POST /v1/account            (top-level engine route, NOT repo-scoped — like POST /v1/repos)
  Authorization: Bearer <per-session engine token>   (as-the-user; subject = the caller's clerk:{org}:{user})
  Idempotency-Key: <uuid>    (REQUIRED — account-delete is irreversible; replay must be a no-op)
  body: { "confirm": "<account-slug>" }   (typed confirmation; MUST equal the caller's own account slug engine-side)
  → 202 Accepted { "state": "requested" }         (lifecycle B: request recorded; execution is its own gated step)
  → 400 { code, reason }   (confirm mismatch / missing idempotency-key)
  → 401                     (anon/operator — no god-path)
  → 403 STEP_UP_REQUIRED    (step-up not satisfied → githugr renders the existing step-up screen)
  → 409                     (idempotent replay of an already-requested erasure)
```

## Ratified decisions (lead)
- **Lifecycle = request (Option B):** the verb APPENDS a NEW `erasure.requested` event kind (account-scoped),
  keyed by the principal; execution (`erasure.executed`) is a SEPARATE gated step. Mirrors the existing
  decide-split (records intent, fail-closed), never a synchronous god-delete.
- **Principal = as-the-user:** append via `crate::writes::asserted_class(principal_chain)` (like
  `write_provision.rs:165`), subject DERIVED from the verified principal (never a body field) — NOT the
  hardcoded `PrincipalClass::Orchestrator` the decide verb uses.
- **Step-up REQUIRED:** the verb is a `STEP_UP_VERB`; since it bypasses `dispatch_repo_write`'s repo-head gate,
  re-apply the step-up gate explicitly on the new path (fresh_auth from `two_tier_auth`).
- **Idempotency + typed confirm:** both mandatory, validated engine-side BEFORE any tombstone.
- **Cascade wiring = staged (Option B):** THIS wave lands the verb + `erasure.requested` lifecycle +
  as-the-user attribution + confirm/idempotency/step-up gating. Wiring the X7 cascade
  (`x7/cascade.rs:448 erase_datapoint`) + X12 verifiability (`x12/erasure.rs:216`) to the persisted
  EventLog/CAS (genuine tombstone across CAS/provenance/context/mirror/corpus) is the follow-up execution WP —
  **owned by the hugit engine TL** (deep engine work, not agent-only). No-waiver note: staging the CASCADE is a
  build-sequence, not a deferral — the user-facing right-to-erasure REQUEST + lifecycle ships now; the execution
  wiring is the very next hugit WP, tracked, not accepted-open.

## Split
- **hugit (engine TL packet):** add the top-level `["v1","account"]` route beside `["v1","repos"]`
  (`server.rs:913`), the `erasure.requested` kind, the as-the-user append, the step-up gate, and (follow-up)
  the X7/X12 execution wiring. Freeze `AccountEraseReq { confirm: String }` in
  `hugit-http-contracts/src/write_requests.rs` (additive).
- **githugr (agent WP, this wave):** DELETE/POST `/v1/account` danger-zone → `LiveActions::erase_account`
  (mirror `create_repo`/`erasure_decide` plumbing: trait method + client + route + handler + fixture stub),
  renders the 403 STEP_UP as the existing step-up screen, and the 202 as an honest "erasure requested" state.
  Consumes exactly the verb above. Build against THIS contract; the button stays disabled-honest until the
  hugit route is live, then flips.
