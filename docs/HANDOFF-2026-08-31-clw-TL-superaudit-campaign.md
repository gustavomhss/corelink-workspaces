# HANDOFF — superaudit remediation campaign (clw / corelink-workspaces)

**Written:** 2026-08-31 · **Corrected:** 2026-09-01 · **Author:** Claude Code session `e22a9a73` (TechLead role)
**Purpose:** let any other model, session or harness resume this campaign from where it stopped,
without re-deriving anything. Read this file first, then `docs/superaudit-2026-08-30/WP-CATALOG-v3.md`.

> **Why this file exists and why it is in the repo.** The campaign's real state — the WP catalogue, the
> execution log, and the seven per-area findings files — lived only in an ephemeral session
> scratchpad under `/private/tmp`. One machine reboot or `/tmp` sweep and 116 KB of reviewed
> specification was gone. It is now committed under `docs/superaudit-2026-08-30/`.

> **Trust model and freshness.** This handoff is the campaign's current-state header, and
> `WP-CATALOG-v3.md` is the operational specification for WPs that have not landed. A WP is complete only
> when this header says so and live git/GitHub state confirms it; catalogue line numbers and preconditions
> must be revalidated against a freshly fetched `origin/main`. `AUDIT-FINDINGS-SUMMARY.md`,
> `EXECUTION-LOG.md`, and the seven `findings-*.md` files are **archival snapshots** of the 2026-08-30
> sessions. They intentionally retain contemporaneous claims and dead session-local pointers; they are
> evidence and provenance, not a live status ledger.

---

## 1. The standing goal, verbatim

> "execute o catalogo e wps completo e2e com profissionalismo e rigor, mantendo um padrao sota e tudo
> absolutamente impecavel. Seja cuidadoso, verifique seu trabalho, e nao faca merge sem cold review
> adversarial em tudo. Mantenha tudo absolutamente impecavel e fucking awesome."

Refinements the owner added during execution, each of which is binding:

- **"cada pr tem que passar por cold review, nao so os cis"** — every PR needs an adversarial cold
  review, not only the ones touching CI.
- **"Irmao a decisao de merge e sua, se passar nos cis e no coldreview e merge"** — merge authority is
  delegated to the TechLead, conditioned on CI green **and** an approving cold review. Both, not either.
- **"Aceitavel nao e o suficiente. Aqui so passa fucking awesome."** — "acceptable" is not a passing
  standard. This was said after I described a 2m06s gate runtime as acceptable.
- **"Transforme as memorias que fizerem sentido transformar, em skills"** — convert durable lessons from
  memory into skills.

Session style: CAVEMAN MODE (full) — terse replies, all technical substance retained, replies in
Portuguese. Content persisted outside chat (this file, commits, PR bodies, docs) is written in normal
prose, per the mode's own boundary rule.

---

## 2. Provenance of the work

Superaudit of `HuGR-Labs/corelink-workspaces` @ `25cfcaf`, run 2026-08-30 by 7 parallel subagents plus
mechanical checks. Result: **0 CRITICAL, 8 HIGH, 31 MED, ~70 LOW/INFO**. No HIGH in Rust code — four
auditors independently called the core "unusually well-hardened". The real problems are governance, CI
and documentation.

That produced: plan v1 (47 WPs) → 5 adversarial cold reviews (58 findings, 7 BLOCKER) → v2 (structural
corrections) → **v3, the executable catalogue**, where every WP is written with the full 7-clause schema
and verified against the code. `docs/superaudit-2026-08-30/WP-CATALOG-v3.md` is v3.

Published audit report (Artifact, owner-visible):
`https://claude.ai/code/artifact/eb5c1690-ff83-48b4-92dd-f7338833d894`

---

## 3. Current state — what has landed

Ten PRs merged on 2026-08-31 and #255 merged on 2026-09-01, each with green CI. The originating session
reports that each also had an approving adversarial cold review before merge; those verdicts were
session-local and were not persisted as GitHub reviews or comments, so that part is not independently
auditable from repository/PR state.

