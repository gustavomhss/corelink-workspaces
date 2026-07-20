# Testing & test-rigor lanes

`clw`'s correctness rests on two things: the **required gate** (`scripts/gates.sh` — fmt, clippy
`-D warnings`, `cargo test`, cargo-deny/audit, windows cross-check) that every PR must pass, and the
**test-rigor lanes** below that make coverage *provable* rather than assumed. The gate proves the code
compiles and the tests pass; these lanes prove the tests are **adequate**.

> Posture: **measure → ratchet → gate.** New rigor lanes start report-only (learn the real number),
> then a floor is set and the lane starts blocking a regression, then it ratchets upward. Fuzzing is
> the exception — a crash always blocks (it is a defect, not a metric).

## The three lanes

| Lane | Workflow | Cadence | What it proves | Blocks? |
|---|---|---|---|---|
| **Coverage** | `.github/workflows/coverage.yml` | every PR + push-main + dispatch | how much code a test actually executes | report-only → floor (Phase 1b) → required (1c) |
| **Fuzz** | `.github/workflows/fuzz.yml` | nightly + dispatch | the untrusted-input parsers never panic/UB on arbitrary bytes | **yes — a crash is red** |
| **Mutation** | `.github/workflows/mutation.yml` | weekly + dispatch | the tests actually *kill* injected faults (not just run) | report-only → follow-on blocks new survivors |
| **Round-trip (property)** | `crates/clw-integration-tests/tests/roundtrip_prop.rs` | every PR (32 cases) + weekly exhaustive (2000) | `snapshot→hydrate` is byte-identical on ARBITRARY trees | gate test — red on any fidelity break |
| **Stress / scale** | `.github/workflows/stress.yml` | weekly + dispatch | the lifecycle survives magnitude (GB file, 100k files, deep+wide, 64-way concurrency) | **yes — crash/hang/OOM/mismatch is red** |
| **Windows runtime** | `.github/workflows/windows.yml` | PR (crates/scripts) + push-main + dispatch | the workspace test suite RUNS on Windows (was compile-only) — path/mode/symlink-copy-fallback behavior | red on any Windows test failure |
| **Shellcheck** | `.github/workflows/shellcheck.yml` | PR (scripts) + push-main + dispatch | the distribution + CI shell scripts (incl. the public `install.sh`) lint clean | red on any shellcheck warning |
| **E2E evidence** | `.github/workflows/e2e-evidence.yml` | **manual only** (`workflow_dispatch`) | the REAL `clw` binary, driven as a user (argv+env) against prod, emits a per-step evidence report | required primary-cred journey skip is red (`REQUIRE_PROD=1`); report artifact always uploaded |

They run on github-hosted runners and are **additive** — they do not touch the required `gates`/`dco`
checks (except the round-trip property test, which runs inside the gate). The Windows lane runs on
`windows-latest` (gnu toolchain, matching the shipped binary); everything else on `ubuntu-latest`.

## Coverage (`cargo-llvm-cov`)
Measures total line coverage on the whole workspace (default-features — the `live` conformance suite is
not run without a backend). Report-only today: it prints the total % to the job summary and uploads
`lcov.info` + JSON. **Phase 1b**: set `COVERAGE_FLOOR` = `floor(baseline)` in the workflow's final step
and flip it to `exit 1` on a drop (deterministic, not flaky). **Phase 1c**: promote to a required check.

Run locally:
```
rustup component add llvm-tools-preview
cargo llvm-cov --workspace --summary-only          # total %
cargo llvm-cov --workspace --html && open target/llvm-cov/html/index.html
```

## Fuzz (`cargo-fuzz`, nightly)
The `fuzz/` package is a **detached, out-of-workspace** cargo-fuzz project (nightly-only, so the stable
gate never sees libfuzzer). Four targets fuzz the untrusted-input surface of `clw-types` — the
path-escape / manifest-parse attack surface:
`fuzz_validate_symlink_target`, `fuzz_validate_rel_path`, `fuzz_manifest_from_bytes`, `fuzz_digest_from_hex`.
The lane runs each for 5 minutes nightly; a crash fails the run and uploads the reproducing input.

Run locally (needs nightly + `cargo install cargo-fuzz`):
```
cargo +nightly fuzz run fuzz_validate_symlink_target -- -max_total_time=300
```

## Mutation (`cargo-mutants`)
Injects faults and re-runs the tests; a **surviving** mutant (`missed`) is a change the suite failed to
catch — a real test gap. Whole-workspace and slow, so it runs **weekly**, sharded across a matrix. It is
report-only today (uploads the survivor report per shard). A harness fault (the unmutated tests fail /
bad args) does fail the job — that is a real break.

Run locally:
```
cargo install cargo-mutants
cargo mutants --workspace            # or -p clw-types to scope
```

