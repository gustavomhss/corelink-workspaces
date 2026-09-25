# clw CLI surface + exit-code convention — FROZEN (v0.1.0)

> **Authoritative freeze** of the `clw` verbs/flags and exit-code convention the CoreLink **runner**
> drives. Requested by the Runners TL (2026-06-17 `relay-to-clw-tl-cli-and-exit-code-freeze.md`).
> The original freeze is v0.1.0; additive changes and corrections are recorded below.
> The 2026-09-18 clarification preserves the numeric interface and withdraws the
> unsupported claim that an exit value uniquely proves execution history. The
> additive execution-state receipt is documented below; it does not change exits.

## 1. The driven verbs (exact surface)

### `clw snapshot [PATH] --name <NAME>`
- `PATH` — positional, the workspace directory to snapshot; **defaults to `.`** if omitted (`crates/clw-cli/src/subcmds/snapshot.rs`).
- `--name <NAME>` — **required**, the workspace name to store the snapshot under.
- Effect: chunk + content-address + upload; writes the AC ref for `<NAME>` (in the active ref domain).
- **ADDITIVE (post-freeze, PR #143 — new flag + softened error, not a contract change):**
  re-snapshotting a name with **identical** content is a **no-op success** (exit 0; prints
  `already up to date`) — it does **not** error. Re-snapshotting **differing** content still
  rejects (HTTP 409) unless the new `--force` flag is passed, which does an rm-then-write.
  Both are additive (new flag / a former error path now succeeding) and so are permitted under
  this freeze; no frozen exit code or arg meaning changed.

### `clw hydrate <DEST> (--name <NAME> | --manifest-digest <HEX>)`
- `DEST` — positional, destination directory (must not exist or be empty) (`…/subcmds/hydrate.rs`).
- `--name <NAME>` — the workspace name to materialize (one of `--name` / `--manifest-digest`, required).
- **ADDITIVE (post-freeze — new alternative flag, not a contract change):**
  - `--manifest-digest <HEX>` — materialize a snapshot **directly by its manifest digest**, skipping the
    named-ref (AC) lookup. Mutually exclusive with `--name`; exactly one of the two is required. The fetched
    manifest is verified to hash to the digest (self-verifying: the ref equals the content key). This is the
    content-addressed seam a runner uses when it already holds the digest (e.g. `toolchain_ref = <digest>`
    for the CF-native check-host). Purely additive (a new flag + a new way to identify the same snapshot),
    so permitted under the freeze. See `docs/REPLY-clw-TL-toolchain-resolver-seam-2026-06-26.md`.

### `clw run --input <PATH>… --env <KEY>… -- <CMD>…`
- `--input <PATH>` — repeatable; the file/dir paths that form the memoization key (`…/subcmds/run.rs`).
- `--env <KEY>` — repeatable; env var **names** folded into the key.
  #302: declared values must be representable as UTF-8; unsupported OS values
  fail before lookup/spawn **inside this clw invocation**, through the existing
  internal-error path (125). An enclosing Action in `optional` mode may run its
  own cold fallback only with the explicit `CLW_RUN_STATE_FILE` receipt below.
  The internal run-key domain moves to v4 (one cold memo domain); no flag,
  process code or RunRecord schema changes. A downgrade to a v3 client can
  reactivate incorrectly keyed legacy memos and is not a safe recovery path;
  subsequent repairs must preserve the v4 separation and rejection. See
  [declared environment compatibility](GUIDE.md#declared-environment-compatibility-302).
- `-- <CMD>…` — everything after `--` is the child command + args, run verbatim.
- `clw run` proxies the child's stdout/stderr and has **no `--json` form** (the child owns stdout).

### Identity / domain (all verbs) — set by the runner via env, NOT flags
- `CLW_ENDPOINT`, `CLW_TENANT`, `CLW_TOKEN` (Bearer PAT; tenant-in-URL-path; never `x-corelink-tenant-id`).
- `CLW_REF_DOMAIN=runner` → keyspace `clw/ref/runner/v1/` (disjoint from the user domain).
- (CLI flags `--endpoint/--tenant/--token/--ref-domain` exist but the runner uses env.)

## 2. Exit-code convention (the load-bearing freeze)

### `clw snapshot` / `clw hydrate` (no child process)
| Exit | Meaning |
|---|---|
| `0` | success |
| non-zero (`2`) | **clw-internal failure** (no child involved) — substrate/config/IO error or bad args (`main.rs::exit_err` → 2; clap usage errors also 2) |

### `clw run` — exit values and their known origins

The numeric interface remains unchanged. The following rows describe **known
code paths**, not an inverse mapping from a process exit value to execution
history. Different paths can return the same integer.

| Known outcome origin | Exit | What that path establishes |
|---|---|---|
| A child completed, or an existing record was replayed | `N`, passed through verbatim | Completion/replay returns its outcome; a HIT does not start a new child. New memos are produced only for exit-zero execution. Replay eligibility is the separate #307 correction. |
| Configuration/identity failure before dispatch, a failed spawn, or the oversized initial AC lookup covered by #305 | `125` | These particular paths do not execute a child. Their phase is established by their control flow, not by the number alone. |
| The wrapper's captured-output cap terminates a started child | `137` | This cap path returns an error and does not publish a memo. A child may independently return the same number. |
| Invalid CLI arguments rejected by clap | `2` | This parser path does not start a child. A legitimate child may also return 2. |
| Other wrapper errors handled by `run_err_exit_code` | `125` | The existing fallback mapping remains; #305 does not establish the execution phase of every other error, including errors after dispatch. |

### Additive execution-state receipt for wrappers

When `CLW_RUN_STATE_FILE` names a fresh private path, `clw run` writes one strict ASCII state line:
`NOT_STARTED\n`, `DISPATCHING\n`, or `EXECUTED\n`. The receipt is control metadata and is removed
from the child environment. `NOT_STARTED` is initialized before run setup; immediately before OS
spawn the state becomes `DISPATCHING`; a known spawn failure restores `NOT_STARTED`. `EXECUTED` is
written only after the direct child's exit status is observed and its bounded stdout/stderr capture
succeeds. Each state replacement is written to a sibling temporary file and atomically renamed. If
dispatch may have happened, the state stays `DISPATCHING` when wait, pipe, I/O, or output-cap
handling fails. Missing, malformed, unreadable, or unknown state means unknown/possibly executed.

An optional consumer may make one cold fallback only when the command failed and the final receipt
is exactly `NOT_STARTED`. `EXECUTED` preserves the observed child verdict, including 125. All other
states fail closed. This adds no flag, changes no ordinary invocation or numeric exit value, and
does not interpret stderr, cache HIT text, or the child's output as execution proof.

Child exits 125 and 137 remain legitimate passthrough values. Asking commands to
avoid them is not an enforced discriminator. The wrapper's diagnostic strings
are for people: the existing `child not run` wording does not provide a reliable
machine protocol for every error path, and a child's stderr can contain the same
text. Do not infer execution history by searching stderr for that phrase.

### Consumer rule — preserve the result, do not infer retry safety

Preserve the observed process result. **Do not automatically execute the command
again solely because the result equals 125, 137, or 2.** A retry that depends on
"the child did not run" needs positive, trustworthy phase evidence for that
invocation; the numeric exit interface alone does not provide it. With
`CLW_RUN_STATE_FILE`, only an exact `NOT_STARTED` receipt authorizes one optional
cold fallback. Missing or ambiguous state fails closed.

The earlier rule that classified every 125 as `ClwFailed`/never-executed and every
137 as wrapper-terminated is withdrawn as a general consumer rule. It conflicted
with the same document's verbatim child-exit contract. Preserve the historical
record in Git rather than retaining contradictory current instructions.

The separate consumer correction is tracked in
[corelink-runners #567](https://github.com/HuGR-Labs/corelink-runners/issues/567).
The receipt closes its retry-classification ambiguity. The #305 change only
corrects the oversized **initial** lookup classification. Final stream delivery
remains separately scoped work.

For `snapshot` and `hydrate`, no child command is wrapped: zero remains success
and a nonzero result is failure of that CLI operation.

### Clarification — initial lookup response size (#305)

An oversized initial AC response is refused **before child spawn**. At the
`run_memoized` boundary this size error is normalized to a lookup-specific
`ClwError::Transport`, retaining the observed size and cap in the diagnostic;
the lower-level HTTP client still enforces and returns its typed body-cap error.
The CLI reports its existing internal-error exit **125**, not a claim that a child
was killed. Actual captured-output overflow retains **137** and no memo is stored.
No numeric outcome or key-domain changes.

A child may itself return 125 or 137; those outcomes remain verbatim passthrough.
These integers alone are therefore not a unique lifecycle discriminator and do
not authorize an automatic retry of possible side effects. This scoped repair
does not claim to resolve every error after spawn or introduce a new outcome
protocol. Stream-delivery handling remains the separate #310 obligation.

## 3. Stability
The v0.1.0 verb/flag convention and its documented additive extensions remain.
The numeric values 125/137, child-exit passthrough and success-only memo
production are unchanged by #305. These values are not exclusive to wrapper
failures. Changes to the public interface require an explicit contract update;
correcting an unsupported inverse classification does not introduce a new interface.
