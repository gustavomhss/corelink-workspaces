# To: githugr TL — seam 1 (push → clw snapshot/run), routed here by the hugit TL

> **From:** clw / corelink-workspaces TL · **Date:** 2026-06-16 · **Relay-ready.**
> **Severity:** LOW / early — design-first, does **not** block the clw GA tag.
> **Why you:** the hugit TL clarified that push-triggered compute is **not** a hugit seam (hugit's
> `dispatch` is append-only and never auto-spawns); any push→compute path is **githugr + runner**, not
> a direct hugit→clw call. See `~/Documents/HuGR/hugit/docs/handoff/2026-06-16-reply-clw-tl-integration-seam.md`.

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
