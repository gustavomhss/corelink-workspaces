# CLAUDE.md — project guide for Claude Code

> **Local CI — owner decision, 2026-09-18:** execute the same applicable checks
> locally, without GitHub Actions. This is the campaign's selected execution mode,
> not an exception requiring renewed authorization for each PR. GitHub remains the
> repository, review and merge surface. Missing Actions jobs are not, by themselves,
> an integration blocker or evidence that a local check failed. Local failures,
> incomplete required results and stale evidence still block acceptance.
>
> Historical provider configuration: workflows were migrated to GitHub-hosted
> runner labels on 2026-09-17, and the repository was made private again. Those
> labels describe the retained YAML, not a requirement to execute this campaign
> through Actions. This decision does not disable workflows, change protections,
> purchase capacity, authorize releases or alter another CoreLink repository.

## Source and operational state

This guide describes commit `be96b8394c66d1dc36ce7ca2f186780f0ea48308`,
workspace version 0.1.12, reconciled on 2026-09-18. A manifest version is not a
claim about the latest published release. Consult current PRs and
[tracker #325](https://github.com/HuGR-Labs/corelink-workspaces/issues/325)
at every resume. Dated handoffs and the Git history preserve earlier observations;
do not turn an old green run, release assertion or branch-policy statement into
current evidence.

The branch API historically reported `protected=false`; that is not a current
proof of the absence of inherited rules. #320 records the Actions-startup diagnosis;
its closed state does not establish that hosted execution or billing was repaired.
The local-CI decision above replaces dependence on Actions execution for this
campaign. Preserve actual review/merge protections and keep local results distinct
from remote statuses. No fabricated success status is needed or permitted.


> **Integrated changes at this source:** [PR #329](https://github.com/HuGR-Labs/corelink-workspaces/pull/329)
> supplies Unix hydrate temporary ownership and pre-publication permissions;
> [PR #334](https://github.com/HuGR-Labs/corelink-workspaces/pull/334) supplies
> checked per-file size validation. Both are already in the pinned main commit.
> Earlier test records retain their original source identities; this documentation
> update is not a new runtime-test or release result.

## What this is

`clw` snapshots selected files, hydrates into a new/empty destination and memoizes
local command stdout/stderr/exit over CAS and AC. It is not a VM checkpoint,
remote scheduler or sandbox. Auth is Bearer PAT with tenant in the URL path;
configuration can be entirely environment-driven. A config file is optional.
The owner is Gustavo; confirm authority for spending, publishing or destructive
operations instead of treating repository write access as blanket permission.

Read [ARCHITECTURE.md](docs/ARCHITECTURE.md), [GUIDE.md](docs/GUIDE.md) and
[THREAT-MODEL.md](docs/THREAT-MODEL.md) for mechanisms and their limits.
Outstanding runtime fixes stay outstanding until accepted and integrated:
#291 bounded small-file read; #292/#294 refs/root repair; #293/#295 GC;
#296/#300/#303 hydrate boundaries; #298/#299 Windows copies;
#301/#302/#305/#307/#310 run identity, replay and delivery. Do not silently
implement a new product feature to correct a documentation claim.

## Build, tests and gates

The pin is Rust 1.91.1 in `rust-toolchain.toml`. Use isolated, campaign-owned
application HOME/cache/temp paths and bounded jobs on a shared host. Preserve
other worktrees, in-progress builds and their caches. Never change a global
compiler installation to make one task pass.

```sh
cargo build --locked
cargo test --workspace --locked
bash scripts/gates.sh
```

`gates.sh` has no supported `--scoped [base]` interface. Local and main runs take
the full lane. A trusted pull-request context selects the automatic diff lane:
Markdown-only skips Rust; runtime and ambiguous changes retain the appropriate
Rust checks. Read `scripts/classify-lane.sh` and the actual gate, not a historical
command recipe. Cross-lint is not native Windows execution. The local cross-skip
policy is enforced by `scripts/ci-cross-policy.sh` before setup or lane selection
(#319 / PR #330). Only `CLW_SKIP_CROSS=1` requests the shortcut. Both `CI` and
`GITHUB_ACTIONS` must be unset, empty, `false` or `0`; any other nonempty marker
refuses it. A legitimate docs-only lane is separate, and a local shortcut cannot
certify a required cross check.

[TESTING.md](docs/TESTING.md) and the workflows define supplementary coverage,
fuzz, mutation and platform lanes. Their existence is not a passing run.
Record exact selectors, nonempty expected counts, source/tree SHA and terminal
results. Coverage, live conformance and signing have distinct acceptance records.

### Local execution and equivalence

Before running a candidate, map its applicable workflow/job checks to the original
repository commands. Keep their selectors, features, targets, pinned tool versions,
exclusions and thresholds. Replace the scheduler and artifact-upload destination,
not the assertions. Record a reason for every genuinely inapplicable check; never
reclassify a missing required environment or a failed command as inapplicable.

For a full code/CI acceptance, run the unchanged `bash scripts/gates.sh` without
`CLW_SKIP_CROSS=1`. A local iteration shortcut remains an iteration shortcut, even
when CI markers are absent. Use an isolated application environment and do not
inherit another job's CI/event variables or claim a synthetic context was Actions.
DCO, title, shell/workflow lint and job-level guard self-tests remain separate
where their workflow commands are not covered by that gate. Use the actual PR
title and commit range, not a passing placeholder.

`ALL GATES GREEN` is the result of that script, not the result of every workflow.
In particular, [coverage.yml](.github/workflows/coverage.yml) is a separate check:
it uses cargo-llvm-cov 0.9.0, workspace default features, excludes
`clw-e2e-evidence`, ignores its source paths in reports, and enforces the 88% floor.
Preserve that configuration when reproducing coverage locally; do not replace it
with an uninstrumented test count. Use the same target/platform for a claimed
comparison. Mac results do not certify Linux coverage or native Windows behavior.

Preserve the existing scope of docs-only, diff-mutation, scheduled stress/fuzz,
live-conformance and release checks. This provider change neither makes every
scheduled/destructive lane a per-PR test nor silently waives a required lane.
A locally hosted appropriate environment is valid; compiler output, Wine smoke
and native runtime evidence remain different claims.

Keep raw logs and reports outside the checkout, with command, start/end, exit,
platform/toolchain, source/base/head/tree identities, test counts and log hashes.
A failed, timed-out or interrupted required command prevents a completed local
acceptance. Preserve its evidence and resolve the cause before integration.

## Integration and hygiene

- DCO: commit with `-s` using the real configured identity. Use Conventional
  Commits titles and the existing DCO/title checks.
- Short branch → reviewed PR → accepted evidence → squash merge → post-merge
  verification. Independent cold review is not the author's second reading.
  Keep one active integration item; coordinate queued drafts and shared files.
- Accept the candidate through the local-CI mode above: all applicable local
  checks must have terminal results and the required technical review must have
  a recorded disposition. This mode changes the executor, not review identity.
- `scripts/pre-merge-gate-check.sh` reads remote Actions checks. Its refusal solely
  for absent Actions results is not the local acceptance verdict; do not retry
  Actions or request another provider exception just to make that script green.
  Conflicts, changed heads, real protection requirements and missing/failed local
  checks still block. Leave the script and remote status history truthful.
- Immediately before a head-guarded merge, verify the actual base/head and the
  accepted local evidence. Afterward verify merge parent, tree and changed-file
  hashes. Close the issue only after its own criteria and post-merge checks pass.
- Never force-push main, discard unrelated work, use `--no-verify`, fabricate
  checks or auto-close an issue with operational acceptance still pending.
  The executor owns review, integration, receipts and cleanup of its own work.
- Fetch and compare actual base/head before integration. Preserve valid evidence
  across docs-only receipts, but repeat affected verification for material changes.
  Keep logs outside the source tree and retain only concise validation notes there.

## Release and distribution

`release.yml` is the source of the five-target build/sign and verification graph.
At this commit, builds run on hosted Intel macOS; the ARM Darwin target and
Linux/Windows GNU targets are cross-built, while Authenticode verification runs
on native Windows. Real releases require their live conformance and signature
proofs. Source configuration is not proof that secrets, runners or released
assets have met these requirements.

An RC/tag/draft can cause external work and uploads; it is not automatically
private, harmless or reversible. Obtain the relevant authority before creating
one. #321 owns RC scheduling, #322 artifact-set retry identity, #323 distribution
verification. Retrying must not silently replace an accepted asset set.
Homebrew publication can be optional under the existing policy; record the
actual selected channels and waivers, never infer universal delivery.

The source remains proprietary; EULA governs use. Binary download destinations
are the public mirror `HuGR-Labs/clw-releases` and tap `HuGR-Labs/homebrew-clw`.
Consult [INSTALL.md](docs/INSTALL.md) and [DISTRIBUTION.md](docs/DISTRIBUTION.md)
for a specifically selected release. Do not assert current signing or residency
from older release notes. #324 owns the independently delivered observer; the
Actions deadman cannot alone demonstrate its own scheduler's liveness.

## Cross-team seam

Consumers drive `snapshot → hydrate → run`; see
[CLI-SURFACE-EXIT-FREEZE.md](docs/CLI-SURFACE-EXIT-FREEZE.md).
`run` forwards the child's exit and records successful producer results, but
125/137 can also be legitimate child exits and do not alone prove lifecycle or
authorize retries. HIT restores streams, not output files. The user/runner ref
hash domains avoid naming collisions, not access-control checks. Server changes
are coordinated explicitly rather than assumed from a client-side method.
