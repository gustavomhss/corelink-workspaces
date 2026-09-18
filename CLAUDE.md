# CLAUDE.md — project guide for Claude Code

> **CI provider override — 2026-09-17, explicitly approved by the owner:** this
> repository was made public temporarily and is private again. Every workflow
> uses standard GitHub-hosted runners. This supersedes older self-hosted-only/private-source instructions
> below; it does not change the proprietary license, release destinations,
> signing requirements, live-test credentials or merge gates. Making the repository
> private again does not revert runner labels and may reintroduce hosted-minute
> quota/billing restrictions. Other CoreLink repositories are not changed.

## Source and operational state

This guide describes commit `be96b8394c66d1dc36ce7ca2f186780f0ea48308`,
workspace version 0.1.12, reconciled on 2026-09-18. A manifest version is not a
claim about the latest published release. Consult current PRs and
[tracker #325](https://github.com/HuGR-Labs/corelink-workspaces/issues/325)
at every resume. Dated handoffs and the Git history preserve earlier observations;
do not turn an old green run, release assertion or branch-policy statement into
current evidence.

The branch API observed during this reconciliation reported `protected=false`.
That does not establish the absence of all inherited rules. The intended process
below remains mandatory independently of that observation; #320 owns effective
policy and Actions execution verification. Recorded billing/startup failures are
not failed Rust assertions, and local validation is not a fabricated remote check.


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
policy remains #319 until its patch lands; a local shortcut cannot certify CI.

[TESTING.md](docs/TESTING.md) and the workflows define supplementary coverage,
fuzz, mutation and platform lanes. Their existence is not a passing run.
Record exact selectors, nonempty expected counts, source/tree SHA and terminal
results. Coverage, live conformance and signing have distinct acceptance records.

## Integration and hygiene

- DCO: commit with `-s` using the real configured identity. Use Conventional
  Commits titles and the existing DCO/title checks.
- Short branch → reviewed PR → accepted evidence → squash merge → post-merge
  verification. Independent cold review is not the author's second reading.
  Keep one active integration item; coordinate queued drafts and shared files.
- Run the applicable gate and `scripts/pre-merge-gate-check.sh`; an explicit
  owner-authorized exception must be recorded with its scope. It never turns
  missing tests into successes or substitutes for independent review.
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
