# HANDOFF — corelink-server dependency track (master brief)

> **For:** the **corelink-server** session (separate repo `clw`/`corelink-workspaces`
> NEVER edits). **From:** the `clw` go-live hardening fleet (ROADMAP v2,
> `docs/ROADMAP-go-live.md`). **What this is:** the single, pick-up-cold brief that
> consolidates **every** server-side dependency the `clw` go-live needs, in
> executable order. Each row points to the detailed spec doc — this brief does not
> duplicate them. **Baseline reference:** `main @ f8f5edf`.
>
> **The frame:** `clw` is a pure client of `corelink-server`. All server work is
> coordinated through the ROADMAP dependency ledger; this document is that ledger,
> expanded into an actionable track with a critical path and a recommended schedule.
> Several items are **GDPR / launch-blocking**; others gate **campaign #1** (the
> CI-acceleration product). Both classes are marked below.

---

## SUMMARY — every server dependency (one screen)

| ID | What | Why / clw feature it unblocks | Severity | Class | Spec doc |
|---|---|---|---|---|---|
| **D-1** | `DELETE /v1/ac/...` ref delete (idempotent, tenant-scoped) | `clw rm` (ROADMAP M1.4) — deletes a known ref by name | HIGH | launch (erasure half) | ROADMAP §M1.4 (audit L5) |
| **D-3** | signup-worker region fix: kill `enam` hardcode + `regionFromColo` colo→macro-region map + make `configureTenant` real (or fold into `createTenant`) | Correct data-**residency** end-to-end (ROADMAP M7.1) | HIGH | **launch-blocking** (GDPR residency) | `HANDOFF-signup-region-hardcode.md` |
| **D-4** | `R2KvStore` — persistent KV (per-tenant prefix, **no** hash-verify), behind turbo + cargo. Closes turbo `TODO(v2)`. **Keystone.** | Campaign #1: persistent Turbo cache + backing for the cargo route (D-5) | HIGH (campaign) | campaign #1 | `FINDING-sccache-adapter-gaps.md` step 1 |
| **D-5** | Native cargo route, **Option A header-trust** (GET/PUT/HEAD `/cargo/:tenant/:key`, trust `x-corelink-tenant-id`, cross-tenant 403), mirror `bazel_v2`, mount + tests. **Backed by D-4.** | Campaign #1: sccache→CoreLink (clw is **NOT** in the data path — sccache speaks its own HTTP to this route) | HIGH (campaign) | campaign #1 | `FINDING-sccache-adapter-gaps.md` step 2 |
| **D-6** | Per-tenant CAS write scaling — Option **B** (shard CAS writes across N sub-DOs by hash prefix, safe because content-addressed) **or** **C** (bypass the DO for stateless CAS PUTs) | Raises the per-tenant write ceiling (the real "429"); campaign #1's defining workload; ROADMAP M5.6 quota UX | MED now / **HIGH for campaign #1** | campaign #1 | `FINDING-per-tenant-write-ceiling.md` |
| **D-6a** | `wrangler.toml max_instances=5` vs **live `20`** config drift — reconcile **before** D-6 | Without it, any B/C scaling is silently capped at the wrong instance ceiling | LOW-but-ordering | campaign #1 (precursor) | `FINDING-per-tenant-write-ceiling.md` (drift note) |
| **D-7** | **Ref-enumeration LIST** — `GET /v1/refs/:tenant` (or `/v1/ac/:tenant`), paginated, returns each ref's `ref_key` + `manifest_hash` (+ name/domain if pre-image stored) | `clw ls --all`, `clw prune` roots, **GDPR Art.17 erasure** (the GC root set) | HIGH | **launch-blocking** (GDPR Art.17) | `HANDOFF-ref-enumeration.md` |
| **D-8** | **CAS DELETE + CAS LIST** — `DELETE /v1/cas/:tenant/:hash` (idempotent 204) + `GET /v1/cas/:tenant` (paginated, `size`+`uploaded_at`) | `clw prune` / CAS-GC sweep candidates + delete; **GDPR Art.17 erasure** + erasure proof | HIGH | **launch-blocking** (GDPR Art.17) | `HANDOFF-cas-delete-list.md` |
| **D-9** | **Per-job runner PAT minting** — who mints, scope (**must be CAS+AC**), TTL, trigger, tenant-binding, revocation; honor the two-layer auth model | Runner ↔ clw contract freeze (M7.2) + **family e2e (M7.5)** | HIGH | family / campaign #1 | `HANDOFF-runner-pat-minting.md` |
| **CONFIRM-a** | **Clerk PAT scope grants CAS+AC** — the drifted live D1 single-`scope` enum may grant only one | Family e2e (M7.5) snapshot↔hydrate↔run↔snapshot-back; also gates D-7/D-8/D-9 auth | HIGH (top blocker) | family / launch | `FINDING-sccache-adapter-gaps.md` blocker 2; D-9 §confirm |
| **CONFIRM-b** | **Region-routing mechanism a/b/c** — (a) server redirects to a regional host, clw follows; (b) clw learns `primary_region` via a new lookup API (**→ D-10**) and maps region→host; (c) the global host region-routes internally → **clw needs no change** | Shapes ROADMAP M7.1 and whether **D-10** even exists | MED (scoping gate) | launch (residency) | ROADMAP §M7.1; `HANDOFF-signup-region-hardcode.md` |