| PR | WP | What landed |
|---|---|---|
| #242 | WP-08a/08b | minimum-scope `permissions:` on the four undeclared workflows |
| #243 | WP-33 | cache eviction is an approximate LRU, not FIFO |
| #245 | WP-35 | `RUNBOOK-ci-fallback` for a self-hosted runner outage |
| #246 | WP-00 | release notes published from a reviewed committed file; publish argv pinned |
| #248 | WP-28 | reject a self-referential symlink on the Windows copy-fallback |
| #249 | WP-23 | an empty `CLW_*` variable means absent, not present-and-empty |
| #250 | WP-09 | deadman watches every scheduled lane, not just the conformance canary |
| #251 | WP-34 | mutation lane scoped to the PR diff; a survivor is red |
| #253 | WP-06a/06b | **the docs-only classifier could skip every Rust gate** (see §5) |
| #254 | — | cfg'd-out mutants are unanswerable, not unkilled (`.cargo/mutants.toml` ledger) |
| #255 | — | mutation verdict selftest: 29 named cases; additive `POSSIBLY_UNCOMPILED` + `SURVIVORS` contract |

`#254` and `#255` are *emergent* — they are not catalogue WPs. They exist because executing WP-34
surfaced a defect in the lane WP-34 had just built.

### Open right now

| PR | Branch | State | Blocking on |
|---|---|---|---|
| **#256** | `claude/handoff-superaudit-campaign` | this handoff rescue; round-2 corrections in flight | green CI + independent cold review + pre-merge check |
| **#247** | `claude/wp-16-forbid-unsafe` | WP-16, ready for the additive `origin/main` merge in §6 | merge + focused verification |

Scratch PR #252 (`claude/wp-34-negative-control`) was **CLOSED** without merge on
2026-09-01 at 16:32:19Z. It is evidence-only and must never be merged.

