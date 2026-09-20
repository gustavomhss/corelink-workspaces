# Unix hydration confinement — issue 296

Historical design base: `2d624d75d0783d1362a21dbd9478b0546f24f27e` (PR 329 included).
Restacked implementation base: `2bd7eeae5e9707f133b5f70c42ff63099e51eaa1` (PR 343 / #303 included).
This is a Unix materializer-only admission correction, not a manifest-v1 serialization change.
The #303 filesystem name-equivalence preflight runs before this patch prepares physical
directories; the two guards are complementary rather than competing normalizers.

## Success criteria

Before creating destination entries, validate the complete tree and bounded link
resolution. Files and links cannot be parent directories of another entry.
Resolve symbolic targets before processing suffix parent components. A link pass
cannot use a manifest-created link as a directory for a later creation.

## Completeness criteria

Both named and direct-digest hydration use the same check. Prepare all implicit
directories before files and links. Reserve every link name with exclusive,
owner-only placeholders before publishing any link, so filesystem name aliases
cannot silently invalidate the checked graph. Clean only acquired placeholders;
published links are not owned by placeholder cleanup.

## Quality standards

Tests use the original hydrate implementation, real temporary files and synthetic
CAS/AC. The exact three-link #296 manifest is fed through the canonical parser and
both hydrate entrypoints, but the corrected materializer refuses it before creating
the destination; no historical out-of-root effect is reproduced. Pure graph tests
cover the same composition plus bounds/cycles, while healthy integration controls
preserve ordinary forward/relative links, bytes and target spelling. Existing #297
private-writer and #303 destination-name tests remain unchanged.

Validation commands: `cargo test --locked --offline -p clw-hydrate --lib`, the
original complete `bash scripts/gates.sh`, and applicable lint/DCO/title checks.
Actual results and the exact tested tree are recorded on the PR, not inferred
from these commands. A stopped build, missing review or skipped platform is not a pass.

## Invariants

No new dependency, unsafe code, global umask mutation, key/format/CLI change,
application credential, production request or change to other in-flight PRs.
The caller-selected root is canonicalized; a deliberate root alias remains valid.
All descendants are checked under that root. Windows keeps its current code path.

## Definition of done

The source and tests are committed, the applicable gate terminates successfully,
a scoped adversarial review records its actual reviewer provenance, and integration
verifies the exact reviewed/tested tree. Until these are present, issue 296 stays
open. This note does not certify release/deployment or native Windows behavior.

## Compatibility and bounds

Legitimate relative links, forward references, contained directory-back links and
normal dangling suffixes remain representable on Unix. Cycles and overlong
expansion detected in the literal-name graph, parent traversal after an unresolved
component and leaf-ancestor layouts are refused explicitly. This is not a universal
cycle classifier for alternate target spellings under filesystem name equivalence;
actual link-name collisions are separately rejected before link publication. Bounds are 40 expansions per target, 4096 pending components
and 4,000,000 resolution steps per operation. These are materialization admission
limits, not new canonical serialization rules.

Directory/file or directory/link conflicts can leave existing completed files or
plain directories from this operation: this is not whole-tree transactional
rollback. Failed link-name reservation occurs before any link publication and
removes only its own empty placeholders. An interrupted process may leave private
placeholders; no crash-scavenging or arbitrary deletion is introduced.

This correction excludes hostile concurrent local writers, sibling-writer
cancellation (#300), Windows copy semantics (#298/#299) and Windows ACLs.
Destination filename aliases are already handled by #303 and remain a separate,
complementary preflight; #296 does not claim that work again. No fixed-graph claim
is made for a directory being modified by another actor while hydration runs.

## Tracking correction

Issue 296 had been marked completed alongside issue 297, although PR 329 explicitly
excluded it and did not change link-graph admission. It was reopened with an
explanation, without changing its original evidence or creating a duplicate.
The reason for the previous closure was not established from the timeline.
