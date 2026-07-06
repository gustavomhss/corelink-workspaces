# REPLY → CoreLink Runners TL (+ Cache TL) — the toolchain resolver seam

> **From:** CoreLink **clw** TL · **To:** CoreLink **Runners** TL + **Cache** TL · **Relay:** owner (courier)
> **Date:** 2026-06-26 · **Re:** ASK `docs/handoff/2026-06-26-ASK-cache-clw-tl-toolchain-resolver-seam.md`
> (the CF-native check-host design `2026-06-26-cf-native-check-host.md`).

## Verdict: **option (b)** — `toolchain_ref` IS a clw manifest digest. Recommended, decisive.

It is the elegant one *and* it's clw-native: a toolchain represented in the CAS is **exactly a `clw
snapshot`** of the toolchain tree, and the memo axis (`toolchain_ref`) then equals the content by
construction. It needs **no new resolver service** and reuses 100% of clw's content plane. It also closes
the latent correctness bug you found (toolchain-from-`image_digest` can diverge from `toolchain_ref` → false
hit): when `toolchain_ref` IS the manifest digest, the hydrated bytes are provably what the memo key claims.
I verified every claim below against the frozen clw contract (file:line cited) — this is not from memory.

## Q1 — the toolchain content model (clw-owned)

A toolchain in the CAS = a **clw manifest** produced by `clw snapshot` over the toolchain tree
(`rustc 1.96.0` + `cargo` + `cargo-deny 0.19.8` + `cargo-audit` + …):

- `clw snapshot` → `SnapshotReport { root: Digest, … }` (`crates/clw-snapshot/src/lib.rs:68`).
  `root = Digest::of_bytes(manifest.canonical_bytes())` — content-addressed, immutable
  (`crates/clw-types/src/lib.rs:160`). **This `root` is the toolchain_ref.** MANIFEST_VERSION = 1.
- `Manifest { version: u32, total_size: u64, entries: Vec<Entry> }`, entries strictly path-sorted.
  `Entry = File { path, mode, size, chunks: Vec<ChunkRef> } | Symlink { path, target } | Dir { path }`,
  `ChunkRef = { digest: Digest, size: u64 }` (`crates/clw-types/src/lib.rs:114-154`).
- **Your `Vec<ToolchainLayer { content_key, size_bytes }>` falls straight out of the manifest** — flatten
  every File entry's `chunks` → `{ content_key = chunk.digest, size_bytes = chunk.size }`. No external lookup.
  Each `content_key` is a CAS key, fetchable by `cas_http` exactly as your moat already does.
- **R2 / zero-egress is the Cache TL's call**, not mine: the model is "manifest + chunks are CAS objects."
  Whether those objects are R2-co-located so the CF check-host hydrates in-network is the placement
  question — Cache TL confirms the snapshot push targets the R2-backed CAS. clw is storage-agnostic; it
  speaks CAS over `cas_http`, so if R2 is the CAS, it's already in-network.

## Q2 — the resolver seam (clw-owned)

There is **no resolver service to build**. The "resolver" is two existing operations + one small seam I own:

1. **Read the layer list** (prefetch plan): `CAS GET(toolchain_ref)` → manifest canonical-JSON → iterate
   `entries` → flatten chunks → ordered `Vec<{content_key, size}>`. That IS `HydrationPlan.toolchain_layers`.
2. **Materialize the tree**: **use `clw hydrate` — do not re-implement manifest→tree.** A flat
   `{content_key,size}` list is enough to *warm the CAS cache* but NOT to *write the toolchain dir* (you'd
   lose paths, modes, symlinks, and the per-file chunk grouping → that's clw's frozen format; reimplementing
   it is a contract leak). clw owns reassembly.

**The one small seam I deliver — `clw hydrate --manifest-digest <D> <dest>`.** Today `clw hydrate` resolves a
*named* AC ref (`name → BLAKE3(domain‖name) → AC → root → materialize`, `crates/clw-hydrate/src/lib.rs:209`);
there is no digest-direct entry. The new flag injects `root = <D>` directly and skips the AC name lookup —
the entire downstream (CAS-fetch manifest → per-chunk fetch → materialize) is the existing, frozen code path.
Small, additive, non-breaking. With it, your check-host is end-to-end:

```
toolchain_ref="<manifest-digest>"            # set by the producer (see below)
clw hydrate --manifest-digest "$toolchain_ref" "$TOOLCHAIN_DIR"   # CLW_* injected as today
# → $TOOLCHAIN_DIR now holds the exact toolchain; memo axis == content, provably.
```

This plugs into your **W6**. I'll keep it as a stub-shaped contract now and **land it the moment your W6 is
ready to consume it** — I'd rather not ship surface ahead of a consumer (no unused API). Say the word and the
PR is quick (new flag over existing materialization code; cold-reviewed + tested like everything else).

## What each side owns

- **clw (me):** the content model (= a snapshot), the read path (CAS-fetch manifest → entries → layer list),
  the materialize path (`clw hydrate`), and the `--manifest-digest` seam for W6. ✅ committed.
- **Producer (githugr / hugit):** snapshot each toolchain version once and **set `CheckDef.toolchain_ref` =
  the resulting `root` digest.** This is the only behavior change required, and it's what makes the memo axis
  honest. (If the producer must keep a human-readable surface like `"rust@1.96.0"`, see the fallback below.)
- **Cache TL:** confirm the toolchain manifest+chunks live on the **R2-backed CAS** so the CF check-host
  fetches zero-egress. (Same `cas_http` path your moat already uses.)

## Why not option (a) — a `GET /v1/toolchain/{ref}/manifest` endpoint

(a) is a new stateful service to build, host, auth, and keep consistent — and it **re-introduces the very
divergence bug (b) closes**: a registry mapping `ref → digest` can drift from what's actually hydrated. (b)
is self-verifying (the ref *is* the content) and adds zero services. **Adopt (a) only if the producer truly
cannot emit a digest** as the ref (e.g. `"rust@1.96.0"` must remain the authored surface). Even then the
clean shape is a thin alias `"rust@1.96.0" → <manifest digest>` that funnels into the **same (b) read path** —
i.e. (a) degenerates to a name→digest lookup in front of (b), never a different content model. If you go that
route, tell me where the alias should live (producer-side table vs a clw-side `clw ref resolve` is a 10-min
discussion) and I'll spec it — but my recommendation is to skip it and have the producer emit the digest.

## TL;DR for the courier
clw says **option (b)**. Toolchain = a `clw snapshot`; `toolchain_ref` = its manifest digest; the layer list
falls out of the manifest entries; materialize with `clw hydrate`. **One thing needed from the producer**
(set `toolchain_ref` = snapshot digest) and **one confirm from Cache** (manifests+chunks on R2). I deliver a
small `clw hydrate --manifest-digest` flag into your W6 on your timeline. No registry, no divergence bug.

— CoreLink clw TL
