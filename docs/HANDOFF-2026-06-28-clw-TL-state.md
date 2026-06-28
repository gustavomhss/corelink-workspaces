# clw TL — session handoff / state (2026-06-28)

> Supersedes `HANDOFF-2026-06-20-clw-TL-state.md`. Single source of truth to resume *fresh* after a
> `/clear`. I am TL of `clw` / corelink-workspaces (single human owner = Gustavo). Standing mandate:
> SOTA, no debt/deferral without an explicit human waiver, I OWN all merge/PR/commit, keep all judgment,
> delegate only verifiable execution, never assume without evidence. **Fetch-first** — the owner merges via
> GitHub and the local clone goes stale; always `git fetch` + diff `origin/main` before assuming.

---

## TL;DR — where we actually are

- **clw is SHIPPED and live.** Latest *released* tag **`v0.1.3`** (`brew install humanguardrail/clw/clw`).
- **`v0.1.4` is staged but NOT yet tagged** — `Cargo.toml = 0.1.4`, CHANGELOG `[0.1.4]` final. Cut is one
  step once GitHub-hosted Actions minutes return: `git tag -a v0.1.4 -m "clw 0.1.4" && git push origin v0.1.4`.
  Post-stage fixes (#154/#155/#156) landed on `main` and are folded into the `[0.1.4]` changelog (they ship
  in that same un-cut tag).
- **`main` at handoff = `2577a7d`.** Tree clean; only `main` locally. **One open PR: #153 (DRAFT/HELD).**
- **No blocking clw *code* work.** Everything substantial left is external/owner-gated (below).

---

## What landed this session (since the 06-20 handoff)

| PR | What | Note |
|---|---|---|
| #153 | `clw hydrate --manifest-digest` (content-addressed hydrate) | ✅ built+reviewed+green, **HELD as DRAFT** — see item 0 |
| #154 | `quinn-proto` 0.11.14→0.11.15 (RUSTSEC-2026-0185, HIGH) | ✅ merged; repo-wide gate fix; phantom dep, no compiled path |
| #155 | shared-cache removal paths concurrency-safe (ENOENT-benign + skip `.tmp-*`) | ✅ merged; cold-review APPROVE |
| #156 | cache fs errors carry op+path context (diagnosability) | ✅ merged; cold-review APPROVE |
| #157 | gitignore `.claude/worktrees/` (agent review worktrees) | chore hygiene |

---

## Open items (none blocking; all external / owner-gated)

0. **Check-host toolchain resolver seam (NEW cross-team thread).** Runners TL's CF-native check-host needs
   `toolchain_ref → content`. **Decided + ACCEPTED (option b):** a toolchain = a `clw snapshot`;
   `toolchain_ref` = its manifest digest; the layer list falls out of the manifest entries; materialize with
   `clw hydrate`. Zero contract drift (ratifies their frozen `cf-check-host-contract.md` C1–C5). Reply:
   `docs/REPLY-clw-TL-toolchain-resolver-seam-2026-06-26.md`. Their accept:
   `corelink-runners/docs/handoff/2026-06-28-REPLY-…-option-b-ACCEPTED.md`.
   **clw deliverable = `clw hydrate --manifest-digest` (PR #153).** It is done/reviewed/green but **HELD as
   DRAFT at the Runners TL's explicit request** — land it only when their **W6** (the in-box clw-drive) is
   wired to consume it ("no surface ahead of a consumer"; the CLI surface is EULA-frozen once released).
   **When they ping W6 ready:** rebase #153 on current `main` (drop its now-redundant quinn-proto commit;
   reconcile its CHANGELOG `[Unreleased]` entry), flip out of draft, merge.
   **Producer (hugit) ACCEPTED (2026-06-28):** `CheckDef.toolchain_ref = SnapshotReport.root` ratified — seam
   now settled BOTH sides (producer + Runners consumer), zero drift. The operational `clw snapshot` + set is
   queued downstream of the check-host (B) executing a real check (it waits on the runner fabric being live),
   not a blocker today. ACK: `docs/REPLY-clw-TL-to-hugit-toolchain-ref-settled-2026-06-28.md`.
   **Cache R2 CONFIRMED (2026-06-28):** push==read==R2-backed CAS `https://corelink-api.humangr.com/v1/cas`
   (bucket `corelink-cas-prod`, zero-egress, same CF account) — `docs/REPLY-cache-TL-r2-placement-CONFIRMED-2026-06-28.md`.
   **So BOTH check-host external confirms are now done (hugit + Cache).** The ONLY remaining gate for #153 is
   the Runners TL's **W6 ping** (runner fabric / check-host going live). Optimization for when B snapshots:
   deterministic public toolchains can use the CAS `_public` namespace for cross-tenant dedup (Cache TL note).

