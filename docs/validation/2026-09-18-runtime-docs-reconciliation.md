# Runtime documentation reconciliation — #311 / WP-20

## Current source reconciliation — 2026-09-18

The current six reference pages now describe main source
`be96b8394c66d1dc36ce7ca2f186780f0ea48308` (0.1.12). The pending local
`5a401c07c191f1fc213d21989acbd35dc37afe91` follow-up was pushed and its
remote Git ref verified. Main was then merged into the existing PR branch without
rewriting its history. Relative to that main, the PR remains documentation-only.
The merge incorporates the existing #329/#297 and #334/#306 implementations; it
does not introduce or revalidate them as new fixes of this documentation PR.

The updated materializer description distinguishes checked per-file addition and
size equality, aggregate equality and fetched-byte integrity, all at their actual
boundaries. The preflight check precedes destination creation and materialization
chunk fetches, not root fetching/parsing. `Manifest::from_bytes` is unchanged.

The source index now points to the pinned main and corrected hydrate ranges.
Earlier source-specific results below are historical. Their source IDs and failed
attempts are preserved, not silently attributed to this newer combination.
The owner-delegated adversarial review route is recorded below. Its final verdict
and authorized main integration remain distinct from publication; a pushed commit
alone does not close #311, #312 or WP-20.

## Historical receipts

The sections below record earlier stages and their source identities. Statements
about pending pushes or older worktree bases describe those stages only; the
current checkpoint above supersedes their execution status.

## Scope and source identity — initial stage

Six current documents are being reconciled: CLAUDE.md, SECURITY.md,
ARCHITECTURE.md, THREAT-MODEL.md, GUIDE.md and the destructive-operations runbook.
The source worktree remains based on `4ff0fcfcd31fa467562f782852ef2f3c5554be24`.
Main's subsequent `2d624d75d0783d1362a21dbd9478b0546f24f27e` changes only hydrate
and CHANGELOG; its writer implementation was separately read at that exact commit
and is reflected in the integrated-source addenda. No old full-gate result is
relabeled as execution of this source combination. No runtime file is modified.

The failed worktree-update tool call ran no command. The original worktree,
other PRs and their source/evidence are preserved. The GUIDE edits in this
continuation succeeded; the earlier refusal no longer describes that file's state.

## Completeness criteria

| Discrepancy family | Current disposition |
|---|---|
| Lifecycle, bounds, blanket confinement | Threat model, architecture and guide name mechanisms and open #296/#298/#299/#300/#303 boundaries. |
| Run-key version/cwd and tool scope | Guide/SECURITY/architecture describe v3, declared inputs and non-hermetic limits; #301/#302 remain. |
| Parsed loopback policy | SECURITY describes the parsed IPv4/IPv6/localhost policy, not a raw prefix test. |
| Platform materialization | Guide separates Windows copies, native assertions and compilation. |
| Hex/chunk/format acceptance | Guide separates producer encoding and reader tolerance; #308 owns acceptance changes, #306 per-file equality. |
| Architecture and publication | Direct dependency kinds and one flat root; per-file publication, not tree transaction. Integrated #297 is described, not reimplemented. |
| Journey/map counts | Separate #312, not edited or closed. |
| Portable/Wine claims | Guide/agent guide distinguish configuration, compile-only and executed native evidence. |
| Uninstall/rotate/doctor | Guide now records executable retained, guidance-only rotation and offline diagnostics. |
| Historical deployment | Runbook avoids interpreting a historical server PR as a current availability check. |
| Post-read/up-front limits | Guide and architecture distinguish the post-read guard and post-chunk manifest cap. #291/#313 remain. |
| Conflict attribution | Guide describes local refusal; #309 owns the runtime diagnostic correction. |

The retained command/output tables were subsequently reconciled against their
handlers: cache scheduling and best-effort errors, fallible reporting after
storage effects, buffered run output, ls foreign-record counts, strict config
fallback, Unix versus Windows permissions, and unchanged-snapshot upload counts.
The original guide remains in Git and the pre-edit owner-local snapshot. This
source review is the author's review, not an independent approval.

## Success criteria

The six pages must consistently distinguish implementation, open issue, integrated
change and operational evidence. No filesystem side effect, credential rotation,
service probe, automatic retention or complete erasure may be promised without its
mechanism. Existing legal/reporting duties are not amended by this guide.

## Quality standards

Production anchors are indexed in ARCHITECTURE.md. Additional sources are the
CLI main/config, doctor/auth/uninstall/prune/erase/ls handlers, transport URL and
retry functions, Cargo member manifests and the configured release graph.
Validate references, anchors, retained help/output examples, actual diff and
applicable repository gates. Record terminal results against their source, never
count a source read or another PR's result as a fresh test. Independent review
remains separate from this author's review.

## Invariants

Documentation only: no Rust, dependency, lockfile, workflow, shell, secret, service
state or public release is changed. Runtime issues and #312 retain their own
acceptance. This change does not authorize live or destructive application tests.
No force push, bypassed hook, fabricated status or review verdict is permitted.

## Definition of done

Publish a bounded documentation PR with exact source identities and recorded
checks, complete factual review and current-base reconciliation, integrate under
the authorized gate policy, then verify publication before closing #311. Until
those steps complete, keep #311 and WP-20 open. No automatic closing keyword is
used by this note. Completing #311 does not complete #312 or all WP-20 criteria.

## Historical evidence and current acceptance

Earlier document checks and nine offline CLI checks remain in the sibling
`evidence/` directory. The original help-redaction assertion failed with a synthetic
marker and is tracked in #333. Help without a token is not a correction of #333.
No real token or production endpoint was used in that recorded smoke.

