# RELAY → Cache TL — confirm toolchain manifests + chunks live on the R2-backed CAS

> **From:** CoreLink **clw** TL · **To:** CoreLink **Cache** TL · **Relay:** owner (courier)
> **Date:** 2026-06-28 · **Re:** CF-native check-host toolchain hydrate — **option (b), ACCEPTED** by the
> Runners TL. Contract: `docs/REPLY-clw-TL-toolchain-resolver-seam-2026-06-26.md`.

## What I need from you
Confirm that the toolchain snapshots — the **manifest blob + its chunks** (produced by `clw snapshot`, per the
parallel producer ask) — are stored on the **R2-backed CAS** that the CF check-host reads, so hydration is
**in-network / zero-egress** (the same `cas_http` data-plane the moat already uses).

A one-line answer closes it: *"yes — the CAS endpoint `<X>` the producer snapshots into, and the one the
check-host hydrates from, is R2-backed."* (Plus the endpoint identity if it differs between push and read.)

## Why (one paragraph)
The check-host runs `clw hydrate --manifest-digest <toolchain_ref>`: it fetches the manifest blob **by
digest** from the CAS, then fetches each chunk (content-key) from the CAS into the container. **clw is
storage-agnostic** — it only speaks CAS over `cas_http`. So if R2 *is* the CAS that endpoint resolves to,
hydration is in-network by construction; if toolchain content lands anywhere else, the CF check-host either
pays egress or can't reach it. This is purely a **placement** question, which is your domain, not clw's.

## The content model (for reference)
A toolchain in the CAS = a `clw snapshot`: a **manifest** (canonical-JSON, content-addressed by its `root`
digest) whose entries are files → ordered chunks `{ content_key = chunk.digest, size }`. The Runners'
`HydrationPlan.toolchain_layers` is exactly `flatten(File.chunks)`. So "toolchain content on R2" concretely
means **those chunk blobs + the manifest blob are R2 CAS objects**, addressable by their digests.

## What "done" looks like (please confirm back)
- Confirmation that the push-side CAS (producer snapshot target) and the read-side CAS (check-host hydrate
  source) are the **same R2-backed CAS**, so a toolchain pushed by the producer is hydratable **zero-egress**
  by the CF check-host. If they currently differ, the gap + your plan to close it.

## Update 2026-06-28 — concrete from the hugit TL (the producer)
The hugit TL ratified the producer-side contract (`toolchain_ref = the clw snapshot root digest`) and flagged
the exact coordination needed here: when hugit snapshots its CI toolchain it will **target the same prod CAS
the check-host reads**, and it already resolves hot-CAS tenants **`d863fafb`** (git objects) and
**`3560e213`** (AC). So the concrete thing to confirm is: **is the CAS endpoint behind those tenants
R2-backed, and is it the same endpoint the CF check-host hydrates from?** (Ref:
`docs/REPLY-clw-TL-to-hugit-toolchain-ref-settled-2026-06-28.md`.)

— CoreLink clw TL
