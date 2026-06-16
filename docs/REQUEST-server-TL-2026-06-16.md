# To: corelink-server TL (+ CoreLink-API / Clerk owners) — open asks from the clw TL

> **From:** clw / corelink-workspaces TL · **Date:** 2026-06-16 · evidence-verified this session.
> **Relay-ready:** hand this whole file to the server TL. It lists only what is still open —
> everything from the 14-Jun thread that has since shipped is recorded as CLOSED so nothing is
> re-asked.

Hey — quick status sync before the asks, so you can trust this is current and not a re-hash.

## ✅ Closed since 2026-06-14 (no action — recorded so we don't re-ask)

| Item | Now | Proof |
|---|---|---|
| **D-1** ref delete (idempotent) | **LIVE** | #269 deployed; `ref_delete_roundtrip` green; live `rm` idempotent (this session) |
| **D-7** ref enumeration (paginated) | **LIVE** | `ref_list_roundtrip` green; live `ls --all` over real records |
| **D-8** CAS delete + list | **LIVE** | `cas_delete_roundtrip` + `cas_list_roundtrip` green; live `prune` fail-safe + `erase` verified-empty |
| **CONFIRM-a** PAT grants CAS+AC | **RATIFIED + proven** | round-2 cred `scope: read-write`; D-8 erasure passed on it (no admin fallback) |
| **CONFIRM-b** region routing | **Resolved → (c)** | single global host region-routes internally; clw needs no change |
| **D-3 / signup-region** fix | **LIVE** | residency #29 shipped inside #269 |
| **D-10** primary_region API | **SUPERSEDED** | CONFIRM-b resolved to (c) — not needed |

> Net: the whole `rm` / `prune` / `ls --all` / **GDPR Art.17 erasure** milestone is live-verified.
> The client carries **zero** open server dependency for the shipped commands. Thank you — that
> unblocked the entire destructive-ops + compliance surface.

---

## P1 — Freeze the **D-9 runner per-job PAT minting** contract  ⟵ highest leverage
**Blocks:** family e2e (M7.5) + the runner↔clw contract freeze. **Full spec:** `docs/HANDOFF-runner-pat-minting.md`.

**Status update since that handoff was written:** its former **#1 blocker — "can a PAT carry CAS+AC?"
— is now ANSWERED** (the round-2 cred proved unified `read-write` = CAS+AC). What remains to pin for
a disposable runner:

1. **Who mints** the per-job token — CoreLink API (`POST /v1/tokens` exchanging a runner identity),
   Clerk machine token, or the dispatcher? Name the **bootstrap credential** the runner presents.
2. **TTL + mid-job expiry** — job-bounded lifetime, hard cap, and whether mid-job expiry means
   re-mint or fail.
3. **Trigger / injection** — dispatch-time env injection vs runner self-fetch. Must reconcile with
   the H2 `$HOME`-config hardening (a disposable runner has **no** `~/.clw/config.toml`).
4. **Revocation** — pre-TTL revoke + teardown invalidation semantics.
5. **`x-corelink-tenant-id` is server-internal** — confirm the server rejects/ignores it when set by
   a client, so PAT + tenant-in-path is the sole client-side tenant authority.

> Freeze this **before** any runner-side (M7.3) or family-e2e (M7.5) work starts.

## P2 — **D-12** authenticated binary-download route  ⟵ needs the OWNER host-decision first
The download surface (R2-behind-CoreLink vs GitHub-SSO vs vendor) is an **owner M3.3 decision**, not
yet made. **No server action yet** — clw will relay the concrete D-12 contract (an authenticated
`GET releases/clw/<version>/…`) the moment the owner picks "R2-behind-CoreLink". Flagged so it is on
your radar; please **do not build until the host decision lands**.

## P3 — Campaign #1 (CI acceleration / sccache)  ⟵ scheduled, NOT launch-blocking
Specs: `docs/FINDING-sccache-adapter-gaps.md` + `docs/FINDING-per-tenant-write-ceiling.md`.
1. **D-6a first** — reconcile `wrangler.toml max_instances=5` vs the live value `20` (config drift).
2. **D-4** `R2KvStore` (persistent KV, per-tenant prefix, no hash-verify) → **D-5** native cargo route
   (Option-A header-trust, mirror `bazel_v2`) → **D-6** per-tenant write scaling (shard-DO **B** or
   bypass-DO **C** — one is required for the campaign to launch credibly).

`clw`/sccache is **docs-only** here (sccache speaks its own protocol to the route; clw is not in that
data path). Nothing pre-launch depends on this.

## P4 — Hygiene / low priority
- **D-2** — back up `SIGNUP_TOKEN_KEY` (write-only deployed secret; unrecoverable if lost).
- **D-11** — distinct CAS-PUT status (created vs already-existed) → faithful `chunks_uploaded`
  accounting (best-effort without it). Optional.
- **`clw auth rotate` endpoint contract** — the one shipped client stub (ships, advises re-login,
  `"rotated": false`). A scoped mint-new + revoke-old endpoint contract closes it. Low.

---

## What `clw` does the moment each lands

| You ratify / decide | clw immediately does |
|---|---|
| **D-9** minting contract | builds the runner-auth path + the runner↔clw seam → unblocks family e2e (M7.5) |
| owner picks R2-behind-CoreLink → **D-12** | relays the auth'd-download contract; wires the `release.yml` publish step to it |
| **D-6a/D-4/D-5/D-6** | docs-only on clw's side; the sccache route ships independently |
| **auth-rotate** endpoint | replaces the `auth rotate` re-login stub with a real one-step rotate |

> Cross-refs: `docs/HANDOFF-corelink-server-track.md`, `docs/HANDOFF-ref-enumeration.md`,
> `docs/HANDOFF-cas-delete-list.md`. Supersedes the server section of the 14-Jun
> `docs/REQUESTS-to-other-TLs.md`.
