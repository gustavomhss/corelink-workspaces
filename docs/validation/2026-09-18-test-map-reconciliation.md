# Journey and JSON map reconciliation — #312 / WP-20

## Scope and source

Baseline: `e46e90b9a3308724da446d8dc3887cfc5c39a882`, 2026-09-18.
Changes are limited to TEST-JOURNEY-MAP, TESTING, the introductory documentation
comment in `crates/clw-cli/tests/e2e_portable.rs`, and this note. The portable
file's imports, attributes, functions and assertions are unchanged. No new
runtime test, schema, workflow, dependency or coverage service is introduced.
The preceding runtime-prose #311 is already integrated; #312 owns this distinct
assertion/selection map. Its correction does not complete other runtime issues.

## Completeness criteria

The six named false-absence findings were checked against their actual assertions.
Every original feature roll-up is mapped to the S/H/R/L/T/A inventory, and all
thirteen old ranked backlog entries have an explicit crosswalk. Remaining gaps
are retained, including native Windows, CLI retry after partial sweep, selected
JSON outcomes, power loss, global-ignore precedence and TTY-isolated color.

The JSON inventory distinguishes the production-builder top-level golden from
per-field CLI assertions and unsupported run/completions JSON modes. The lane
inventory distinguishes current hosted configuration, cross-compilation, Wine,
portable source, live helpers and the separate evidence binary. An existing live
suite is no longer presented as a future duplicate implementation.

## DoD — Definition of done

The source map and bounded comment change require factual/adversarial review,
reference and selector checks, unchanged-executable-text verification and the
applicable repository checks. Publish the exact reviewed candidate, verify
integration and its resulting files before closing #312. An empty remote check
list is not green; any authorized local-CI path retains its recorded scope.
Completion of a source map does not require implementing every mapped runtime gap.

## Invariants

Presence is not execution; execution without the load-bearing assertion is not
acceptance. A stale-HIT witness documents a limitation, not a hermeticity fix.
Power-loss, native Windows and production service results are not inferred from
ordinary library/CLI tests. No source, attributes, tests or selection logic is
removed to make the documentation true. Historical evidence retains its source
and dates. No real credentials, application data or parallel checkout is changed.

## Success criteria

A reader can locate an existing test by its exact name and source, identify its
entry point, explain what it actually asserts and distinguish the remaining gap.
The original six false absences disappear without converting narrower assertions
into broader coverage claims. Native/Wine/compiled/early-return states remain
separate; JSON coverage does not invent a universal envelope.

## Quality standards

Review the actual fixture and assertions rather than names/comments alone. This
review checked the post-clear hydrate recovery, NO_COLOR/non-TTY confound,
rename-versus-write failure, reduced pagination caps, marker/cold-cache/pre-scrub
boundaries, chunked-versus-malformed body framing and optional live skip branches.
Selectors and pinned source objects are checked locally; structural validity
alone is not semantic approval. Applicable check results and exact candidate
identities will be recorded on the PR; no new product-suite pass is claimed here.

## Review and performed verification

A separate adversarial pass by the same assistant checked the old roll-up against
75 explicitly identified journey groups and the 13-item ranked-backlog crosswalk.
This is not an independent human/model identity or approval of another PR.
The review narrowed five misleading inferences: re-run versus in-place recovery,
NO_COLOR versus non-TTY output, live-set marking versus actual sweep/read survival,
shared quota rendering versus run-specific CLI dispatch, and Wine/optional live
cases versus native/complete execution. These distinctions are in the map itself.

Seventeen selected static/policy commands completed with exit 0, without timeout:
whitespace, actionlint, repository-policy ShellCheck, existing release/permissions/
unsafe-posture/deadman/minisign/lane/target/title checks, rustfmt on the comment-only
target and locked offline Cargo metadata. Metadata resolves the cited Cargo target
families; it does not run their assertions. DCO is checked over the nonempty commit
range after committing. No Rust build, product test suite, live service request,
fuzzing, mutation campaign or native Windows run was performed for this change.

The initial local reference checker rejected the workflow inventory because its
filename expression accidentally excluded digits in `e2e-evidence.yml`. Correcting
that task-local expression made the inventory agree with the 14 actual workflow
files; no repository workflow or product check was weakened. The initial failure
is retained with the corrected result. The checker is a bounded local review aid,
not a new in-repository documentation or coverage framework.

Selector/source-object checks, old-to-new feature crosswalk, non-documentation
suffix comparison and final hashes are retained in the task-owned evidence
folder. Findings and exact candidate/check identities are published on the PR.
The comparison of the portable file starts at its unchanged imports and includes
all attributes/functions/assertions thereafter. This is comment-only source
identity, not a claim that its tests executed natively on any OS.
