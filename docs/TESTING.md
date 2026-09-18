# Testing, assertion scope and test-rigor lanes

Source/configuration reviewed: **`e46e90b9a3308724da446d8dc3887cfc5c39a882`**,
2026-09-18. This page describes existing selectors and oracles, not the latest
CI outcome. The [journey map](TEST-JOURNEY-MAP.md) gives per-journey assertions,
limitations and remaining gaps. Older observations remain in Git and dated
validation notes; an earlier green run does not certify a different source tree.

Tests, coverage, mutation, fuzzing and live observations provide different evidence.
A line reached is not necessarily an assertion about its behavior; a passing suite
cannot prove that all relevant faults would be detected. Keep **presence**, **oracle
adequacy**, **execution**, **result** and **environment** separate.

## Current campaign execution mode

The owner's 2026-09-18 decision selects **local execution of the same applicable
checks, without Actions**, as recorded in [CLAUDE.md](../CLAUDE.md#local-execution-and-equivalence).
The workflow inventory below retains the checked-in commands, selectors and
configured platforms; it is not a requirement to schedule this campaign through
Actions. Local failures, missing required evidence and stale SHAs still block.
No workflow, test, threshold or actual repository protection is changed by this
execution-provider decision.

## Workflow inventory at the reviewed source

The versioned workflows now use standard **GitHub-hosted runners**, following the
owner-authorized runner migration. Most jobs use `ubuntu-24.04`; release build/sign
uses `macos-15-intel`, and its Authenticode verification uses `windows-2025`.
This is configured placement, not proof a job started. Historical CoreLink-fabric
or self-hosted observations do not describe the current runner labels.

| Lane | Workflow / trigger | Selected behavior and limits |
|---|---|---|
| Gate + DCO | [ci.yml](../.github/workflows/ci.yml): PR, main push, dispatch | Real `gates.sh`, DCO and script self-tests. The gate's selected lane determines Rust work; absence of jobs is not a pass. |
| PR title | [pr-title.yml](../.github/workflows/pr-title.yml): PR metadata events | Conventional title check from trusted base source, not execution of PR code. |
| Coverage | [coverage.yml](../.github/workflows/coverage.yml): PR, main push, dispatch | Linux default-feature line coverage, excludes evidence harness; configured floor 88%. Effective merge enforcement must be checked separately. |
| Fuzz | [fuzz.yml](../.github/workflows/fuzz.yml): daily 08:00 UTC, dispatch | Six input targets, seed/restored corpus comparison and finite fuzz budgets. A configured target is not an executed campaign. |
| Mutation | [mutation.yml](../.github/workflows/mutation.yml): PR, dispatch | Diff mode rejects survivors/timeouts/harness faults; dispatched workspace mode retains a report-only survivor/timeout backlog. |
| Stress | [stress.yml](../.github/workflows/stress.yml): Monday 02:00 UTC, dispatch | Ignored scale cases with explicit sizes plus a separate 2,000-case property run, in release profile. Not the default unit run. |
| Windows-shaped binary smoke | [windows.yml](../.github/workflows/windows.yml): selected paths on PR, main push, dispatch | Ubuntu/MinGW build, explicit Wine command smoke and selected lexical tests. Not native Windows or the portable CLI test target. |
| Shell lint | [shellcheck.yml](../.github/workflows/shellcheck.yml): PR, main push, dispatch | Distribution/CI shell-script policy. |
| E2E evidence | [e2e-evidence.yml](../.github/workflows/e2e-evidence.yml): dispatch | Builds real CLI/evidence binaries, drives personas, requires primary and optional inputs with both strict modes. Can write remote data. |
| Conformance canary | [conformance-canary.yml](../.github/workflows/conformance-canary.yml): daily 13:00 UTC, dispatch | `clw-conformance --features live`, required primary identity. Library and mixed-entry-point tests, not one universal CLI journey. |
| Live journeys | [live-journeys.yml](../.github/workflows/live-journeys.yml): Monday 06:17 UTC, dispatch | Builds `clw`, selects `--test live_journeys --features live`. Optional infrastructure may still soft-skip under conformance REQUIRE. |
| Metered probe | [metered-probe-canary.yml](../.github/workflows/metered-probe-canary.yml): Monday 12:00 UTC, dispatch | Dedicated metered-tenant boundary; not a free or authority-independent test. |
| Deadman | [canary-deadman.yml](../.github/workflows/canary-deadman.yml): daily 16:00 UTC, dispatch | Checks registered lanes' freshness/outcomes; cannot by itself establish its own scheduler's liveness. |
| Release | [release.yml](../.github/workflows/release.yml): `v*` push | Preflight, live conformance, five builds/signing, verification and publication. Do not use a tag as a harmless docs test. |

## Select the relevant checks without changing their meaning

For runtime changes, the ordinary local gate is:

```sh
bash scripts/gates.sh
```

The gate has no `--scoped [base]` interface. Local/main execution defaults to the
full lane; trusted PR context permits the repository's diff classifier to select
its existing docs/scoped/full behavior. Do not fabricate PR context to turn a
runtime change into a docs lane. `CLW_SKIP_CROSS` is not acceptable evidence for a
required cross check. Read the checked-in script rather than an old recipe.

For a bounded documentation or comment-only change, select content/reference and
policy checks under [REVIEWER-BRIEF](../REVIEWER-BRIEF.md) and the work package.
Prove any comment-only source edit leaves executable text, attributes and selectors
unchanged. This does not disable workflow-triggered checks or invent remote success.
DCO, title validation and the documented integration path still apply. The
[pre-merge checker](../scripts/pre-merge-gate-check.sh) retains its remote-only
meaning: no missing, stale or cancelled run becomes green. Under the selected
local mode, a refusal solely for absent Actions results is not a failed local
check or a request to rerun Actions. Use the current local acceptance and
head/base/review checks in CLAUDE.md. The historical #320 diagnosis is not proof
that Actions billing was repaired and is not a dependency on hosted execution.

Use an isolated checkout and application HOME/cache/temp for commands that touch
configuration or fixture files. Limit build/test parallelism on shared hosts and
use a candidate-specific target when compiling. Shared-target artifacts alone do
not establish which worktree produced a binary. Keep credentials absent from
ordinary tests; select explicit authorized endpoints for any live command.

### Locate a test without claiming it ran

Source references in the map give exact function names and target files. Once a
suitable toolchain and owned target are available, libtest listing can resolve
the host-selected names:

```sh
cargo test --locked --offline -p clw-run --lib -- --list
cargo test --locked --offline -p clw-cli --test e2e -- --list
cargo test --locked --offline -p clw-cli --test e2e_portable -- --list
```

These commands can **compile** before listing. `--list` itself does not run the
assertions. `--offline` requires dependencies already present; failure to resolve
them is not a source failure or permission to change the lockfile. Cross-target
`--no-run` compiles only. A pre-existing binary's listing identifies that binary,
not a freshly built current candidate. Do not use a stale binary as acceptance.

For an actual focused execution, use the listed full selector and check nonzero
expected selection, terminal summary, source/build identity and fixture behavior:

```sh
cargo test --locked --offline -p clw-run --lib \
  tests::command_arg_change_yields_different_key -- --exact
cargo test --locked --offline -p clw-cli --test e2e_portable
```

Do not reuse these Unix/environment-sensitive examples as proof of native Windows.
A process exit 0, a test enumeration or a zero-selected-test summary is insufficient.

## Coverage: platform and denominator are part of the result

The configured Linux measurement is:

```sh
cargo llvm-cov --workspace --exclude clw-e2e-evidence --no-report
cargo llvm-cov report --ignore-filename-regex 'crates/clw-e2e-evidence/' \
  --lcov --output-path lcov.info
cargo llvm-cov report --ignore-filename-regex 'crates/clw-e2e-evidence/' \
  --json --output-path coverage.json
cargo llvm-cov report --ignore-filename-regex 'crates/clw-e2e-evidence/' --summary-only
```

The workflow obtains these reports from the same instrumented run and enforces
`FLOOR=88`. Default features exclude feature-gated live assertions; the evidence
harness is outside this denominator. Its own logic has separate tests. A local
macOS report, all-features report or different excluded set is supplemental,
not a directly interchangeable floor measurement. Record source, platform,
selector, exclusions and actual output. Do not infer current branch-protection
settings from comments in the coverage workflow.

## Fuzz and mutation

The detached `fuzz/` workspace uses nightly and these existing targets:
`fuzz_validate_symlink_target_contained`, `fuzz_validate_rel_path`,
`fuzz_manifest_from_bytes`, `fuzz_digest_from_hex`, `fuzz_ref_record_from_bytes`
and `fuzz_validate_tenant`. The lane compares restored-corpus coverage with the
cold seed using the configured finite budgets, then fuzzes and preserves crashes.
A cache bootstrap, replay and novel fuzzing interval have different meanings;
record the actual stage and artifacts. No fuzz execution is implied by this map.

Mutation is per-PR diff by default. Survivors, timeouts and baseline/harness faults
are distinguishable, not a single percentage. The existing `mutation-verdict.sh`
uses low Build+Test time only as a **POSSIBLY_UNCOMPILED** hint; it does not establish
that a survivor was absent from the compiled target. Excluding a mutant requires
source/build evidence and an explicit ledger reason, not timing alone.

An empty mutant list is **INCONCLUSIVE**, not proof that tests detect faults. A
comment-only/test-only Rust diff can legitimately produce none; the workflow reports
that rather than rejecting good test additions. Workspace dispatch preserves its
report-only backlog semantics, without relaxing diff-mode requirements. Historical
2026 runner interruptions and the replaced weekly sweep remain documented in the
[previous TESTING revision][old-testing] and workflow history; they are not today's
runtime result. Do not launch an unbounded sweep to validate prose.

## Round trips and stress

[roundtrip_prop.rs](../crates/clw-integration-tests/tests/roundtrip_prop.rs) calls
snapshot/hydrate APIs with an in-process store and generated trees, asserting
selected bytes/structure/modes/links. It is Unix-gated; a target that excludes its
items has not run the property. The source's default case budget and the stress
workflow's override are separate. Generated examples are not separate independent
unit functions, and listing one property does not establish how many examples ran.

[stress.rs](../crates/clw-integration-tests/tests/stress.rs) contains explicitly
ignored scale cases using disk-backed stores. The configured lane selects large
file, many-files success, manifest-cap rejection, deep/wide and concurrent cases;
its property job sets `PROPTEST_CASES=2000`. These are not default `cargo test`
results, HTTP production tests or disk-residency guarantees. Use approved resources
and record the actual sizes, selectors, ignored counts and terminal state.

[adversarial.rs](../crates/clw-integration-tests/tests/adversarial.rs) separately
uses controlled HTTP behavior for bounded failure/round-trip scenarios. Neither
in-process recovery nor a test that clears the destination before rehydrating
establishes power-loss durability, incremental resume or global tree transactions.

## Windows: what each configured path can actually establish

The main gate cross-lints Windows-GNU targets; it does not execute them. The
Windows workflow builds a Windows-GNU binary on Ubuntu with MinGW, then runs
explicit `--version` and doctor invocations under Wine. Doctor's shell smoke
checks successful dispatch, not every field or redaction assertion.

That workflow separately builds the hydrate test executable and runs only its
`lexical_` functions through Wine, with a source-derived expected count. It does
not run `canonical_guard_*`, the copy materializer, the Unix CLI E2E suite or
[e2e_portable.rs](../crates/clw-cli/tests/e2e_portable.rs). The latter is portable
source for offline cases; source selection is not execution on every OS. Its
stdout-only doctor assertion does not silently cover stderr.

A native Windows acceptance needs an actual Windows execution environment and
checks for the relevant filesystem, ACL, subprocess and command behavior. Release's
native Authenticode check is not such a runtime suite. A Wine result and compile-only
result must remain identified as such in the journey map and receipts.

## Live tests and the real-binary evidence runner

Real CLI coverage already exists in `clw-cli/tests/e2e.rs` against local HTTP
fixtures. The live conformance/lifecycle suites primarily call APIs; live journeys
adds several API journeys and a binary-driven destructive path. They are not all
interchangeable with a user driving the distributed executable.

The separate [evidence runner](../crates/clw-e2e-evidence/src/lib.rs) executes the
selected real CLI through `Runner::step`. `CLW_E2E_BIN` selects its path (the default
is a sibling `clw`); an artifact acceptance must supply and verify an explicit
path/digest rather than infer which build the default selected. It records command, expected/observed exit,
streams, JSON when applicable, timing and observations. Captured text is sanitized
for **registered** secrets. That protects the report, but it neither proves the
program never emitted a secret nor removes every possible sensitive value.

| Existing story / entry point | Intended assertion slice when actually executed |
|---|---|
| Offline smoke | Version, completions and doctor via the selected executable. |
| Marina / `story_ship_service` | File lifecycle, byte/mode checks, fresh-cache reads, memoization and occupied-reference behavior. |
| Dana / `story_security` | Selected raw-output token checks, endpoint display and consent preservation. Not every help/error path. |
| Ravi / `story_automation` | Child exit 3 passthrough; a particular internal-failure marker plus 125; separate ref domains. Numbers alone are not lifecycle proof. |
| Priya and Sam / `story_team` | Authorized two-identity sharing and dedup; requires second identity. |
| Omar / `story_offboarding` | Scoped prune/erase, refusal and cold-read survival. Does not alone certify permanent offboarding, every catalog or backups. |
| Nadia / `story_quota` | A dedicated metered tenant's selected quota response. |
| Local adversarial/efficiency journeys | Real binary against controlled loopback responses and measured requests. Not live-service acceptance. |

Realistic fixture names are a test-design choice, not proof the service cannot
distinguish tests from customers. Test credentials/tenants must remain explicitly
authorized and disposable where appropriate; production data is not a fixture.

### Skip policies are not interchangeable

`CLW_CONFORMANCE_REQUIRE=1` makes primary missing-identity/capability branches fail
in conformance helpers. Optional second-identity and dedicated-infrastructure
journeys can still soft-skip; the replica-lag placeholder always soft-skips. A
provided credential ticket uses the exact consumed/expired 410 exception; other
redemption failures must not be misclassified as optional absence. Read the actual
selector and skip output; early returns can appear as libtest successes.

The evidence executable has two separate policies:
`CLW_E2E_REQUIRE_PROD=1` rejects skipped primary personas;
`CLW_E2E_REQUIRE_ALL=1` rejects **any** skipped persona. The evidence workflow enables
both. It is dispatch-only and can perform real writes; it is not an automatic
per-PR check. The weekly live-journeys workflow and daily conformance canary use
their own conformance policy, not evidence REQUIRE_ALL.

An unset or empty endpoint can select production in existing helpers. Before any
live run, resolve an explicit approved endpoint, tenant, credential source and
side-effect/budget/cleanup scope. The destructive path needs its own disposable
tenant. No call, provisioning, workflow dispatch, tag or expense is authorized
by reading this page. Tests with absent prerequisites remain unexecuted, not green.

### Non-vacuous observations

For persistence of **nonempty** content, cold-cache reads with expected bytes and
positive downloads, or direct authoritative store/request inspection, distinguish
remote survival from replaying a local copy. Empty snapshots legitimately download
zero file bytes. Do not rerun a producer and then call regenerated data preserved.

For redaction, `StepEvidence.leaked_secret` is computed before harness sanitation.
Check that signal or the intended raw output path; token absence from a sanitized
report merely re-tests sanitation. The existing runner test deliberately echoes a
synthetic registered marker to demonstrate this distinction. It is not a new token
exposure observation about the CLI.

For no-request/redirect claims, require evidence that the intended initial endpoint
was reached and the forbidden destination was not. For no-child claims, observe
an appropriate marker/counter and positive control in addition to the exit code.
For cleanup, distinguish owned temporaries, already published files and other
processes' resources. An error return or dropped handle is not automatically a
proof all filesystem work stopped.

## Acceptance and maintenance

The relevant work package selects evidence, not a universal checklist demanding
every lane for every change. Runtime, platform, coverage, live and release claims
need their own matching execution. Documentation needs accurate source-to-assertion
mapping, safe applicable examples and normal policy/reference checks. Neither can
silently waive a required missing result.

Each acceptance identifies source/tree, build/artifact when relevant, command and
selector, environment, terminal outcome, skipped/ignored/early-return scope and the
observed property. Keep failures and limitations visible. Review and integration
remain separate from producing a report; no automated metric certifies the absence
of all defects. The [#312 validation note](validation/2026-09-18-test-map-reconciliation.md)
records this map's actual review/check scope without claiming new product results.

[old-testing]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/docs/TESTING.md
