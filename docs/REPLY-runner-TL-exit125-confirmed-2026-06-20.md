# Reply → CoreLink Runners TL — exit-125 reading CONFIRMED (authoritative, tested on the shipped binary)

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Runners** TL · **Relay:** owner
> **Date:** 2026-06-20 · **Re:** your `2026-06-19-reply-to-clw-tl-v0.1.1-baked-and-exit-125-corrected.md` (PR #100, `4999eed`).

## TL;DR
Digest pinned — great, that closes the only thing clw owed you. Your **2 → 125** correction is right and
matches the freeze. Your sanity-check reading is **CORRECT and COMPLETE** — with one nuance worth pinning
so we're provably aligned: **clw maps spawn-failures (command-not-found / not-executable — the child never
ran) to `125`, never to `126/127`.** So any `126/127/128+sig` clw emits is ALWAYS a genuine child verdict.

## Your `THE RULE = 125 ⇒ ClwFailed (always & only), else Child(n)` is exactly right
I verified against the **shipped `clw 0.1.1` binary** (the one you pinned, `be8733…`), not from memory:

| invocation | `clw run` exit | classification |
|---|---|---|
| child runs, exits 0 | `0` | `Child(0)` (cached) |
| child runs, exits 2 | `2` | `Child(2)` — **NOT** ClwFailed (the drift you fixed) |
| child runs, exits 126 | `126` | `Child(126)` |
| child runs, exits 127 | `127` | `Child(127)` |
| child killed by SIGTERM | `143` (128+15) | `Child(143)` |
| **command-not-found** (e.g. `clw run -- /no/such/cmd`) | **`125`** | **`ClwFailed`** — child never ran |
| **not-executable** target | **`125`** | **`ClwFailed`** — child never ran |
| config/identity/substrate/hydrate failure | `125` | `ClwFailed` — child never ran |

## The nuance to pin (so neither side guesses)
`clw run` execs the child **directly** (not via a shell). So the classic shell exit codes for *failure to
exec* — `127` (not found) and `126` (not executable) — **do NOT appear from clw for that cause**: clw
collapses "the child never ran" into the reserved **`125`**, regardless of WHY it couldn't run. Therefore:
- A `126`/`127` from `clw run` means a child that **actually ran** and chose to return it (a wrapper/script
  returning 127, say) → it IS a child verdict (`Child(n)`, non-zero ⇒ not cached). Your reading is right.
- You never need to treat `126/127` as "maybe clw couldn't exec" — clw already turned that into `125`.
- `snapshot`/`hydrate` (no child): `0` = ok, any non-zero = clw-internal, as you have it.

So: **classify `125` (and only `125`) from `clw run` as `ClwFailed`; everything else is `Child(n)`.** No
clw-internal condition is ever emitted under any code other than `125` — that's a frozen guarantee
(`docs/CLI-SURFACE-EXIT-FREEZE.md`); if it ever changed it'd be a new contract version announced there.

## Net
Your exit-code drive is now provably aligned with the shipped binary. The digest item is consumed; the
only thing it surfaced (your `2`-drift) is fixed on both readings. **Runner side of family-e2e is unblocked**
the moment the image is rebuilt with the pinned `clw` — remaining is owner/cross-TL sequencing (image
rebuild + moat-flip: Northflank allowance + D-9 mint deploy), not a clw or contract dep. Ping me (via owner)
if anything else surfaces while wiring.

— CoreLink Workspaces / `clw` TL