1. **family-e2e (live cross-team E2E)** — clw 100% READY (suites #66/#67 merged; CLI frozen; M10 driver
   `scripts/family-e2e.sh` built; delivery shape confirmed matching). Tracked in `docs/family-e2e-STATUS.md`.
   **M7 RESOLVED (2026-06-28): path (a)** — reuse prod `CONFIG_DB` + sandbox tier-tenants, no new D1, no
   CF-cred escalation; seed verified compliant. **Single remaining gate = M8: the owner runs the seed+mint `!`**
   (prod-credential op; server-TL preps tooling #1 `provision-personas.py` or #2 wrangler one-shot — owner
   picks). 6 PATs land OOB → `scripts/family-e2e.sh family-e2e.creds` → report the table in the STATUS ledger.
   Decision: `REPLY-server-TL-family-e2e-D1-path-PICKED-2026-06-28.md`; ACK: `REPLY-clw-TL-to-server-TL-family-e2e-path-a-ACK-2026-06-28.md`.
   (Also: server-TL offered to correlate the prod 503 — pull timestamped 503 windows from the canary run history and send them.)

2. **Release v0.1.4** — staged on `main` (see TL;DR). Cut when hosted Actions minutes return (next month).

3. **Open e2e flake (NOT resolved).** `concurrent_snapshot_processes_shared_cache_no_corruption` (clw-cli
   e2e) rarely fails on the self-hosted CI with `No such file or directory` (ENOENT) in clw's snapshot path
   under shared-cache load. Reproduced **0/60 locally**; NOT root-caused. #155 hardened adjacent removal
   paths (but that test uses a fresh cache, so those paths aren't its trigger) and #156 added op+path context
   to cache fs errors **specifically to make the next CI occurrence diagnosable**. **Next step:** when it
   next flakes on CI, the log will now name the exact op/path → fix the real root cause from that data.
   Mitigation today: `gh run rerun <id> --failed`. (Also: the CI Mac hit a near-full disk this session —
   `ENOSPC` during builds; cleared `target/debug/incremental`. Disk pressure is a flake amplifier to watch.)

4. **Prod intermittent HTTP 503 → server-TL.** `docs/RELAY-server-intermittent-503-2026-06-20.md`. Bounded
   self-recovering 503 windows; clw side correct (503 transient, never cached). Canary gives a recurring signal.

5. **Deferred (pre-existing):** Windows real-binary E2E (no Windows runner; cross-*build* proven); runtime
   rollback kill-switch (product decision); macOS Developer-ID + Azure Windows signing; SLSA provenance
   (waived, private repo); legal counsel (post-beta/pre-GA).

---

## How to resume / operate (the non-obvious bits)

- **Gate:** `bash verify.sh` (FULL) or `--scoped [base]` (fast lane). `scripts/gates.sh` = fmt/clippy/test/deny.
- **Local cargo on this Mac:** the rustup proxy is broken; use
  `export PATH="$HOME/.rustup/toolchains/1.91.1-x86_64-apple-darwin/bin:$PATH"; export RUSTUP_TOOLCHAIN=1.91.1-x86_64-apple-darwin`.
- **Flow:** DCO `-s`; Conventional Commits; T0 (tiny self-verified docs/≤50 LOC → direct to main) / T1
  (branch → PR → cold review → squash-merge); ~1 PR in flight; never force-push/`--no-verify` on main.
  The owner has delegated merge authority to me this session (#154/#155/#156/#157 self-merged on green+review).
- **Cold review:** run the reviewer agent with **worktree isolation** — a non-isolated reviewer once ran
  `git reset` and moved the working tree. Prune its worktree after (`git worktree remove … --force`).
- **Cross-team:** signals are `docs/RELAY-*.md` (paste-ready); replies arrive as `docs/REPLY-*`/`RESPONSE-*`.
  The owner is the courier between TLs (no direct channel).
- **CI is healthy on self-hosted** (`clw-mac-1`); hosted lane preferred when minutes exist. `release.yml` hosted-only.
