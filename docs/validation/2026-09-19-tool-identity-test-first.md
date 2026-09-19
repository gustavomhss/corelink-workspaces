# #301 / WP-09 — executable identity, test-first checkpoint

> Historical baseline at `242afb5...`: production was unchanged at that checkpoint.
> The [Unix implementation and its boundaries](2026-09-19-unix-tool-identity.md)
> supersede that live status; baseline failures below remain evidence, not current results.

**Historical status: desired-correctness regressions executed; production fix not yet applied.**
This change is not merge-ready and does not close #301 or WP-09.
Baseline: `ae30b4b5e035eadf331702a6b2f49e531820bddd`.
Platform: Intel macOS, Rust 1.91.1, locked/offline Cargo, two build jobs.
The real `clw_run::run_memoized` and real subprocesses run with isolated
process environments and temporary files; only CAS/AC are transport doubles.

## Executed baseline

```sh
cargo test --locked --offline -p clw-run \
  --test tool_selection --test tool_selection_binding --no-run
cargo test --locked --offline -p clw-run \
  --test tool_selection --test tool_selection_binding --no-fail-fast \
  -- --test-threads=1 --nocapture
```

Compilation completed with exit 0. Tests completed with exit 101:
`tool_selection`: 4 passed / 5 failed; `tool_selection_binding`: 1 passed / 3 failed.
The 13 harness entries contain three inert subprocess helpers. Thus there are
**10 substantive scenarios: two healthy controls passed and eight desired
regressions failed**, not 13 independent product scenarios or a green CI result.

The healthy controls cover a symlinked tool/no-shadow path and native argv0/cwd.
The failing cases cover a non-executable PATH shadow, permission promotion,
legacy-v4 polluted memo reuse, missing/unreadable-tool lookup, selection changing
at the AC boundary, selected-file replacement, and post-dispatch memo publication.
The first execute-only fixture incorrectly required a copied Mach-O to launch
without read permission on macOS. That precondition failure is retained, excluded
from product evidence, and removed from the corrected fixture. The corrected
case establishes effective unreadability and requires refusal before AC lookup;
it does not claim the OS supports execute-only launch or infer phase from exit 137.
Each parent removes its previous result file before launching its worker: an inert
worker or an empty selector cannot satisfy assertions using stale output.

## Provenance and limits

Baseline compile log SHA-256:
`bb880ca20169928dbba827123aa445cb847d57b2f0c7fde5932458d86af26721`.
Baseline test log SHA-256:
`a1b9090fa53cc0ba5f9936d9e48b142e822b4531224a7ec377fbca438d130f21`.
Owner-local evidence is retained in
`/Users/gustavoschneiter/clw-tool-301-_js52bmk/evidence/execution-resume-20260919/`.
The earlier logs remain separate. These are not GitHub-hosted artifacts.
No full gate, coverage, mutation, native Windows or release acceptance is claimed.
Runtime code and dependency/build configuration remain identical to baseline.

## Five acceptance axioms — pending implementation

### Completeness criteria
Bind executable selection, readable identity, key and launch; retain both test
files and the #302 environment/legacy-GC contract. Resolve explicit platform,
PATH, interpreter and permission policy; these baseline tests alone are not complete.

### Success criteria
Changing the actual selected tool prevents stale reuse. Healthy repeat runs hit.
A selected tool cannot be silently replaced by a second PATH search after lookup.

### Invariants
No weak metadata/literal identity authorizes a HIT; no discovery subprocess,
secret fixture, legacy-record deletion, duplicate child or claimed atomic-exec lease.

### Quality standards
Keep baseline-red and fixed-green separate. Assert real output, effects, keys,
HIT/MISS and storage changes; use distinct healthy and mutation targets.

### Definition of done
Apply and validate the production correction, finish all applicable original local
checks, publish the owner-delegated review, integrate the exact accepted head,
verify post-merge identity and remove only this task's completed resources.
No failing test is ignored, weakened or merged merely to complete this checklist.
