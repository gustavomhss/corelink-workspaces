# To: hugit / githugr TL — define the clw integration seam (design-first)

> **From:** clw / corelink-workspaces TL · **Date:** 2026-06-16 · **Relay-ready.**
> **Severity:** LOW / early — this is a **design ask, not a build ask**, and does **not** block the
> clw GA tag. Raising it now so the seam is deliberate, not accidental, before either side writes
> integration code.

No contract exists today between `clw` and hugit/githugr. Two seams are worth pinning early:

## 1. githugr → clw (push-triggered snapshot/run?)
Does **githugr** trigger a `clw snapshot` / `clw run` on push (e.g. to materialize a portable,
content-addressed workspace per commit)? If so, pin:
- **Trigger surface** — git hook, CI step, or githugr-internal event?
- **Auth** — githugr runs as which tenant, with what PAT scope/TTL? (If it runs on disposable
  compute, it inherits the **D-9 per-job PAT** story — see `docs/REQUEST-server-TL-2026-06-16.md`.)
- **Invocation** — shell out to the `clw` binary vs link the crate (same question the runners TL
  faces — worth answering consistently across both).

## 2. hugit ← clw (consume clw refs for portable workspaces?)
Does **hugit** consume `clw` refs to hydrate portable workspaces? If so, pin:
- **Ref hand-off shape** — how does hugit learn a ref name? (`ref_key = BLAKE3(domain‖name)` is
  one-way; hugit needs the human name or a handed `ref` token, not the key.)
- **Hydrate contract** — `clw hydrate <ref> <dest>` is the existing surface; confirm it covers
  hugit's needs (mode/symlink preservation, cache reuse — both already shipped + live-verified).

## What `clw` offers today (stable surfaces you can design against)
- `clw snapshot` / `clw hydrate` (byte-identical, mode + symlink preserving — live-verified) ·
  `clw run` (content-addressed memoization) · `clw ls` / `rm` / `prune` / `erase`.
- The frozen `AcTransport` / `CasTransport` traits + the Bearer-PAT + tenant-in-path auth model.

No action required for the clw GA tag — this just opens the design conversation so the seam is
intentional when it's time. Ping the clw TL to co-design either seam.

> Supersedes the hugit/githugr section of the 14-Jun `docs/REQUESTS-to-other-TLs.md`.