**Conditional / low-priority server deps (not on the critical path, listed for completeness):**

| ID | What | Trigger condition | Spec |
|---|---|---|---|
| D-2 | Back up `SIGNUP_TOKEN_KEY` (write-only secret) | always (ops hygiene) — ROADMAP M4.4 | BUGREPORT §secrets |
| D-10 | Tenant `primary_region` lookup API | **only if** CONFIRM-b resolves to path (b) | ROADMAP §M7.1 |
| D-11 | Distinct CAS-PUT status (created vs existed) | optional — faithful `chunks_uploaded` accounting (M1.2 is best-effort without it) | ROADMAP §M1.2 |
| D-12 | Authenticated binary download route | **only if** the download surface is R2-behind-CoreLink (vs GitHub-SSO/vendor) — ROADMAP M3.3 | ROADMAP §M3.3 |

> **Already landed upstream (do not re-do):** AC-500 root cause — PR #139 (empty
> `R2_AC_BUCKET`); wrangler@4 deploy fix (on `main`). D-3's `enam` value is
> schema-valid and was **NOT** the AC-500 cause — D-3 is the *latent residency* gap.

---

## DEPENDENCY ORDER / critical path

**Intra-track ordering constraints (hard edges):**

- **D-6a → D-6.** Reconcile the `max_instances` drift before sharding/bypass, or scaling is silently capped.
- **D-4 → D-5.** The cargo route has no durable backing store until `R2KvStore` exists; D-5 is "designed, blocked on D-4."
- **D-7 + D-8 → GDPR erasure (ROADMAP M6.4).** Both required as a unit. Ref-LIST yields the GC **roots**; CAS-LIST yields the **sweep candidates**; CAS-DELETE does the reclaim. D-1 (`rm`) deletes a *known* ref but orphans blobs — insufficient alone. Land D-7 + D-8 **together** so M6.4 closes as a unit.
- **D-9 + CONFIRM-a → family e2e (ROADMAP M7.5).** The runner needs a per-job PAT that grants **both** CAS and AC; if the live single-`scope` enum can't express a CAS+AC compound, the runner flow breaks halfway. CONFIRM-a is the **#1 blocker** and also underpins the auth scope of D-7/D-8.
- **CONFIRM-b → D-10 (conditional).** D-10 exists *only* if region-routing resolves to mechanism (b). Resolve CONFIRM-b before scoping any region work.

**Launch-blocking vs campaign-#1 split:**

| Class | Items | Gates |
|---|---|---|
| **GDPR / launch-blocking** | **D-3** (residency), **D-7 + D-8** (Art.17 erasure), **D-1** (rm, erasure half), CONFIRM-a (auth scope), CONFIRM-b (residency routing) | Exit-gate #6 (AC delete round-trip), #8 (residency + right-to-erasure), #9 (compliance) |
| **Campaign #1 (CI-acceleration)** | **D-4 → D-5** (cargo route), **D-6a → D-6** (write scaling), D-9 + CONFIRM-a (runner PAT for the e2e) | M7.4 (campaign), M7.5 (family e2e) |
| **Conditional / low** | D-2, D-10, D-11, D-12 | per trigger above |

**The critical path (longest chain to a launchable, family-proven product):**

```
                 ┌─ D-3 ─────────────────────────────► residency correct (M7.1)
                 │
 CONFIRM-a ──────┼─ D-7 + D-8 ──► GDPR Art.17 erasure (M6.4) ─► launch gate #8
 (PAT CAS+AC)    │      ▲
                 │      └─ D-1 (rm) feeds the erasure flow
                 │
                 └─ D-9 ──┐
 D-4 ► D-5 ──────────────►├─► family e2e (M7.5) ─► campaign #1 launch
 D-6a ► D-6 ─────────────►┘     (also needs M7.3 runner repo — owner-less today)
 CONFIRM-b ─► (D-10?) ─────────► region endpoint mechanism for M7.1/M7.5
```