A prior separate Codex review hit its usage limit without a verdict. It is not
approval. New checks, publication identity and remaining acceptance are recorded
in the PR/update for this continuation rather than presuming success here.


## Retained-table follow-up — 2026-09-18

| Corrected claim | Pinned production evidence / observation |
|---|---|
| Cache maintenance is not an unconditional quota | [LocalCache::put/gc_to_budget](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/crates/clw-cache/src/lib.rs#L364-L431); existing-blob early return and per-instance PUT counter. Hydrate final sweep is best-effort; snapshot has no unconditional final sweep. |
| Reporting follows storage and may itself fail | [snapshot reporting](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/crates/clw-cli/src/subcmds/snapshot.rs#L77-L149); output writes do not roll back publication. |
| Unchanged ref does not imply zero chunk uploads | [snapshot no-op report](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/crates/clw-cli/src/subcmds/snapshot.rs#L124-L146); force can upload missing chunks before the unchanged-root return. |
| Listing classifies more than workspaces | [ls aggregate JSON](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/crates/clw-cli/src/subcmds/ls.rs#L386-L420); `foreign_ac_entries` is present. Zero workspace count does not establish an empty store. |
| Config warning/fallback is not universal command success | [config reader](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/crates/clw-cli/src/config.rs#L421-L452); required identity validation remains separate. |
| Erase results describe the selected enumeration | [partial/complete results](https://github.com/HuGR-Labs/corelink-workspaces/blob/4ff0fcfcd31fa467562f782852ef2f3c5554be24/crates/clw-cli/src/subcmds/erase.rs#L125-L203); no permanent-offboarding inference. |
| Doctor, rotate and uninstall effects | Fresh offline execution of the existing identified baseline binary: report masking, `rotated:false`, `binary.removed:false`, unchanged synthetic files and zero requests to a loopback sentinel. |

### Applicable checks and their actual scope

[REVIEWER-BRIEF, section 0](../../REVIEWER-BRIEF.md#0-frame-the-change-first-30-seconds)
explicitly makes Rust gates N/A for documentation-only changes. The plan's
WP-20-QS-02 and #311 require document checks and relevant safe examples, not a
fresh workspace rebuild for prose. The earlier draft's blanket full-Rust-gate
requirement was overbroad; this clarification does not waive a runtime gate or
relabel the failed preparatory target copy as a passed run. No workflow, classifier,
check implementation or threshold has been changed.

Sixteen selected static/policy commands passed, including DCO over the existing
nonempty commit range and the real PR-title checker. This is not a claim that
`gates.sh` or the Rust suite ran. DCO and whitespace must also pass for the final
commit. Structural verification resolved 90 local links and 24 fragments,
preserved all 22 guide section IDs, and verified the then-present 16 pinned
source references and ranges from Git objects. Those counts precede this receipt's
additional links; final structural results are attached to the PR.

Twelve fresh offline CLI checks passed using the **previously built**, unchanged
baseline executable (SHA-256
`c12813b78c4d846b360fe1472ddcc9d916fadb26a4a24b3eb5888b31381ec057`).
The eight version/help checks supplied no token; the four JSON checks used only
a synthetic marker. Assertions checked exits, report fields, unchanged fixture
files and zero requests to a temporary loopback sentinel. The sentinel, thread
and temporary fixture were closed/removed. This does not test the new hydrate
writer, recompile the binary or fix the existing token-in-help issue #333.

A fresh separate read-only Codex invocation on this follow-up exited 1 at its
usage limit, before writing a verdict. It is not approval. No alternative paid
reviewer, purchased credits or impersonated approval was used. Independent factual
review and authorized integration remain the acceptance blockers; the retained-table
editing task is no longer deferred. #311 stays open until its DoD is met.

Records for this continuation are under the owner-controlled
`clw-docs-311-20260918/finish-20260918T1712Z/evidence/` directory, with command results,
source hashes, offline logs and the failed review attempt. Its path alone does not
establish access by another reviewer; source claims and this bounded receipt are
versioned in the PR. Earlier attempts remain historical, not silently overwritten.

## Owner-delegated adversarial review — 2026-09-18

After the separate-reviewer attempts failed without a verdict, the owner explicitly
delegated this review to the assistant. This records a PR-specific review decision,
not a claim of another human/model identity or a general relaxation of review policy.
The same assistant conducted a new source-based adversarial pass. Its initial
FIX-FIRST disposition is [review 5250888008](https://github.com/HuGR-Labs/corelink-workspaces/pull/335#pullrequestreview-5250888008),
anchored to `f10e65c46dc03a67390209820ba1c222307c9d4f`.

| Finding | Correction | Production source at the documented baseline |
|---|---|---|
| DR-01 | Reporting instructions no longer imply all diagnostic paths hide token values; managed reports, parser help and arbitrary command output are distinguished. | CLI token declaration and doctor/auth report builders; #333 remains a separate runtime defect. |
| DR-02 | Toolchain text acknowledges the release preflight equality check without treating configuration agreement as compiler provenance. | `.github/workflows/release.yml`, `assert RUST_TOOLCHAIN matches rust-toolchain.toml`. |
| DR-03 | Output-cap wording describes direct-child termination request and the 137 mapping, not a universally observed SIGKILL. | `clw-run::run_memoized` capture branches and the CLI `run_err_exit_code` mapping. |

The changes are prose only. No runtime file, secret, workflow, existing guard or
legal commitment was changed. Final selected checks and the terminal verdict are
recorded against the resulting candidate in the PR review; this note does not
anticipate a passing command. Prior failures and source identities remain preserved.
The delegated review does not certify other queued PRs, close #320, or complete
#312's separate test-map obligation. Integration still needs the authorized
local-CI disposition, a matching head/base, and post-merge verification.
