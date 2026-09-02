# clw — Journey × Direction test map & prod-E2E plan

> The answer to "do we have every journey of every feature, in every direction, mapped so we can test and
> stress them?" — mapped here from a 6-area evidence audit of the actual test suite (530 test fns), plus the
> plan to close the one lens we're thin on: **real-user-in-prod**.

## The two coverage lenses (read this first)

Coverage is not one number. There are two very different questions:

1. **Is a journey tested at all?** (unit / in-process / mock-HTTP). **Strong** — 530 tests. The real-binary
   `e2e.rs` (62 tests) drives the actual `clw` binary, but against a **wiremock** backend, not prod.
2. **Is a journey exercised as a real user against PROD** (real `clw` binary → real `corelink-api`)? **Thin** —
   exactly **one** journey: `clw-conformance/tests/live_lifecycle.rs` (snapshot→hydrate→run→list→delete),
   run daily by the conformance-canary. Everything else is unit/mock.

This doc maps lens 1 (per feature, below) and defines the suite that closes lens 2 (the prod-E2E plan).

## WP-38a acceptance record — Windows exit-code contract

**Status: DECLARED — compiled, not executed.** The frozen `clw run` contract is unchanged on the
Windows target: `125` means a clw-internal failure before the child ran, `137` means clw killed a
running child for the output cap, and a child result is passed through **verbatim**. The Windows
cross-platform gate in [`scripts/gates.sh`](../scripts/gates.sh) invokes the `x86_64-pc-windows-gnu`
build with `--all-targets`; this is compile/type-check evidence only and does not execute the exit-code
journeys.

The CLI E2E harness is `#![cfg(unix)]`, and the Wine lane is intentionally limited to smoke and
pure-lexical checks because subprocess and filesystem semantics are not trustworthy there. The
125/137/verbatim journeys therefore do **not** run under Wine, and no test has been moved into the
portable/Wine lane. Runtime evidence requires a native Windows runner; until one exists, this explicit
declaration is the WP-38a acceptance evidence rather than an implied green test.

## Journey inventory by feature (lens-1 status)

Legend: **C** covered · **P** partial (unit/parse-level, not end-to-end) · **G** gap. Full cell-level matrices +
test citations live in the audit; this is the journey roll-up.

### snapshot (+ walk/ignore, manifest)
first-snapshot C · re-snapshot-identical-noop C · re-snapshot-changed+lineage C · differing-409 C ·
`--force` replace C · corrupt-ref recovery C · lost-ACK-409-reconcile **P (unit only, G at HTTP)** ·
walk `.git` excluded **P** · `.gitignore`/`.clwignore` **P** · nested/`.ignore`/global-excludesFile **G** ·
dotfiles-included **P** · non-UTF8-name-reject **G** · symlink intra/external C (external-skip **G at CLI**) ·
secret-warning collected C / **stderr-print G** · concurrency/dedup C · size edges C · empty-tree **G** ·
TOCTOU-size-growth P · failure/transport C · ref-domain C · Windows-runtime **G**.

### hydrate (+ digest, containment, caps, durability)
by-name C · by-`--manifest-digest` C · ref-not-found C · corrupt-manifest C · dest-not-empty C ·
missing-chunk C · perm/IO-on-dest P (mid-**write** IO **G**) · empty-snapshot **G** · manifest/chunk
integrity C · cache-cold C / **warm `bytes_from_cache` P (unit only, G at transport)** · containment
(abs/`..`/backslash/colon/symlink-escape, intra-`..` allowed) C · caps (1M/100k/1TiB/64M/16M, lying
total_size) C · atomicity/temp-cleanup/re-run-heal C · fsync-power-loss **G (inherently hard)** · files-before-
symlinks C · Windows copy-fallback P (dir-target + dangling **G**) · large/stress round-trip C.

