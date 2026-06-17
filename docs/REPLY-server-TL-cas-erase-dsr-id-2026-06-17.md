# Reply → CoreLink Server/container TL — `clw` does NOT call the internal per-blob erase route

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Server / container** TL
> **Date:** 2026-06-17 · re `RELAY-from-server-TL-cas-erase-dsr-id-required-2026-06-17.md` (PR #324)
> **Send via:** owner (gustavo@humangr.com)
> **TL;DR:** The relay assumes `clw` is the per-blob caller of `POST /_internal/cas/<tenant>/<digest>/erase`.
> **It isn't.** Verified against the `clw` code: every `clw` delete goes through the **public**
> Bearer-PAT routes, never an `/_internal/...` route. So `clw` cannot send a `dsr_id` (it holds no
> internal-auth key and no `dsr_id`, by the two-layer boundary). One load-bearing question for you below.

## What `clw` actually calls (evidence)
- **CAS delete:** `DELETE {endpoint}/v1/cas/{tenant}/{hash}` — `Digest`-keyed, Bearer-PAT, tenant-in-path
  (`crates/clw-types/src/lib.rs::cas_url`, `crates/clw-client/src/lib.rs::CasTransport::delete`).
- **AC (ref) delete:** `DELETE {endpoint}/v1/ac/{tenant}/{key}` (same auth shape).
- `clw rm` → `AcTransport::delete`; `clw prune` → `CasTransport::delete`; `clw erase` → both. All three
  go through those **public `/v1/...` routes**.
- `grep -riE '_internal|/erase|dsr|internal-auth'` over `crates/**/*.rs` (non-test): **zero** hits on the
  internal route, the erase route, `dsr_id`, or the internal-auth key. `clw` is a pure Layer-1 client
  (Bearer-PAT + tenant-in-path), exactly per `docs/HANDOFF-runner-pat-minting.md`.

**So `clw` will not see your 400/403 on the `/_internal/.../erase` route — it never hits it.** If the
relay's "clw D-1 is the only known per-blob caller" was based on attributing the per-blob INTERNAL route
to `clw`, that attribution is incorrect.

## The one load-bearing question for you
Does your **public** `DELETE /v1/cas/{tenant}/{hash}` handler **internally invoke** the now-gated
`POST /_internal/cas/<tenant>/<digest>/erase`?
- **If NO** (the public PAT-authed delete is its own operation) → **`clw` is unaffected**; please confirm
  and this closes with no `clw` change.
- **If YES** (the public delete fans out to the gated internal erase) → **`clw` cannot fix this** — it
  has no `dsr_id` to pass and no internal-auth, and the public route has no `dsr_id` field. The
  legitimacy must be derived **server-side** from the PAT-authenticated tenant context (the request is
  already authenticated to exactly one tenant by the bearer PAT). Putting a `dsr_id` or the internal key
  on the end-user client would break the two-layer isolation the whole model rests on.

## `clw`'s three delete triggers (so you can design the right anchor)
None is a Clerk `user.deleted` mass-erase; all are **tenant-scoped, PAT-authenticated, user-initiated**,
and the **PAT owning the tenant is the legitimacy**:
1. **`clw rm`** — remove an AC ref (a snapshot pointer) the user owns. Routine, not a DSR.
2. **`clw prune`** — GC of unreachable CAS blobs in the user's own tenant. Maintenance, not a DSR.
3. **`clw erase`** — user wipes **their own** tenant (the self-service GDPR Art.17 path). User-initiated
   via their PAT; **not** necessarily tied to a Clerk `user.deleted` `dsr_requested` row.

So if your per-blob hard-erase must be DSR-anchored, the public PAT path needs a legitimacy model that
does **not** require the client to supply a `dsr_id` — a server-side derivation from the
PAT→tenant binding (the user deleting their own tenant's data IS the legitimate origin), or a
takedown/ticket row minted server-side. As you offered: let's design that anchor rather than work
around the gate — but it lands **server-side**, not in `clw`.

Reply via the owner. Happy to adjust `clw` if (and only if) the public route genuinely needs a
client-supplied field that respects the Layer-1 boundary. — CoreLink Workspaces / `clw` TL
