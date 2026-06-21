# clw TL — session handoff / state (2026-06-20)

> Supersedes `HANDOFF-2026-06-17-clw-TL-state.md`. Single source of truth to resume *fresh* after a
> `/clear`. I am TL of `clw` / corelink-workspaces (single human owner = Gustavo). Standing mandate:
> SOTA, no debt/deferral without an explicit human waiver, I OWN all merge/PR/commit, keep all judgment,
> delegate only verifiable execution, never assume without evidence. **Fetch-first** — the owner merges via
> GitHub and the local clone goes stale; always `git fetch` + diff `origin/main` before assuming.

---

## TL;DR — where we actually are

- **clw is SHIPPED and live.** Latest tag **`v0.1.3`** (`brew install humanguardrail/clw/clw`). Beta.
- **All post-launch hardening is CLOSED.** Three waves done and merged: user-simulation findings
  (#140–#146), brutal audit #2 (#147 canary, #148 docs, #149 code — all 7 findings), and in-tree
  fuzzing (#150). `main` = `dc3f182` at handoff; 0 open PRs; only `main` locally; tree clean.
- **Core verdict: zero surviving findings** in exit-contract / data-integrity / security / error-handling
  across two full adversarial audits + a hostile-server suite + 26 fuzzers (no crashes at 50k).
- **No open clw *code* work.** What's left is external/owner-gated (below).

---

## What landed since the 06-17 handoff

| PR | What | Note |
|---|---|---|
| #133, #135–#140 | E2E test growth 29→45+; symlink `..` root fix | power-user findings |
| #141–#146 | userinfo redaction; `snapshot --force`; re-snapshot no-op; honest `rm`; ls noise/human ts; clearer msgs; CHANGELOG `[Unreleased]` | user-sim hardening, cold-reviewed |
| #144 | self-hosted CI fallback (`clw-mac-1`) for gates+dco | hosted minutes were exhausted |
| #147 | daily live-conformance **canary** (cron `0 13 * * *`, `CLW_CONFORMANCE_REQUIRE=1`) | monitors prod |
| #148 | audit-2 **doc** findings (#5 CLI-CONTRACT runner invocations, #6 `--force`/no-op docs, #7 honest-rm example) | cross-team + surface docs |
| #149 | audit-2 **code** findings: #1 force gates on raw AC presence, #2 prune preserves run-record blobs, #3 gc excludes `.tmp-*`, #4 pagination page-cap | cold-review APPROVE, prune data-safety verified |
| #150 | **26 proptest fuzzers** over every untrusted-input parser; gate-friendly `fuzz_cases()` 1k/50k | cold review caught 3 false-confidence fuzzers (version-bias HIGH), fixed + re-APPROVE |

---

## Open items (none blocking; all external or owner-gated)

1. **Release v0.1.4** — staged in `CHANGELOG.md [Unreleased]` (now includes the audit-2 fixes + fuzz +
   canary). One-step cut (`changelog-bump` is already satisfied; tag `v0.1.4`) **next month when
   GitHub-hosted Actions minutes reset** — `release.yml` is still hosted-only (5-target cross-builds).
   Self-hosted release plan only if an urgent fix must ship before minutes return.
2. **Prod intermittent HTTP 503 → server-TL.** Flagged via `docs/RELAY-server-intermittent-503-2026-06-20.md`.
   Pattern: a bounded window where ~all authenticated requests 503, then self-recovers (seen in a manual
   conformance run 8/9 and the canary's first dispatch 8×). clw side is correct (503 transient, never
   cached). Ask: confirm whether the windows are deploy-correlated (expected) or an availability blip. The
   canary now gives a recurring timestamped signal.
3. **Windows real cross-platform** (deferred) — needs a Windows runner; none exists (self-hosted is
   macOS-only). Cross-*build* is proven (PE32+ artifact); real-binary E2E on Windows is the gap.
4. **Observability + rollback kill-switch** (deferred) — tracing exists; a runtime kill-switch is a
   product decision (owner). Not started by design.
5. **Signing / legal** (deferred, pre-existing) — macOS Developer-ID + Azure Windows signing ("último do
   último"); SLSA provenance waived (private repo); legal counsel post-beta/pre-GA.

---

## How to resume / operate (the non-obvious bits)

- **Gate:** `bash verify.sh` (FULL) or `--scoped [base]` (fast lane). `scripts/gates.sh` = fmt/clippy/test/deny.
- **Local cargo on this Mac:** the rustup proxy is broken; use
  `export PATH="$HOME/.rustup/toolchains/1.91.1-x86_64-apple-darwin/bin:$PATH"; export RUSTUP_TOOLCHAIN=1.91.1-x86_64-apple-darwin`.
- **Fuzz deep run:** `CLW_FUZZ_GATE=1 cargo test -p <crate> <fuzzer-name>` (50k cases).
- **Flow:** DCO `-s`; Conventional Commits; T0 (tiny self-verified docs/≤50 LOC → direct to main) / T1
  (branch → PR → cold review → squash-merge); ~1 PR in flight; never force-push/`--no-verify` on main.
- **Cross-team:** signals are `docs/RELAY-*.md` (paste-ready). Replies arrive as `docs/REPLY-*` / `RESPONSE-*`.
- **CI is healthy on self-hosted** (`clw-mac-1`, labels self-hosted/macOS/X64); hosted lane preferred when
  minutes exist. `release.yml` is hosted-only.