### run (memoization)
miss→store→hit-replay-stdout C · **hit-replay-stderr G** · cache-key busts: content/dir/mode/env/tool-identity/
cwd C, **command-ARG G**, undeclared-env→hit C, **ignored-file-invisible G**, **undeclared-file-stale-replay G** ·
non-zero-never-cached C · child-never-ran→125 C · panic→125 C · output caps stdout C / **stderr-alone P** ·
137 / 128+signal C · **255 fallback G** · 402-on-run C · `--json` hint P (**e2e G**) · store concurrent C /
**30s-deadline-skip G** · secret-env-strip C · Windows-runtime P.

### lifecycle (status / ls / rm / prune / erase)
**status**: clean/dirty C · added/removed split P · error→exit2 **P (no e2e)** · `--json` **P (literal, not real)**.
**ls**: `--name` human C / **`--json` G** · `--all`+json C · run_cache C · corrupt-tolerance C · pagination C ·
capability-unavailable P. **rm**: name/ref-key/idempotent/refuse-without-yes C · declined-shape P · conflict C.
**prune**: dry-run/`--yes`/only-unreferenced C · fail-safes (missing-manifest, replica-lag-404, empty-live-set)
C · run-memo-kept-live C · **mid-sweep/`failed_at`/resume G** · `--yes`-json P. **erase**: **dry-run-default G** ·
happy `--yes`+confirm C · wrong-confirm-aborts C · **fails-loud-if-not-empty (remaining_refs/blobs) G** ·
**verify-empty MAX_PASSES=3 exhaustion G** · mid-delete-resume P · **all `--json` shapes G** · wrong-but-typed-
tenant **G**.