> **Note (not a server task, but it gates M7.5):** the **CoreLink runners repo**
> (M7.3) is a *third* repo with **no owner and no WP today**. The family e2e cannot
> complete until someone owns the runner-side consumer of the frozen clw contract.
> Flag this to the owner — it is the non-server hole on the family path.

---

## RECOMMENDED SEQUENCING for the server session

Schedule in this order. Several items are independent and can run in parallel lanes;
the constraint is the intra-track edges above.

**Schedule FIRST (unblockers + cheap confirmations, do these on day one):**

1. **CONFIRM-a — Clerk PAT scope grants CAS+AC.** Cheap to answer, the **#1 blocker**: it gates the auth scope of D-7, D-8, **and** D-9, and the entire family e2e. Resolve before scoping any of them. (Clerk/signup own the enum.)
2. **CONFIRM-b — region-routing mechanism a/b/c.** Determines whether **D-10** exists and what (if anything) clw must do for M7.1. Cheap; unblocks D-3's scope.
3. **D-6a — reconcile `max_instances` drift (5 vs 20).** Tiny, but must precede D-6; do it now so the scaling work lands on the right ceiling.

**Then the GDPR / launch-blocking core (the residency + erasure promise the product sells):**

4. **D-3 — signup-worker region fix.** Small, self-contained, in *your* active GAP-5 lane (`apps/signup-worker/src/webhooks/clerk.ts`). Residency correctness; do it early — it's low-risk and high-value.
5. **D-7 + D-8 together — ref-LIST + CAS-DELETE/LIST.** The erasure unit. Same Bearer-PAT + tenant-in-path two-layer auth model; land as a pair so ROADMAP M6.4 closes at once. (Auth scope depends on CONFIRM-a.)
6. **D-1 — `DELETE /v1/ac/...`.** Idempotent, tenant-scoped ref delete; completes the `clw rm` + erasure flow (`rm` each ref → prune → assert LIST empty).

**Then campaign #1 (CI-acceleration) — the long pole, start its keystone early in parallel:**

7. **D-4 — `R2KvStore`.** The keystone; its own PR + tests. Unblocks both sccache (via D-5) **and** persistent Turbo caching. Start this **in parallel** with the GDPR core — it has the longest lead time.
8. **D-5 — native cargo route (Option A header-trust)** on top of D-4; mirror `bazel_v2`'s `caller_tenant` read, **no** PatValidator/adapter-crate auth, cross-tenant 403, mount + tests. Then an owner-scheduled off-peak prod deploy + measurement.
9. **D-6 — per-tenant CAS write scaling (B or C).** After D-6a. Recommended: option **B** (shard by hash prefix — cheapest, content-addressing makes it safe) or **C** (bypass DO for CAS PUT — biggest win). The CI campaign cannot launch credibly without one.

**Then the family auth + e2e:**

10. **D-9 — per-job runner PAT minting.** Pin the minting contract (who/scope/TTL/trigger/binding/revocation) per the two-layer model; freeze as an anchor contract **before** runner-side (M7.3) or e2e (M7.5) work. Depends on CONFIRM-a for the CAS+AC compound scope.

**Conditional / opportunistic (schedule when their trigger fires):**

11. **D-2** (back up `SIGNUP_TOKEN_KEY`) — ops hygiene, do alongside D-3 while in the signup lane.
12. **D-10** — only if CONFIRM-b → (b). **D-11** — optional accounting nicety. **D-12** — only if the download surface is R2-behind-CoreLink.

**One-line scheduling heuristic:** *confirmations first (CONFIRM-a/b, D-6a) → D-3 + the D-7/D-8/D-1 erasure unit (launch-blocking) → kick off D-4 in parallel as the campaign keystone → D-5 → D-6 → D-9 → flag the ownerless runner repo (M7.3) to the owner.*

---

## Cross-references

- Master ledger + milestones + waves: `docs/ROADMAP-go-live.md` (dependency ledger, §M1.4, §M6.4, §M7).
- Detailed specs (do not duplicate — read at pickup):
  - D-3: `docs/HANDOFF-signup-region-hardcode.md`
  - D-7: `docs/HANDOFF-ref-enumeration.md`
  - D-8: `docs/HANDOFF-cas-delete-list.md`
  - D-9: `docs/HANDOFF-runner-pat-minting.md`
  - D-4/D-5 + CONFIRM-a: `docs/FINDING-sccache-adapter-gaps.md`
  - D-6/D-6a: `docs/FINDING-per-tenant-write-ceiling.md`
  - AC-500 forensics (context): `docs/BUGREPORT-prod-ac-500.md`
- The shared auth invariant across D-7/D-8/D-9: **Bearer tenant-PAT + tenant-in-URL-path is the sole client-side tenant authority; `x-corelink-tenant-id` is server-internal and MUST be ignored/rejected when set by a client.**
