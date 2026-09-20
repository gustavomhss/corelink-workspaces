# Destination namespace preservation — #303 / WP-04

Base: `7aec84509d20a7fffe371be2731d8e7134443dd4`.
Test-first commit: `83915012567c0d7e5f5b1bd4ef7e46d9ff781fc3` (PR #343).
This records implementation and native-volume evidence, not final merge approval.

## Contract and implementation

Hydration probes literal file, symlink and directory names, including implicit
parent directories, inside a private payload-free directory on the destination
filesystem before requesting file chunks. No lowercase or Unicode normalization
is substituted for the filesystem's actual name equivalence. Symlinks in the
manifest are represented by empty leaf placeholders; their targets are not followed.
Only identical literal directory prefixes are reused. Other collisions refuse
with an explicit namespace diagnostic. Canonical manifest bytes stay unchanged.

The namespace budget is 1,000,000 nodes, including implicit directories, using
borrowed source spellings. Exceeding it refuses before constructing the probe.
The temporary probe uses owner-only Unix permissions and is removed before payload
fetches. Failure can leave the initially empty destination directory, but not a
successful report for a partially representable tree. Cleanup errors prevent
acceptance and do not replace an earlier collision error.

Regular files retain private exclusive temporaries, complete bytes, final mode
and sync before publication. The already-locked tempfile 3.24.0 no-clobber primitive
refuses an occupied final name. Its hard-link fallback can leave the temporary
name; that name is explicitly removed before success, with the existing guard
retaining cleanup ownership on failure. There is no exists-then-overwrite fallback.

## Exact-name cleanup correction

The Linux shared-volume run exposed two probe-residue failures in the earlier
recursive cleanup path. The correction reuses the ordered namespace index and a
count of successful creates as an ownership journal. Cleanup walks only that
prefix in reverse order, using the original spelling and file/directory kind;
children are removed before parents. The failed alias and uncreated suffix are
never removed. No second manifest-sized name allocation is needed.

Automatic recursive cleanup is disarmed after validation, before exact-name
cleanup. Final empty-directory removal refuses unknown residue instead of
recursively deleting it. Cleanup continues over other owned names after an error
and preserves the first error; an earlier validation error remains primary.
Missing names are already absent, but other removal failures still block success.
This is ordinary completion cleanup, not a crash-recovery guarantee.

Three new unit tests cover reverse order, preservation of an uncreated suffix
and continued reclamation after a removal failure. The original namespace/writer
assertions are unchanged. On the formatted cleanup source, the 14 namespace
entries, all 74 hydrate-package tests and strict package Clippy passed on macOS.
Final Linux revalidation and full-candidate acceptance remain required; historical
failed coverage is not reclassified as successful.

The production correction was captured in a851164 while its commit title still
said diagnostic tests. That title is not the scope authority: its actual diff
contains runtime cleanup plus tests. The follow-up preserves history and records
this explicitly. The older 71/767-test results do not validate the cleanup fix.

## Executed evidence

Rust 1.91.1 on the connected Intel Mac, locked/offline Cargo and a task-owned
target/HOME/TMP. The six original regressions compiled on the unmodified runtime:
one healthy control passed and five desired-correctness assertions failed.
All six passed after this correction, without altering their assertions.
The first complete hydrate package passed 68 entries; strict all-target/all-feature
package Clippy passed. Three additional native/writer scenarios were then added.

The expanded nine-scenario matrix passed on the usual case-insensitive volume
and again on a private case-sensitive APFS image. Actual file probes confirmed
that the latter distinguishes ASCII case but aliases precomposed/decomposed
accented names. Both concurrency 1 and 4 branches in the selected loops completed.
A two-writer barrier test requires exactly one complete-file publication and an
AlreadyExists refusal for the other, without a partial final name or leftover temp.
Native tests compare bytes, directory counts, inode identities and unchanged
manifest bytes; they do not infer behavior from the operating-system label.

The sensitive image was detached and removed after execution. Logs/receipts are
retained outside the checkout at the owner-local task root:
`/Users/gustavoschneiter/clw-alias-303-n8rylmyb/evidence/`.
Original red log: `baseline-tests.log`; initial green: `implemented-regressions.log`;
expanded usual-volume run: `expanded-regressions.log`; sensitive-volume run:
`native-sensitive-regressions.log` and `native-sensitive-result.json`.
These overlapping counts are not additive distinct tests. Full current-candidate
CI, coverage, mutation and final review remain separately recorded in PR #343.

## Completeness criteria

WP-04-CC-01: literal names, implicit parents, file/file, file/directory, nested
paths and native normalization are covered by the namespace/writer matrix.
WP-04-CC-02: retain preflight cleanup and no-clobber publication checks, concurrent
writers and both measured native-volume behaviors. Final applicable CI is required.

## Definition of done

WP-04-DOD-01/02: the red/green native regressions and late-collision preservation
must be accompanied by final candidate checks, review, guarded integration,
post-merge identity verification and task-only cleanup before issue closure.
This implementation checkpoint alone does not satisfy those remaining steps.

## Invariants

WP-04-INV-01/02/03: never silently merge distinct source names, rewrite canonical
manifest bytes, replace existing data or expose incomplete bytes at final names.
The #297 pre-publication permissions and ownership rules remain in effect.

## Success criteria

WP-04-SC-01/02: preserve all representable identities and content, or explicitly
refuse without overwriting an earlier occupant; healthy names retain successful
restoration on sensitive and insensitive volumes.

## Quality standards

WP-04-QS-01/02: actual filesystem observations and original Rust production seams,
small fixtures, deterministic barriers, precise source identities and terminal
results. Same-assistant review is not described as independent approval.

## Boundaries and failure semantics

The probe assumes stable destination naming rules; it is not a lock against an
uncooperative process changing the destination. A late regular-file conflict is
still refused by publication itself. A failed publication may leave already
completed files; hydration is not a whole-tree transaction. If fallback temporary
unlink fails after publication, the call reports an error rather than pretending
cleanup completed, and never removes the final occupant.

Cancellation can leave the blocking probe running until its owned cleanup
completes; it cannot authorize payload publication after a cancelled preflight.
Ordinary sibling-writer quiescence remains #300, and complete symlink-graph
confinement remains #296. Windows symlink-copy expansion/dependency behavior
remains #298/#299. This change does not claim those issues fixed or certify native
Windows filesystem equivalence from a cross-build or Wine run.
