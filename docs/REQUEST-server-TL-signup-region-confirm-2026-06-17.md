# Request → CoreLink Server TL — confirm the signup-worker region/residency fix (gate 8)

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Server / signup-worker** TL
> **Date:** 2026-06-17 · **Send via:** owner (gustavo@humangr.com)
> **Why:** clw go-live **exit-gate 8** (region/residency correct + real right-to-erasure). The erasure
> half is DONE and live (`clw erase`, conformance-green). The **residency/region half is server-side**
> and clw cannot verify it from the client; this is the one confirmation that closes gate 8.

## What clw's published docs assert (and therefore depends on you to back)
- **Privacy Policy / DPA:** Customer Content is stored in a defined region/residency posture; EU/UK
  data handling follows the stated transfer mechanism. clw is a pure client — it does not choose or
  enforce region; the **signup worker + backing infra** do.
- So if the signup-worker region routing is wrong (e.g. a tenant provisioned in the wrong colo, or
  residency not enforced), clw's residency claims would be **unbacked** — a gate-8 failure.

## Please confirm (one reply closes the gate)
1. **The signup-worker region fix landed in prod** — tenants are provisioned/stored in the correct
   region per the residency posture, and a non-NA colo (e.g. LHR) resolves to the intended primary
   region (the launch checklist notes a `wrangler d1 execute` check: `primary_region='weur'` for an
   EU colo — confirm the live value).
2. **`x-corelink-tenant-id` remains server-internal** — a client setting it cannot escape or redirect
   the PAT's tenant/region binding (the two-layer model in `docs/HANDOFF-runner-pat-minting.md`).
3. **Residency matches the docs** — what clw's Privacy/DPA claim about where Customer Content lives is
   what the infra actually does, or tell me the correct posture so I can reconcile the docs.

A short confirmation (or a correction I can fold into the legal docs) is all that's needed. Reply via
the owner.

— CoreLink Workspaces / `clw` TL
