# RUNBOOK — returning CI to GitHub-hosted runners during a self-hosted outage

**Audience:** the incident owner, the repository administrator who can merge the
emergency PR, and the Actions/billing owner.
**Scope:** how to move this repo's `corelink` lanes to GitHub-hosted Linux runners,
what it costs, what it breaks, and how to undo it. This document is not authority to
push directly to `main`, to bypass the pre-merge control, or to redesign the Mac
release matrix.

**Inventory basis:** `origin/main` at `a5bcbcc047ce9e522b2b80afe33b36181dacb3c0`
(2026-09-02). The commands below deliberately fail if this inventory has drifted;
re-audit rather than changing the expected numbers during an incident.

> **Status of this runbook — read this first.**
> Acceptance criterion 1 of the work package (a *fresh* test PR proving the required
> lanes green on `ubuntu-latest`) is **UNMET**. See [§9](#9-what-is-proven-and-what-is-not)
> for exactly what is proven, what is not, and why. §4's runner inventory and label
> replacement are mechanically checked against this tree; the historical production
> evidence applies only to the named predecessor jobs. Portability of the remaining
> current jobs is explicitly unproven, not inferred from that history.

---

## 1. The two outages are not the same outage

This repo has **two** independent self-hosted dependencies. They fail separately and
have completely different fallbacks.

| # | Dependency | What it is | What dies with it | Fallback |
|---|---|---|---|---|
| A | `corelink` | CoreLink's own ephemeral CF-Firecracker runner fabric. 22 of the 23 `runs-on:` declarations in this repo. A fresh microVM per job. | **Merging.** The fallback PR cannot merge while a `corelink` check is pending: `pre-merge-gate-check.sh` rejects every non-pass/non-skip check. Its PR path includes the four `ci.yml` jobs, coverage, lint, mutation and (for a full swap) Windows smoke. It also carries release preflight/conformance/sbom/publish, the canaries, and scheduled/manual lanes. | **Partly — §4.** A controlled label swap exists for the ordinary `pull_request` lanes; `pr-title.yml` is `pull_request_target` and needs separate recovery design. |
| B | `clw-mac-1` | ONE physical Intel Mac (labels `self-hosted, macOS, X64`). Exactly 1 `runs-on:` declaration: `release.yml`'s 5-target `build+sign` matrix. | **Releasing.** No binaries, no signatures, no tag ships. | **No trivial one.** See §6. During a Mac outage, **releases stop; merging continues.** |

If **both** are down, releases cannot be restored cheaply (§6), and the current
merge-recovery procedure is incomplete because of `pr-title.yml` (§4.3). Resolve that
trusted-metadata blocker before claiming either recovery path is available.

---

## 2. Runner inventory (verified against `origin/main`, 2026-09-02)

Every `runs-on:` in `.github/workflows/`. There are **14 workflow files, 23 runner
declarations: 22 `corelink` and one physical Mac.**

| Workflow | Job | Line | `runs-on:` | Class | Trigger | Blocks a merge? |
|---|---|---|---|---|---|---|
| `ci.yml` | `gates` | 48 | `corelink` | fabric | PR + push(main) + dispatch | Required-presence check |
| `ci.yml` | `dco` | 89 | `corelink` | fabric | PR + push(main) + dispatch | Required-presence check |
| `ci.yml` | `gate-check-selftest` | 117 | `corelink` | fabric | PR + push(main) + dispatch | Must be green if reported |
| `ci.yml` | `release-guards-selftest` | 138 | `corelink` | fabric | PR + push(main) + dispatch | Must be green if reported |
| `coverage.yml` | `llvm-cov (floor 88%)` | 43 | `corelink` | fabric | PR + push(main) + dispatch | Required-presence check |
| `shellcheck.yml` | `shellcheck + actionlint` | 35 | `corelink` | fabric | PR + push(main) + dispatch | Must be green if reported |
| `pr-title.yml` | `pr-title` | 24 | `corelink` | fabric | `pull_request_target` | **Cannot use the fallback PR's head; see §4.3** |
| `windows.yml` | `windows binary smoke (mingw build, wine run)` | 57 | `corelink` | fabric | PR when `windows.yml`, `crates/**`, or `Cargo.*` changes; push(main) + dispatch | Full fallback PR must pass it |
| `release.yml` | `preflight` | 73 | `corelink` | fabric | tag `v*` | — |
| `release.yml` | `conformance` | 280 | `corelink` | fabric | tag `v*` | — |
| `release.yml` | **`build+sign` (×5 targets)** | **371** | **`[self-hosted, macOS, X64]`** | **physical Mac** | tag `v*` | — |
| `release.yml` | `sbom` | 702 | `corelink` | fabric | tag `v*` | — |
| `release.yml` | `publish` | 782 | `corelink` | fabric | tag `v*` | — |
| `conformance-canary.yml` | `canary` | 59 | `corelink` | fabric | cron daily 13:00 UTC + dispatch | — |
| `canary-deadman.yml` | `scheduled-lane dead-man's-switch` | 100 | `corelink` | fabric | cron daily 16:00 UTC + dispatch | — |
| `metered-probe-canary.yml` | `probe` | 40 | `corelink` | fabric | cron Mon 12:00 UTC + dispatch | — |
| `live-journeys.yml` | `journeys` | 52 | `corelink` | fabric | cron Mon 06:17 UTC + dispatch | — |
| `e2e-evidence.yml` | `evidence` | 75 | `corelink` | fabric | dispatch only | — |
| `fuzz.yml` | `fuzz supply-chain` + `fuzz <target>` | 44, 83 | `corelink` | fabric | cron daily 08:00 UTC + dispatch | — |
| `mutation.yml` | `cargo-mutants (new survivors in this diff)` | 121 | `corelink` | fabric | PR + dispatch | Must be green if reported |
| `stress.yml` | (2 jobs) | 33, 54 | `corelink` | fabric | cron Mon 02:00 UTC | — |

**Count: 22 `corelink` + 1 physical Mac. Zero hosted.** There is currently no
GitHub-hosted lane anywhere in this repo (org mandate *"self-hosted tudo ou não roda"*).

**Historical-count deviation.** WP-35's 2026-08-30 catalog and the first version of
this runbook correctly described that revision as 19 `corelink` + 1 Mac. Commit
`252efc6` subsequently added `ci.yml`'s `release-guards-selftest` on `corelink`; the
current tree was therefore 20 + 1. `fuzz.yml` subsequently gained its `supply-chain`
job and `pr-title.yml` added a trusted metadata job, so this tree is 22 + 1. The
emergency command must expect the current count, not silently preserve the historical
one.

**Correction to a common assumption:** CLAUDE.md still says the required gate was moved
to `ubuntu-latest` in PR #225. That was true between 2026-07-20 and 2026-08-12. **PR #233
(`77bbda9`, 2026-08-12) moved every Linux lane — including `gates` and `dco` — to
`corelink`.** The required gate is *not* on hosted today; it needs this fallback like
everything else.

---

## 3. Preconditions before you flip anything

1. **Get explicit incident authority.** The incident owner records the fabric outage,
   approves the temporary hosted-minutes exception, and designates the repository
   administrator who may merge the emergency PR. The executor opens a normal PR; no
   direct push, force-push, branch-protection bypass, or release-matrix change is part
   of this runbook.
2. **Confirm the fabric is actually the problem.** A `corelink` outage shows up as a job
   that never starts (queued, then "no runner"), not as a red gate. Check:
   ```sh
   gh run list --repo HuGR-Labs/corelink-workspaces --workflow ci.yml --limit 5
   gh api /repos/HuGR-Labs/corelink-workspaces/actions/runs/<id>/jobs \
     --jq '.jobs[]|[.name,(.labels|join(",")),.status,.conclusion]|@tsv'
   ```
3. **Confirm hosted Actions can actually run.** This is not automatic. The org is on the
   **`free` plan** (`gh api /orgs/HuGR-Labs --jq .plan.name` → `free`), which includes
   2,000 hosted minutes/month for private repos. This repo has previously sat under a
   **GitHub-Actions billing block**, during which hosted jobs did not start at all — see
   the header comment in `.github/workflows/windows.yml`, which records that the old
   `windows-latest` lane "never started (steps=0, no runner) and therefore tested
   NOTHING". A silently-blocked hosted lane is worse than a red one. This tree has no
   hosted job to dispatch before the swap; the fallback PR's eight-job smoke (§4.2) can
   only probe the ordinary `pull_request` lanes. It is **not** a merge-restoring drill:
   `pr-title.yml` still executes its base workflow on `corelink` (§4.3).
   The last successful hosted run in this repo was **2026-08-03** (run `30780253565`),
   so hosted worked at that date; it has not been exercised since.
4. **You are deliberately spending money/minutes.** §7 has the measured numbers.

---

## 4. Scenario A — `corelink` is down: attempted merge recovery

### 4.1 The change

It is a **label swap only**. Historical hosted evidence directly covers `gates` and
`dco`; their migration changed the runner label and comments, not their steps. A
predecessor coverage lane also passed an active 88% floor on hosted, but it had a
different check name and an unpinned `cargo-llvm-cov`; that is useful history, not proof
of the current renamed/pinned coverage shape. See §9.

The `gate-check-selftest` and `release-guards-selftest` jobs are **not** in that proof:
both postdate the migration (#233), so neither has current-tree hosted evidence. They
are offline bash/python/jq self-tests and are expected to be portable, not proven.

The remaining fabric jobs are **expected to be portable, not proven**. Five of
them did get fabric-specific step edits in #233 (`rustup show` replacing
`rustup toolchain install/default` in `conformance-canary` and `live-journeys`; the
`blake3` venv in `metered-probe`; the `shellcheck` apt install; the
`CLW_CRED_TICKET`/`LEASE_ID`/`FABRIC_ENDPOINT` nulling). Reading each, all are
portable or merely redundant on `ubuntu-latest` — a venv works anywhere, hosted
images ship `rustup` and `shellcheck`, and the credential nulling restores the
pre-migration hosted behaviour. But nothing here executed them hosted after #233,
so §9 lists them as unproven rather than this section claiming them.

### 4.2 The fallback PR has more checks than the required-presence list

`scripts/pre-merge-gate-check.sh` requires the **presence** of `dco`, `gates`,
`llvm-cov`, and `pr-title`, the four always-present signals. That is not a permission to
leave the other checks queued: after that presence test, the script rejects
**every** check whose bucket is not `pass` or `skipping`, including `pending` and
`cancel`. A fabric outage therefore blocks a fallback PR if any workflow that its diff
triggers is still on `corelink`.

For a short outage, **seven** runner-line swaps in four files can smoke-test the ordinary
PR-head workflows: the four `ci.yml` jobs, `coverage.yml`, `shellcheck.yml`, and
`mutation.yml`. All four workflows have a bare `pull_request` trigger. `mutation.yml`
obtains a runner even when its workflow-only diff has no Rust changes (then its mutation
step skips and the job succeeds). This is explicitly **not merge-restoring**: no PR-head
edit can move `pr-title.yml`'s `pull_request_target` job off the base branch's
`corelink` label during the outage.

Adding the **eighth** swap, `windows.yml`, broadens this partial smoke to the Windows
lane. It still cannot be merged as an outage recovery while the trusted PR-title check
uses the base workflow. The all-lane mechanical change below includes it, but does not
remove that blocker.

For an inventory audit or a future owner-approved recovery design, §4.3 gives the exact
all-lane mechanical change. Because it also edits `windows.yml`, that PR additionally
triggers Windows smoke. Its ordinary PR-head smoke set is exactly these **eight** jobs:

* `gates`, `dco`, `gate-check-selftest`, and `release-guards-selftest` (`ci.yml`)
* `llvm-cov (floor 88%)` (`coverage.yml`)
* `shellcheck + actionlint` (`shellcheck.yml`)
* `cargo-mutants (new survivors in this diff)` (`mutation.yml`)
* `windows binary smoke (mingw build, wine run)` (`windows.yml`)

The eight-job result is evidence only for those ordinary PR-head workflows. It cannot
authorize a merge during an outage: all four required-presence names must appear, and
the base-branch `pr-title` check remains pending on `corelink`. A red, cancelled, or
pending lint/mutation/Windows check is also a stop condition even though it is not one
of the four names in `REQUIRED_PRESENT`.

### 4.3 Exact all-lane mechanical commit

The complete mechanical label change is **22 line replacements in 14 workflow files**.
It leaves the physical Mac expression untouched. Run this from a clean checkout of the
current base; it fails rather than guessing when the inventory has changed. This is an
inventory-and-diff procedure, **not** proof that the resulting PR can merge: the
`pull_request_target` exception immediately after the command is a separate blocker.

**Read this before running it.** The target list matches 14 files, and that includes
`release.yml` — whose four Linux jobs (`preflight`, `conformance`, `sbom`, `publish`)
run on `corelink` and DO need the swap during a fabric outage. This does not conflict
with §6: §6 says do not try to relocate the release **build+sign** matrix, and that
matrix is safe here because `[self-hosted, macOS, X64]` does not match the replacement.
It is left exactly as it is. `windows.yml`, `e2e-evidence.yml`,
`canary-deadman.yml`, and `release.yml` being in the diff is intentional.

```sh
# From the repo root, on a new emergency branch based on origin/main.
set -euo pipefail
expected_inventory_base=a5bcbcc047ce9e522b2b80afe33b36181dacb3c0
git fetch origin main
test "$(git rev-parse origin/main)" = "$expected_inventory_base" || {
  echo "origin/main differs from the audited runner inventory; stop and re-audit this runbook" >&2
  exit 1
}
test -z "$(git status --porcelain)"
git switch -c ci/fallback-corelink-outage origin/main
test "$(git rev-parse HEAD)" = "$expected_inventory_base"
command -v rg >/dev/null || { echo "rg is required for these fail-closed inventory checks" >&2; exit 127; }

workflow_files="$(rg --files .github/workflows | rg '\.ya?ml$' | wc -l | tr -d ' ')"
runner_declarations="$(rg -n '^[ \t]*runs-on:' .github/workflows | wc -l | tr -d ' ')"
corelink_lines="$(rg -n '^[ \t]*runs-on:[ \t]*corelink[ \t]*$' .github/workflows | wc -l | tr -d ' ')"
corelink_files="$(rg -l '^[ \t]*runs-on:[ \t]*corelink[ \t]*$' .github/workflows | wc -l | tr -d ' ')"
physical_mac="$(rg -n '^[ \t]*runs-on:[ \t]*\[self-hosted, macOS, X64\][ \t]*$' .github/workflows | wc -l | tr -d ' ')"
test "$workflow_files" = 14
test "$runner_declarations" = 23
test "$corelink_lines" = 22
test "$corelink_files" = 14
test "$physical_mac" = 1
rg -l '^[ \t]*runs-on:[ \t]*corelink[ \t]*$' .github/workflows | sort

# Perl edits each workflow line in place on macOS and Linux; it does not select
# the physical-Mac expression. The clean locale avoids host locale-dependent Perl
# failures; horizontal whitespace avoids consuming the YAML newline. Do not replace
# this with a history revert.
env LC_ALL=C LANG=C LANGUAGE=C perl -pi -e 's/^([ \t]*runs-on:[ \t]*)corelink[ \t]*$/${1}ubuntu-latest/' .github/workflows/*.yml

test "$(rg -n '^[ \t]*runs-on:[ \t]*corelink[ \t]*$' .github/workflows | wc -l | tr -d ' ')" = 0
test "$(rg -n '^[ \t]*runs-on:[ \t]*\[self-hosted, macOS, X64\][ \t]*$' .github/workflows | wc -l | tr -d ' ')" = 1
test "$(git diff --name-only -- .github/workflows | wc -l | tr -d ' ')" = 14
test "$(git diff --numstat -- .github/workflows | awk '{ add += $1; del += $2 } END { print add ":" del }')" = 22:22
git diff --check
git diff -- .github/workflows

git add .github/workflows
git commit -s -m 'ci: temporary hosted fallback for corelink outage'
```

The preflight count is intentionally 14 files: this repository currently has 14 workflow
files and 23 `runs-on:` declarations total. A dirty start, a new workflow, or a new
runner line is a reason to stop and update the inventory, not to weaken the assertions.

**`pull_request_target` blocker — do not overclaim this procedure.** `pr-title.yml`
runs only the base branch workflow. A fallback PR that changes its `runs-on:` line still
dispatches that PR-title check from the base, where it remains `corelink`; during a fabric
outage it can stay pending. `pre-merge-gate-check.sh` rejects a reported pending check.
Consequently, this label-only PR is **not an actually executed, merge-restoring hosted
fallback drill for the current tree**. It cannot establish current Actions health, and it
does not authorize a direct push or branch-protection bypass to alter the base workflow.
An owner-approved recovery design for this trusted metadata lane, then a fresh hosted
drill, is required before claiming this runbook closes WP-35's acceptance criterion.

### 4.4 Do NOT do it by reverting the migration commit

`git revert 77bbda9` (PR #233) looks like the obvious move. **It is wrong, twice over:**

* **#233 was not workflow-only.** It also shipped the clw-cli *test-hermeticity* fixes
  (`crates/clw-cli/src/config.rs`, `tests/e2e.rs`, `tests/e2e_portable.rs`) that the
  migration surfaced — the config + e2e tests were inheriting the fabric's ambient env-0
  broker credentials. Reverting the commit reverts those source fixes too.
* **`ci.yml` has moved on since.** `gate-check-selftest` and
  `release-guards-selftest` were added after #233. Restoring the pre-#233 file
  wholesale silently deletes both current self-tests.

Edit the `runs-on:` lines in place. Do not restore old file versions.

### 4.5 The bootstrap problem

For the ordinary `pull_request` workflows, the PR that performs the swap is expected to
pass the eight-job set in §4.2: its head changes their labels. Verify those eight jobs
run on `ubuntu-latest`, rather than remaining queued. **That is not sufficient to
merge this tree during a fabric outage:** `pr-title.yml` uses `pull_request_target`, so
its pending base-workflow check is still rejected by `pre-merge-gate-check.sh`. The
separate §4.3 blocker must be resolved before treating the smoke as an outage recovery.

Also note that a `.github/` touch escalates `scripts/gates.sh` to the **FULL** lane
through `scripts/classify-lane.sh:65` — the fallback PR will pay full-workspace compile
time, not the docs-only fast path.

---

## 5. What breaks when you move to hosted

Verified against the tree and against the last hosted runs. In descending order of how
much it will surprise you.

1. **Builds are cold per CI job by design — this is NOT a new breakage.** `gates.sh`
   assigns `CARGO_TARGET_DIR` under `RUNNER_TEMP`, keyed to `GITHUB_RUN_ID` and attempt.
   Cargo commands (including the unsafe-posture compiler sentinel) reuse it within that
   gates invocation, but a later run gets a different directory. There is no
   `actions/cache` build-cache step anywhere in `.github/workflows/`; each CI job starts
   cold. Moving to hosted therefore costs nothing extra here — the measured
   hosted-vs-fabric delta in §7 is ~1.5 min, which is CPU, not cache.

2. **The host-toolchain assumption is Mac-only and does not apply here.** The Linux lanes
   provision Rust with `rustup show` against the image's preinstalled rustup, which works
   identically on `ubuntu-latest` (measured: 7s on hosted, run `30780253565`; 1s on
   `corelink`, run `32776068642` — the fabric image bakes the pinned toolchain, hosted
   downloads it). The load-bearing host-toolchain assumption lives in
   `scripts/ci-use-host-toolchain.sh`, which is called **only** from the Mac release
   matrix — see §6.

3. **The windows-gnu cross-check needs mingw, and it installs the same way.**
   `gates.sh` line 151 runs `cargo check --workspace --target x86_64-pc-windows-gnu` in
   *both* lanes as a universal tripwire; `ring`'s C build script needs
   `x86_64-w64-mingw32-gcc`. `ci.yml` installs it via `sudo apt-get install -y
   gcc-mingw-w64-x86-64`. Passwordless sudo + apt exist on **both** `corelink` and
   `ubuntu-latest`, so the step is portable as written. Measured: 25s hosted vs 19s
   fabric. **The cross-check is a `cargo check` (type-check only) — it needs no
   cross-linker and no Windows runner.** Do not "helpfully" set `CLW_SKIP_CROSS=1` to
   speed up a fallback lane; that silently removes the only Windows tripwire on the
   required gate.

4. **`windows.yml` is a Wine lane, not a Windows lane.** It builds `clw.exe` with mingw
   on `corelink` (Linux) and runs it under Wine. On `ubuntu-latest` the same
   `apt-get install wine gcc-mingw-w64-x86-64` works. Do **not** "fix" it by switching to
   `windows-latest`: that lane previously existed, never started under the billing block,
   and tested nothing (see the file's own header). Wine fidelity for NTFS ADS /
   drive-letter / symlink-fallback semantics is explicitly *not* proven by this lane
   either way — a real Windows runner remains a Phase-3 item.

5. **Nothing is secret-blocked.** Every credential the fabric lanes use is a **GitHub
   repository secret**, not runner-local state — for example `CLW_CI_TOKEN`,
   `CLW_CI_TENANT`, `CLW_CI_ENDPOINT`, and the minisign key. Secrets are injected by
   GitHub into whatever runner the job lands on. The live-conformance and canary lanes
   worked on hosted for that reason until 2026-08-12.

6. **Required-check names are unaffected.** Where branch protection exists it matches
   **job names**, not runner labels. `gates`, `dco`, `llvm-cov (floor 88%)`, and
   `pr-title` keep their names across a label change, and
   `scripts/pre-merge-gate-check.sh` requires all four. The special problem is not a
   renamed check: `pr-title` executes the base workflow for `pull_request_target`, so
   a PR-head label change cannot affect its runner (§4.3).

   **But see §10.4: there is probably no branch protection on this repository at all.**
   If so, the practical version of this point is stronger and less comforting — nothing
   mechanically re-checks anything after the swap, so during a fallback the person doing
   it is the control. Run the full gate locally before merging, as CLAUDE.md already
   says, and do not rely on a red check to stop you.

7. **Egress differs.** `corelink` boxes are CF microVMs; `ubuntu-latest` runs inside
   GitHub's network. Lanes that talk to live prod (`conformance-canary`,
   `live-journeys`, `metered-probe-canary`, `e2e-evidence`, `release.yml`'s conformance
   gate) will hit the CoreLink API from a different source network and a different
   region. That changes latency and, if any allowlist is IP-based, reachability. These
   lanes ran fine on hosted before 2026-08-12, so no allowlist is known to block them —
   but re-verify before you trust a canary's green.

8. **The org "zero-hosted" mandate is being deliberately suspended.** Every fabric lane
   carries a comment saying so (*"self-hosted tudo ou não roda"*). The fallback PR should
   say plainly in its body that this is a temporary, cost-incurring exception and name
   the outage.

---

## 6. Scenario B — `clw-mac-1` is down: releases stop

**State this plainly: during a Mac outage, releases stop. Merging continues.**

`release.yml`'s `build+sign` matrix (line 371) builds all five targets on ONE physical
Intel Mac. There is **no label-swap fallback**, for reasons that are structural, not
budgetary:

* **`scripts/ci-use-host-toolchain.sh` hard-fails on an unprovisioned hosted macOS
  runner.** It refuses to provision anything and looks for
  `$HOME/.rustup/toolchains/<channel>-x86_64-apple-darwin` (`HOST_TRIPLE` defaults to
  `x86_64-apple-darwin`), exiting 1 with *"the workspace-pinned toolchain … is not
  installed on this runner host"* when absent. The historical exercise's `macos-14`
  label was arm64; [GitHub also offers hosted Intel labels today](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
  That availability does not recreate this physical Mac's provisioned toolchain or make
  an Intel hosted label a drop-in replacement. The script exists specifically to forbid
  hosted-style `rustup install` on the shared-`$HOME` fleet.
* **zig comes from `brew install zig` on the host, and the step is fail-closed.**
  `release.yml` line 426 checks `command -v zig` and errors out with
  *"do NOT fall back to a hosted setup action — zero-hosted mandate"*. Four of five
  targets (both Linux, windows-gnu) cross-compile through `cargo-zigbuild`.
* **The whole matrix shape is Mac-specific.** Reverting to hosted is not a `runs-on:`
  edit; it is undoing ~15 steps of PR #234 (`6cdf136`): the per-OS `matrix.os` fan-out,
  native `cargo build` per platform, the mingw linker step, and dropping the zigbuild
  path, the zigcc `chmod +x` exit-126 guard, and the `windows-rs` cache re-fetch.

**Why an Intel hosted label is still not a trivial fallback:** the signing material is
GitHub-secret based rather than intrinsically tied to this host (§5.6), but its current
availability and any release-state assumptions must be audited for a replacement runner.
More importantly, the pinned toolchain and zig live on the physical host filesystem and
the workflow deliberately refuses to provision them in-place. The historical arm64
`macos-14` result does not prove an Intel hosted path, and the existence of current Intel
labels does not reproduce the physical runner's toolchain, zig, secrets access, or
provisioned state. The conclusion — no trivial fallback — is correct.

**It is not imaginary, though.** A fully hosted release topology existed in this repo and
**shipped v0.1.11 green on 2026-07-17** (run `29544932935`: `preflight`/`conformance`/
`sbom`/`publish` on `ubuntu-latest`, both darwin targets on `macos-14`, windows-gnu on
`windows-latest`, aarch64-linux on `ubuntu-24.04-arm`). So the recovery path for a
prolonged Mac outage is *restore the PR #234 predecessor shape*, not *invent one* — a
half-day of work, with a historical legacy allowance-equivalent of ~77 hosted minutes
(§7), and it has not been re-executed against today's `release.yml`. Treat it as a
planned project, not an incident action.

**Incident action for a Mac outage:** do nothing to `release.yml`. Announce that releases
are paused. `vX.Y.Z-rc.N` dry-runs are equally blocked (same matrix). Merging, canaries
and all PR gates are unaffected — they are on `corelink`.

---

## 7. Cost in hosted minutes

Plan: **GitHub Free org, private repo → 2,000 included minutes/month.** Every job is
rounded up to the next whole minute. Current GitHub-hosted standard-runner rates are
Linux x64 $0.006/min, Linux arm64 $0.005/min, Windows $0.010/min, and macOS $0.062/min;
see [GitHub's Actions billing documentation](https://docs.github.com/en/billing/concepts/product-billing/github-actions).
The fallback in this section uses Linux runners, so its estimates are runner minutes;
do not apply the old Windows/macOS allowance multipliers to current pricing.

### Measured wall-clock (real runs, not estimates)

| Job | On `corelink` | On `ubuntu-latest` | Hosted delta |
|---|---|---|---|
| `gates` (FULL lane, sample 1) | 6m19s — main, `32791835263` | 7m32s — main, `30780253565` | +1m13s |
| `gates` (FULL lane, sample 2) | 6m13s — PR, `32776068642` | 8m16s — main, `29714656627` | +2m03s |
| `dco` | 9s | 5s | — |
| `llvm-cov (floor 88%)` | 4m43s (`32791835190`) | 6m23s (`30780253564`) | +1m40s |
| `gate-check-selftest` | 9s | *unmeasured* (job postdates the migration) | — |
| `release-guards-selftest`, lint, mutation, Windows smoke | *unmeasured on hosted in this shape* | *unmeasured* | Budget at least one Linux minute per started job |

### Lower-bound runner minutes for Scenario A (Linux fallback)

| Event | One-minute-floor calculation | Lower bound before unmeasured overage |
|---|---|---|
| Seven-swap PR-head smoke (not merge-restoring) | gates 8–9 + dco 1 + two CI self-tests 2 + coverage 7 + lint 1 + mutation 1 | **≥20–21 + U₇** |
| Eight-swap PR-head smoke (not merge-restoring) | Seven-swap floor + Windows smoke 1 (because `windows.yml` changes) | **≥21–22 + Ufull** |
| Hypothetical post-merge push after separate PR-title recovery | gates 8–9 + dco 1 + two CI self-tests 2 + coverage 7 + lint 1 + Windows smoke 1 | **≥20–21 + Umain** |
| Eight-swap smoke + hypothetical post-merge push | sum of the preceding smoke and main-push floors | **≥41–43 + Ufull + Umain** |

`U₇` is the hosted runtime above the one-minute floor for the two CI self-tests, lint,
and mutation; `Ufull` adds Windows smoke; `Umain` covers the two self-tests, lint, and
Windows smoke on the post-merge run. None is measured for the current hosted shape, so
the table is a **lower bound, not an approval estimate**. In particular, lint's apt
install, mutation's setup, and Windows' apt/build/smoke can materially exceed one minute.
`ci.yml`, `coverage.yml`, `mutation.yml`, and `windows.yml` use
`cancel-in-progress: true`; those guards reduce superseded-run spend and must not be
removed during a fallback.

Adding the scheduled lanes (canaries daily, fuzz nightly, stress/journeys weekly) on top
of that consumes the allowance considerably faster. Neither a seven- nor eight-swap PR
can restore merging while `pr-title.yml` is evaluated from the base branch. Do not infer
a green post-merge run from either smoke. Only after an owner-approved PR-title recovery
design and fresh drill may an operator budget a complete fallback. The 22-line all-lane
mechanical change remains useful inventory evidence, not incident authorization.

### Historical hosted-release workload (run `29544932935`, v0.1.11)

The next table preserves the **legacy allowance-equivalent** calculation recorded with
that 2026-07 run. It is not a current-price quote: current per-minute prices are above.

| Job | Runner | Wall | Legacy multiplier | Legacy allowance-equivalent |
|---|---|---|---|---|
| preflight | ubuntu-latest | 7m13s | ×1 | 8 |
| conformance | ubuntu-latest | 1m28s | ×1 | 2 |
| sbom | ubuntu-latest | 20s | ×1 | 1 |
| build x86_64-pc-windows-gnu | windows-latest | 6m04s | ×2 | 14 |
| build x86_64-apple-darwin | macos-14 | 1m21s | ×10 | 20 |
| build aarch64-apple-darwin | macos-14 | 1m38s | ×10 | 20 |
| build aarch64-unknown-linux-gnu | ubuntu-24.04-arm | 2m20s | ×1* | 3 |
| build x86_64-unknown-linux-gnu | ubuntu-latest | 3m03s | ×1 | 4 |
| checksum + publish | ubuntu-latest | 4m25s | ×1 | 5 |
| | | | | **≈ 77 (historical)** |

\* This was the historical calculation's conservative arm64-Linux assumption, not a
current billing multiplier.

**The two macOS legs were 40 of the historical 77 allowance-equivalent minutes** for
3 minutes of actual work. That historical comparison motivated the Mac topology; use the
current per-minute rates above for any new cost decision.

---

## 8. Switching back

1. Open a PR reverting **only** the `runs-on:` lines (`ubuntu-latest` → `corelink`).
   If the fallback landed as a single self-contained commit touching nothing but
   `runs-on:` lines, `git revert <sha>` is safe here — verify with `git show --stat`
   that the commit really is workflow-only before trusting that.
2. After a separately approved recovery design has made a hosted fallback possible, the
   revert PR's ordinary head-ref checks run on `corelink` again. Verify those runner
   labels and the trusted PR-title check separately before trusting a green result. Then
   run `bash scripts/pre-merge-gate-check.sh <PR>`; do not merge while any check is
   pending, cancelled, red, or missing from the four required-presence names.
3. Do **not** use `gh workflow run shellcheck.yml` on the default branch as a recovery
   probe: after a full fallback, that branch still contains `ubuntu-latest`. If a manual
   dispatch is necessary, use an explicit recovery ref whose workflow file has first
   been checked to contain `runs-on: corelink`, and verify the dispatched run's job
   labels in Actions. The revert PR's own checks remain the primary recovery proof.
4. Delete any temporary `actions/cache` steps if you added them; they are not part of
   the baseline (§5.1) and leaving one behind is silent drift.
5. Update CHANGELOG under `[Unreleased] → CI / Infrastructure` noting the window during
   which the repo spent hosted minutes, so the "zero-hosted" claim in the docs stays
   honest.

---

## 9. What is proven and what is not

### Acceptance criterion 1 — **UNMET**

The work package asks for a fresh test PR demonstrating the required lanes green on
`ubuntu-latest`, linked from here. **No such PR was opened.** Reasons, stated plainly:

* The work package's own scope boundary forbids modifying `.github/workflows/*.yml`
  (three other work packages are in flight on those files), and a test PR requires
  exactly that edit.
* This work package was instructed to deliver a branch locally — not to push or merge.

**There is no link to a green hosted test run for the current tree.** Anyone relying on
this runbook in an incident is relying on §9's historical evidence, not on a fresh test.
The outstanding work first needs a recovery design for `pr-title.yml`'s
`pull_request_target` behavior (§4.3), then a fresh hosted drill. A full all-lane
mechanical change has a **≥21–22-minute floor plus Ufull** before any post-merge run;
that lower-bound estimate does not establish that Actions is currently healthy or
billable.

### What IS proven — by production execution, not by reasoning

| Claim | Evidence |
|---|---|
| `gates` passes on `ubuntu-latest` with the exact steps `ci.yml` has today | Run `30780253565` (main, 2026-08-03, 7m32s, success) and `29714656627` (main, 2026-07-20, 8m16s, success). PR #233's diff shows the gates job's *steps* were unchanged by the migration — only the label and the comments changed. |
| `dco` passes on `ubuntu-latest` | Same runs (5s, success). |
| A predecessor coverage lane enforced an 88% floor on `ubuntu-latest` | Run `30780253564` (2026-08-03, 6m23s, success). At that point its check was named `llvm-cov (report-only)` and used unpinned `cargo-llvm-cov`; its script did enforce the 88% floor. This is not fresh proof of today's renamed/pinned coverage job. |
| The label swap is sufficient — no step edits needed | `git show 77bbda9 -- .github/workflows/ci.yml`: the `gates` job diff is the `runs-on:` line plus comments. |
| A fully hosted release works | Run `29544932935` — v0.1.11 shipped from hosted runners, 2026-07-17. |
| Gates target reuse is job-local, not persistent | WP-50 (`5b29d69`) made `gates.sh` derive `CARGO_TARGET_DIR` below `RUNNER_TEMP` from `GITHUB_RUN_ID` and attempt; `scripts/ci-build-target-selftest.sh` verifies reuse within one invocation and separation across runs. |

### What is NOT proven

* **The current self-tests on hosted.** `gate-check-selftest` and
  `release-guards-selftest` postdate the migration and have no `ubuntu-latest` run for
  this tree. They are offline bash/python/jq jobs, so portability is expected, not
  proven.
* **The current coverage shape on hosted.** The historical successful coverage run is
  the predecessor described above; current `llvm-cov (floor 88%)` uses a renamed check
  and pinned `cargo-llvm-cov@0.9.0`, neither freshly exercised on hosted.
* **`windows.yml` (Wine) on `ubuntu-latest` in its current shape.** Wine + mingw install
  from apt on both images, but this lane was authored for `corelink`.
* **Hosted Actions being currently *billable*.** Nobody has confirmed the org's hosted
  minute balance since 2026-08-03 (the billing API endpoints used here return 403/410
  without `admin:org`). See §3.2 — this is the single most likely way the fallback fails
  in an incident.
* **The Scenario-B hosted release path against today's `release.yml`.** §6.
* **A normal fallback PR can restore this tree while `pr-title.yml` remains on
  `pull_request_target`.** Its job executes the base workflow, so the label change in
  the PR head does not take effect for that check; §4.3 explains why this blocks the
  claimed drill.
* **The non-historical fabric lanes on hosted, after #233.** The current `shellcheck`,
  mutation, Windows-smoke, scheduled, manual, and release-support jobs have not all been
  exercised on hosted in their current form. Reading each is not execution: treat the
  all-lane command as mechanically verified but operationally unproven beyond the three
  historically hosted gates above.

---

## 10. Repository facts that contradict the docs (found while writing this)

Recorded here because a runbook is only as good as the assumptions under it. None of
these were fixed by this work package (docs-only scope).

1. **CLAUDE.md is stale on the required gate's runner.** It says PR #225 moved `gates` +
   `dco` to `ubuntu-latest`. PR #233 moved them to `corelink` three weeks later. As of
   this tree, **no lane in this repo runs on a GitHub-hosted runner.**
2. **`scripts/gates.sh` line 141 has a stale comment**: *"CI runs on a Linux (ubuntu)
   GitHub-hosted runner."* CI runs on `corelink`. The comment's *conclusion* (the
   cfg-divergent target needs cross-checking) is still correct.
3. **`release.yml` contradicts itself about the Mac's architecture.** Line 327 and the
   `X64` label say Intel host; the matrix comments at lines 343–350 describe an
   "Apple-Silicon host" doing native cross. `scripts/ci-use-host-toolchain.sh` defaults
   `HOST_TRIPLE=x86_64-apple-darwin`, which agrees with the Intel reading. One of the two
   comment blocks is wrong.
4. **`main` is NOT branch-protected, and on this plan it cannot be.** Verified
   independently, three endpoints:

   ```
   GET /repos/HuGR-Labs/corelink-workspaces/branches/main/protection  -> 403
   GET /repos/HuGR-Labs/corelink-workspaces/rulesets                  -> 403
   GET /repos/HuGR-Labs/corelink-workspaces/rules/branches/main       -> 403
       "Upgrade to GitHub Pro or make this repository public to enable this feature."
   GET /orgs/HuGR-Labs           -> plan: free
   GET /repos/.../corelink-workspaces -> private: true
   ```

   A private repository on a Free organisation plan cannot have protected branches or
   rulesets — the feature is not available, so this is not "protection is switched off"
   and there is nothing to switch back on. CLAUDE.md states that `main` requires the
   `gates` + `dco` checks green with `enforce_admins`, force-push blocked and linear
   history. None of that is in force.

   **This is the most consequential item in this section**, because it changes what every
   other control in the repository is worth: `scripts/gates.sh` is a convention, not a
   merge blocker, and "merge on green" is a discipline people keep, not a rule the
   platform enforces. The remedies all cost something and none is an engineering
   decision — make the repository public, move to a paid plan, or stop asserting the
   control and say plainly that process discipline is what holds the line. Owner call;
   raise it ahead of this runbook.
5. **The four required-presence checks constrain fallback recovery.**
   `pre-merge-gate-check.sh` requires `dco`, `gates`, `llvm-cov`, and `pr-title`.
   The first three are ordinary PR-head checks; `pr-title` is the trusted
   `pull_request_target` workflow. The subsequent all-check loop also blocks any
   reported lint, mutation, self-test, or title job that is pending, cancelled, or red.
   That executable behavior is why the partial seven/eight-swap smoke cannot restore
   merging (§4.2–§4.3).
6. **WP-50 removed the old `CARGO_TARGET_DIR` persistent-cache assumption.** In CI,
   `gates.sh` now derives a path below `RUNNER_TEMP` from `GITHUB_RUN_ID` and
   `GITHUB_RUN_ATTEMPT`; `ci-build-target-selftest.sh` proves one gates invocation
   reuses it while another run cannot. It is deliberately job-local, not an
   `actions/cache` restore.

---

## Appendix — commands used to build this runbook

```sh
grep -rn "runs-on" .github/workflows/                       # the inventory (§2)
git log --oneline -- .github/workflows/ci.yml               # #122 -> #144 -> #225 -> #233
git show 77bbda9 -- .github/workflows/ci.yml                # the exact label-swap diff
git show 6cdf136 -- .github/workflows/release.yml           # the Mac release migration (#234)
gh api /orgs/HuGR-Labs --jq .plan.name                      # -> free
gh api /repos/HuGR-Labs/corelink-workspaces/actions/runners # -> clw-mac-1 only (corelink is org-level)
gh api /repos/HuGR-Labs/corelink-workspaces/actions/runs/<id>/jobs \
  --jq '.jobs[]|[.name,(.labels|join(",")),.started_at,.completed_at]|@tsv'
```

Run IDs cited: `32791835263`, `32776068642`, `32791835190` (corelink);
`30780253565`, `30780253564`, `29714656627` (hosted CI); `29544932935` (hosted release,
v0.1.11).