**Follow-on ratchet**: triage the survivors (critical crates first — `clw-types`, `clw-manifest`,
`clw-hydrate`, `clw-client`: the untrusted-input, containment, atomicity and retry/cap logic), add tests
that kill them, then flip mutation to `--in-diff` per-PR blocking on any **new** survivor.

## E2E round-trip + stress (Phase 2)
`crates/clw-integration-tests/tests/roundtrip_prop.rs` is a **property-based** lifecycle test: proptest
generates arbitrary workspace trees (varied unicode/space names, nested dirs, empty + multi-byte files,
exec bits, in-tree symlinks) and asserts `snapshot→hydrate` is byte-identical with structure + mode +
symlink target preserved. It runs in the gate at 32 cases; the stress lane cranks `PROPTEST_CASES=2000`.

`crates/clw-integration-tests/tests/stress.rs` holds the `#[ignore]`d **scale** scenarios (GB file, 100k
files, deep+wide tree, N-way concurrency) run by `stress.yml` at aggressive magnitude via env
(`CLW_STRESS_BIG_BYTES` / `_FILES` / `_DEPTH` / `_WIDE` / `_CONC`). Both use in-process/disk-backed stores
(no HTTP, no quota). Run locally:
```
cargo test -p clw-integration-tests --test stress -- --ignored          # moderate defaults
CLW_STRESS_FILES=100000 cargo test -p clw-integration-tests --test stress -- --ignored --nocapture
```
Fault-injection / chaos E2E (5xx/429/corrupt-body/mid-op-401, large round-trip, concurrency) already lives
in `tests/adversarial.rs` and runs in the gate.

## E2E evidence — the real binary, driven as a user (Phase 3)

The lanes above all reach `clw` through a **library or test-harness** path (in-process stores,
`--features live` conformance calls, proptest generators). That proves the *code* — but it is, in the
owner's words, "outro caminho": it is NOT how a user reaches `clw`. A user runs the **binary**, with
**argv + env**, against a **real backend**, and reads **stdout / stderr / exit-code**. The E2E-evidence
lane closes exactly that gap.

**The vehicle (non-negotiable).** Every step spawns the real `clw` binary and captures the exact
command, the exit code (expected vs observed), stdout, stderr, the parsed JSON, the timing, and a
**behaviour/security observation with its rationale**. The ONLY entry point is the binary via
`crates/clw-e2e-evidence`'s `Runner::step` — no journey ever calls a clw library function. The token is
a `Secret`: passed via the `CLW_TOKEN` env (never argv, never `ps`-visible) and scrubbed from every
captured string **before** it is stored, so nothing secret can reach the report.

**The product is the report, not the green tick.** Each run emits `evidence.md` + a self-contained,
theme-aware `evidence.html` — the artifact a human reads to analyse behaviour and security per step.
Pass/fail is a floor; the *evidence* is the deliverable.

**Story-driven (the key move).** Each journey is a **PERSONA** — a real-looking customer living a
realistic narrative — so the backend can't tell it's a test and the evidence is judged against the
user's **intent**, not a step-list with baked asserts. Nothing signals "test": realistic project data
(`realistic::orders_api`), names a user would type, no sentinels, provisioned through the real front
door. The report reads as a story: each beat narrates what the persona is doing, then shows the real
`clw` invocation + the full-shape verification.

**The cast** (`crates/clw-e2e-evidence/src/journeys/story_*`, driven by the `clw-e2e` binary):

