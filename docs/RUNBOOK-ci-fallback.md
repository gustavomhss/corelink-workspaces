# RUNBOOK — returning CI to GitHub-hosted runners during a self-hosted outage

**Audience:** whoever is on the hook for "nothing can merge / nothing can ship."
**Scope:** how to move this repo's CI lanes off the self-hosted runners and back onto
GitHub-hosted runners, what it costs, what it breaks, and how to undo it.

> **Status of this runbook — read this first.**
> Acceptance criterion 1 of the work package (a *fresh* test PR proving the required
> lanes green on `ubuntu-latest`) is **UNMET**. See [§9](#9-what-is-proven-and-what-is-not)
> for exactly what is proven, what is not, and why. Everything in §4 (the Linux
> fallback) is nonetheless grounded in configurations this repo **actually ran in
> production** — run IDs are cited throughout. Nothing here is a guess unless it is
> explicitly labelled an estimate.

---

## 1. The two outages are not the same outage

This repo has **two** independent self-hosted dependencies. They fail separately and
have completely different fallbacks.

| # | Dependency | What it is | What dies with it | Fallback |
|---|---|---|---|---|
| A | `corelink` | CoreLink's own ephemeral CF-Firecracker runner fabric. 19 of the 20 `runs-on:` declarations in this repo. A fresh microVM per job. | **Merging.** `gates`, `dco`, `gate-check-selftest`, `llvm-cov` — every PR-blocking check. Plus release preflight/conformance/sbom/publish, all canaries, all nightly lanes. | **Yes — §4.** A `runs-on:` label swap. Proven in production until 2026-08-12. |
| B | `clw-mac-1` | ONE physical Intel Mac (labels `self-hosted, macOS, X64`). Exactly 1 `runs-on:` declaration: `release.yml`'s 5-target `build+sign` matrix. | **Releasing.** No binaries, no signatures, no tag ships. | **No trivial one.** See §6. During a Mac outage, **releases stop; merging continues.** |

If **both** are down, merging can be restored (§4) and releasing cannot be restored
cheaply (§6). Restore merging first.

---

## 2. Runner inventory (verified against the tree, 2026-08-30)

Every `runs-on:` in `.github/workflows/`. 13 workflow files, 20 declarations.

| Workflow | Job | Line | `runs-on:` | Class | Trigger | Blocks a merge? |
|---|---|---|---|---|---|---|
| `ci.yml` | `gates` | 27 | `corelink` | fabric | PR + push(main) | **YES — required check** |
| `ci.yml` | `dco` | 54 | `corelink` | fabric | PR + push(main) | **YES — required check** |
| `ci.yml` | `gate-check-selftest` | 71 | `corelink` | fabric | PR + push(main) | Runs on every PR |
| `coverage.yml` | `llvm-cov (report-only)` | 36 | `corelink` | fabric | PR + push(main) | Yes — `pre-merge-gate-check.sh` demands it be PRESENT |
| `shellcheck.yml` | `shellcheck` | 28 | `corelink` | fabric | PR (`scripts/**`) + push(main) | Conditionally |
| `windows.yml` | `smoke` | 51 | `corelink` | fabric | PR (`crates/**`, `Cargo.*`) + push(main) | Conditionally |
| `release.yml` | `preflight` | 73 | `corelink` | fabric | tag `v*` | — |
| `release.yml` | `conformance` | 243 | `corelink` | fabric | tag `v*` | — |
| `release.yml` | **`build+sign` (×5 targets)** | **334** | **`[self-hosted, macOS, X64]`** | **physical Mac** | tag `v*` | — |
| `release.yml` | `sbom` | 665 | `corelink` | fabric | tag `v*` | — |
| `release.yml` | `publish` | 744 | `corelink` | fabric | tag `v*` | — |
| `conformance-canary.yml` | `canary` | 48 | `corelink` | fabric | cron daily 13:00 UTC | — |
| `canary-deadman.yml` | (deadman) | 30 | `corelink` | fabric | cron daily 16:00 UTC | — |
| `metered-probe-canary.yml` | (probe) | 40 | `corelink` | fabric | cron Mon 12:00 UTC | — |
| `live-journeys.yml` | (journeys) | 43 | `corelink` | fabric | cron Mon 06:17 UTC | — |
| `e2e-evidence.yml` | (evidence) | 61 | `corelink` | fabric | manual only | — |
| `fuzz.yml` | (fuzz) | 36 | `corelink` | fabric | cron daily 08:00 UTC | — |
| `mutation.yml` | (mutants) | 39 | `corelink` | fabric | cron Mon 04:00 UTC | — |
| `stress.yml` | (2 jobs) | 33, 54 | `corelink` | fabric | cron Mon 02:00 UTC | — |

**Count: 19 `corelink` + 1 physical Mac. Zero hosted.** There is currently no
GitHub-hosted lane anywhere in this repo (org mandate *"self-hosted tudo ou não roda"*).

**Correction to a common assumption:** CLAUDE.md still says the required gate was moved
to `ubuntu-latest` in PR #225. That was true between 2026-07-20 and 2026-08-12. **PR #233
(`77bbda9`, 2026-08-12) moved every Linux lane — including `gates` and `dco` — to
`corelink`.** The required gate is *not* on hosted today; it needs this fallback like
everything else.

---

## 3. Preconditions before you flip anything

1. **Confirm the fabric is actually the problem.** A `corelink` outage shows up as a job
   that never starts (queued, then "no runner"), not as a red gate. Check:
   ```sh
   gh run list --repo HuGR-Labs/corelink-workspaces --workflow ci.yml --limit 5
   gh api /repos/HuGR-Labs/corelink-workspaces/actions/runs/<id>/jobs \
     --jq '.jobs[]|[.name,(.labels|join(",")),.status,.conclusion]|@tsv'
   ```
2. **Confirm hosted Actions can actually run.** This is not automatic. The org is on the
   **`free` plan** (`gh api /orgs/HuGR-Labs --jq .plan.name` → `free`), which includes
   2,000 hosted minutes/month for private repos. This repo has previously sat under a
   **GitHub-Actions billing block**, during which hosted jobs did not start at all — see
   the header comment in `.github/workflows/windows.yml`, which records that the old
   `windows-latest` lane "never started (steps=0, no runner) and therefore tested
   NOTHING". A silently-blocked hosted lane is worse than a red one. Before relying on
   the fallback, dispatch one cheap hosted job and confirm it gets a runner.
   The last successful hosted run in this repo was **2026-08-03** (run `30780253565`),
   so hosted worked at that date; it has not been exercised since.
3. **You are deliberately spending money/minutes.** §7 has the measured numbers.

---

## 4. Scenario A — `corelink` is down: restore merging

### 4.1 The change

It is a **label swap only**, and for THREE jobs that is proven: `gates`, `dco` and
`llvm-cov` ran on `ubuntu-latest` before the migration with their `rustup show`,
`taiki-e/install-action` and `sudo apt-get install` steps unchanged — see the runs
linked from §9.

The fourth merge-blocking job, `gate-check-selftest`, is **not** in that proof and must
not be read into it: it was added by #240, *after* the migration (#233), so it has never
run on a hosted runner. §9 lists it as expected-not-proven and that is where it stays.

The other fifteen fabric jobs are **expected to be portable, not proven**. Five of
them did get fabric-specific step edits in #233 (`rustup show` replacing
`rustup toolchain install/default` in `conformance-canary` and `live-journeys`; the
`blake3` venv in `metered-probe`; the `shellcheck` apt install; the
`CLW_CRED_TICKET`/`LEASE_ID`/`FABRIC_ENDPOINT` nulling). Reading each, all are
portable or merely redundant on `ubuntu-latest` — a venv works anywhere, hosted
images ship `rustup` and `shellcheck`, and the credential nulling restores the
pre-migration hosted behaviour. But nothing here executed them hosted after #233,
so §9 lists them as unproven rather than this section claiming them.

