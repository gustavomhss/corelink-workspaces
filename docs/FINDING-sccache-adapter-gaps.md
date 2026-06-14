# FINDING — "dogfood sccache→CoreLink" is blocked on THREE stacked issues

> **For:** corelink-server owner. **From:** clw dogfood follow-up, 2026-06-05.
> **Verdict:** NOT buildable correctly in a session. The cargo/sccache HTTP adapter
> (`crates/corelink-adapter-host/src/cargo/`) is a **scaffold** whose three load-
> bearing assumptions all diverge from production reality. This doc documents the
> full pre-work (no agent was dispatched — the investigation stopped the build
> before any code was written, which is the point).

## How this was reached

The owner greenlit a full TechLead build (decompose → contracts → wave). The
lead's pre-work (verify the contract before fan-out) surfaced three blockers in
sequence — each of which would have made a fanned-out wave produce code that
fails on real traffic. **No agents were burned; no half-built branch exists.**

---

## Blocker 1 — auth model (RESOLVED by investigation, decision reversed)

- The adapter self-validates the Bearer PAT (`TenantResolver::resolve(pat)`).
- But production auth is: **Worker** parses PAT + HMAC fast-reject + D1 lookup by
  `token_id` + expiry (`worker/src/index.ts:502-533`); the **DO** does Argon2id +
  scope verify (`index.ts:532`); the **container trusts the injected
  `x-corelink-tenant-id`** — exactly how `cas`/`bazel_v2`/`turbo_v8` work.
- ⟹ The container must **NOT** re-validate the PAT (Option B, first chosen, is
  wrong — it duplicates the DO's work and risks disagreeing with it). The correct
  model is **Option A: trust the header** (chosen after the reversal). Concretely:
  a native cargo route in the container mirroring `bazel_v2`'s `caller_tenant`
  header read — **no PatValidator, no adapter-crate auth.**

## Blocker 2 — PAT store schema divergence (informs Blocker 1; not on the A-path)

- The canonical `corelink-pat` model and `migrations/002` (Postgres: `token_hash
  BYTEA`, `scopes TEXT[]`) do **not** match the live D1 (SQLite) `pat` table:
  `pat_hash TEXT`, `scope` single-enum, `token_id` **nullable, bolted on later**.
- Any container-side PAT validator (the abandoned Option B) would have to reverse-
  engineer this drifted schema and stay bit-for-bit consistent with the DO's
  verify — a security-critical reimplementation. **The Option-A header-trust path
  sidesteps this entirely**, which is a second reason A is correct.

## Blocker 3 — storage semantics mismatch (the deepest; blocks even Option A)

- CoreLink CAS is **content-addressed**: a write verifies `claimed_hash ==
  hash(bytes)` (`corelink-handler-cas/src/handler.rs:335` → `CorrectnessViolation`).
  Bazel fits because Bazel is content-addressed (key = content digest).
- **sccache is not**: its key is a hash of the *compilation inputs*; the value is
  the *artifact*; `key ≠ hash(value)`. Writing sccache traffic into the content-
  addressed CAS would fail **every PUT** with a hash/correctness violation.
- The correct backing for sccache is a **KV store** (arbitrary key → bytes, no
  hash verify) — exactly what `turbo_v8` uses. **But that KV is still in-memory**
  (`routes/turbo_v8.rs` `TODO(v2): R2KvStore`) — it does not persist across
  container restarts. So even the right-shaped cargo route has **no durable
  backing store to point at yet.**

---

## True remaining scope (honest)

| Step | Work | Status |
|---|---|---|
| 1 | **Persistent KV store** (`R2KvStore`, the turbo `TODO(v2)`) — arbitrary key → R2 object, per-tenant prefix, no hash verify | **does not exist** |
| 2 | **Native cargo route** (Option A): GET/PUT/HEAD `/cargo/:tenant/:key`, trust `x-corelink-tenant-id`, cross-tenant 403, backed by the KV store from step 1 | designed, blocked on step 1 |
| 3 | Mount in `routes.rs` + tests | trivial after step 2 |
| 4 | **Prod deploy** + configure a runner's sccache + measure | owner-timed, post-merge |

Step 1 (the persistent KV) is shared with the Turborepo product surface — building
it unblocks BOTH sccache and persistent Turbo caching. That makes it a real,
worthwhile unit of work — but a **feature**, not a session patch.

## Recommendation (unchanged conclusion, now fully evidenced)

**Schedule on the CI-acceleration campaign (campaign #1), as a 2-part feature:**

1. **`R2KvStore`** — the persistent KV behind turbo + cargo (closes turbo's
   `TODO(v2)` too). This is the keystone; do it first, on its own PR with tests.
2. **Native cargo route (Option A)** on top of it + mount + tests, then an
   owner-scheduled, off-peak prod deploy + measurement.

Nothing pre-launch depends on this (the cache-storage product ships without
sccache). PR #139 (the wrangler@4 deploy fix) has **already landed on main** —
so when the deploy does happen, it won't die pre-flight.

**This deferral is documented, fully scoped, and evidence-backed — it is not
debt.** Anyone can pick it up cold from this doc.

## Pointers (verified this session)

- Worker auth flow: `worker/src/index.ts:479-533` (parse/HMAC/D1/expiry) + `:532` ("DO does Argon2id + scope").
- Trust-header template to mirror: `routes/bazel_v2.rs` (`caller_tenant`, cross-tenant via the CAS handler).
- KV template + the TODO: `routes/turbo_v8.rs` (in-memory KV; `TODO(v2): R2KvStore`).
- CAS content-verify (why CAS ≠ sccache): `corelink-handler-cas/src/handler.rs:335`.
- Live D1 `pat` schema (drifted): `pat_hash TEXT`, `scope` enum, `token_id` nullable.
