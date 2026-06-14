# REVIEWER-BRIEF — cold-context review standard for a `clw` change

> **Audience:** a reviewer with **no prior context** on the change under review.
> **Purpose:** review any `clw` (CoreLink Workspaces) diff against the frozen spec
> and the shipped security invariants, run the load-bearing checks, and return one
> structured verdict. `clw` is a **proprietary, commercial, pure client** of the
> CoreLink API — it never edits `corelink-server`. Treat every diff as if it ships
> to a paying tenant handling a credential and their file contents.
>
> Pairs with: `CONTRIBUTING.md` (how to build/gate), `scripts/gates.sh` (the gate),
> `docs/ROADMAP-go-live.md` (milestones + exit gate), `docs/AUDIT-clw-2026-06-09.md`,
> `docs/THREAT-MODEL.md`, `docs/SPEC-manifest.md`, `SECURITY.md`.

---

## 0. Frame the change first (30 seconds)

- What milestone / WP is this (M-id)? Does the diff stay **scoped** to it?
- Which crates does it touch? Does it touch **`clw-types`** (the frozen contract),
  `Cargo.toml`/`Cargo.lock`, `scripts/`, or `.github/`? Any of those escalate the
  gate to the **FULL** lane and raise the bar (contract / supply-chain / CI surface).
- Is this docs-only? (Then the Rust gates are N/A — verify content accuracy only.)

## 1. Spec & contract invariants — check every one that the diff can affect

1. **Frozen `clw-types` contract.** `crates/clw-types/src/lib.rs` is the source of
   truth (hash `fe251941`, see `docs/SPEC-manifest.md`). A signature/trait/enum
   change is a **contract change**: it must ripple to **all impls** (real
   `clw-client` + the fakes in snapshot/hydrate/run), carry a **decision record**,
   and ship in its **own commit/PR** separate from behavior changes. Reject silent
   contract drift.
2. **Manifest canonicalization.** Manifest v1 is canonical JSON, Merkle-style,
   deterministic. Any change to entry ordering, encoding, or `schema_version` must
   preserve byte-determinism and be covered by a golden vector. (`docs/SPEC-manifest.md`)
3. **Path-traversal + Windows path validation.** `validate_rel_path` /
   `validate_symlink_target` reject absolute paths, any `..`/`.` component, **and**
   Windows backslash / drive-letter / UNC forms **on every platform**. Validation
   happens at `Manifest::from_bytes` **and** is re-checked in `hydrate`
   (defense-in-depth). A diff touching paths must not weaken either layer. *(audit H1, M2.2)*
4. **BLAKE3 integrity.** Every chunk is content-addressed and **re-verified on
   fetch**; manifests are verified; hydrate re-verifies fetched-chunk digests. No
   path may trust bytes without verifying the digest.
5. **PAT handling.** Token **redacted** in all output/diagnostics; `~/.clw/config.toml`
   written **`0600`**, `~/.clw` and the cache root **`0700`** (not umask-dependent);
   endpoint must be **`https://`** (loopback exception via **exact-host** check —
   `127.0.0.1`/`localhost`/`[::1]`, never a prefix match). *(audit H2, M2, M0.5 CRIT)*
6. **No `unsafe`.** Shipped crates contain **zero** `unsafe`. Reject any addition.
7. **No prod-path `unwrap`/`expect`/`panic`** and no `TODO`/`FIXME`/`best-effort`
   in shipped code. Tests may use them; production paths may not.
8. **Windows still compiles.** If the diff touches any OS-sensitive construct
   (perms, symlink, home dir, shell, path join), it must stay `#[cfg]`-correct and
   compile for Windows — proven via `cargo check --workspace --target x86_64-pc-windows-gnu`.

## 2. Process invariants

- **cargo-deny clean** (advisories + licenses + bans + sources); `publish = false`
  on every crate (proprietary-leak guard); license posture valid (no `UNLICENSED`).
- **DCO sign-off** on every non-merge commit (`Signed-off-by:` trailer) —
  `scripts/dco-check.sh` must pass.
- **Change-scoped gate green** — `bash scripts/gates.sh` (see §3). Never `--no-verify`.
- **Conventional-commit** subject (`feat(scope):` / `fix(scope):` / `docs:` / `chore:`).
- Tests are **non-vacuous** (e.g. a chunking proptest must force a `>4 MiB` body;
  a "zero HEAD requests" claim must be asserted by a wiremock request-count test,
  not just by an in-memory fake that cannot observe it).

## 3. Must-run commands (run these, do not take them on faith)

```bash
# The full gate (fmt + clippy -D warnings + workspace tests + cargo-deny [+ cargo-audit]):
bash scripts/gates.sh

# Windows compile proof (any OS-sensitive diff):
cargo check --workspace --target x86_64-pc-windows-gnu

# DCO sign-off on the range under review:
bash scripts/dco-check.sh origin/main

# Banned-construct sweep over the diff (must be empty in shipped, non-test code):
git diff origin/main...HEAD | grep -nE 'unsafe|\.unwrap\(\)|\.expect\(|panic!\(|TODO|FIXME|best-effort'

# Supply chain (if not already covered by the full gate lane):
cargo deny check
```

A reviewer who cannot run a command must say so explicitly — never assume green.

## 4. Verdict — return exactly one

- **APPROVE** — every applicable invariant in §1–§2 holds, all §3 commands the
  reviewer ran are green, the change is scoped, and tests are non-vacuous. State
  what you ran.
- **FIX-FIRST** — sound direction, but specific defects must land before merge.
  List each defect with the file/line and the invariant it violates; re-review
  after the fix.
- **REJECT** — violates a frozen invariant (contract drift, traversal/integrity/PAT
  regression, `unsafe`, prod `unwrap`/`panic`), is out of scope, or cannot be made
  green without redesign. Say why and what the correct approach is.

**Verdict format:**

```
VERDICT: APPROVE | FIX-FIRST | REJECT
RAN: <commands actually executed + result>
INVARIANTS: <any §1/§2 item at risk, or "all clear">
FINDINGS: <file:line — issue — invariant>   (empty for a clean APPROVE)
NEXT: <merge | the exact fixes required | redesign direction>
```