| Persona (story) | What its evidence proves (against PROD unless noted) |
|---|---|
| **smoke-offline** | the credential-free surface (version/completions/doctor) — harness self-test, always runs |
| **Marina** — solo dev | config `0600`/`0700`, flag>env, full lifecycle, dedup + pin-by-`--manifest-digest`, hydrate byte+mode+unicode-identical (cold), memoize (replica-lag-tolerant), create-only conflict + `--force`, the error surface, rm-then-gone |
| **Dana** — security | no-PAT-in-`-vv` (pre-scrub), endpoint-userinfo redaction, plaintext-`http://` refusal, `rm`/`logout` consent gates preserve identity |
| **Ravi** — automation/CI | exit-code passthrough (child's `3` → `3` verbatim), reserved exit `125` (clw-internal, step never ran), runner ref-domain (`CLW_REF_DOMAIN=runner`) is a keyspace disjoint from developers' |
| **Priya + Sam** — team *(opt: `CLW_TOKEN_B`)* | cross-identity read + dedup, create-only stops a teammate clobbering a shared name, two concurrent identical saves converge on one root |
| **Omar** — offboarding *(opt: erase creds)* | `prune` sweeps orphans only (live data survives, verified cold), erase refuses the wrong `--confirm-tenant`, the correct confirmation empties the tenant (GDPR Art.17), verified gone |
| **Nadia** — quota *(opt: quota creds)* | a metered tenant at its monthly budget → a real, actionable **402** (not a cryptic 500) |
| **J9 adversarial** | the real binary vs a controllable HOSTILE backend (loopback) — SSRF-redirect NOT followed, 5xx/429 bounded-backoff-then-clean-error, 413 clean fail. Needs no creds. |
| **JE efficiency** | a divergent re-snapshot fails-fast on the ref (0 uploads before the 409), not upload-then-reject. Needs no creds. |

**Two-tier skip (honest AND operable).** A story that cannot run reports a **skip**, never a false
pass. A skip of a **primary-cred** persona (Marina / Dana / Ravi) trips `CLW_E2E_REQUIRE_PROD=1` → hard
failure, so CI can never silently stop exercising prod (the canary-blind failure mode). A skip of an
**optional-infra** persona (team needs a 2nd identity, offboarding a throwaway erase tenant, quota a
low-ceiling tenant) is reported but never require-fails — so the lane is green with just the primary PAT
(mirrors the conformance suite's `soft_skip`).

**Provisioning (indistinguishable from a real user).** Each persona gets a real `{tenant, PAT}` via the
actual admin/mint path — the data plane (CAS/AC/auth) can't distinguish it from a genuine customer.
`mint-dogfood-pat.sh` provisions a PAT for an **existing** tenant (the `pat` row FK-references `tenant`,
so it cannot *create* tenants — fresh dedicated tenants still need the signup/seed path). Optional-infra
personas need dedicated tenants (a throwaway for erase; a low-ceiling one for the 402).

**Mechanized:** `.github/workflows/e2e-evidence.yml` builds the binary + the runner, drives the stories
against prod **on demand** (manual `workflow_dispatch` — no schedule; the daily prod watch is already
the conformance/metered/deadman canaries' job, and this lane exercises prod + burns CI minutes) with
`REQUIRE_PROD=1`, and uploads the evidence report as an artifact **always** (a red run is when the
evidence matters most). It reuses the `CLW_CI_*` secrets from `live-journeys.yml` (+ `CLW_CI_TOKEN_B` for
the team, `CLW_CI_ERASE_*` for offboarding, `CLW_CI_QUOTA_*` for quota).

### The two anti-vacuity laws (learned the hard way — every journey's cold review broke on one)

An E2E assertion can *pass while proving nothing*. The independent cold review of these journeys found a
real over-claim on **every single one**; two rules stop them:

1. **A "survives / reproduces / persisted" claim MUST hydrate on a COLD cache and assert
   `bytes_downloaded > 0`.** A warm local cache replays without ever contacting the backend — so a
   backend-side failure (a swept blob, a lost ref) would still "hydrate" green. Only a cold fetch with
   bytes actually pulled proves the backend held the data.
2. **A "no secret leaked" claim MUST check the pre-scrub `leaked_secret` signal, never the stored
   output.** The harness redacts registered secrets from captured text *before* storing it, so any
   assertion that a secret is ABSENT from the stored report is **vacuous** — it was scrubbed by
   construction. The only non-vacuous leak signal is `StepEvidence.leaked_secret`, computed on the raw
   output before redaction.

Corollary (SSRF/redirect): a "did NOT contact the internal host" claim must first prove the binary
**reached the backend** (saw a non-internal request in the server's own log) — else a mere connect
failure passes falsely.

### The security-observation catalog (each is a step with evidence, observed via the binary)

**Observed today** (a report row a human can audit): token redaction (every command + `--json` + logs,
verified **pre-scrub** via `leaked_secret`) · config **0600** / dir 0700 · endpoint-userinfo redaction ·
plaintext-`http://` refusal (non-loopback) · **401** on a bad token · **409** create-only (no
last-write-wins) · consent gates on `rm` / `prune` / `erase` / `logout --all` (declined → non-zero exit,
identity preserved) · SSRF/redirect NOT followed · 5xx/429 bounded backoff then clean error (no hang, no
hammer) · 413 clean fail.

**Not yet driven via the evidence binary** (the doctrine's own standard: don't claim what the harness
doesn't observe): the runner ref-domain KEYSPACE is now exercised (Ravi/automation proves it's disjoint
from the user's), but the **no `CLW_TOKEN` fallback** property specifically still waits on J8
(cred-ticket). And the setuid-strip-on-hydrate + external-symlink-skip defenses
(clw *does* enforce them — see the v0.1.6 / v0.1.7 hardening + `crates/clw-integration-tests`) are not
yet E2E-evidence steps; a hostile-fixture journey to observe them via the binary is a follow-on.

## The done-line
"Working as it should" = the required gate green **and** coverage ≥ floor (ratcheting) **and** the fuzz
lane green **and** no new mutation survivors — plus the empirical half: real workloads succeeding, watched
via the conformance/metered/deadman canaries. Robustness past that comes from real usage, not more audits.
