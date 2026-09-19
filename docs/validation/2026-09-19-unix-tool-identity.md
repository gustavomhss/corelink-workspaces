# Unix selected executable identity — #301 implementation scope

Base: `ae30b4b5e035eadf331702a6b2f49e531820bddd`; test-first checkpoint:
`242afb5ada2be405309fb2d41b539f283aaff2f7`. This is a source-level Unix
implementation slice under the owner's execution/review delegation, not a
release or a declaration that every platform obligation is complete.

## Decision and behavior

The existing `clw/run/v4/` domain is retained. Its Unix `tool` field now has the
explicit `unix-executable-v1:` prefix and a lossless native-path-byte/content-digest
payload. That representation is disjoint from the old `digest:`, `meta:` and
`literal:` tool fields, so old incorrectly selected v4 memos are not reused.
Records and their stdout/stderr references remain recognizable by GC; nothing
is deleted or rekeyed. A downgrade to the old field encoding may reactivate old
incorrect memos and is not a safe rollback.

Unix selection checks regular-file type and effective execution permission using
the already-locked rustix safe API, rather than testing arbitrary executable mode
bits. The selected path is hashed with the existing bounded fingerprint cache.
Unreadable or unstable identity is explicitly refused before memo lookup; metadata
or literal fallback cannot authorize a Unix HIT. The same selected path constructs
the process, with native argv0 preserved, original arguments and child cwd. No
probe/discovery subprocess is executed.

Relative command paths and relative PATH entries resolve against child cwd.
For PATH consisting entirely of empty entries (`""`, `":"`, `"::"`), preserve
its exact captured value on the child and use the original bare spelling. All
search entries identify the same cwd location; no other directory can be selected.
This preserves Linux script `$0`, unlike unconditionally prefixing `./`.

On Linux, selecting an empty component from a mixed-directory PATH is explicitly
refused before lookup. Use an explicit `./tool` or `PATH=.`. This is a conservative
compatibility restriction: do not change the script's output or inherited PATH,
and do not silently select a later executable. A nonempty winning component is
unaffected. Existing native-oracle assertions remain unchanged; new controls cover
empty/colon-only values, the exact child environment, refusal and explicit recovery.

A **missing PATH** with a bare program is intentionally refused with guidance
to use an explicit path, rather than guessing OS default directories. This is
stricter than native default search, not an unchanged compatibility promise.
An explicit path with PATH absent still works.

Fingerprints and execution permission are checked after the AC wait, before
returning replayed streams, and before spawn. Detected post-execution drift
suppresses memo publication while preserving the actual child result. The
checks detect the tested stable-file changes; they are not an atomic descriptor-
bound exec, an ABA guarantee, a sandbox, or transitive interpreter/toolchain
identity. Ordinary scripts still rely on their interpreter. Input/environment
mutation remains a separate contract boundary (#326).

The #302 declared-environment rejection and v4 isolation stay intact. No-dispatch
means this clw invocation did not dispatch; an enclosing optional Action may
execute its own fallback. Numeric exit values do not prove lifecycle state.

## Platform boundary

The non-Unix branch is deliberately unchanged in this slice. In particular,
Windows bare-name resolution is **not** assumed to follow shell PATHEXT rules:
the pinned Rust implementation uses application/system/Windows/PATH search and
special .exe handling. A Unix test, Windows cross-build or Wine startup failure
cannot establish native Windows identity parity. #301 stays open until that
remaining disposition and applicable acceptance are completed. No required
platform check is waived by this document.

## Success criteria

The non-executable PATH-shadow and permission-change specimens cannot mask a
changed launched tool. A selected tool stays bound across lookup; detectable
drift cannot authorize replay/new memo. Healthy repeats retain HITs and real
child output/argv0/cwd.

## Completeness criteria

Cover CLI and original library, historical v4 identity, effective permissions,
missing/unreadable tools, relative/empty/absent PATH, symlinks, lookup-to-launch
binding, post-dispatch no-store, and the existing #302 controls. Preserve other
run/replay/error behavior and every unrelated source change.

## Quality standards

Use real tiny files/native child processes, controlled CAS/AC, desired red baseline
and green corrected assertions. Count helper entrypoints separately. Keep raw logs,
source identities and healthy versus mutated build targets separate. Native
Windows and exact Linux coverage remain distinct from Mac results.

## Definition of done

This implementation checkpoint is not merge-ready or issue closure. Require
applicable original local gates and supplemental checks on the exact candidate,
owner-delegated same-assistant review, and a final platform disposition before
normal guarded integration and post-merge/hygiene verification. No untested scope
is claimed repaired.

## Invariants

No weak Unix tool identity authorizes memoization. No extra command discovers or
repairs identity. No rejected OS values in diagnostics. No old-record deletion,
unsafe FFI, global permission/environment change, weakened checks, production
fixture, release, deployment, or spending.

## Mutation review follow-up — 2026-09-19

The 32-candidate automatic diff run at `155b5a9` completed with 23 caught,
4 unviable and 5 missed, with no timeout. This was a failed acceptance, not
32 passing product checks. Two missed expressions were in Linux-only code
and were not compiled in that Mac execution; they are not evidence of a
Linux test pass or a Windows result.

The follow-up preserves the existing selection/capture predicates but extracts
them into private, production-used functions with direct truth-table tests.
The Linux predicate can be tested on a non-Linux test host without pretending
to run the Linux launcher there. Original Linux launch integration remains a
separate requirement. No mutation exclusion or threshold was changed.

Real-file integration tests now require an unexpected lookup error to stop
selection before a later executable or memo is used. Missing candidates and
permission-denied candidates must retain their distinct I/O classifications.
Fixtures remain bounded and disposable; no source or workload outside the
fixture is executed or changed.

The updated clw-run package passed 84 harness entries on macOS, the original
CLI PATH-shadow test passed, and strict package Clippy passed. These focal
results do not replace final-tree full gates, Linux coverage or a new mutation
result. The failed mutation log and all prior results remain outside source;
#301 remains open until the remaining acceptance and platform disposition.