Minimum viable set to unblock merging — four jobs:

```diff
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ jobs.gates
-    runs-on: corelink
+    runs-on: ubuntu-latest
@@ jobs.dco
-    runs-on: corelink
+    runs-on: ubuntu-latest
@@ jobs.gate-check-selftest
-    runs-on: corelink
+    runs-on: ubuntu-latest

--- a/.github/workflows/coverage.yml
+++ b/.github/workflows/coverage.yml
@@ jobs.coverage
-    runs-on: corelink
+    runs-on: ubuntu-latest
```

Full set (all 19 fabric jobs), if the outage is long enough to matter for canaries too.

**Read this before running it.** The `grep` matches 13 files, and that includes
`release.yml` — whose four Linux jobs (`preflight`, `conformance`, `sbom`, `publish`)
run on `corelink` and DO need the swap during a fabric outage. This does not conflict
with §6: §6 says do not try to relocate the release **build+sign** matrix, and that
matrix is safe here because `[self-hosted, macOS, X64]` does not match the regex. It
is left exactly as it is. `windows.yml` and `e2e-evidence.yml` are rewritten too, and
that is also intentional — those two plus `release.yml` are the three files #233
deliberately left with hosted targets pending the cross-platform release rework, so
they are the ones an operator is most likely to be surprised to see in the diff.

