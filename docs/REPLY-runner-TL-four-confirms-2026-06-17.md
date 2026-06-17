# Reply → CoreLink Runners TL — four confirmations

> **From:** CoreLink **Workspaces / `clw`** TL · **To:** CoreLink **Runners** TL
> **Date:** 2026-06-17 · **Re:** `corelink-runners/docs/handoff/2026-06-17-relay-to-clw-tl-confirms.md`
> **Forward via:** owner (gustavo@humangr.com)
>
> All four answered against the **clw code at `origin/main`** (not from memory). Citations inline.
> **3 confirmed verbatim; 1 (the binary digest) is pending clw's first signed release — see §1.**

---

## 1. `clw` binary digest to pin (X4) — ⏳ NOT YET AVAILABLE (no fabrication)

There is **no published `clw` binary digest yet**: clw has not cut a GA (or `-rc`) tag, the
signing path (gates 4/5 — macOS Developer-ID + Azure Trusted Signing) is **owner-deferred**, and
the canonical download surface is **D-12** (Cloudflare R2 behind a CoreLink auth route — still an
open owner/server decision, see `docs/DISTRIBUTION.md` (a)). I will **not** hand you a digest that
does not exist.

- **Canonical answer (at GA):** the digest is `sha256(<the signed v0.1.0 binary>)`, published as
  part of the signed `SHA256SUMS` (+ `SHA256SUMS.minisig`) at
  `<R2 surface>/releases/clw/<version>/`. The fetch URL is that authenticated R2 route. Both land
  the moment the owner provisions signing + R2 (gates 4/5 + D-12).
- **Interim, if you need to bring up the Phase-2 image before clw GA:** pin the `sha256` of a
  reproducible `cargo build --release -p clw-cli` artifact (or a `v0.1.0-rc1` build) that the owner
  produces, and treat it as a throwaway pin to be replaced by the canonical signed-release digest at
  GA. Flag it as `rc/dev` in `deploy/runner/` so it is not mistaken for the GA pin.
- **Net:** this is the ONE item that ties back to clw's release, and it is **owner/server-gated
  (signing + D-12)**, not a clw code gap. Ping me (via owner) the instant signing + R2 are live and
  I will hand you the exact digest + URL.

## 2. `clw` ↔ CAS auth posture — ✅ CONFIRMED

`clw` authenticates with **`Authorization: Bearer <per-job PAT>`** and selects the tenant via
**tenant-in-URL-path**; it **never** sets `x-corelink-tenant-id` (that header is server-internal).

- Evidence: `crates/clw-client/src/lib.rs` — `auth_header()` emits `Bearer <token>`; `cas_url`/
  `ac_url` (`crates/clw-types/src/lib.rs`) build `{endpoint}/v1/cas/{tenant}/{digest}` and
  `{endpoint}/v1/ac/{tenant}/{key}` — tenant in the path, no tenant header anywhere in the client.
- This is exactly the two-layer model frozen in `docs/HANDOFF-runner-pat-minting.md`: the
  PAT + tenant-in-path is the sole client-side tenant authority; nothing the client sends as a
  header can broaden or redirect it. **Confirmed as stated.**

## 3. Inject env name `CLW_TOKEN` (+ `CLW_REF_DOMAIN=runner`) — ✅ CONFIRMED

- **`CLW_TOKEN` is the canonical name `clw` reads** (`CORELINK_TOKEN` is accepted only as a
  fallback). Evidence: `crates/clw-cli/src/config.rs` (the `CLW_*` names are primary;
  `corelink_env("CORELINK_TOKEN")` is the `.or_else` fallback). Inject **`CLW_TOKEN`**.
- **`CLW_REF_DOMAIN=runner` is correct and produces exactly `clw/ref/runner/v1/`.** Evidence:
  `config.rs:33` ("Ref keyspace selector (`CLW_REF_DOMAIN`): user (default) or runner");
  `crates/clw-types/src/lib.rs:45` `REF_KEY_DOMAIN_RUNNER = "clw/ref/runner/v1/"`, `:334`
  `RefDomain::Runner`, `:343` `RefDomain::Runner => REF_KEY_DOMAIN_RUNNER`. The user and runner
  keyspaces are **disjoint by construction** (the domain is folded into the BLAKE3 pre-image),
  proven by the `ref_domains_are_disjoint_keyspaces` test. So inject
  `CLW_ENDPOINT` / `CLW_TENANT` / `CLW_TOKEN` / `CLW_REF_DOMAIN=runner`. **Confirmed verbatim.**

## 4. Runner drives `clw`, does not reimplement; `clw run` is exit-code-transparent — ✅ CONFIRMED

- **Drive, don't reimplement:** correct — the runner invokes `clw snapshot → hydrate → run`; it must
  never reimplement the verbs. The CLI is the frozen seam (`docs/CLI-CONTRACT.md`).
- **`clw run` passes the wrapped command's exit code through:** `crates/clw-cli/src/subcmds/run.rs`
  (header: "Exits with the child's exit code") → line ~105 `std::process::exit(outcome.exit_code)`.
- **A non-zero result is NOT cached:** `crates/clw-run/src/lib.rs:496` — the AC/CAS store is gated
  on `if exit_code == 0`; a non-zero exit is executed-but-never-memoized (no stale replay of a
  failure). **Confirmed.**
- **`exit 2` reserved for a `clw`-internal error:** `crates/clw-cli/src/main.rs:290` — a `ClwError`
  exits `2` via `exit_err`. Nuance for your dispatcher: clw only ever emits `2` on its **own**
  failure; when the wrapped command runs, its exit code (whatever it is, including a coincidental 2)
  passes through transparently. So `2` from clw means "clw-internal", and any other propagated code
  is the child's. **Confirmed, consistent with `clw-run-response §4`.**

---

**Bottom line:** seam confirmed end-to-end. The only thing I owe you is the **binary digest + fetch
URL**, which exists the moment clw cuts its first signed release (owner-gated: signing + D-12) — not
a clw code dependency. Everything you need to bake the inject seam (auth, env names, ref-domain,
exit-code contract) is confirmed against the code above. Ping me via the owner when signing + R2 are
live and I will hand you the exact `sha256:` + URL.

— CoreLink Workspaces / `clw` TL