### transport / reliability (clw-client)
retries 5xx/429/exhaustion C · transport-error-retry **P (only via CLI e2e)** · 4xx-non-retry C · backoff-ladder
values **P** · Retry-After int C / **date-form-ignored G** / >3600-quota C / **huge-clamp-sleep G** · deadline
default/clamp/overflow/black-hole/**under-contention** C · deadline-bounds-backoff-sleep P · AIMD internals C /
**end-to-end-wiring G** · integrity-on-GET C · **lying/absent Content-Length G (top load-bearing branch)** ·
402 UX (direct+run) C · SSRF/redirect/userinfo/redaction C · caps: 10M-keys/1M-pages C, **16 MiB list-page G**,
cred-ticket-64KiB C.

### auth / config / cred-ticket / runner / platform / misc
config precedence C (CORELINK-vs-file **G**) · missing/unreadable/over-permissive/corrupt C · parser+proptests C ·
lenient-vs-strict C · tenant-validation C (control-char P) · tilde C. **auth**: login/logout/whoami C ·
**logout-`--json` no-op shapes G** · rotate C. **cred-ticket**: happy C · no-static-fallback C · 410 C ·
**401/404-distinct P** · **transport-error-fail-closed G** · **both-ticket+token→no-fallback P** · missing-lease/
fabric C · lease-injection C · 64KiB C. **runner keyspace**: disjoint C · other-value→exit2 **P (no e2e)**.
**init**: 0600/0700/preserve C · **already-exists/`--force` behavior G**. **doctor**: problems/redaction C.
**uninstall**: dry-run/`--yes`/never-self-delete/safety C. **completions**: 5 shells/offline C. **platform**:
**Windows-runtime skips the whole `#![cfg(unix)]` CLI e2e suite — unit-only, and non-required G** · HOME→
USERPROFILE C · **NO_COLOR-env G** · **RUST_LOG-override G** · `-v/-q` C.

## Ranked gap backlog (add these tests)

**Tier 1 — destructive-op guards & the GDPR truth (HIGH; add as unit + prod-E2E):**
1. `erase` **fails-loud-if-not-empty** (`remaining_refs`/`remaining_blobs`, non-zero, `complete:false`) — the
   core Art.17 guarantee; **untested**.
2. `erase` **dry-run default** (no `--yes` → blast-radius, delete nothing) — rail #1 of the most destructive
   verb; **untested**.
3. `prune` **mid-sweep failure / `failed_at` / resume** — destructive partial-failure path; **untested**.
4. `erase` verify-empty **MAX_PASSES=3 exhaustion** → loud error; **untested**.

**Tier 2 — silent-safety-regression risks (HIGH/MED):**
5. `snapshot` **secret-warning + external-symlink note actually printed to stderr** — user-facing safety output
   with no assertion; a refactor could drop it silently.
6. transport **lying/absent Content-Length** streaming-ceiling branch — the single most load-bearing untested
   defensive branch (anti-OOM on a hostile length).
7. run **cache-key: ARG-only bust** + **ignored-file-invisible** + **undeclared-file stale-replay** — hermeticity
   boundaries with no lock.
8. AIMD **end-to-end wiring** (a real 429 through `with_retry` shrinks the client governor).
9. cred-ticket **transport-error fail-closed** + **401/404 distinct** + **both-ticket+token→no-static-fallback**.

**Tier 3 — coverage holes (MED/LOW):**
10. **Windows runtime CLI suite** — `e2e.rs`/`roundtrip_prop` are `#![cfg(unix)]`, so `windows.yml` validates
    Windows at unit-level only; the shipped Windows binary's command behavior is never observed on Windows.
11. `--json` shapes never driven e2e: `erase` (all), `ls --name`, `status`, `logout` no-op.
12. `status` error→exit-2 e2e; `init` already-exists/`--force`; runner other-ref-domain→exit-2 e2e.
13. `NO_COLOR` / `RUST_LOG` env branches; empty-tree snapshot/hydrate; warm-cache `bytes_from_cache` at
    transport level; hydrate mid-write IO fault; nested/`.ignore` sources.

**Honestly hard / inherently non-unit (document, don't fake):** fsync power-loss durability (needs crash/
`dm-flakey` injection), the real `MAX_TOTAL_DIGESTS`/`MAX_PAGES` values (infeasible to drive), TOCTOU size-growth
at e2e.

## Prod-E2E plan — the `live-journeys` suite (closes lens 2)

**Goal:** a real user, in prod. Each test spawns the actual `clw` binary with `CLW_ENDPOINT`=prod and a real
PAT, and drives a full journey end-to-end against `corelink-api` — organized **by feature, by journey** (not a
grab-bag of unit asserts). Modeled on `clw-conformance/tests/live_lifecycle.rs`, expanded from 1 journey to the
inventory above.

**What only prod exercises (the reason this suite must exist):**
- Real cross-file/cross-snapshot **dedup** counters against a live CAS.
- **Warm-cache** re-hydrate (fewer real GETs the second time).
- **Team share** — two identities (two PATs) on one tenant: A snapshots, B hydrates + dedups.
- **Quota** 402 / storage 429 against the real metering.
- **Replica-lag** false-orphan on prune (eventual consistency).
- **Cred-ticket (C2c)** redeemed against the real fabric.
- Real network reliability: retries, deadline, AIMD under real latency.

**Suite shape (`crates/clw-conformance/tests/live_journeys.rs`, `#[cfg(feature = "live")]`, `REQUIRE=1` in CI):**
one module per feature; one test per journey; helpers spawn the binary + assert on real exit codes / stdout /
`--json`. Each test **creates its own throwaway ref names under a dedicated test tenant and cleans up** (`rm` /
`prune` / `erase`) so runs are idempotent and don't accrete data.

**Guardrails (prod is real money + real data):**
- A **dedicated test tenant** (not a customer tenant), created/pointed by CI secret; the destructive-journey
  tests (`erase`, `prune`) run ONLY against it and it is `erase`d at the end.
- Cost-bounded: small blobs; the metered-probe tenant already proves the 402 wall, so quota-journey uses it.
- PAT(s) via CI secrets / local env only — **never** printed or committed.

**Where it runs:** a scheduled + dispatchable GitHub-hosted lane (like `conformance-canary.yml`), and runnable
locally with `CLW_ENDPOINT` + `CLW_TOKEN` + `CLW_LIVE_TENANT` exported. Not a per-PR required gate (prod calls);
it's a daily/nightly confidence lane, with the existing canary as the always-on happy-path tripwire.

**Owner decision needed to run it for real:** (1) which **test tenant + PAT(s)** — recommendation: a fresh
dedicated `clw-e2e` tenant the suite fully owns and erases; (2) confirm the scheduled lane. The suite can be
authored now (endpoint/creds are injected at run time); it only executes against prod once those are wired.
