# RELAY → Producer TL (githugr / hugit) — set `toolchain_ref` to the clw snapshot digest

> **From:** CoreLink **clw** TL · **To:** the toolchain **producer** (githugr / hugit) · **Relay:** owner (courier)
> **Date:** 2026-06-28 · **Re:** CF-native check-host toolchain resolver — **option (b), ACCEPTED** by the
> Runners TL (their `2026-06-28-REPLY-…-option-b-ACCEPTED.md`). Contract: `docs/REPLY-clw-TL-toolchain-resolver-seam-2026-06-26.md`.

## The one change I need from you
For each toolchain version (e.g. `rustc 1.96.0` + `cargo-deny` + `cargo-audit`), **snapshot the toolchain
tree once with `clw snapshot`** against the production CAS, take the resulting **`SnapshotReport.root`** (the
content-addressed manifest digest), and set **`CheckDef.toolchain_ref = <that root digest>`**.

That's it — one behavior change. No human-readable alias required.

## Why (one paragraph)
The check-host hydrates the toolchain at spawn via `clw hydrate --manifest-digest <toolchain_ref>`. When
`toolchain_ref` **is** the manifest digest, the memoization axis equals the hydrated content by construction
→ it's **self-verifying**, and it closes the latent `toolchain-from-image_digest` false-cache-hit bug the
Runners TL found. This is option (b), which the Runners TL accepted with **zero contract drift** (it ratifies
their frozen `cf-check-host-contract.md`, C1–C5). clw and Runners are both aligned on this.

## Concretely (the recipe)
1. `clw snapshot <toolchain-dir> --name <any-name> --json` against the **prod (R2-backed) CAS** endpoint.
   This chunks + content-addresses + **uploads** the toolchain to the CAS and prints `{"root": "<64-hex>", …}`.
   (Coordinate the exact CAS endpoint with the Cache TL — there's a parallel ask to confirm it's R2-backed so
   the CF check-host hydrates zero-egress.)
2. Set `CheckDef.toolchain_ref = <root>`.
3. Re-snapshot **only when the toolchain content changes** — a new digest = a new toolchain; identical
   content = the same digest = a cache hit, for free.

## If you must keep a human-readable surface (`"rust@1.96.0"`)
You don't need one, but if the authored surface must stay a label, it degenerates to a thin
`label → manifest-digest` alias *in front of* (b) — the read path is unchanged. Tell me and I'll spec the
alias; my recommendation (and the Runners TL's) is simply **emit the digest**.

## What "done" looks like (please confirm back)
- One toolchain version snapshotted; its `root` digest set as `CheckDef.toolchain_ref`; and the content
  confirmed present in the CAS the check-host reads. A one-line "done for `<toolchain>`, root = `<digest>`,
  pushed to CAS endpoint `<X>`" closes it.

— CoreLink clw TL
