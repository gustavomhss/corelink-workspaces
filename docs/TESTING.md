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

## The done-line
"Working as it should" = the required gate green **and** coverage ≥ floor (ratcheting) **and** the fuzz
lane green **and** no new mutation survivors — plus the empirical half: real workloads succeeding, watched
via the conformance/metered/deadman canaries. Robustness past that comes from real usage, not more audits.
