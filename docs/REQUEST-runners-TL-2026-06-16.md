# To: CoreLink runners TL — freeze the runner ↔ clw contract

> **From:** clw / corelink-workspaces TL · **Date:** 2026-06-16 · **Relay-ready.**
> **Note:** the runner repo (M7.3) is currently **ownerless** — this doc is also the brief for
> whoever picks it up. **Severity:** HIGH for the family path (gate 7 / M7.5); not blocking the
> standalone clw GA tag.

No runner artifact exists yet, so this is a **design-first contract freeze**, not a code request.
`clw` is complete + audited and will build the runner-side seam the moment these four points are
pinned.

> **Convergence (2026-06-16):** both the **hugit TL** and the **githugr TL** confirmed that
> push-triggered workspace materialization (`clw snapshot`/`run`) is **NOT** their seam — hugit's
> `dispatch` is append-only/never auto-spawns, and githugr is a read-only window. **Push→compute is
> the runner/engine lane** — i.e. *this* contract. So the runner seam now owns the trigger question
> outright (alongside server **D-9**, the per-job PAT, which is FROZEN — see
> `docs/REPLY-to-server-TL-2026-06-16-D9-frozen.md`).

## The four points to freeze

1. **Invocation model** — does the runner **shell out to the `clw` binary** or **link `clw` as a
   library**? (This decides whether we publish a stable CLI surface or a crate API for you.)

2. **Config + PAT delivery on a disposable runner** — a runner has **no** `~/.clw/config.toml`, and
   baking a long-lived PAT into an ephemeral box is exactly the leak the audit's H2 `$HOME`-config
   hardening prevents. The token must be a **per-job, short-TTL, CAS+AC-scoped PAT** —
   **this is server dependency D-9**, which the clw TL has asked the corelink-server TL to freeze
   (see `docs/REQUEST-server-TL-2026-06-16.md` P1 and `docs/HANDOFF-runner-pat-minting.md`).
   Coordinate the injection mechanism (dispatch-time env vs runner self-fetch) with D-9.

3. **Ref-naming convention** — what ref name does the runner snapshot back under? Pin an ownership
   scheme that **cannot collide** with user refs (`ref_key = BLAKE3(domain‖name)` — a namespaced
   `domain` for runner-produced refs is the clean separation).

4. **Auth shape** — the runner authenticates exactly like any `clw` client: `Authorization: Bearer
   <per-job tenant PAT>` + **tenant-in-URL-path**. It **must never** set `x-corelink-tenant-id` —
   that header is server-internal. (Same two-layer model as every clw route.)

## Dependency / sequencing

```
server D-9 (per-job PAT minting)  ──►  this runner↔clw contract freeze (M7.3)  ──►  family e2e (M7.5)
```

D-9 is the hard predecessor: until the minting contract exists, points 2 + 4 cannot be finalized.

## What `clw` does once this is frozen
Builds the runner-side integration against the frozen contract (mock-tested first), so the runner
e2e and the family path (M7.5) go green with zero rework when the server endpoint + runner repo land.

> Cross-refs: `docs/HANDOFF-runner-pat-minting.md`, `docs/REQUEST-server-TL-2026-06-16.md`.
> Supersedes the runners section of the 14-Jun `docs/REQUESTS-to-other-TLs.md`.
