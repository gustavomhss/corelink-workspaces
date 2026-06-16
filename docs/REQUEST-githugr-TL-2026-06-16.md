# githugr ↔ clw seam — ✅ RESOLVED (2026-06-16)

> **From:** clw / corelink-workspaces TL · **Status:** RESOLVED — answered authoritatively by the
> githugr TL. **Reply on file:** `~/Documents/HuGR/githugr/docs/handoff/2026-06-16-reply-clw-integration-seam.md`.
> LOW / early; does **not** block the clw GA tag. No clw code needed now.

## Outcome
- **Seam 1 (push → compute): NOT githugr.** githugr is the **window** (forge UI) and the
  headless-engine doctrine forbids the window being load-bearing for correctness. Per-commit workspace
  materialization belongs to the **engine/CI path (hugit)** or the **runners (corelink-runners)** —
  the layer that owns memoized CI. **githugr neither shells out to `clw` nor links the crate, and
  holds no clw PAT.** ⇒ push→compute fully collapses onto the **runner contract** (M7.3 +
  `docs/REQUEST-runners-TL-2026-06-16.md`) and **D-9**.
- **githugr's only contact surface = read-layer DISPLAY.** If the engine exposes clw run/workspace
  facts (cache hit, content address, portability) over the frozen `Provider` seam, githugr **surfaces**
  them on Landing / checks / PR-detail (display obligation, ADR-0001) — **read-only, never
  initiating.** Data source is the engine, not a direct githugr→clw call.
- **Seam 2 (hugit ← clw):** hugit's call — already resolved with the hugit TL
  (`docs/REQUEST-hugit-TL-2026-06-16.md`).

## clw-side note (no action now)
clw **already emits** the display facts the read-layer would surface: `clw run` reports cache
miss/hit and `clw snapshot` reports the content-address root (both observed in the 2026-06-16 live
run) — so when the engine→window display seam is built, the data already exists in clw's JSON output;
no clw change needed. Happy to co-design the display shape when the engine's CI/workspace facts firm
up.

---

## Original ask (kept for context)

> **Why you (original routing):** the hugit TL clarified that push-triggered compute is **not** a
> hugit seam; any push→compute path is **runner + engine**, not a direct hugit→clw call. The githugr
> TL has now confirmed it is **not githugr either** — it is the runner/engine lane.

## The question
Does **githugr** materialize a per-commit / per-push workspace by triggering `clw snapshot` (and
maybe a memoized `clw run`)? If yes, pin the seam **before** either side writes integration code.

## What to decide (design-first)
1. **Trigger surface** — git hook, a githugr-internal event, or a CI step? (Note: per the hugit TL,
   this rides the **runner** seam — the frozen runner contract
   `corelink-runners/docs/spec/hugit-integration-contract.md` — not a direct githugr→clw binary call
   on the forge host.)
2. **Invocation model** — **shell out to the `clw` binary** vs **link the `clw` crate**. Answer this
   **consistently with the runners TL** (same question lands there — see
   `docs/REQUEST-runners-TL-2026-06-16.md`).
3. **Auth** — githugr-triggered compute runs as which tenant, with what PAT? If on disposable compute,
   it inherits the **per-job, short-TTL, CAS+AC-scoped PAT** story = server **D-9**
   (`docs/REQUEST-server-TL-2026-06-16.md` P1). It must **never** set `x-corelink-tenant-id`
   (server-internal); PAT + tenant-in-path is the sole client-side tenant authority.

## Dependency / sequencing
```
server D-9 (per-job PAT)  ─┐
runner↔clw contract (M7.3) ─┼─►  githugr push→compute seam  ─►  family e2e (M7.5)
hugit dispatch (append-only, decided) ─┘
```

## What clw offers today (stable surfaces to design against)
`clw snapshot` (content-addressed; chunk upload) · `clw run` (memoized execution, cache-hit verified)
· `clw hydrate` (byte+mode+symlink identical — live-verified). Frozen `AcTransport`/`CasTransport`
traits; Bearer-PAT + tenant-in-path auth.

No action required for the clw GA tag — this opens the design conversation so the seam is intentional.
Ping the clw TL to co-design. Supersedes the githugr half of the 14-Jun
`docs/REQUESTS-to-other-TLs.md`.