```sh
# From the repo root, on a branch. Review the list before you rewrite anything.
grep -rl 'runs-on: corelink' .github/workflows/    # 13 files, 19 job lines

# GNU sed (Linux, and the CI runners):
grep -rl 'runs-on: corelink' .github/workflows/ \
  | xargs sed -i    's/^\( *\)runs-on: corelink$/\1runs-on: ubuntu-latest/'

# BSD sed (macOS) — the empty '' is the mandatory backup suffix:
grep -rl 'runs-on: corelink' .github/workflows/ \
  | xargs sed -i '' 's/^\( *\)runs-on: corelink$/\1runs-on: ubuntu-latest/'

git diff --stat                                    # expect 19 changed lines
```

**Pipe into `xargs`; do not collect the list in a variable.** A draft of this runbook
used `files=$(grep -rl …)` and then `sed … $files`, which depends on the shell splitting
that variable on whitespace. **zsh does not word-split, and zsh is the macOS default
shell** — so the form labelled "macOS" silently did nothing there: `sed` took the whole
newline-joined list as a single filename, errored, and all 19 lines stayed. The attempted
fix (`grep -rlZ … | xargs -0`) was worse, because it looks portable and is not: **BSD
grep on macOS emits newlines for `-Z`, not NULs**, so `xargs -0` received one giant
argument and the command did nothing in *both* shells. Each form above was exercised in
zsh and bash on macOS: the form matching the platform's `sed` swapped 19 lines and left
`[self-hosted, macOS, X64]` untouched; the other errored and changed nothing.

Running the wrong `sed` form errors and changes nothing, so a mistake costs a re-read
rather than a broken tree — but check `git diff --stat` says 19 either way, because
"it printed an error" and "it did nothing" look the same at 3am.

Commit it as a normal PR (`git commit -s`, Conventional Commits — see CLAUDE.md).
Suggested title: `ci: temporary hosted fallback — corelink fabric outage`.

### 4.2 Do NOT do it by reverting the migration commit

