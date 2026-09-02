# Testing & test-rigor lanes

`clw`'s correctness rests on two things: the **required gate** (`scripts/gates.sh` — fmt, clippy
`-D warnings`, `cargo test`, cargo-deny/audit, windows cross-check) that every PR must pass, and the
**test-rigor lanes** below that make coverage *provable* rather than assumed. The gate proves the code
compiles and the tests pass; these lanes prove the tests are **adequate**.

> Posture: **measure → ratchet → gate.** New rigor lanes start report-only (learn the real number),
> then a floor is set and the lane starts blocking a regression, then it ratchets upward. Fuzzing is
> the exception — a crash always blocks (it is a defect, not a metric).

## Workflow and test lanes

This is the complete inventory of the 13 versioned workflow files. Unless noted, jobs run on
[`corelink`](../.github/workflows/ci.yml), the ephemeral CoreLink runner fabric; they are not
GitHub-hosted `ubuntu-latest` or `windows-latest` jobs. The one physical-runner exception is the
release build job on `[self-hosted, macOS, X64]`.

| Lane | Workflow | Cadence | What it proves / enforcement |
|---|---|---|---|
| **Required gate + DCO** | [`ci.yml`](../.github/workflows/ci.yml) | PR, push to `main`, dispatch | `gates.sh`, DCO, and guard self-tests; the round-trip property test runs inside `gates.sh`. |
| **Coverage** | [`coverage.yml`](../.github/workflows/coverage.yml) | PR, push to `main`, dispatch | Default-feature workspace line coverage, excluding `clw-e2e-evidence`; an active 88% floor makes a lower result red. |
| **Fuzz** | [`fuzz.yml`](../.github/workflows/fuzz.yml) | daily 08:00 UTC, dispatch | Four untrusted-input fuzz targets; a crash is red. |
| **Mutation** | [`mutation.yml`](../.github/workflows/mutation.yml) | PR, dispatch | Per-PR `diff` mode fails the job on a survivor, timeout, or harness fault; dispatched `workspace` backlog sweeps remain report-only for survivors/timeouts. |
| **Stress / scale** | [`stress.yml`](../.github/workflows/stress.yml) | Mondays 02:00 UTC, dispatch | 2 GiB, 25k-file success, 40k-file expected-cap rejection, deep/wide, 64-way, and 2,000 property cases; a crash, hang, OOM, or mismatch is red. |
| **Windows binary smoke** | [`windows.yml`](../.github/workflows/windows.yml) | relevant PRs, push to `main`, dispatch | Builds the shipped `windows-gnu` binary with MinGW and runs its smoke checks under Wine on `corelink`; it is not a hosted or full-fidelity Windows-runtime lane. |
| **Shell lint** | [`shellcheck.yml`](../.github/workflows/shellcheck.yml) | PR, push to `main`, dispatch | Shellcheck for distribution and CI scripts. |
| **E2E evidence** | [`e2e-evidence.yml`](../.github/workflows/e2e-evidence.yml) | dispatch only | Real `clw` binary, argv+env, and evidence report; a required primary-credential skip is red. |
| **Conformance canary** | [`conformance-canary.yml`](../.github/workflows/conformance-canary.yml) | daily 13:00 UTC, dispatch | Live API conformance against its canary credentials. |
| **Live journeys** | [`live-journeys.yml`](../.github/workflows/live-journeys.yml) | Mondays 06:17 UTC, dispatch | Live production journey suite; a required primary-credential skip is red. |
| **Metered-probe canary** | [`metered-probe-canary.yml`](../.github/workflows/metered-probe-canary.yml) | Mondays 12:00 UTC, dispatch | A dedicated low-ceiling tenant reaches and verifies the production 402 boundary. |
| **Canary deadman** | [`canary-deadman.yml`](../.github/workflows/canary-deadman.yml) | daily 16:00 UTC, dispatch | Registered scheduled lanes exist, are fresh, and conclude successfully; its own liveness remains out of scope. |
| **Release** | [`release.yml`](../.github/workflows/release.yml) | `v*` tag push | Preflight, release checks, signing/build/publish flow; non-build jobs run on `corelink`, while the cross-target build/signing host is the physical macOS runner. |

## Coverage (`cargo-llvm-cov`)
Measures total line coverage on the whole workspace with default features (the `live` conformance suite
is not compiled or run without a backend). [`coverage.yml`](../.github/workflows/coverage.yml) uploads
`lcov.info` + JSON and actively enforces `FLOOR=88`: a result below 88% exits 1. The workflow check is
not yet a branch-protection required check, but the floor itself is already a deterministic hard gate.

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
catch — a real test gap. It runs per-PR, scoped to the PR's own diff; a survivor or harness fault
(the unmutated tests fail / bad args) makes that workflow job red. The check is not yet required by
branch protection. It replaced a weekly
sharded whole-workspace sweep in WP-34; that history, and the exhaustive sweep that survives it, are
below.

