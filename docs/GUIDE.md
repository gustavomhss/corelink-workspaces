# CoreLink Workspaces (`clw`) — The Complete Guide

> Reference for the client commands, configuration and selected runtime guarantees.
> Examples describe the pinned source below, not a complete JSON schema or live-service certification.
> Source baseline: `be96b8394c66d1dc36ce7ca2f186780f0ea48308` (workspace 0.1.12), reconciled 2026-09-18.
> This is source documentation, not latest-release or production-availability evidence.

---


> **Integrated changes at this source:** [PR #329](https://github.com/HuGR-Labs/corelink-workspaces/pull/329)
> supplies Unix hydrate temporary ownership and pre-publication permissions;
> [PR #334](https://github.com/HuGR-Labs/corelink-workspaces/pull/334) supplies
> checked per-file size validation. Both are already in the pinned main commit.
> Earlier test records retain their original source identities; this documentation
> update is not a new runtime-test or release result.

## Table of contents
1. [What CoreLink Workspaces is](#1-what-corelink-workspaces-is)
2. [The problems it solves](#2-the-problems-it-solves)
3. [Who uses it (personas)](#3-who-uses-it)
4. [How it works (architecture)](#4-how-it-works)
5. [Install & getting started](#5-install--getting-started)
6. [The three verbs: snapshot / hydrate / run](#6-the-three-verbs)
7. [Command reference (the manual)](#7-command-reference)
8. [Configuration & environment variables](#8-configuration--environment)
9. [Exit codes & the cross-team contract](#9-exit-codes)
10. [Auth & credentials](#10-auth--credentials)
11. [Features in depth](#11-features-in-depth)
12. [Use cases & user stories](#12-use-cases--stories)
13. [When to use it — and when not](#13-when-to-use)
14. [Security & safety model](#14-security--safety)
15. [Distribution, signing & licensing](#15-distribution--licensing)
16. [Troubleshooting & FAQ](#16-troubleshooting--faq)
17. [Recipes (cookbook)](#17-recipes)
18. [Command outputs & `--json` schemas](#18-outputs-schemas)
19. [The manifest format (wire spec)](#19-manifest-spec)
20. [Platform support](#20-platform-support)
21. [Compliance, privacy & legal](#21-compliance-legal)
22. [Roadmap & status](#22-roadmap-status)

---

<a id="1-what-corelink-workspaces-is"></a>
## 1. What CoreLink Workspaces is

`clw` is a Rust client for saving selected directory contents to a content-addressed
store, reconstructing them into a new/empty directory and memoizing local command
stdout/stderr/exit. It does not execute remotely, snapshot a running process,
freeze a mutable tree or restore output artifacts on a run-cache HIT.

Source, dependencies and tools are captured only when their files are included by
the walk rules in §6. CAS contains BLAKE3-addressed blobs; AC contains named refs
and run records. The root is one flat JSON manifest, not recursive directory
manifests. CAS blobs and the root manifest have a 5 MiB body cap; listing pages have a
separate 16 MiB cap. The whole workspace may span many blobs.

The client sends a bearer credential to the configured tenant URL. Config can be
entirely environment-driven. Server authorization, scheduler isolation, retention,
backups and service availability are separate from this client's guarantees.

---

<a id="2-the-problems-it-solves"></a>
## 2. The problems it solves

| Need | Mechanism | Boundary |
|---|---|---|
| Save/move selected files | `snapshot` and cache-first `hydrate` | Ignores apply; no machine image or capture transaction. |
| Avoid uploading duplicate chunks | Content-defined chunks and remote presence checks | Dedup does not promise a single global upload under concurrency. |
| Reuse command results | `run` keys declared inputs/env, command/tool and cwd | HIT replays streams, not filesystem effects; not hermetic. |
| Compare selected content | `status` returns clean/dirty/error | Uses the same walk exclusions; not a proof about excluded files. |
| Inspect/manage references | `ls`, `rm`, dry-run-first `prune` | `parent` is lineage, not a retention pin; shared GC obligations remain open. |
| Clear enumerated storage | Consent-gated `erase` with re-enumeration | Not by itself permanent offboarding or proof about every server catalog/backup. |
| Supply runner identity | Command-gated bootstrap ticket redemption | Ref domains avoid name collisions, not authorization checks. |

Performance depends on bytes, locality, network and filesystem; no fixed
seconds-to-hydrate or zero-risk deletion guarantee is asserted here.

---

<a id="3-who-uses-it"></a>
## 3. Who uses it (personas)

A developer can save a selected working tree and restore it elsewhere. A runner
can drive the existing CLI with explicit identity and use a disposable local
workspace. A platform can pin a trusted root digest and reconstruct those bytes.
All three must account for exclusions, artifact retention and platform semantics.

Consumers should pin the intended executable and use the documented command
surface. `run` is a local subprocess wrapper: exit integers alone do not prove
whether a child ran or authorize retries. In particular, a child can itself return
125 or 137. See §9 and [the frozen contract](CLI-SURFACE-EXIT-FREEZE.md).

---

<a id="4-how-it-works"></a>
## 4. How it works (architecture)

### Two stores and one flat root

CAS uses `/v1/cas/{tenant}/{digest}` for immutable blob contents; AC uses
`/v1/ac/{tenant}/{key}` for records. Both expose listing and deletion through the
client's transport. The named-ref contract is create-only/idempotent, not an
unrestricted overwrite. Forced replacement in this baseline uses DELETE then
PUT, not compare-and-swap ([#292](https://github.com/HuGR-Labs/corelink-workspaces/issues/292)).

The manifest has `version`, `total_size` and ordered `entries`. Files reference
ordered chunks, links carry targets and `Dir` represents empty directories.
Producer canonical bytes and reader acceptance are distinct; see §19.

### Chunking and sharing

FastCDC uses 256 KiB / 1 MiB / 4 MiB min/average/max settings. Snapshot has
in-memory and streaming paths with an equivalence test. A post-read length check
in the small-file path is not an allocation bound
([#291](https://github.com/HuGR-Labs/corelink-workspaces/issues/291)).

Remote sharing is within the configured, authorized tenant. Actual authorization
is the server's responsibility. User/runner ref hash domains avoid name collisions
but do not prohibit a credential from addressing other accessible keys. The local
cache is a separate per-host path; its permissions are platform-specific, its
maintenance best-effort, and its budget is not a hard disk quota.

### Snapshot

The selected tree is walked, chunks processed, and the root serialized. For a new
publication, content is handled before its ref. This is not a transaction with GC.
An occupied, non-forced divergent snapshot is refused locally without uploading
its candidate content; the diagnostic currently overattributes this to a server
409 ([#309](https://github.com/HuGR-Labs/corelink-workspaces/issues/309)).
Same-root success can occur before repairing a missing root blob, including under
force ([#294](https://github.com/HuGR-Labs/corelink-workspaces/issues/294)).
The root-size check follows chunk work; it does not avoid every earlier upload.

### Hydrate

Name resolution or a supplied digest selects root bytes. The client verifies the
root hash, parses it and enforces resource policy before materialization. Files
are reconstructed cache-first, then links are materialized. On Unix links remain
links; Windows uses copies with separate ordering and expansion limitations.

The writer publishes a temporary only after explicit `Finish`; EOF aborts that
writer. The integrated #297 change creates Unix temporaries owner-only, arms
cleanup after exclusive creation and applies the intended final mode before
sync/rename; public final modes are intentionally possible. Publication is per file, not a tree transaction. Completed files may
remain after another file fails. A returned outer error does not yet prove all
sibling writers are quiescent ([#300](https://github.com/HuGR-Labs/corelink-workspaces/issues/300)).
Use a new destination for recovery; do not race cleanup against unfinished work.

### Run

`clw/run/v3/` keys selected inputs/modes, declared environment presence/values,
command/tool identity and absolute cwd. An initial AC transport failure propagates.
A usable memo replays captured stdout/stderr/exit; an absent or unusable memo
causes one local child execution. This baseline's producer stores only exit-zero
results, but received nonzero records are not independently rejected
([#307](https://github.com/HuGR-Labs/corelink-workspaces/issues/307)).

HIT has no output-file inventory and does not recreate build artifacts. Known
credential variables are removed from the child environment, but HOME and
same-user filesystem access remain. The caller supplies isolation and all
output-relevant inputs; `clw` does not make an arbitrary command hermetic.

See [ARCHITECTURE.md](ARCHITECTURE.md) for dependencies, source anchors and the
complete boundary table, and [THREAT-MODEL.md](THREAT-MODEL.md) for open findings.

---

<a id="5-install--getting-started"></a>
## 5. Install & getting started

Select a specific approved version and its actual distribution evidence. The
configured Homebrew entry is `brew install hugr-labs/clw/clw`; the fallback
installer is documented in [INSTALL.md](INSTALL.md). This reconciliation does not
verify the currently served assets or designate a latest release.

The installer requires `CLW_VERSION`, uses an embedded minisign trust root and
always verifies SHA-256. When minisign is installed, signature failure is fatal;
`CLW_SKIP_SIGNATURE=1` only allows the missing-minisign case to proceed on checksum
alone. It is not a general signature-failure bypass. The default install directory
is `~/.local/bin`; keep it on PATH. No in-binary self-update exists.

Windows downloads require verification of the selected `.exe` and its evidence.
macOS quarantine handling is part of the installer; do not remove quarantine from
an unverified download. See [DISTRIBUTION.md](DISTRIBUTION.md) for the selected
platform signing policy, rather than assuming every current asset is unsigned.

### Quickstart

Use an explicitly authorized endpoint and tenant. Supply credentials through your
approved environment/config path, never in pasted examples or captured help.
Before any network command, inspect the selected tree and its ignore rules.

```sh
clw --json doctor                 # offline configuration report, not authentication
clw init                          # optional local persistence; --force replaces config
clw snapshot ./project --name project-checkpoint
clw hydrate ./restored --name project-checkpoint  # new/empty destination
clw status ./project --name project-checkpoint
```

Configuration precedence is flag → `CLW_*` → `CORELINK_*` → config file → default.
Set `CLW_ENDPOINT` explicitly for tests; absence may select the built-in service.
For memoization, declare all relevant files/environment and use a deterministic
command whose required outcome is its output, not regenerated filesystem artifacts.
A HIT does not create output files. `doctor` returning 0 does not authenticate a token.

`clw uninstall --dry-run` previews local footprint cleanup. A confirmed uninstall
still leaves the executable and does not revoke a server credential. `auth rotate`
only prints guidance; obtain/revoke credentials through the approved account-side
process. See §7 for the precise effects.

---

<a id="6-the-three-verbs"></a>
## 6. The three verbs: snapshot / hydrate / run

CoreLink Workspaces has one job — move a *workspace* (a directory tree) in and out of a
content-addressed backend, and memoize work done against it. Everything is built on three verbs:

- **`snapshot`** — capture a directory, content-address + upload it, and record it under a name.
- **`hydrate`** — materialize a snapshot back onto disk (on the same or a different machine).
- **`run`** — execute a command and memoize its output, keyed by its declared inputs; replay the
  cached stdout/stderr/exit on a hit instead of re-running.

The rest are lifecycle + identity helpers: `status`, `ls`, `rm`, `prune`, `erase`, `init`,
`auth`, `doctor`, `uninstall`, `completions`.

### What a snapshot captures (the walk rules)

`snapshot` (and `run`'s `--input <dir>` walk) don't capture *everything* under the path — they apply a
deliberate, git-aware filter. Know these so a "byte-identical tree" holds no surprises:

- **`.git/` is always excluded** — unconditionally, on every snapshot, regardless of any ignore file. A
  snapshot captures your *working tree*, not the repo database. (So a hydrated tree is byte-identical for
  everything that was captured — minus `.git` and anything ignored below.)
- **The full ignore stack is honored**, not just `.gitignore`: also `.clwignore`, `.ignore`,
  `.git/info/exclude`, your global `core.excludesFile`, and parent-directory `.gitignore` files — and
  gitignore rules apply **even outside a git repo**.
- **Hidden / dotfiles ARE included** by default (e.g. `.env`, `.config/…`) unless an ignore rule drops them.
  This is why `snapshot` **secret-scans** and warns about `.env`/keys — they're captured, not skipped (clw
  does not exclude a file merely because its name looks secret; add it to
  `.clwignore` before snapshotting if it must not upload).
- **Symlinks that point outside the workspace are skipped** with a warning (they can't round-trip safely);
  in-tree symlinks (including intra-workspace `..`) are preserved. **Empty directories are preserved.**

---

<a id="7-command-reference"></a>
## 7. Command reference (the manual)

`clw` — *CoreLink Workspaces client — snapshot, hydrate, and memoize your workspace.*
Run `clw --help` or `clw <cmd> --help` for the live text. `--version` prints the version.

### Global flags (apply to every subcommand)

| Flag | Short | Default | Env | What it does |
|---|---|---|---|---|
| `--endpoint <URL>` | | `https://corelink-api.humangr.com` | `CLW_ENDPOINT` (alias `CORELINK_ENDPOINT`) | Remote API base URL (trailing `/` trimmed). |
| `--tenant <ID>` | | *(required for network cmds)* | `CLW_TENANT` (alias `CORELINK_TENANT`) | Account/namespace — goes in the URL path; namespaces all keys. |
| `--token <PAT>` | | *(required for network cmds)* | `CLW_TOKEN` (alias `CORELINK_TOKEN`) | Personal access token. Identity reports mask it; generated help has #333. Avoid sharing credential-bearing help or process arguments. |
| `--cache-dir <DIR>` | | `~/.clw/cache` | `CLW_CACHE_DIR` | Local blob cache. |
| `--ref-domain <D>` | | `user` | `CLW_REF_DOMAIN` | `user` or `runner`; `runner` uses a disjoint keyspace (set by the runner fabric — users never need it). |
| `--json` | | off | | Supported result variants go to stdout; diagnostics go to stderr. Some operational refusals also emit JSON on stdout. Not a uniform error envelope. |
| `--concurrency <N>` | | `8` | | Per-invocation ceiling for concurrent CoreLink requests in `snapshot`/`hydrate`/`prune`/`erase` (`0` is clamped to 1; includes `ls --all`; does **not** apply to `run`). CLI flag only: no environment or config-file setting. |
| `--no-color` | | off | `NO_COLOR`, non-TTY | Disable ANSI color. |
| `--verbose` | `-v` | warn | `RUST_LOG` overrides | `-v`=info, `-vv`=debug. Diagnostics to stderr. Conflicts with `-q`. |
| `--quiet` | `-q` | off | `RUST_LOG` overrides | Only errors. Conflicts with `-v`. |

---

### `clw init [--force]`
Write `~/.clw/config.toml` from the resolved config values (`endpoint`, `tenant`, `token`, `cache_dir`).

- On Unix, the managed `~/.clw` directory uses **0700** and config file **0600**.
  Windows uses the separate warning-on-ACL-failure policy (#317), not Unix modes.
  Unrelated parsed keys are preserved. An existing file requires **`--force`**.
  The token is never echoed (only the path, or `{wrote, token:<set>/<unset>}` with `--json`).

| Flag | Default | Meaning |
|---|---|---|
| `--force` | off | Overwrite an existing config file (default: error if it exists). |

Examples with identity already configured through an approved environment: `clw --tenant acme init` · `clw init --force`

---

### `clw snapshot [PATH] --name <NAME> [--force]`
Content-address a directory and upload it, recorded under `NAME`.

- `PATH` is **positional** (default `.`), *not* `--path`.
- Same-root content returns unchanged, but is not an integrity scrub and may not repair a missing root (#294).
- A differing occupied name without force is refused locally before candidate upload. The wording overattributes it to server 409 (#309). Force uses separate deletion/publication, not an atomic replacement (#292).
- After the storage result, **stderr advisories** report ignore-file presence,
  uploaded secret-looking names and skipped external links. They do not reject
  a publication because of those findings. Output writes themselves can fail;
  completion of remote effects is not transactional with displaying the report.
  Inspect the ref after a reporting failure instead of assuming nothing happened.
  Warnings are advisory name checks, not secret-content scanning or prevention.

| Arg/flag | Default | Required | Meaning |
|---|---|---|---|
| `[PATH]` | `.` | no | Directory to snapshot. |
| `--name <NAME>` | — | **yes** | Workspace name (the ref key). |
| `--force` | off | no | Replace an existing name whose content differs. |

Examples: `clw snapshot --name my-ws` · `clw snapshot ./proj --name my-ws --force` · `clw --json --concurrency 4 snapshot src/ --name build-inputs`

---

### `clw hydrate <DEST> (--name <NAME> | --manifest-digest <HEX>)`
Materialize a snapshot into `DEST` (which must not exist or be empty).

- `DEST` is **positional**.
- Identify the snapshot **either** by `--name` (AC named-ref lookup) **or** by `--manifest-digest <HEX>`
  (content-addressed — skips the name lookup; the fetched manifest is **self-verifying**: its bytes
  must hash to the digest). Exactly one is required; giving both or neither is rejected.
- Honors `--concurrency`.

| Arg/flag | Required | Meaning |
|---|---|---|
| `<DEST>` | **yes** | Destination dir (must not exist or be empty). |
| `--name <NAME>` | one-of | Snapshot name. Conflicts with `--manifest-digest`. |
| `--manifest-digest <HEX>` | one-of | 64-hex manifest digest; content-addressed, self-verifying. |

Examples: `clw hydrate ./out --name my-ws` · `clw hydrate /tmp/ws --manifest-digest 95a05db3…`

---

### `clw status [PATH] --name <NAME>`
Diff the remote snapshot against the local working tree. **Exit 0 = clean, 1 = dirty, 2 = error.**

- Human output lists `added:` / `removed:` / `changed:` (`+`/`-`/`~`); `--json` → `{clean, added, removed, changed}`.

| Arg/flag | Default | Required | Meaning |
|---|---|---|---|
| `[PATH]` | `.` | no | Local dir to compare. |
| `--name <NAME>` | — | **yes** | Workspace name to compare against. |

Examples: `clw status --name my-ws` · `clw --json status ./proj --name my-ws`

---

### `clw run --input <PATH>... [--env <KEY>...] -- <CMD>...`
Run a command and **memoize** its output; replay stdout/stderr/exit on a cache hit.

- **Cache key** = selected input digests/modes, declared env presence/values, command/tool identity and absolute cwd, under `clw/run/v3/`. Tool-resolution and OS-environment gaps are #301/#302.
- **Hermeticity caveat**: undeclared inputs (network, clock, RNG, files not passed via `--input`) are
  invisible to the key — only memoize **deterministic** commands.
- Input **directories** are walked respecting `.gitignore`/`.clwignore`; an ignored file inside a dir
  is *not* in the key — pass it as an explicit `--input <FILE>` (named files are digested regardless).
- Prints `[clw] cache hit` / `[clw] cache miss` to stderr, then forwards captured
  stdout/stderr and the child result on successful delivery. This is buffered
  capture, not live streaming. Delivery/diagnostic failures are tracked in #310.
  `--json` and `--concurrency` do **not** select another run-output/concurrency mode.
- The producer stores only exit-zero results. Received nonzero-record replay is a separate open #307 issue. HIT does not restore output files.

| Arg/flag | Required | Meaning |
|---|---|---|
| `--input <PATH>` (repeatable) | **yes** | Files/dirs forming the cache key. |
| `--env <KEY>` (repeatable) | no | Env var **names** whose values are folded into the key. |
| `<CMD>...` (after `--`) | **yes** | Command + args to run. |

Examples: `clw run --input src/ -- cargo build` · `clw run --input a --input b --env CC --env GOPATH -- make`

---

### `clw ls (--name <NAME> | --all)`
List ref info for one workspace name, or every ref with `--all`.

- `--name`: prints that ref's `name`, `root`, `created_at`, `parent`, `files`, `bytes_total`.
- `--all`: enumerates tenant AC keys and classifies fetched records. Recognized
  workspaces populate `refs`/`count`; run memos populate `run_cache_entries`, and
  recognized foreign/non-JSON values populate `foreign_ac_entries`. Corrupt
  ref-shaped records and per-record failures are reported in `skipped`/`errors`;
  an auth error is fatal. A zero workspace count is not an empty-store proof.

Examples: `clw ls --name my-ws` · `clw ls --all` · `clw --json ls --all`

---

### `clw rm (--name <NAME> | --ref-key <KEY>) [--yes]`
Delete a workspace ref — by name, or by raw 64-hex `--ref-key` (to remove a dangling/corrupt ref
that `ls --all`/`prune` surfaced but that has no parseable name).

- Consent required: interactive y/N prompt unless `--yes`; non-interactive/`--json` without `--yes`
  refuses. A **declined confirmation exits non-zero.** Deleting an absent ref is idempotent success.

| Flag | Meaning |
|---|---|
| `--name <NAME>` | Ref by name. Conflicts with `--ref-key`. |
| `--ref-key <KEY>` | Raw 64-hex ref key (dangling/corrupt refs). |
| `--yes` | Skip the prompt (required for `--json`/non-interactive). |

Examples: `clw rm --name my-ws --yes` · `clw rm --ref-key 00112233…eeff --yes`

---

### `clw prune [--yes]`
Garbage-collect unreferenced CAS blobs (mark-and-sweep). **Dry-run by default** — reports what it
*would* delete and changes nothing without `--yes`.

- Marks current recognized workspace roots/chunks and RunRecord streams; it
  does not traverse `parent`. Other root families and concurrent publication
  remain #293/#295. Use only the root/writer envelope accepted in the runbook.
- **Fail-safe**: an unreadable listed record or an unresolved recognized workspace
  root aborts marking before any DELETE. Recognized RunRecords are handled
  separately rather than parsed as workspace manifests. This does not prove
  complete coverage of every server-side catalog.
- **Safety floor**: an empty live set over a non-empty CAS (would wipe the tenant) is refused
  unconditionally — even in dry-run — and points at `clw erase`.

Examples: `clw prune` (dry-run) · `clw prune --yes`

---

### `clw erase [--yes] [--confirm-tenant <TENANT>]`
Delete and re-enumerate the selected tenant AC/CAS scope. This is not by itself permanent offboarding, writer exclusion or verification of every catalog/backup (#316). Three consent rails:
(1) **dry-run by default**; (2) **`--yes`** for intent; (3) **typed-tenant confirmation** — you must
reproduce the exact tenant id (interactively, or `--confirm-tenant <TENANT>` for non-interactive/`--json`;
a mismatch or a non-TTY without the flag aborts non-zero).

- Executes with **verification**: delete refs then blobs, re-enumerate, repeat up to 3 passes, and only
  reports success once the selected AC/CAS enumerations are empty. Otherwise it
  returns a partial/incomplete error. A retry is a new pass over current state,
  not a transaction rollback; confirm writers and scope before retrying.

Examples: `clw erase` (dry-run) · `clw erase --yes` (prompts for tenant) · `clw erase --yes --confirm-tenant acme`

---

### `clw doctor`
Print an **offline** configuration/filesystem report. Its ordinary report path returns 0 even for missing configuration; this does not authenticate, contact the service or prove connectivity.

- Reports: clw version, os/arch, config path + mode, endpoint (with `user:pass@` userinfo stripped),
  tenant (shown in full — it's an account id), token as `<set>`/`<unset>` (**never the value**),
  cache dir, and a **problems** advisory list. `--json` for a structured bundle.

Examples: `clw doctor` · `clw --json doctor`

---

### `clw auth <login | logout | whoami | rotate>`
Manage locally stored identity. These command reports mask token values as presence (`<set>`/`<unset>`); generated help has the separate #333 limitation;
the managed config persists the token in plaintext with platform-specific protections.
These commands do not revoke server credentials or isolate same-user file access.

- **`login`** — persist `endpoint`/`tenant`/`token`/`cache_dir` to `~/.clw/config.toml` (Unix 0600, parent 0700; Windows ACL policy #317),
  preserving unrelated keys.
- **`logout [--all] [--yes]`** — remove the stored token (or `--all` clears the whole file). `--all` is
  destructive: needs `--yes` non-interactively; a declined prompt exits non-zero.
- **`whoami`** — print the resolved identity, redacted, offline (to stdout).
- **`rotate`** — guidance only: JSON reports `rotated:false`. It makes no mint/revoke or replacement call. `auth login` persists an already obtained token; server issuance/revocation remains separate.

Examples: `clw auth login` · `clw auth whoami` · `clw auth logout --all --yes`

---

### `clw uninstall [--yes] [--dry-run]`
Remove clw's on-disk footprint: `~/.clw` (config + PAT) and the cache dir.

- **Safety guard**: refuses to remove a path that is the filesystem root, the home dir, the CWD, or an
  ancestor of the CWD (guards a misconfigured `cache_dir` like `.` or `/`).
- Consent required (`--yes` non-interactively; a declined prompt exits non-zero). Does **not** self-delete
  the running binary — it prints the path + the `rm <path>` command to run.

Examples: `clw uninstall --dry-run` · `clw uninstall --yes`

---

### `clw completions <shell>`
Generate a shell-completion script to **stdout** (an install hint goes to stderr). Pure/offline — works
before any config/login. Shells: `bash`, `zsh`, `fish`, `powershell`, `elvish`. Completion output is raw
shell source, so `clw --json completions <shell>` is incompatible: it exits 2 with no generated script,
before resolving configuration or redeeming a C2c credential ticket.

Examples: `clw completions bash > ~/.local/share/bash-completion/completions/clw` · `clw completions zsh > "${fpath[1]}/_clw"`

---

<a id="8-configuration--environment"></a>
## 8. Configuration & environment variables

**Precedence (highest → lowest):** `--flag` › `CLW_*` env › `CORELINK_*` env › `~/.clw/config.toml` › built-in default.

### Environment variables

| Var | Sets | Default | Notes |
|---|---|---|---|
| `CLW_ENDPOINT` / `CORELINK_ENDPOINT` | API base URL | `https://corelink-api.humangr.com` | Trailing `/` trimmed. |
| `CLW_TENANT` / `CORELINK_TENANT` | Tenant id (URL path + key namespace) | — (required) | Validated (rejects `/`, `..`, control chars). |
| `CLW_TOKEN` / `CORELINK_TOKEN` | Bearer PAT | — (required) | Managed reports mask values; help has #333. Static token is not used on the broker-redemption path. |
| `CLW_CACHE_DIR` | Local blob cache | `~/.clw/cache` | No `CORELINK_` alias. |
| `CLW_CACHE_BUDGET_BYTES` | Local cache eviction budget (bytes) | `8589934592` (8 GiB) | A raw byte count. Unset/`0`/invalid → default. Raise it for a working set > 8 GiB; lower it on a disk-tight machine. |
| `CLW_REF_DOMAIN` | Ref keyspace | `user` | `user` \| `runner`; other values → exit 2. |
| `CLW_OP_DEADLINE_SECS` | Per-transport-operation retry deadline, not whole-command time | `300` (5 min) | Clamped to 24h; `0`/invalid → default. |
| `CLW_CRED_TICKET` + `CLW_LEASE_ID` + `CLW_FABRIC_ENDPOINT` | C2c single-use runner credential trio | — | Only in `runner` ref-domain; a CAS/AC command redeems the ticket once before dispatch. |
| `NO_COLOR` | Disable color | unset | Presence only. |
| `RUST_LOG` | Log level/filter (stderr) | unset | **Overrides** `-v`/`-q`. |
| `HOME` / `USERPROFILE` | Home dir (config path, `~` expansion) | — | |

*(Internal-only: `PATH`/`PATHEXT` for `run` tool resolution; `CLW_FUZZ_GATE`/`CLW_TEST_INTERNAL_PANIC` are test seams, not user knobs.)*

### The config file `~/.clw/config.toml`
- Written by `clw init` / `clw auth login`; Unix file **0600**, parent **0700**.
  Windows ACL application can warn and continue (#317). The file holds plaintext identity.
- A minimal `key = "value"` format (flat keys: `endpoint`, `tenant`, `token`, `cache_dir`, `ref_domain`).
- A missing file is ignored. Unreadable or unusable input warns and can fall back
  to other sources; an overbroad Unix mode warns rather than proving privacy.
  Strict commands still fail if the resulting required identity is absent or
  invalid. Unix writes use temporary/sync/rename. No same-user isolation is implied.

### Resource policies and limits (some configurable; scopes differ)

The table distinguishes per-body, manifest, captured-stream and local-cache policies. It does not establish a global memory, disk or whole-command quota. The cache budget is explicitly configurable; other listed constants are not CLI knobs.

| Bound | Value | Effect when reached |
|---|---|---|
| **Local cache GC budget** | **8 GiB**, overridable with `CLW_CACHE_BUDGET_BYTES` | Best-effort target, not a quota. New-blob PUTs trigger a sweep after 256 writes on that cache instance; a re-PUT of an existing blob returns before the counter. Recency refresh is amortized to 60 seconds. Successful hydrate also requests a final sweep; snapshot has no equivalent unconditional final sweep (#304). Errors, concurrent writers and excluded temporaries can leave usage above budget. Evicted content is recoverable only while another authoritative copy remains available. |
| CAS blob / manifest / `run` output cap | **5 MiB** | A single CAS blob is ≤ 5 MiB (`CAS_BLOB_CAP_BYTES`). A manifest is one blob → an oversized root is rejected after chunk processing, which may already have uploaded bytes. When either captured stream exceeds 5 MiB, `run` requests termination of its direct child and maps the output-limit error to 137. That number alone does not prove a particular OS signal or child history; see §9 and #305. |
| Snapshot stream threshold | **16 MiB** | Files > 16 MiB stream (one chunk in memory at a time); the small-path length check currently happens after reading; it does not prevent that allocation (#291). |
| Hydrate resource caps | ≤ **1M** entries · **100k** symlinks · **1 TiB** total · **64M** total chunks · **16M** chunks/file | Checked after root parsing and before destination materialization against declared chunk sums/counts. Not all-allocation or physical-disk protection; Windows copy expansion remains #298. |
| `ls --all` / `prune` enumeration caps | **16 MiB** per page · **10M** keys total · **1M** pages · page size **1000** | A tenant enumeration that blows past 10M keys aborts with `TooLarge` rather than growing unbounded. |
| HTTP timeouts | request **120s**, connect **10s** (cred-ticket redeem: **30s** / **10s**) | A single stalled request is abandoned at 120s; the per-transport-operation deadline (`CLW_OP_DEADLINE_SECS`, default 300s) bounds the retry budget. This is why the 300s default ≈ a couple of full 120s attempts. |
| Download body cap | **5 MiB**, streamed | CAS GET accumulation rejects bodies above the cap before hashing; this is not a total process-memory bound. |
| Cred-ticket redeem body cap | **64 KiB** | Caps the redemption response body, not all initialization allocations. |
| 429 quota-vs-transient split | `Retry-After` **> 3600s** | After retry exhaustion, the hint affects quota/backpressure wording. It does not independently verify the account plan or prevent initial retries. |
| `run` post-success cache store | **30s** phase deadline | The post-success store phase is best-effort and time-boxed to 30 seconds; it can add up to that phase budget before returning. |
| Retry policy | **6** attempts, ladder `[200ms, 800ms, 2s, 5s, 10s]` | Applies to connect/transport/5xx/429, capped by the remaining op deadline. AIMD concurrency: a 429 halves the in-flight window (floor 1); 8 straight successes grow it back toward `--concurrency` (default 8). |

### Owned local-cache temporary cleanup

This is a scoped #314 / PR #338 update to the historical source baseline in this
guide. Its [implementation](https://github.com/HuGR-Labs/corelink-workspaces/blob/3c71ff98c06918428527d982c78e886771ac5f1b/crates/clw-cache/src/lib.rs)
supersedes that baseline's failed-PUT temporary behavior; it is not a claim about
a published release.

Ordinary PUT write, flush, sync and rename failures, and Rust unwinding, close
this operation's temporary handle and attempt to remove only its unpublished
`.tmp-*` name. Cleanup is armed after exclusive creation succeeds and disarmed
after successful publication. A failed removal emits a warning without replacing
the primary I/O error; other writers and committed digest names are not targets.

SIGKILL, process abort and power loss do not run Drop. Crash leftovers remain
excluded from normal GC and may consume space beyond the committed-blob budget.
Any manual removal requires all writers of that exact cache root to be stopped
and the names to be established as abandoned cache temporaries; age or PID alone
is insufficient. Do not follow symlinks or apply that procedure to a workspace
destination. This cleanup fix (#314) does not change #304's maintenance schedule.


---

<a id="9-exit-codes"></a>
## 9. Exit codes & the cross-team contract

See [CLI-SURFACE-EXIT-FREEZE.md](CLI-SURFACE-EXIT-FREEZE.md) and the real dispatchers.
The distinction between intended contract and known baseline defects matters.

| Path | Code | Interpretation |
|---|---|---|
| Ordinary command success | 0 | Includes dry-run/no-op results; inspect the operation/result variant. |
| `status` | 0 / 1 / 2 | Clean / dirty / error. |
| Other command/config/usage error | 2 | The diagnostic supplies the error category. |
| `run` completed child | Child code | Passed through; includes legitimate child exits 125 and 137. |
| `run` internal error mapping | 125 | Intended internal-failure code; do not infer lifecycle from the integer alone. |
| `run` captured stream exceeds cap | 137 | Output-limit mapping; baseline misclassifies some pre-spawn size errors (#305). |
| Unix signal termination | 128 + signal | When the child's signal can be obtained. |
| Unrecoverable abnormal termination | 255 | Includes the non-Unix fallback path. |

The initial oversized AC response is a known pre-spawn classification issue, not
proof of an executed/killed child. Delivery failures and diagnostics after a
result exists have their separate #310 obligation. Consumers must not blindly
reexecute a command based on 125/137 or on text claiming that no child ran.

Error variants include `Auth`, `Http`, `NotFound`, `TooLarge`, `Integrity`,
`InvalidDigest`, `InvalidManifest`, `InvalidTenant`, `Config`, `Transport`, `Io`,
`Json` and `Message`. They do not each have an exclusive CLI exit code. HTTP 402,
409 and 429 are HTTP statuses, not process codes; local guards can also produce
an error whose wording mentions a server status. #309 owns conflict attribution.

Producer memo storage is restricted to successful results. The separate received
record eligibility issue #307 remains at this source. No format/key/lifecycle
contract is changed by this documentation.

---

<a id="10-auth--credentials"></a>
## 10. Auth & credentials

The transport sends `Authorization: Bearer <PAT>` and places the validated tenant
in `/v1/cas/{tenant}/...` or `/v1/ac/{tenant}/...`; no `x-corelink-tenant-id` header
is required. The server controls the credential's permitted operations.

With the runner bootstrap trio, a CAS/AC command redeems the ticket before
operation dispatch. An invalid/incomplete redemption fails without falling back
to a static token. Offline identity/help commands do not redeem a ticket. A
single-use bootstrap ticket is distinct from the resulting bearer credential.

`CLW_REF_DOMAIN=runner` selects a different name-hash prefix. It is not an ACL;
list/raw-key/deletion behavior and shared CAS still require server authorization.
`run` strips listed credential environment variables but preserves other ambient
state and same-user filesystem access. Config masking is not a sandbox.

### Operational bounds

`CLW_OP_DEADLINE_SECS` bounds one transport operation across retries and permit
acquisition; a CLI command can make many such operations. It does not bound all
file I/O or child runtime. Requests/connects also have their individual timeouts.
Retry policy handles eligible transport/5xx/429 errors with a bounded ladder.
`Retry-After` supports integer delta-seconds with a 10-second sleep cap; HTTP-date
values fall back to the ladder. A large hint informs exhaustion diagnostics; it
is not independent knowledge of an account's actual quota.

The adaptive governor can shrink concurrency on 429 and recover after successes.
Captured stdout and stderr each have a 5 MiB cap; the post-success cache-write
phase has its own 30-second deadline. None of these is a whole-command sandbox
or an unlimited guarantee against hanging local resources.

---

<a id="11-features-in-depth"></a>
## 11. Features in depth

The command reference above is the capability inventory. The table below records
important cross-command properties without converting local mechanisms into
system-wide guarantees.

| Feature | What is implemented | What must not be inferred |
|---|---|---|
| Chunk deduplication | Per-operation digest set and remote presence checks | Global exactly-once upload or permanent availability. |
| Streaming | Large source files are chunked incrementally | Every path/allocation is bounded by one chunk; see #291/#313. |
| Cache integrity | Cached bytes are rehashed; corrupt content is discarded | Durable retention, hard quota or identical Windows/Unix access. |
| Local eviction | Approximate mtime-based recency, amortized maintenance | Every short invocation returns within budget; #304 remains. |
| Hydrate publication | File temporary, explicit finish, sync and rename | Atomic directory, full rollback or sibling quiescence. |
| Content pinning | Fetched bytes must match the expected BLAKE3 digest | Trusted origin, safe semantics or retention of that digest. |
| Metadata | Producer records supported modes, links and empty directories | Every filesystem can represent every accepted namespace. |
| Memoization | Declared-input key and bounded captured streams | Artifact restoration, transitive tool discovery or sandboxing. |
| Env hygiene | Known credential names omitted before child spawn | The child cannot read same-user files or other secrets. |
| Retries | Bounded per-transport-operation retries/time budget | One deadline for the entire multi-object CLI command. |
| Ref lineage | A ref can record its prior root as `parent` | Traversable retained history; prune does not mark parents. |
| Deletion | Consent, fail-safe branches and partial-progress reporting | Completeness of all shared root families or permanent offboarding. |
| Observability | Redacted identity reports and configured workflow lanes | Parser help is fully redacted (#333), or the independent observer is deployed. |

Current defects have canonical owners in
[tracker #325](https://github.com/HuGR-Labs/corelink-workspaces/issues/325).
A correct boundary description does not close those obligations.

---

<a id="12-use-cases--stories"></a>
## 12. Use cases & user stories

**Move selected workspace contents.** Snapshot under a distinct name, restore into
a new directory, then verify the selected bytes and intended platform behavior.
Include dependencies/build state explicitly; `.git` and ignored content are absent.

**Reuse a deterministic check.** Declare input files and relevant environment,
and keep cwd/tool identity stable. A HIT returns recorded output, not fresh test
execution or rebuilt artifacts. Use a direct build when filesystem outputs matter.

**Hold a trusted content identity.** Supply `--manifest-digest` when the caller
already trusts the digest. Keep an appropriate live ref and coordinate GC:
possession of a digest alone does not reserve its blobs.

**Manage storage.** Inspect the full dry-run and all listing errors. Execute a
sweep only inside a verified root/writer envelope; unknown shared consumers are
a stopping condition, not a reason to bypass the safety floor.

**Operate a runner.** Inject scoped identity and required external isolation.
Bootstrap tickets, bearer permissions and user/runner name domains are different
concepts. Do not classify arbitrary child lifecycle solely by exit 125/137.

---

<a id="13-when-to-use"></a>
## 13. When to use it — and when not

Use it for selected file-tree transfer with content deduplication and for reusable
results of deterministic, side-effect-light local commands. It is useful when the
caller already knows the relevant inputs and can provide any required isolation.

It is not a machine/process checkpoint, transactional capture of a changing tree,
remote execution service, full artifact cache or standalone offboarding protocol.
The root manifest must fit in one 5 MiB blob; splitting the selected tree may be
necessary. A manifest fitting that bound is not proof of destination representability
or sufficient disk space. Hydrate requires a new/empty destination.

`run` preserves ambient non-secret state. Undeclared files, network, clock and
transitive tools can invalidate assumptions without changing the key. Changing
cwd also changes reuse. Execute the real command when these dependencies cannot
be enumerated or when its output files need to be created.

Prune requires a complete understanding of shared roots and publication. An open
GC obligation is not mitigated simply by a plausible dry-run count. Windows copy
semantics and ACL behavior need native evidence, not Unix mode extrapolation.
Backend quotas/authorization come from the selected service; this client neither
sets plan limits nor independently certifies them.

---

<a id="14-security--safety"></a>
## 14. Security & safety model

Read [SECURITY.md](../SECURITY.md) and [THREAT-MODEL.md](THREAT-MODEL.md).
They distinguish implemented protections, historical evidence and open obligations.

Transport requires parsed HTTPS without embedded credentials, except permitted
loopback HTTP for local development/tests; redirects are disabled. The operator
still chooses a trusted destination. Hash verification proves expected bytes,
not who selected them, whether their metadata is valid or whether they are retained.

Literal path/link validation is required but not equivalent to complete combined
link confinement or a representable filesystem namespace (#296/#303). Resource
policy runs after manifest parse and bounds logical counts/bytes, not all memory
or physical disk consumption. Windows copy expansion is a separate boundary.

Managed identity reports mask token values; tenant IDs and paths may be visible.
**Generated help has a separate known issue (#333): do not share help collected
with credentials configured.** Scrubbing known child environment names does not
prevent same-user file access and does not sanitize arbitrary command output.

Unix file modes, Windows ACL application, config/cache paths and arbitrary hydrate
destinations need distinct verification. Do not infer them from a final chmod
alone. A per-file publish protocol is not a tree rollback or a promise that every
writer has stopped when an outer operation fails.

Consent controls and empty-state checks do not certify shared-root completeness,
writer exclusion or all contractual erasure duties. Use the
[destructive-operations runbook](go-live/RUNBOOK-destructive-ops.md); do not execute
unverified deletion as a documentation smoke test.

---

<a id="15-distribution--licensing"></a>
## 15. Distribution, signing & licensing

The source is proprietary and the binding [EULA](../EULA.md) governs use. The
configured binary destinations are the public `HuGR-Labs/clw-releases` mirror and
`HuGR-Labs/homebrew-clw` tap. Repository privacy and binary availability are
separate properties; a release, draft or RC may cause external uploads.

At the documented baseline, [release.yml](../.github/workflows/release.yml) defines
five builds on hosted Intel macOS, live-conformance requirements, minisign and
platform signing/verification steps, checksum publication and optional tap updates.
Real-release policy is stricter than an RC path that explicitly permits skips.
A configured step is not proof that signers are provisioned or that a particular
artifact passed. RC scheduling, immutable retry sets and distributed-artifact
acceptance remain #321/#322/#323.

Select a specific version and verify its actual digest-bound evidence under
[INSTALL.md](INSTALL.md) and [DISTRIBUTION.md](DISTRIBUTION.md). Do not repeat an
old release's unsigned/notarized status as a timeless fact. Source version 0.1.12
is not a claim that it is the latest release or that this document shipped a binary.

The existing gate covers compiler/tests and supply-chain policy in its selected
lanes. Coverage, fuzz, mutation, stress, native-platform and authenticated live
results have separate selectors and evidence. See [TESTING.md](TESTING.md);
a green default-feature run can execute zero live tests. This guide neither
reports a new measured coverage floor nor waives the unresolved test-map #312.

---

<a id="16-troubleshooting--faq"></a>
## 16. Troubleshooting & FAQ

- **`clw doctor`** is the first stop — a redacted, support-ready diagnostics bundle (it never prints your token).
  Its `problems:` list flags a missing/invalid tenant, a missing token, an empty endpoint, or an invalid
  ref-domain.
- **"missing tenant / token"** (exit 2) — set `CLW_TENANT` + `CLW_TOKEN` (or `clw init` / `clw auth login`).
- **Occupied-name conflict** — differing content without force is refused locally in this baseline; #309 owns its misleading 409 wording. Review the non-atomic force/replacement limitation before changing the ref.
- **`hydrate` says "destination not empty"** — hydrate refuses to write into a non-empty directory. Choose a
  new path, or remove/empty the target first.
- **"not found in CAS: <digest>"** — usually a wrong `--name`, a wrong `CLW_TENANT`, or a snapshot that was
  pruned/erased. Check the tenant + name (`clw ls --all`).
- **"config file already exists"** on `clw init` — you already have `~/.clw/config.toml`. Re-run with
  `clw init --force` to overwrite it.
- **HTTP 402** — your CoreLink account hit its plan's monthly usage/$-ceiling. It's a billing limit, not a clw
  error; raise your plan (the message links the pricing page). **HTTP 429** — a server-side rate/storage limit;
  clw honors a `Retry-After` and retries as transient backpressure — but if the limit is *persistent* (e.g. a
  storage cap you're over), the retries exhaust at the op deadline and it fails. Both are backend/plan limits
  clw only reacts to; check your plan with CoreLink.
- **Stale replay** — check undeclared/concurrently changed inputs and the open
  tool/environment identity findings (#301/#302). Declaring known inputs is
  necessary, not proof that these distinct defects are already resolved.
- **"too many entries" on snapshot** — the workspace's manifest would exceed the single 5 MiB CAS blob. The
  entry count where this hits is **path-length dependent** (short paths fit far more). The enforced limit
  is a **byte cap** (`CAS_BLOB_CAP_BYTES = 5 MiB`), not a file count. The client
  checks before uploading the root, but chunk processing/uploads may already
  have happened. A server 413 is an additional transport response. Split the workspace or trim tracked files.
- **macOS download rejected** — verify the selected asset and its approved
  signing/notarization evidence under INSTALL/DISTRIBUTION. Do not remove
  quarantine just to defeat an unexplained platform-verification failure.
- **A hung operation** — the 300-second default covers one transport retry sequence, not all requests, local I/O and child work in a command. Identify which phase is blocked rather than assuming a total deadline.
- **Full cleanup** — `clw uninstall` removes `~/.clw` (config + PAT) and the cache; it prints the `rm <path>` for
  the binary (it won't delete its own running binary).

---

<a id="17-recipes"></a>
## 17. Recipes (cookbook)

### Save a selected checkpoint and restore

```sh
clw snapshot ./project --name project-checkpoint-001
clw hydrate ./restored --name project-checkpoint-001
clw status ./restored --name project-checkpoint-001
```

Use distinct names for intentionally retained checkpoints and verify the applicable
retention/GC policy. `parent` does not pin history. Reusing a different occupied
name requires force's non-atomic replacement in this baseline; force is not scrub.
Do not clean a partial destination while another writer may still be active.

### Handle status without confusing an error with dirty content

```sh
if clw status ./project --name project-checkpoint-001; then
  echo clean
else
  code=$?
  case "$code" in
    1) echo dirty ;;
    *) echo "status failed: $code" >&2; exit "$code" ;;
  esac
fi
```

### Memoize a deterministic, output-only check

Declare its complete files and environment, including relevant manifest/lock/config
inputs and any tools not automatically resolved. Run directly when the command's
filesystem outputs must exist. Cache output says what a prior execution returned;
it is not a fresh test run, proof of isolation or artifact restoration.

### Local identity and footprint inspection

```sh
clw --json doctor
clw --json auth whoami
clw --json auth rotate             # reports rotated:false; guidance only
clw --json uninstall --dry-run     # preview; binary.removed remains false
```

Use approved account-side issuance/revocation plus `auth login` to persist a
replacement credential. Login itself neither mints nor revokes. Do not put tokens
in command history or attach help captured with a configured token (#333).

### Storage preview

```sh
clw --endpoint "$APPROVED_ENDPOINT" --tenant "$APPROVED_TENANT" --json ls --all
clw --endpoint "$APPROVED_ENDPOINT" --tenant "$APPROVED_TENANT" prune
clw --endpoint "$APPROVED_ENDPOINT" --tenant "$APPROVED_TENANT" erase
```

These deletion commands are previews without `--yes`. Their output and exit status
must still be inspected. For any actual deletion, follow the authorization,
root-completeness, writer-coordination and verification requirements in
[the runbook](go-live/RUNBOOK-destructive-ops.md). A zero workspace count alone
is not proof of an empty tenant.

### Completion and concurrency

```sh
clw completions bash
clw --concurrency 4 snapshot ./project --name project-checkpoint-002
clw --no-color status ./project --name project-checkpoint-002
```

Completion output is shell source; inspect it before installation. `--json`
completions is rejected. `--concurrency` is not a run-command knob.

### Authenticated conformance

This is an opt-in service test, not an offline recipe. It can write to the chosen
service. Obtain explicit endpoint/tenant/identity authorization and use the
required-mode policy under the release/conformance workflow. Do not rely on the
built-in endpoint fallback or treat zero executed tests as conformance success.

---

<a id="18-outputs-schemas"></a>
## 18. Command outputs & `--json` schemas

`--json` is a **global flag before the subcommand** (`clw --json <cmd>`). Data commands support it; `run`
forwards captured child streams, while `completions` emits raw shell source and rejects `--json` with exit 2.
The examples below are abbreviated display/shape references, not literal fixtures or
a universal response schema. Ordinary output requires working stdout/stderr; a
reporting failure does not roll back earlier filesystem or remote effects.

### snapshot
```
snapshot complete
  name:            myapp-main
  root:            af1349b9f5f9a1a6...        (64-char BLAKE3 hex)
  files:           312
  bytes_total:     41943040
  chunks_total:    41
  chunks_uploaded: 3                          (< chunks_total ⇒ dedup working)
```
An unchanged result reports that the ref already points at this content.
`chunks_uploaded` is normally zero, but may be nonzero when force caused missing
chunks to be uploaded before the same-root return. `unchanged` does not mean scrub.
`--json`: `{ name, root, files, bytes_total, chunks_total, chunks_uploaded, unchanged(bool), secret_warnings:[string], skipped_external_symlinks:[string] }`. `secret_warnings` is the same advisory list printed to stderr for interactive users and is `[]` for a clean tree.

### hydrate
```
hydrate complete
  source:            myapp-main            (the name, or "manifest digest <hex>" with --manifest-digest)
  root:              af1349b9f5f9a1a6...
  files:             312
  bytes_total:       41943040
  bytes_from_cache:  0
  bytes_downloaded:  41943040
```
`--json`: `{ root, files, bytes_total, bytes_from_cache, bytes_downloaded }` (no name/source field in JSON).

### status
```
clean — workspace matches remote ref 'myapp-main'          # exit 0
# or, dirty (exit 1):
added:
  + new-file.rs
removed:
  - old-file.rs
changed:
  ~ src/main.rs
```
`--json`: `{ clean(bool), added:[string], removed:[string], changed:[string] }`

### ls
```
# --name (single ref):
name:         myapp-dev
root:         af1349b9f5f9...
created_at:   2025-06-04 00:00:00 UTC
parent:       c3d7e2a1b4f8...        (or "none")
files:        312
bytes_total:  41943040

# --all (tenant enumeration): one row per ref, then a summary:
af1349b9f5f9a1a6  2025-06-04 00:00:00 UTC  myapp-dev
c3d7e2a1b4f80011  2025-06-03 11:20:00 UTC  myapp-main
(2 refs in tenant 'my-tenant')
```
`--json --name`: `{ name, root, created_at_unix, parent:(hex|null), files, bytes, bytes_total }`. `bytes_total` is the preferred spelling; legacy `bytes` remains for this deprecation cycle.  ·  `--json --all`: `{ tenant, count, refs:[{name,root,created_at_unix,parent}], run_cache_entries, foreign_ac_entries, skipped, errors:[{ref_key,error}] }`

### run (no `--json`)
```
[clw] cache miss       # then the child's stdout/stderr verbatim, then exits the child's code
[clw] cache hit        # replays cached stdout/stderr/exit without re-running
```

### doctor
```
clw doctor — diagnostics (redacted)
===================================
clw version:    0.1.12
os/arch:        macos/aarch64
config file:    /Users/you/.clw/config.toml
config exists:  true
config mode:    0o600
endpoint:       https://corelink-api.humangr.com
tenant:         <set>
tenant value:   my-tenant
token:          <set>              (never the value)
cache dir:      /Users/you/.clw/cache
cache exists:   true
problems:       none
-----------------------------------
note: only the token is redacted (the tenant ID is an account identifier, shown in full — support needs it).
```
`--json`: `{ clw_version, os, arch, config_file, config_exists, config_mode, endpoint, tenant, tenant_value, token, cache_dir, cache_exists, problems:[string] }`

### auth
```
# login (stderr):
logged in — wrote identity to /Users/you/.clw/config.toml
  endpoint: https://corelink-api.humangr.com
  tenant:   my-tenant
  token:    <set>
# whoami (stdout): endpoint / tenant / token:<set>
# logout (stderr): removed from /Users/you/.clw/config.toml: token
# rotate (stderr): clw auth rotate: to rotate your token, run `clw auth login` with the new token…
```
`--json`: login `{logged_in,wrote,endpoint,tenant,token}` · whoami `{endpoint,tenant,token}` · logout — on an actual removal `{removed:[string],file}`, but on a **no-op** (no config file / no token set) the shape is `{removed:[],note}` **with no `file` key** · rotate `{rotated:false,action_required,note}`

### prune / erase / rm / init
```
# prune dry-run:   DRY RUN — would delete 3 orphaned blob(s) (128 live). Re-run with --yes to delete: …
# prune --yes:     pruned 3 orphaned blob(s) (128 live)
# erase dry-run:   DRY RUN — would IRREVERSIBLY erase tenant 'my-tenant': 12 ref(s) + 480 blob(s). …
# erase success:   erased tenant 'my-tenant': 12 ref(s) + 480 blob(s) deleted; tenant verified empty
# rm:              deleted ref 'myapp-dev'   |   no such ref 'x' (nothing to delete)
# init (stderr):   wrote config to /Users/you/.clw/config.toml
```
`--json`: prune `{dry_run,live,orphans?,would_delete?[],deleted?}` · erase `{dry_run,tenant,refs|refs_deleted,blobs|blobs_deleted,complete?}` · rm `{name|ref_key,deleted}` · init `{wrote,token}`.

**Partial-failure & refusal shapes** (the JSON differs from the happy path — a runner must handle these):
- **prune** — a mid-sweep failure adds `failed_at:"<hex>"`; a server capability regression (405/501) emits `{error:"capability-unavailable",note}`; a replica-lag unresolvable ref emits `{error:"live-set-incomplete",note}`; an empty-live-set-over-non-empty-CAS refusal emits `{error:"empty-live-set-refused",note}`. All with a non-zero exit.
- **erase** — an incomplete verify adds `remaining_refs` + `remaining_blobs`; a mid-delete failure adds `note`; a pre-flight abort emits a different shape `{tenant, erased:false, note}` (the `erased` key appears only here).
- **rm** — a declined/`--yes`-less confirmation emits `{ref, deleted:false, note}` (the `ref` and `note` keys are absent from the happy `{name|ref_key,deleted}` shape).

### uninstall
```
# dry-run: lists config/cache targets; nothing removed; executable is instruction-only
# --yes: removes accepted config/cache targets; the running executable is NOT removed
```
`--json`: `{ dry_run, targets:[{path, existed, removed}], binary:{path, removed:false, remove_command} }`. The binary is never an automatic deletion target. Dry-run marks every target `removed:false`.

---

<a id="19-manifest-spec"></a>
## 19. The manifest format (wire spec)

[`clw-types`](../crates/clw-types/src/lib.rs) and
[SPEC-manifest.md](SPEC-manifest.md) are the implementation and specification.
[#308](https://github.com/HuGR-Labs/corelink-workspaces/issues/308) owns their
acceptance reconciliation; this guide does not introduce a new format.

A root is one flat object with `version: 1`, `total_size` and ordered `entries`.
File fields are `kind`, `path`, `mode`, `size`, `chunks`; link fields are `kind`,
`path`, `target`; directory fields are `kind`, `path`. Each chunk has `digest`
and `size`. Chunks reconstruct the file in their list order, without framing or
compression. Empty files require no chunks; empty directories are explicit.

The producer emits compact deterministic JSON with strictly increasing unique
entry paths and lowercase BLAKE3-256 hex. Reader acceptance is not a raw-byte
canonical-JSON equality check. `Digest::from_hex` accepts uppercase as well as
lowercase digits; serialization emits lowercase. Do not reject/normalize a
received root by pretending newly serialized bytes have its original digest.

Entry paths are relative and reject empty/dot/dotdot, backslash and colon
components. Symlink targets use a separate contained-relative validator that
permits internal `..`. These checks do not guarantee every combined-link graph
or filesystem namespace will materialize safely (#296/#303).

Producer file sizes derive from chunked bytes. Hydrate now computes each file's
chunk-size sum with checked addition and rejects overflow or disagreement with
`File.size` before creating the destination or fetching materialization chunks
(integrated #306 / PR #334). Aggregate equality with `total_size`, resource caps,
and received chunk-length/integrity checks remain in place. This is materializer
validation, not a new `Manifest::from_bytes` acceptance rule. Chunk-size range
acceptance at parse remains #308; the transport caps each fetched/posted body.
Passing parse does not promise hydrate success or available disk space. Unix mode output uses safe permission bits;
Windows cannot inherit a Unix permission guarantee merely from the numeric field.

A `RefRecord` in AC has `name`, `root`, `manifest_size`, `created_at_unix`,
`parent` and `tool_version`. Ref names use `clw/ref/v1/` or
`clw/ref/runner/v1/` hash domains. Parent is lineage, not a GC traversal edge.
Forced replacement uses separate deletion/publication; see #292.

A `RunRecord` is also stored in **AC**, with `exit_code`, `stdout` and `stderr`.
The two stream digests address CAS blobs; the record has no filesystem-output
inventory. Producers store successful exits only; received replay eligibility
is #307. Recognizing references for GC must remain separate from replay policy.
Required field sets distinguish ordinary producer records, not a blanket claim
that arbitrary extended JSON cannot satisfy more than one parser.

For source anchors covering these claims, see
[the architecture index](ARCHITECTURE.md#source-to-claim-review-index).

---

<a id="20-platform-support"></a>
## 20. Platform support

The configured release targets are:

| OS | Arch | Target |
|---|---|---|
| macOS | arm64 | `aarch64-apple-darwin` |
| macOS | x86_64 | `x86_64-apple-darwin` |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` |
| Linux | arm64 | `aarch64-unknown-linux-gnu` |
| Windows | x86_64 | `x86_64-pc-windows-gnu` |

Darwin targets are not GNU targets. At this source, build jobs run on hosted Intel
macOS: x86_64 Darwin is native, ARM Darwin uses a system-linker cross-build, and
Linux/Windows use cross toolchains. Authenticode verification runs natively on
Windows. That does not establish full native Windows runtime coverage.

The general gate's Windows-GNU cross-Clippy is compilation/lint. Wine coverage
and portable tests have their own configured scope; inactive tests are not
executed assertions. Match any acceptance claim to target, selector and actual
result rather than the existence of a workflow.

Windows link entries materialize as copies, not persistent link relationships.
Expansion/order (#298/#299), filename aliases (#303) and ACL behavior (#317)
remain explicit. Current signing/Gatekeeper status must be checked on the selected
artifact, not inferred from the operating system or from a prior release.

---

<a id="21-compliance-legal"></a>
## 21. Compliance, privacy & legal

This is a technical guide, not an amendment or fresh verification of legal and
operational policy. The source-of-record documents are
[EULA](../EULA.md), [Privacy Policy](PRIVACY-POLICY.md),
[DPA/subprocessors](DPA-subprocessors.md), [Export Control](EXPORT-CONTROL.md),
[DR/business continuity](DR-business-continuity.md) and [Security Policy](../SECURITY.md).
Consult their current approved versions for entities, residency, retention,
subprocessors, contracts, rights, contacts and response commitments. This guide
does not certify those commitments from source inspection.

The client transfers selected content/paths and result streams and may store
plaintext identity locally. `rm` removes a ref; `prune` sweeps selected CAS
candidates; `erase` clears and rechecks the enumerated AC/CAS scope. None alone
proves deletion of every other catalog, log or backup, or exclusion of new writes.
Resolve server-side scope and completion under #316 before recording a broader
erasure/offboarding outcome. `uninstall` removes local footprint, not remote data
or server credentials, and does not delete its own executable.

Existing policy contacts and contractual duties remain unchanged. Sensitive
content, tokens and operational logs should not be copied into public reports.

---

<a id="22-roadmap-status"></a>
## 22. Roadmap & status

This page describes source `be96b8394c66d1dc36ce7ca2f186780f0ea48308`, workspace
version 0.1.12. It does not designate a latest release or a newly verified service.
Historical release and deployment records remain in Git and their dated documents.

The current reconciliation is #311 / plan v1.3 WP-20. #312 separately owns the
full test/JSON map. Runtime issues retain their own acceptance and are not closed
by correcting a claim here. Read [tracker #325](https://github.com/HuGR-Labs/corelink-workspaces/issues/325)
for execution and [ARCHITECTURE.md](ARCHITECTURE.md) for source-to-claim anchors.

Features such as atomic ref replacement, new retention protocols, sandboxing or
broader erasure guarantees require their own decisions and implementation. They
are not silently approved by this documentation change. Independent review,
applicable gates and current-base reconciliation remain integration requirements.

---