`git revert 77bbda9` (PR #233) looks like the obvious move. **It is wrong, twice over:**

* **#233 was not workflow-only.** It also shipped the clw-cli *test-hermeticity* fixes
  (`crates/clw-cli/src/config.rs`, `tests/e2e.rs`, `tests/e2e_portable.rs`) that the
  migration surfaced — the config + e2e tests were inheriting the fabric's ambient env-0
  broker credentials. Reverting the commit reverts those source fixes too.
* **`ci.yml` has moved on since.** The `gate-check-selftest` job was added later, in
  PR #240 (`5dfb51d`). Restoring the pre-#233 `ci.yml` wholesale silently deletes it.

Edit the `runs-on:` lines in place. Do not restore old file versions.

### 4.3 The bootstrap problem

The PR that performs the swap is expected to pass `gates` + `dco` + `llvm-cov` — which
are the very jobs that cannot start. (Expected, not enforced: see §10.4. Those checks
are not *required* by any platform rule, because this repository cannot have one. That
makes the bootstrap less dangerous and the discipline more important — nothing will
stop a merge over red.) The swap takes effect for the PR's *own* checks
(workflows run from the PR's head ref for `pull_request` events), so the four jobs above
will run on `ubuntu-latest` on the fallback PR itself. Verify that before merging:
`gh pr checks <n>` should show them on hosted runners, not queued forever.

Also note that a `.github/` touch escalates `scripts/gates.sh` to the **FULL** lane
(gates.sh line 68) — the fallback PR will pay full-workspace compile time, not the
docs-only fast path.

---

## 5. What breaks when you move to hosted

Verified against the tree and against the last hosted runs. In descending order of how
much it will surprise you.

1. **`gates.sh`'s persistent-cache assumption is already dead — this is NOT a new
   breakage.** `scripts/gates.sh` lines 23–25 read:
   ```sh
   if [ "${CI:-}" = "true" ]; then
     export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/ci-cache/clw-target}"
     mkdir -p "$CARGO_TARGET_DIR"
   ```
   The script **creates** the directory (`mkdir -p`); it does not require it to
   pre-exist, so nothing fails. But the *intent* — a warm target dir surviving between
   runs — is not satisfied on `corelink` either: each job gets its own ephemeral
   microVM (distinct runner names per job in the same run, e.g. `cf-runner-acac4906` /
   `cf-runner-ccbe1553` / `cf-runner-99fe751c` in run `32791835263`), and **there is no
   `actions/cache` step anywhere in `.github/workflows/`**. Every CI run today already
   compiles cold. Moving to hosted therefore costs nothing extra here — the measured
   hosted-vs-fabric delta in §7 is ~1.5 min, which is CPU, not cache.
   *(Historical note: this assumption dates from the 2026-06 self-hosted-Mac topology and
   was flagged for removal in `docs/go-live/WP-BRIEFS.md`; it was never removed.)*

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

5. **Nothing is secret-blocked.** Every credential the fabric lanes use
   (`CLW_CI_TOKEN`, `CLW_CI_TENANT`, `CLW_CI_ENDPOINT`, the minisign key) is a **GitHub
   repository secret**, not runner-local state. Secrets are injected by GitHub into
   whatever runner the job lands on. The live-conformance and canary lanes work on hosted
   for that reason — they did until 2026-08-12.

6. **Required-check names are unaffected.** Where branch protection exists it matches
   **job names**, not runner labels. `gates`, `dco`, `llvm-cov (report-only)` keep their
   names across the swap, so nothing needs re-registering, and
   `scripts/pre-merge-gate-check.sh` (whose `REQUIRED_PRESENT = ["dco", "gates",
   "llvm-cov"]`) is label-agnostic either way.

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

`release.yml`'s `build+sign` matrix (line 334) builds all five targets on ONE physical
Intel Mac. There is **no label-swap fallback**, for reasons that are structural, not
budgetary:

* **`scripts/ci-use-host-toolchain.sh` hard-fails on any hosted macOS runner.** It
  refuses to provision anything and looks for
  `$HOME/.rustup/toolchains/<channel>-x86_64-apple-darwin` (`HOST_TRIPLE` defaults to
  `x86_64-apple-darwin`), exiting 1 with *"the workspace-pinned toolchain … is not
  installed on this runner host"* when absent. GitHub's current hosted macOS images are
  **arm64**, so the path does not exist and the triple is wrong. The script exists
  specifically to forbid the hosted-style `rustup install` on a shared-`$HOME` fleet.
* **zig comes from `brew install zig` on the host, and the step is fail-closed.**
  `release.yml` line 386 checks `command -v zig` and errors out with
  *"do NOT fall back to a hosted setup action — zero-hosted mandate"*. Four of five
  targets (both Linux, windows-gnu) cross-compile through `cargo-zigbuild`.
* **The whole matrix shape is Mac-specific.** Reverting to hosted is not a `runs-on:`
  edit; it is undoing ~15 steps of PR #234 (`6cdf136`): the per-OS `matrix.os` fan-out,
  native `cargo build` per platform, the mingw linker step, and dropping the zigbuild
  path, the zigcc `chmod +x` exit-126 guard, and the `windows-rs` cache re-fetch.

**What the spec gets wrong, for the record:** it is *not* true that hosted macOS lacks
"the provisioned secrets." The signing material is GitHub secrets and is fully portable
(§5.6) — and macOS Developer-ID signing is *waived-open* today anyway (the step
`exit 0`s with a warning when `MACOS_CERT_P12_BASE64` is absent, which it always is).
What is not portable is the **toolchain and zig, which live on the host filesystem**,
plus the **Intel/arm64 host-triple mismatch**. The conclusion — no trivial fallback — is
correct; the reason is different.

**It is not imaginary, though.** A fully hosted release topology existed in this repo and
**shipped v0.1.11 green on 2026-07-17** (run `29544932935`: `preflight`/`conformance`/
`sbom`/`publish` on `ubuntu-latest`, both darwin targets on `macos-14`, windows-gnu on
`windows-latest`, aarch64-linux on `ubuntu-24.04-arm`). So the recovery path for a
prolonged Mac outage is *restore the PR #234 predecessor shape*, not *invent one* — a
half-day of work, ~77 hosted minutes per release (§7), and it has not been re-executed
against today's `release.yml`. Treat it as a planned project, not an incident action.

**Incident action for a Mac outage:** do nothing to `release.yml`. Announce that releases
are paused. `vX.Y.Z-rc.N` dry-runs are equally blocked (same matrix). Merging, canaries
and all PR gates are unaffected — they are on `corelink`.

---

## 7. Cost in hosted minutes

Plan: **GitHub Free org, private repo → 2,000 included minutes/month.** Multipliers:
Linux ×1, Windows ×2, macOS ×10. **Every job is billed rounded up to the next whole
minute**, so short jobs cost 1 minute each.

### Measured wall-clock (real runs, not estimates)

| Job | On `corelink` | On `ubuntu-latest` | Hosted delta |
|---|---|---|---|
| `gates` (FULL lane, sample 1) | 6m19s — main, `32791835263` | 7m32s — main, `30780253565` | +1m13s |
| `gates` (FULL lane, sample 2) | 6m13s — PR, `32776068642` | 8m16s — main, `29714656627` | +2m03s |
| `dco` | 9s | 5s | — |
| `llvm-cov (report-only)` | 4m43s (`32791835190`) | 6m23s (`30780253564`) | +1m40s |
| `gate-check-selftest` | 9s | *unmeasured* (job postdates the migration) | — |

### Billed cost of Scenario A (Linux fallback)

| Event | Jobs | Billed minutes (rounded up, ×1) |
|---|---|---|
| One push to a PR | gates 8–9 (worst measured) + dco 1 + selftest 1 + coverage 7 | **~18** |
| Post-merge push to `main` | same four | **~18** |
| One merged PR (1 push + merge) | | **~36** |

**≈ 2,000 ÷ 18 ≈ 110 pushes/month** before the included allowance is gone — i.e. roughly
**55 merged PRs/month** at one push each, fewer in practice. `ci.yml` and `coverage.yml`
both set `cancel-in-progress: true`, so superseded pushes stop billing immediately; that
concurrency guard is load-bearing under a fallback and must not be removed.

Adding the scheduled lanes (canaries daily, fuzz nightly, mutation/stress/journeys
weekly) on top of that will consume the allowance considerably faster — during a
short outage, flip only the four merge-blocking jobs (§4.1) and leave the cron lanes red.

### Billed cost of a hosted release (measured from run `29544932935`, v0.1.11)

| Job | Runner | Wall | Mult. | Billed |
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
| | | | | **≈ 77** |

\* GitHub does not publish an included-minutes multiplier for hosted arm64 Linux; its
per-minute price is below x64, so ×1 is a conservative upper bound.

**The two macOS legs are 40 of those 77 minutes** for 3 minutes of actual work — that
10× multiplier is the single strongest cost argument for the Mac.

---

## 8. Switching back

1. Confirm the fabric is healthy: dispatch a cheap `corelink` job
   (`gh workflow run shellcheck.yml`) and confirm it gets a runner and goes green.
2. Open a PR reverting **only** the `runs-on:` lines (`ubuntu-latest` → `corelink`).
   If the fallback landed as a single self-contained commit touching nothing but
   `runs-on:` lines, `git revert <sha>` is safe here — verify with `git show --stat`
   that the commit really is workflow-only before trusting that.
3. The revert PR's own checks run on `corelink` (head-ref workflows again) — that is
   the verification. Do not merge until they are green *on the fabric*.
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
Re-running that test is the outstanding item; it costs ~18 hosted minutes.

### What IS proven — by production execution, not by reasoning

| Claim | Evidence |
|---|---|
| `gates` passes on `ubuntu-latest` with the exact steps `ci.yml` has today | Run `30780253565` (main, 2026-08-03, 7m32s, success) and `29714656627` (main, 2026-07-20, 8m16s, success). PR #233's diff shows the gates job's *steps* were unchanged by the migration — only the label and the comments changed. |
| `dco` passes on `ubuntu-latest` | Same runs (5s, success). |
| `llvm-cov (report-only)` passes on `ubuntu-latest` | Run `30780253564` (2026-08-03, 6m23s, success). |
| The label swap is sufficient — no step edits needed | `git show 77bbda9 -- .github/workflows/ci.yml`: the `gates` job diff is the `runs-on:` line plus comments. |
| A fully hosted release works | Run `29544932935` — v0.1.11 shipped from hosted runners, 2026-07-17. |
| Every CI run already compiles cold | No `actions/cache` in `.github/workflows/`; distinct ephemeral runner names per job. |

### What is NOT proven

* **`gate-check-selftest` on hosted.** The job was added in PR #240, after the migration;
  it has never run on `ubuntu-latest`. It is pure bash + python3 with no fabric
  dependency, so it is expected to pass — expected, not proven.
* **`windows.yml` (Wine) on `ubuntu-latest` in its current shape.** Wine + mingw install
  from apt on both images, but this lane was authored for `corelink`.
* **Hosted Actions being currently *billable*.** Nobody has confirmed the org's hosted
  minute balance since 2026-08-03 (the billing API endpoints used here return 403/410
  without `admin:org`). See §3.2 — this is the single most likely way the fallback fails
  in an incident.
* **The Scenario-B hosted release path against today's `release.yml`.** §6.
* **The fifteen non-merge-blocking fabric lanes, hosted, after #233.** §4.1 lists the
  five that received fabric-specific step edits in that PR. Reading each, they are
  portable or merely redundant on `ubuntu-latest` — but reading is not running, and the
  proof table above covers only `gates`, `dco` and `llvm-cov`. Treat §4.1's full-set
  `sed` as the right action with an unproven tail, not as a validated procedure.

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
5. **`gate-check-selftest` is not in `pre-merge-gate-check.sh`'s `REQUIRED_PRESENT`.**
   `ci.yml` has a bare `on: pull_request` with no `paths:` filter, so all three of its
   jobs run on every PR, but the pre-merge guard only asserts the presence of `dco`,
   `gates`, and `llvm-cov`. A silently-absent `gate-check-selftest` would not be caught.
6. **The `CARGO_TARGET_DIR` persistent-cache assumption is vestigial** (§5.1) — flagged
   for removal in `docs/go-live/WP-BRIEFS.md` back in the M2.x wave, never removed, and
   now satisfied by nothing on any runner.

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
