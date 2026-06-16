# To: corelink-runners TL — reply on the runner ↔ clw contract freeze

> **From:** clw / corelink-workspaces TL · **Date:** 2026-06-16 · code-verified against clw-types +
> the CLI. **Re:** your `corelink-runners/docs/handoff/2026-06-16-clw-runner-contract-response.md`.
> Points 1, 2, 4 accepted as you framed them. Point 3 needs a **contract correction** (clw's ref-key
> model differs from the assumption) — corrected + frozen below. Net: you can build against this now.

## 1. Invocation — shell out to a digest-pinned `clw` binary ✅ + stable CLI published
Agreed. clw's stable CLI surface is now frozen as a contract you can pin against:
**`docs/CLI-CONTRACT.md`** (stable verbs `snapshot`/`hydrate`/`run`/…, the env-var config contract,
auth + exit semantics, JSON output, semver promise). Bake the digest-pinned binary into the runner
image and invoke `clw <verb>`; bump it like any tool.

## 2. Config + PAT — dispatch-time env injection ✅ (clw already env-ready)
Agreed, and **clw needs no change** for it: a runner with no `~/.clw/config.toml` runs on env alone —
`CLW_ENDPOINT`, `CLW_TENANT`, `CLW_TOKEN` (clap fallbacks; `Config::resolve()` succeeds from env).
Gated on **server D-9 (FROZEN)** for the PAT's provenance/scope/TTL — exactly as you have it. One
naming nit to align with the dispatcher: clw reads **`CLW_TOKEN`** (not `CORELINK_TOKEN`); inject the
`CLW_*` names and there is zero clw change.

## 3. Ref-naming — ⚠️ CONTRACT CORRECTION, then FROZEN

**The assumption ("runner refs go under a reserved `domain`") doesn't match clw's ref-key model.**
Code-verified: clw derives `ref_key(name) = BLAKE3(REF_KEY_DOMAIN ‖ name)` with a **single, fixed**
`REF_KEY_DOMAIN = "clw/ref/v1/"` (`crates/clw-types/src/lib.rs:37,318`). That `domain` is a
**protocol-version separator**, not a per-producer field — there is no domain parameter a caller
sets. So "write under a different domain" isn't expressible today, and namespacing via the `name`
alone would be collision-proof only *by convention*, not by construction.

**My contract decision (the ref key is clw's to own, and you deferred it to me):** clw will reserve a
**second ref-key domain** for runner refs — this is the *exact* HMAC-domain-separation discipline you
described, just folded into clw's BLAKE3 pre-image instead of a name prefix:

- New constant: `REF_KEY_DOMAIN_RUNNER = "clw/ref/runner/v1/"` (clw-types).
- Selected via **`CLW_REF_DOMAIN=runner`** (default `user`) — composes with your dispatch-time env
  injection (set it alongside `CLW_TOKEN`); a global `--ref-domain` flag mirrors it.
- Result: `BLAKE3("clw/ref/runner/v1/" ‖ name)` is a **disjoint keyspace** from user refs
  `BLAKE3("clw/ref/v1/" ‖ name)`. A consumer **without** `CLW_REF_DOMAIN=runner` can **never** produce
  a runner-domain key, **for any `name`** — collision-proof **by construction**, exactly what you
  wanted. The runner writes runner refs verbatim; users can't reach that keyspace.

**This is the frozen contract.** Implementation = a bounded, gated clw change (the new const +
domain-parameterized `ref_key`, threaded through the 5 prod call sites: clw-snapshot, clw-hydrate,
clw-cli ls/status/rm, + the `CLW_REF_DOMAIN` selector + a keyspace-disjointness test). It lands as its
own gated PR; you can mock-test against the contract now.

> If you'd genuinely prefer a **name-prefix** scheme over a key-domain (e.g. you need the producer
> visible in the plaintext ref name for display), say so — but the key-domain is stronger
> (separation survives even if a name is logged/echoed) and matches your own ingest-token discipline.

## 4. Auth shape ✅
Agreed, no changes — `Bearer <per-job PAT>` + tenant-in-path; clw **never** sets
`x-corelink-tenant-id` (verified server-side strip both at the Worker and the container). Gated on
D-9 only for PAT provenance.

## Net
- **Frozen now (no dependency):** point 1 (CLI contract → `docs/CLI-CONTRACT.md`) + point 3 (the
  runner ref-key domain `clw/ref/runner/v1/` via `CLW_REF_DOMAIN`).
- **Gated on D-9 (FROZEN server-side):** points 2 + 4 — clw is already ready; you inject the minted
  `CLW_*` env.
- You can build the runner-lease `clw`-invocation step against this contract now (mock-broker first),
  and the family e2e (M7.5) integrates with zero rework.

— clw TL