Run locally:
```
cargo install cargo-mutants
cargo mutants --workspace            # or -p clw-types to scope
```

**The lane is scoped to the PR's own diff** (`--in-diff`), and any survivor in it is red: a fault was
injected into code this PR touches and the suite still passed. Reproduce a CI red locally with

```
git diff "$(git merge-base origin/main HEAD)" HEAD -- '*.rs' > pr.diff
cargo mutants --workspace --in-diff pr.diff
```

**A Rust diff that yields no mutants is INCONCLUSIVE, not a pass.** A test-only, comment-only or
rename-only change generates nothing to inject, and `cargo mutants` exits 0 in that case — so the lane
asks `--list` first and reports "nothing was verified" rather than claiming the tests are adequate. It
does **not** fail: adding tests is the right thing to do and must not redden a PR.

**Survivors in code this runner truly did not compile are unanswerable here, but timing does not prove
that.** cargo-mutants is syn-based and can generate `#[cfg(windows)]` mutants on Linux, reporting them as
`MissedMutant`. However, it can also baseline several packages and test a mutant only in its own package,
so a real survivor in a small package may finish far faster than the aggregate baseline.
`scripts/mutation-verdict.sh` therefore uses unusually cheap complete Build+Test timings only as a
**POSSIBLY_UNCOMPILED** triage hint. Every `MissedMutant` remains **SURVIVORS** and actionable until the
uploaded build logs provide separate evidence that the mutated source was absent from the build. Only
then may `.cargo/mutants.toml` record the exclusion with a written reason. Missing, partial, failed or
unrecognised timing does not even raise the hint. This fail-safe contract is covered by
`scripts/mutation-verdict-selftest.sh` in the required gate, because the mutation lane itself only runs on
Rust diffs that yield mutants and so cannot be relied on to exercise it. When the ledger is what produced
a zero, the INCONCLUSIVE report says so and gives the count — a ledger-manufactured zero and a genuinely
mutation-free diff must not read the same.

**The exhaustive sweep** — for triaging the survivor backlog (critical crates first: `clw-types`,
`clw-manifest`, `clw-hydrate`, `clw-client` — the untrusted-input, containment, atomicity and retry/cap
logic) — is `workflow_dispatch` with `mode: workspace`, which is the dispatch default. That mode keeps
the old report-only posture (survivors and timeouts are counted, not failed) and the old hard failures
(a broken baseline is still red). It is no longer on a schedule, so sweeping the backlog is now a
deliberate act rather than something CI does on its own.

It was a weekly whole-workspace run, report-only, until WP-34 (2026-08-31). That lane was red in 4 of
its last 6 runs, and none of the four was a mutation finding — but they were not all the same fault
either, and the lane changed runners mid-window. 2026-07-20/08-03/08-10 ran on GitHub-hosted
`ubuntu-latest` and were **hosted-runner reclaims** (`The runner has received a shutdown signal.`);
2026-08-24 and 08-31 ran on `corelink`, where the ephemeral VM was destroyed mid-step and, on 08-31,
3 of 6 shards queued 89 minutes for a runner. What the two eras share is that a five-hour job is a
five-hour exposure to losing its runner, and that report-only, the lane gated nothing to show for the
cost. On `corelink` there was the further cost that six concurrent 300-minute jobs starve the fleet the
required `gates` check also uses. See `.github/workflows/mutation.yml`.

## E2E round-trip + stress (Phase 2)
`crates/clw-integration-tests/tests/roundtrip_prop.rs` is a **property-based** lifecycle test: proptest
generates arbitrary workspace trees (varied unicode/space names, nested dirs, empty + multi-byte files,
exec bits, in-tree symlinks) and asserts `snapshot→hydrate` is byte-identical with structure + mode +
symlink target preserved. It runs in the gate at 32 cases; the stress lane cranks `PROPTEST_CASES=2000`.

`crates/clw-integration-tests/tests/stress.rs` holds the `#[ignore]`d **scale** scenarios. The scheduled
[`stress.yml`](../.github/workflows/stress.yml) job uses a 2 GiB file, 25,000 files that must round-trip,
40,000 files that must fail cleanly at the manifest cap, depth 500, width 20,000, and 64 concurrent flows;
its separate property job runs 2,000 cases. Both use in-process/disk-backed stores (no HTTP, no quota).
Run locally:
```
cargo test -p clw-integration-tests --test stress -- --ignored          # moderate defaults
CLW_STRESS_FILES=25000 CLW_STRESS_FILES_OVERCAP=40000 cargo test -p clw-integration-tests --test stress -- --ignored --nocapture
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
