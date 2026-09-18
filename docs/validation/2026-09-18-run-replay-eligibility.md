# Run replay eligibility — issue #307

The baseline sections below preserve their original execution scope. Current-main
integration is described in the final section; historical counts are not reused
as acceptance of a different tree.

Baseline: `4ff0fcfcd31fa467562f782852ef2f3c5554be24`.
Original runtime blob: `774c7f50ed790d798ae80bdb1a8329d6f6d9e67f`.

A parsed RunRecord with a nonzero exit code is ineligible for replay. It remains
structurally recognized, and `run_record_blobs` retains the same references for
listing and garbage collection. This is not deletion or repair of that record.

The fallback command runs once. A create-only occupied action-cache key can reject
storage of the new result; that does not change the real child result. Repeated
misses against the same ineligible occupant are intentional until a separate,
authorized lifecycle operation changes it. No automatic replacement is added.

## Executable regression

`cargo test --locked --offline -p clw-cli --test run_replay_eligibility`

The fixture invokes Cargo's actual `clw` executable against a loopback HTTP store,
with synthetic credentials, isolated HOME and a stable explicitly declared input.
A successful original run creates its own key and streams; the test changes only
the stored exit code while retaining the stream digests. AC PUT is create-only.

Before the runtime patch, four healthy controls passed and the desired nonzero
replay test failed: cached exit 42 was returned instead of the fallback's zero.
After the guard, all five cases passed. They cover zero HIT, nonzero MISS with
one child per invocation, failed-child non-storage, malformed and missing-stream
fallback, occupied-key preservation and unchanged GC reference extraction.

## Evidence boundaries

The test verifies reference extraction directly; it is not a proof of distributed
GC, production availability or a native Windows run. Its Unix scope is explicit.
The initial five-case revision did not apply or execute the proposed GC fixture.
That historical limitation is superseded by the test-only continuation below;
distributed GC, native Windows and production availability remain unverified.

Only the replay decision changes in production. The producer's success-only rule,
reference extraction, key derivation and exit/error mapping remain unchanged.
The concurrent error-phase change (#305 / PR #331) must be reconciled before
integrating these shared-file changes; it is not silently included here.

Raw baseline/patched logs, binary and test hashes are retained in the
owner-controlled `clw-replay-307-20260918/evidence/` directory. The PR receipt records
full-gate results against its exact tested tree after they have completed.
Review, merge and post-merge acceptance are separate from these local results.

## GC preservation continuation — 2026-09-18

`cargo test --locked --offline -p clw-cli --test run_record_gc --test run_replay_eligibility -- --test-threads=1`

The new Unix-only `run_record_gc` target drives the real snapshot/run/prune CLI
against an in-memory loopback HTTP store. It preserves a separate workspace ref,
creates the memo through the real producer and changes only its stored exit code.
The separate workspace prevents an empty-live-set refusal from masking lost memo
references. A genuine orphan must be reclaimed; refusing all deletion is not a pass.

### Completeness criteria

Three new cases cover nonzero and zero memo stream retention plus the existing
empty-live-set refusal. Dry-run preserves both stores. The confirmed sweep removes
only the orphan and preserves all reference bytes. The original five replay cases
run alongside them; the two targets completed with eight passed and no failures.

### Success criteria

A fresh HTTP client reads both streams after sweep and the fixture records new
CAS GETs. Verification never invokes the child again, so fallback cannot repair
missing streams and conceal deletion. The actual child execution count stays one.

### Quality standards

The attached draft was imported with its exact Git blob, then rustfmt changed
only formatting. The first shared-target invocation passed GC but failed replay;
that failed log is retained, not treated as current-candidate acceptance. Source
inspection confirmed the expected replay guard. A dedicated copy-on-write target,
with all 13 first-party package artifacts removed from that copy, rebuilt the
workspace code and passed both suites. The original shared target was not cleaned.
Future runs of this continuation use the dedicated target, not a path shared
between worktrees. Command and binary identities are in the execution receipts.

### Invariants

This continuation changes only tests and this note. No replay/GC runtime policy,
key format, dependency, quota, credential scope or foreign worktree is changed.
Only synthetic in-memory stores are swept; no application service is contacted.

### Definition of done

Current-head applicable gates, independent review, integration and post-merge
verification remain necessary. A full-gate result belongs to its recorded tested
tree; it must not be inferred from the eight focal tests. Receipts, including the
initial failure, are retained under the owner-controlled
`clw-replay-307-20260918/gc-acceptance-20260918/` directory and summarized in PR #332.

## Integration with #305 — 2026-09-18

Integration base: `e46e90b9a3308724da446d8dc3887cfc5c39a882`.
The existing PR branch incorporates #331 initial-lookup error normalization,
#329 private temporary publication, #334 per-file metadata validation and #335
runtime documentation. Only the shared lookup block needs semantic reconciliation:
`.map_err(initial_lookup_error)?` remains outside the replay-record filter, so an
oversized initial response is still a pre-execution failure. A parsed nonzero
record instead follows the normal one-child fallback. No record deletion,
forced replacement, new exit code or shared public type is introduced.

### Success criteria

The combined tree preserves both initial-lookup phase classification and replay
eligibility. Valid zero records replay exact streams without a new command;
nonzero records remain GC roots without authorizing replay.

### Completeness criteria

Run the eight real-CLI replay/GC cases together with the original complete gate,
including the #305 lookup/output and passthrough regressions. Inspect current
source identity and keep prior main changes intact. An occupied create-only memo
may cause repeated safe misses; this patch does not add a repair lifecycle.

### Quality standards

Use the task-owned dedicated target, rebuild first-party packages after combining
worktrees, retain raw terminal logs and report filtered/ignored tests explicitly.
Original code and real child effects, not a recording compiler or a status string,
provide behavioral evidence. The owner-requested adversarial review is performed
by the integrating assistant and is not an independent-model approval.

### Invariants

Initial lookup failure is not a child result. Replay ineligibility is not garbage
eligibility. No new execution from a valid HIT and at most one fallback per call.
No mutation of existing memo records, altered key/serialization, weakened body
caps, public disclosure or production action.

### Definition of done

Fresh applicable validation and review must identify the exact integration tree.
The PR records commit/push, merge authority and post-merge tree verification;
until those receipts exist this section describes acceptance requirements, not a
completed merge. Native Windows, live conformance, new coverage measurements and
release/signing remain outside this bounded integration.