Nine catalogue PRs delivered **eleven catalogue headings** (WP-00, WP-06a/06b, WP-08a/08b, WP-09,
WP-23, WP-28, WP-33, WP-34 and WP-35), plus two emergent PRs (#254 and #255): eleven merged PRs total. The catalogue
carries **48 `## WP-` headings** (one bundles WP-38a/b/c); eleven have landed and **37 remain**, including
the in-flight WP-16 and the still-open WP-36, all listed in §7.

---

## 4. How to work here — the process that is actually binding

- **DCO: commit with `-s`.** Conventional Commits for titles.
- **Branch → PR → adversarial cold review → squash-merge on green.** No exceptions, including for
  docs-only PRs.
- **Run the FULL gate locally first:** `bash scripts/gates.sh`. Then `bash scripts/pre-merge-gate-check.sh <PR#>`.
- **Read the file list before every merge:** `git diff --name-only origin/main...<branch>`. Not
  `git show --stat` on the tip, which shows one commit of several. This is not optional — see §8.
- **Never `git add -A`.** Stage by name. See §8.
- `main` is *not* protected and *cannot* be — see §5. Nothing mechanically stops a bad merge; the
  discipline above is the entire control.
- The local clone goes stale. `git fetch` and diff `origin/main` before assuming anything.
- **Cold review dispatch:** use a subagent in its **own worktree**, told explicitly not to merge, and
  asked at the end *"what did my framing of this review get wrong?"* — that question has produced the
  single most valuable finding in several rounds.

---

## 5. Two findings that change what everything else is worth

### 5.1 `main` is not branch-protected, and cannot be

```
gh api repos/HuGR-Labs/corelink-workspaces/branches/main/protection
  -> 403 "Upgrade to GitHub Pro or make this repository public to enable this feature."
gh api repos/HuGR-Labs/corelink-workspaces/rulesets   -> 403 (same)
gh api orgs/HuGR-Labs --jq '.plan.name'               -> free
gh api repos/HuGR-Labs/corelink-workspaces --jq .private -> true
```

A private repository on a free org plan cannot have branch protection or rulesets **at all**. This is
not "protection was switched off" — nobody can turn it back on.

`CLAUDE.md` still asserts, as a repo convention, that `main` requires `gates` + `dco` and enforces
admins. **None of that is in force.** Correcting that assertion is WP-19 and has not been done.

The consequence for the catalogue: `gates.sh` is a convention, not a merge blocker. Every WP whose
acceptance rests on "the gate catches it" actually rests on "the gate catches it, provided a person runs
it and honours the result."

**This is an owner decision** — make the repo public, pay for Pro/Team, or stop asserting a control that
does not exist. It is a plan/visibility question with a cost, not an engineering one.

### 5.2 The docs-only classifier could skip every Rust gate (fixed in #253)

`scripts/gates.sh`'s lane classifier could route a diff containing Rust down the docs-only lane and print
`ALL GATES GREEN` having compiled nothing. Observed live on PR #247, job 99505002408: **54 seconds** on a
33-file diff containing Rust.

`pre-merge-gate-check.sh` cannot see this — it reads check *conclusions*, and the conclusion was
`success`. Nothing merged on the defect: five WPs ran full lanes (5m31s–8m40s) and #245 exited docs-only
at 27s and is genuinely docs-only.

**Open follow-up:** `pre-merge-gate-check.sh` should learn the vacuous-green signature (a `gates` run far
too fast for the diff's shape). Filed as a WP, not yet written.

---

## 6. Exactly where I stopped

### #255 — mutation-lane classification, landed

Squash-merged as `6b20a49d499a92c12df58a4eaa6d13bd8008f5f6` on 2026-09-01. All persisted GitHub
checks were green. The originating session reports a round-2 cold-review **APPROVE**; the verdict was not
persisted as a GitHub review/comment and is therefore session evidence, not independently auditable state.

**What it does.** Extracts the cargo-mutants exit-2 survivor classification out of
`.github/workflows/mutation.yml` into `scripts/mutation-verdict.sh`, adds
`scripts/mutation-verdict-selftest.sh` (**29 named cases**), registers it in `scripts/gates.sh`, adds a
ledger-exclusion delta to the lane's count step, and fixes contradictory prose in `docs/TESTING.md`.

**Why a self-test and not "the next PR will exercise it".** The mutation lane only runs when a diff
touches Rust **and** yields mutants. #254 touched no Rust, so its own check went green having never
executed the new logic. The obvious follow-up — "#247 will be its first real run" — was **tested and is
false**:

```
--in-diff pr247.diff --list --no-config  -> listed=17
--in-diff pr247.diff --list  (w/ ledger) -> listed=0
```

`listed=0` makes the run step's condition false, so cargo-mutants never runs on #247 at all. Neither
landing nor a scratch PR gives durable coverage. Hence the self-test.

**Round-1 review returned REQUEST CHANGES** with a sharp diagnosis: the suite was green against five
defects because it pinned the verdict token and nothing else, on degenerate inputs. All addressed in
round 2:

- **MED-1** — transposing `missed`/`uncompiled` in the printf left everything green, though the lane
  prints the second number to the author. Renaming a verdict token consistently in script *and* workflow
  also stayed green while the lane emitted no `::error::` at all (the workflow's `case` arms have no
  default). Now pinned by `contract-line` and `workflow-vocabulary`.
- **MED-2 / LOW-1** — every `UNCOMPILED` case used a survivor of duration 0, which passes under any
  positive threshold, so `0.05 -> 0.0001` and `else 1 -> else 10000` both stayed green. Four new cases
  bound both constants from both sides with non-degenerate durations.
- **MED-3** — the ledger delta failed silently in the branch it exists for. Now captures the status and
  reports unknown as unknown.
- **MED-4** — the header said "Sourced by" the workflow; the workflow *executes* it.
- **LOW-2 / LOW-3** — the `pass -lt 12` floor was a hand-maintained constant (add a trivial case, delete
  a real one, total unchanged, green) and ran *before* the failure check, so a real regression was
  misdiagnosed as a missing case. Now a roll-call of case names, reported after failures.

**Final contract:** every `MissedMutant` remains `SURVIVORS`; complete low-cost timings add the diagnostic
hint `POSSIBLY_UNCOMPILED` without excusing the survivor. The 29-case roll-call pins both tokens, field order,
workflow wiring, threshold boundaries, malformed evidence, direct execution and harness non-vacuity.

**One thing worth carrying forward.** While fixing MED-3 I wrote a regex over `.cargo/mutants.toml` that
enumerated the permitted keys, to decide whether the delta was attributable to exclusions. It **failed on
the real file** — appending `timeout = 300` went undetected — and it was the losing race this repo has
already learned to refuse. I removed it rather than repairing it, and relabelled the summary to say what
`--no-config` actually does (ignores the whole file), which is true whatever the file holds.

### #247 — WP-16, `forbid(unsafe_code)`

Round 13. #255 is now on `origin/main`; the branch needs one **additive merge**, never a history rewrite.
The working copy also has the owner's signed local typo-fix commit `36f5946`; preserve it.

The earlier conflict resolution `b11215e` is load-bearing: it kept the literal classifier term
`crates/[^/]+/Cargo\.toml`, which escalates a member-manifest-only diff to FULL. Taking either side wholesale
can silently drop it. Resume with this exact safety sequence:

1. In the existing #247 worktree, confirm `git status`, HEAD and that `36f5946` plus `b11215e` are ancestors.
2. `git fetch origin`, then confirm `origin/main` is at least `6b20a49`.
3. Run `git merge --no-ff --no-commit origin/main`. Do not clean or overwrite the owner's local change.
4. Before committing, inspect `git status`, `git diff --cached --name-status` and every conflict resolution.
5. Confirm the literal `crates/[^/]+/Cargo\.toml` survived; run `bash scripts/gate-lane-selftest.sh`.
6. Confirm #255's workflow still executes `scripts/mutation-verdict.sh`, then run
   `bash scripts/mutation-verdict-selftest.sh` and require its **29 cases passed** line. The output contract
   must still be additive `POSSIBLY_UNCOMPILED SURVIVORS`, never an exclusive choice.
7. Run the FULL gate, inspect the eventual full PR diff, and only then create the merge commit with `git commit -s`.

---

## 7. What remains

From `docs/superaudit-2026-08-30/WP-CATALOG-v3.md`, grouped as the catalogue groups them. Each WP there
carries its full schema (statement, preconditions, steps, acceptance, risk, rollback, dependencies).

**Session 1 — release truth and unblocking:** WP-0A, WP-05, WP-04, WP-01 (cut v0.1.12), WP-02-DEC /
WP-02-IMP (main protection posture — owner decision, §5.1), WP-29-DEC, WP-22.

**Session 2 — the `gates.sh` chain:** WP-14, WP-14b, WP-03 (SHA-pin all 33 actions + Dependabot),
WP-07, WP-10.

**Session 3 — observability and permissions:** WP-13a, WP-13b, WP-16 (**in flight as #247; it remains
open until that PR lands**), WP-15 (rewrite THREAT-MODEL), WP-17, WP-18, WP-19 (**CLAUDE.md real state —
includes the §5.1 correction**), WP-20a, WP-20b, WP-21, WP-36 (**still open: provide an observer outside
GitHub Actions, or record the absence explicitly and with a date in both `docs/` and `CLAUDE.md`**),
WP-46, WP-48, WP-49, WP-50.

**Session 4 — `release.yml` hardening:** WP-24, WP-25, WP-26, WP-26b, WP-12, WP-27, WP-29-IMP,
WP-38a/b/c, WP-38d, WP-42.

### Standing owner decisions (not mine to make)

1. **Branch protection** — plan/visibility question with a cost (§5.1).
2. **The 20 already-published releases still carry the false SLSA/provenance claim.** #246 fixes new
   releases only: the workflow reaches a published tag only if that tag's run is repeated, and nobody
   re-runs an old release. `CHANGELOG.md` names this under "Still outstanding".
   **There is no backfill tool in the branch tip.** A first attempt was written and then *withdrawn* from
   #246 after review found two defects in it — its target confirmation compared a variable against itself,
   and a failed release listing reported success having done nothing. An early revision of this handoff PR
   copied that defective draft into `docs/`; cold review removed it. It now exists only in prior git history:
   **do not recover it and do not run it.** A replacement must be implemented from the requirements and
   reviewed as new outward-mutating tooling. Correcting published releases edits public content and remains
   an owner action.
3. **The base of #256 has 64 commits unreleased since v0.1.11** (tag 2026-07-16; measured at
   `origin/main` `6b20a49` after #255). If #256 is the next squash merge, the count becomes 65; recompute
   from fresh state instead of treating either number as durable. The range includes the h2
   RUSTSEC-2026-0258 bump (#239).
   WP-01 cuts v0.1.12; cut `v0.1.12-rc.1` first — rc dry-runs have repeatedly caught tag-only bugs.

---

## 8. Traps in this repository, measured, not theorised

The originating session reported that these cost real time. Most were not persisted as independently
replayable artifacts, so treat them as session-derived cautions rather than audited facts. They are also
written into the user-level skills `gate-authoring`, `instrument-calibration` and
`agent-dispatch-and-landing`.

- **The session reports that `git add -A` swept tool state into commits four times** — 227 `.atlas/` files across two branches,
  plus two PR-body files, one of them in the very commit whose message was about documentation falsified
  by its own commit. It also reports green CI and an approving cold review, but that review verdict was not
  persisted and cannot be independently audited here. Neither signal alone proves the shape of a diff;
  inspect `git diff --name-only <base>...<branch>` directly.
- **A teeth-probe that stays GREEN is a finding.** It means the line you deleted was not carrying the
  property, and if your comment claimed it was, the comment is now false. Measured here: `| numbers`
  appears twice in the classifier's jq and only the one in the **length guard** is load-bearing;
  the `[ -f ]` guard is likewise redundant. Probe every line the prose credits, one at a time, plus one
  no-op control.
- **A guard that must enumerate what an adversary might write has already lost.** Delegate to whatever
  already decides: the compiler (`forbid` makes every permission-attribute spelling `error[E0453]`), a
  content hash, the OS (`canonicalize`), or structure (move the text into a committed file review already
  reads). WP-00 took 9 rounds and WP-16 took 6 before this was accepted. **But do not over-learn it** —
  abandoning a *closed, documented* set (a tool's own documented flags) in the same breath is the control
  failing at its job.
- **The session reports compound shell verification returning plausible wrong answers.** Its examples — a `grep -c`
  missing backticks "proved" a merge had dropped a CHANGELOG entry; a `perl` sabotage silently failed to
  apply so a suite ran green against an unchanged script; a `\s` -vs- `[[:space:]]` hypothesis was wrong
  twice before direct measurement settled it. Measure the single thing directly before believing a
  pipeline.
- **`git checkout <file>` reverts to HEAD, not to your working state.** It destroyed a rewritten
  self-test here. Copy first.
- **cargo-mutants reads `.cargo/mutants.toml`**, not `mutants.toml` at the repo root. A root file is
  inert and silently does nothing.
- **cargo-mutants is syn-based and never evaluates `cfg`** — which is why it generates `#[cfg(windows)]`
  mutants on a Linux runner and then cannot build them. It is also why such exclusions *are* testable on
  a non-Windows host.
- **The gate runs on `runs-on: corelink`** (self-hosted CF-Firecracker), not `ubuntu-latest`. No file in
  the repo declares that image's package set, so tool availability there is answerable only by watching a
  live job.

---

## 9. Immediate next actions, in order

1. **Finish #256 (this PR):** push the round-2 correction, wait for green CI, obtain a new independent cold
   review, run the pre-merge check, inspect the full file list, and squash only on APPROVE.
2. **Merge final `origin/main` additively into #247** with the exact no-commit/inspection sequence in §6;
   preserve `36f5946`, `b11215e` and `crates/[^/]+/Cargo\.toml`, and verify #255's 29-case additive contract.
   Land only after its own round-13 confirmation.
3. **Resume the catalogue at §7**, Session 1. WP-19 is worth pulling forward out of order: `CLAUDE.md`
   currently tells every future session something false about branch protection (§5.1), and every session
   reads it first.
4. File the `pre-merge-gate-check.sh` vacuous-green WP (§5.2).
