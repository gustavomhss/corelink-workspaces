# CoreLink Workspaces (`clw`) — The Complete Guide

> The full reference for CoreLink Workspaces: what it is, the problems it solves, how it works,
> every command and flag, and how / when / why to use it.
> Status: **v0.1.11 — open beta.** `brew install humanguardrail/clw/clw`.

---

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

> **The workspace as a content-addressed object; compute as a disposable cursor.**

CoreLink Workspaces (`clw`) **snapshots** a directory tree into CoreLink's content-addressed store,
**hydrates** it anywhere in seconds, and **memoizes** deterministic commands so they never re-run when
their inputs haven't changed.

The idea: a *workspace* — its source, dependencies, toolchain, and build state — is a durable,
immutable, content-addressed **object**. The machine that holds it (a CI runner, an AI agent's sandbox,
a developer's laptop) is a **disposable cursor**: it hydrates the workspace from a cache that grows
warmer with every snapshot, attaches, does work, writes back a delta as a new immutable manifest, and is
thrown away. The durable thing is the workspace, not the machine.

Technically, `clw` is a **pure client** (zero server code) of the live CoreLink API. It talks to two
backend primitives:

- **CAS** — a content-addressed store of immutable blobs keyed by their BLAKE3 digest (5 MiB per blob).
- **AC** — an action-cache / ref store: named pointers that map a *workspace name* → a *manifest digest*,
  and that also hold `clw run` memoization records.

Auth is `Authorization: Bearer <PAT>` with the tenant in the URL path. Configuration is environment-driven
— no config file is required. Files are split by content-defined chunking, and **every** byte fetched
from CAS is BLAKE3-verified before it's used.

---

<a id="2-the-problems-it-solves"></a>
## 2. The problems it solves

| The pain | How `clw` removes it |
|---|---|
| **"Works on my machine."** No durable, exact, restorable snapshot of a working tree (source + build state). | `clw snapshot` captures the tree into an immutable, named, content-addressed object — restored byte-, mode-, and symlink-identical. |
| **Re-shipping unchanged bytes** every time (slow uploads/downloads). | Content-addressed **dedup**: snapshot uploads only chunks not already in CAS; hydrate serves from a warm local cache. Only *new* bytes move. |
| **Moving a workspace between machines** is manual and lossy. | `clw hydrate <DEST> --name <NAME>` materializes anywhere in seconds — cache-first, integrity-verified, atomic. |
| **Re-running deterministic work** (CI rebuilds/tests that didn't need to run) burns minutes. | `clw run` memoizes stdout/stderr/exit keyed by input digests + command + declared env + cwd. A cache hit replays instantly; only exit-0 is cached. |
| **Detecting drift** — is my tree still the known-good snapshot? | `clw status` diffs local vs remote: exit 0 = clean, 1 = dirty, 2 = error — scriptable. |
| **Storage lifecycle / cost** — orphaned blobs pile up. | `clw rm` (delete a ref) + `clw prune` (mark-and-sweep GC, dry-run by default, fail-safe). |
| **GDPR/LGPD right-to-erasure** — actually deleting all of a tenant's data. | `clw erase` irreversibly wipes every ref + blob for a tenant (Art.17), triple-guarded and verify-empty. |
| **Ephemeral CI/runner containers need identity without a baked-in secret.** | Env-injected identity, a runner keyspace, and the **single-use cred-ticket** — the ticket, not the PAT, is injected; clw redeems it once at boot. |
| **A consumer that already holds a content digest** shouldn't need a name lookup. | `clw hydrate --manifest-digest <HEX>` materializes directly by content; the manifest is verified to hash to the digest. |

---

<a id="3-who-uses-it"></a>
## 3. Who uses it (personas)

1. **The CI runner / build fabric — the primary consumer.** It shells out to `clw` (pinning it by binary
   digest — no library/ABI coupling) and **drives `snapshot → hydrate → run`; it never reimplements them.**
   Identity is injected via `CLW_*` env at dispatch; it uses `CLW_REF_DOMAIN=runner` for a
   collision-proof-by-construction keyspace and the cred-ticket for secret-free auth. It depends on the
   **frozen exit-code contract** (child code passed through verbatim; `125` = clw-internal failure where the
   child never ran; non-zero is never cached).

2. **The individual developer / power-user.** Snapshots a project at end of session, hydrates it on another
   machine to resume, uses `clw status` as a dirty-check, `clw run` to memoize local builds/tests, `clw ls`
   to inspect lineage. Installs via Homebrew; configures with env vars or `clw auth login`.

3. **The platform / engine integrating clw.** Any CI/engine that pins the stable v0.1.0 CLI surface and
   parses `--json` output (content-address root, cache hit/miss, byte counts). Example: a toolchain resolver
   that pins `toolchain_ref = <clw manifest digest>` and hydrates content-addressed via `--manifest-digest`.

---

<a id="4-how-it-works"></a>
## 4. How it works (architecture)

### Two stores
- **CAS** (`/v1/cas/{tenant}/{hash}`) — immutable blobs, keyed by **BLAKE3-256** digest (64 hex chars). 5 MiB
  cap per blob. `GET`/`PUT`/`DELETE` + `GET /v1/cas/{tenant}` to list.
- **AC** (`/v1/ac/{tenant}/{key}`) — a mutable key→bytes ref store. `GET`/`PUT`/`DELETE` + list.

### The manifest — a workspace as one object
A snapshot produces a **manifest**: `{ version, total_size, entries[] }`, where each entry is a
`File{path, mode, size, chunks[]}`, a `Symlink{path, target}`, or an (empty) `Dir{path}`. Paths are always
`/`-separated, root-relative, and never contain `.`/`..`. The manifest has **one** legal (canonical) JSON
form — compact, fields in declared order, **entries strictly ascending by path** (which gives both a
deterministic order *and* uniqueness). The manifest's own bytes are stored as a CAS blob; its digest is the
workspace's content address. A **ref** (`RefRecord{ name, root, manifest_size, created_at_unix, parent,
tool_version }`) maps a name → that root digest, with a `parent` pointer for git-style lineage.

### Chunking & dedup
Files are split with **FastCDC content-defined chunking** (256 KiB min / 1 MiB avg / 4 MiB max — frozen, since
changing them "re-chunks the world"). Content-defined boundaries mean identical byte-runs yield identical
chunks *regardless of position*, so dedup works **across files and across snapshots** — only chunks missing
from CAS are uploaded. Files ≤ 16 MiB take an in-memory path; larger files **stream** (at most one chunk in
memory at a time). A proptest proves the two paths produce byte-identical chunks, so a file's digest never
depends on which path it took.

### Snapshot: blobs before ref
`walk → chunk → upload missing chunks → upload manifest blob → write ref`. Chunks and the manifest are
uploaded **before** the ref is written, so a ref never points at absent content. `exists` is checked before
every `put` (dedup); a per-run `HashSet` avoids re-handling the same digest; a single global semaphore bounds
in-flight uploads so a tree of many small files saturates `--concurrency` instead of serializing.

### Hydrate: ref → manifest → materialize (atomic, cache-first)
`resolve root → fetch manifest (re-verify it hashes to root) → enforce caps → create dirs → materialize all
files → materialize symlinks LAST`. Files-before-symlinks removes the window where a link could redirect a
later write outside the tree. Each file is written to a temp file, fsync'd (data+metadata) **before** the
rename to its final name (hydrate never re-hashes on read, so a torn file would be silently trusted), then the
parent dir is fsync'd. Chunks are fetched **cache-first** (the local cache holds only verified bytes); on a
cache miss the bytes are re-verified against their digest before being cached or written.

### Run: memoized execution
The cache **key** = the content digests of the declared `--input` paths (+ their modes) + the *values* of the
declared `--env` names + the full command vector + the **tool identity** of `command[0]` (its content digest,
so an in-place toolchain upgrade invalidates stale hits) + the **absolute cwd**. On a hit, clw fetches the
stored stdout/stderr blobs and **replays them without re-executing**. On a miss, it runs the child, captures
output (each of stdout and stderr is bounded at 5 MiB — a child that exceeds it is killed → exit 137), and
memoizes **only if the child exited 0**. Before spawning the child, clw strips secret env vars (`CLW_TOKEN`,
the cred-ticket trio, …) so an untrusted command can't read them. (An env var passed via `--env` is folded
into the key with a presence-distinguishing encoding — *unset* hashes differently from *set-but-empty*.)

### Everything is BLAKE3-verified
`CasTransport::get` re-hashes every blob; hydrate re-verifies the manifest and every chunk; the local cache
self-heals (a corrupt blob is detected on read and removed). `--manifest-digest` makes hydrate end-to-end
self-verifying — a hostile or buggy CAS cannot substitute different content.

---

<a id="5-install--getting-started"></a>
## 5. Install & getting started

**Install (macOS / Linux — preferred):**
```sh
brew install humanguardrail/clw/clw
```
Brew resolves the OS/arch binary, verifies its SHA-256, and (on macOS) installs a formula binary that is
**not** Gatekeeper-quarantined.

**Fallback (any \*nix), the curl installer** — `CLW_VERSION` is **required** (there is no `latest` alias):
```sh
curl -fsSL https://raw.githubusercontent.com/HumanGuardrail/clw-releases/main/install.sh \
  | CLW_VERSION=0.1.11 sh
```
It downloads the binary + a **minisign-signed `SHA256SUMS`** + the trust-root `minisign.pub`, verifies the
signature over the checksum manifest, then the binary's SHA-256 (never skippable), **strips the macOS
quarantine attribute**, and installs to `~/.local/bin/clw` (ensure that's on your `PATH`).

**Windows:** download `clw-<version>-x86_64-pc-windows-gnu.exe` from the mirror release
(`github.com/HumanGuardrail/clw-releases`) and verify it with **minisign** over `SHA256SUMS` + the SHA-256
of the `.exe`, then put it on `PATH`. *(Authenticode signing via Azure is not yet live — `Get-AuthenticodeSignature`
will show `NotSigned`; minisign is the verification path today.)*

> **macOS note:** a *raw browser download* of the binary is Gatekeeper-blocked (it isn't Apple-notarized yet).
> `brew` and `install.sh` both strip quarantine; otherwise run `xattr -d com.apple.quarantine <asset>` after
> verifying the signature.

**Upgrade / pin / rollback:** `brew upgrade clw` · `brew pin clw` / `brew unpin clw` · or re-run `install.sh`
with a new `CLW_VERSION=<x.y.z>`. There is no in-binary self-update; clw never phones home.

### Quickstart (copy-paste)
```sh
# 1. Identity (required: tenant + token; endpoint defaults to the CoreLink API)
export CLW_TENANT=my-tenant
export CLW_TOKEN=pat_xxxxxxxxxxxxxxxx
clw doctor            # redacted diagnostics — confirm no problems (never prints the token)
clw init              # optional: persist identity to ~/.clw/config.toml (0600). Re-run needs --force.

# 2. Snapshot the current directory
clw snapshot . --name my-ws
#   → snapshot complete: root=9c1f… files=128 bytes_total=4.2 MiB chunks_total=61 chunks_uploaded=61

# 3. Hydrate it somewhere else (must be empty/new); a warm cache serves later runs
clw hydrate /tmp/restore --name my-ws
#   → hydrate complete: files=128 bytes_from_cache=0 bytes_downloaded=4.2 MiB

# 4. Check for drift (exit 0 = clean, 1 = dirty, 2 = error)
clw status . --name my-ws
#   → clean — local tree matches my-ws

# 5. Memoize a deterministic command (first run executes; identical inputs later replay instantly)
clw run --input src/ -- cargo build
#   → [clw] cache miss        (first time)
clw run --input src/ -- cargo build
#   → [clw] cache hit         (replays stdout/stderr/exit without re-running)
```
Config resolves **flag › `CLW_*` env › `CORELINK_*` env › `~/.clw/config.toml` › default**. Endpoint defaults
to `https://corelink-api.humangr.com`; only `tenant` + `token` are required. Add shell completions with
`clw completions <bash|zsh|fish|powershell|elvish>`. Remove clw's footprint with `clw uninstall`.

*(Advanced/dev: `cargo test -p clw-conformance --features live` runs the live conformance suite against a
CoreLink deployment — for verifying a new backend, not day-to-day use.)*
<!-- SECTIONS 11-16 appended below -->

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
| `--token <PAT>` | | *(required for network cmds)* | `CLW_TOKEN` (alias `CORELINK_TOKEN`) | Personal access token. **Never printed.** Prefer env/config over `--token` (visible in `ps`/history). |
| `--cache-dir <DIR>` | | `~/.clw/cache` | `CLW_CACHE_DIR` | Local blob cache. |
| `--ref-domain <D>` | | `user` | `CLW_REF_DOMAIN` | `user` or `runner`; `runner` uses a disjoint keyspace (set by the runner fabric — users never need it). |
| `--json` | | off | | Machine-readable JSON on stdout (errors still on stderr). |
| `--concurrency <N>` | | `8` | | Max concurrent CAS requests for `snapshot`/`hydrate` (min 1; does **not** apply to `run`). |
| `--no-color` | | off | `NO_COLOR`, non-TTY | Disable ANSI color. |
| `--verbose` | `-v` | warn | `RUST_LOG` overrides | `-v`=info, `-vv`=debug. Diagnostics to stderr. Conflicts with `-q`. |
| `--quiet` | `-q` | off | `RUST_LOG` overrides | Only errors. Conflicts with `-v`. |

---

### `clw init [--force]`
Write `~/.clw/config.toml` from the resolved config values (`endpoint`, `tenant`, `token`, `cache_dir`).

- The `~/.clw` dir is hardened to **0700**, the file to **0600** (it holds the plaintext PAT); unrelated
  pre-existing keys are preserved. Errors if the file already exists — re-run with **`--force`** to overwrite.
  The token is never echoed (only the path, or `{wrote, token:<set>/<unset>}` with `--json`).

| Flag | Default | Meaning |
|---|---|---|
| `--force` | off | Overwrite an existing config file (default: error if it exists). |

Examples: `clw --tenant acme --token $PAT init` · `clw init --force`

---

### `clw snapshot [PATH] --name <NAME> [--force]`
Content-address a directory and upload it, recorded under `NAME`.

- `PATH` is **positional** (default `.`), *not* `--path`.
- Re-snapshotting **identical** content is a no-op success ("already up to date").
- A **differing** re-snapshot of an existing name returns HTTP **409** (the ref store is create-only)
  → use `--force` (rm-then-write) or `clw rm --name … --yes` first.
- After a snapshot, non-fatal **stderr notes** may appear (they never change the exit code):
  a `.gitignore`/`.clwignore` exclusion note; a **secret-scan warning** listing secret-looking
  files that were uploaded (advises adding them to `.clwignore`); an **external-symlink skip** note
  (symlinks pointing outside the workspace — absolute or `..`-escaping — are skipped and listed).

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

- **Cache key** = the content digests of the declared `--input` paths + the values of the `--env`
  variable names + the full command vector + the resolved tool identity (a digest of `command[0]`).
- **Hermeticity caveat**: undeclared inputs (network, clock, RNG, files not passed via `--input`) are
  invisible to the key — only memoize **deterministic** commands.
- Input **directories** are walked respecting `.gitignore`/`.clwignore`; an ignored file inside a dir
  is *not* in the key — pass it as an explicit `--input <FILE>` (named files are digested regardless).
- Prints `[clw] cache hit` / `[clw] cache miss` to stderr, streams the child's stdout/stderr verbatim,
  and **exits with the child's exit code**. `--json`/`--concurrency` do **not** apply.
- Result is cached **only if the child exited 0** (a non-zero run is never cached).

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
- `--all`: enumerates every ref in the tenant. **Read-only tolerant** — a corrupt/unparseable record
  or a transient per-record blip is *skipped and reported*, not fatal; a concurrent `rm` drops out;
  a 401 aborts the listing. `clw run` memo records are counted as `run_cache_entries`, not warned.

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

- Computes the **live set** (every manifest reachable from a ref + every chunk they reference) and
  deletes CAS blobs outside it. `clw run` memo blobs are marked live (never orphaned).
- **Fail-safe**: if *any* ref can't be resolved to a parsed manifest (corrupt record, missing manifest
  blob, or a 404 from AC replica-lag), the whole prune **aborts** ("live set is incomplete"), deleting
  nothing and naming the offending ref.
- **Safety floor**: an empty live set over a non-empty CAS (would wipe the tenant) is refused
  unconditionally — even in dry-run — and points at `clw erase`.

Examples: `clw prune` (dry-run) · `clw prune --yes`

---

### `clw erase [--yes] [--confirm-tenant <TENANT>]`
**Irreversibly** delete ALL data for a tenant — GDPR Art.17 "right to erasure". Three rails:
(1) **dry-run by default**; (2) **`--yes`** for intent; (3) **typed-tenant confirmation** — you must
reproduce the exact tenant id (interactively, or `--confirm-tenant <TENANT>` for non-interactive/`--json`;
a mismatch or a non-TTY without the flag aborts non-zero).

- Executes with **verification**: delete refs then blobs, re-enumerate, repeat up to 3 passes, and only
  reports success once enumeration confirms the tenant is empty — else it **fails loud** ("do NOT record
  this tenant as erased"). Resumable on a mid-delete failure.

Examples: `clw erase` (dry-run) · `clw erase --yes` (prompts for tenant) · `clw erase --yes --confirm-tenant acme`

---

### `clw doctor`
Print a support-ready, **redacted** diagnostics bundle. **Always exits 0**, runs even on a broken config.

- Reports: clw version, os/arch, config path + mode, endpoint (with `user:pass@` userinfo stripped),
  tenant (shown in full — it's an account id), token as `<set>`/`<unset>` (**never the value**),
  cache dir, and a **problems** advisory list. `--json` for a structured bundle.

Examples: `clw doctor` · `clw --json doctor`

---

### `clw auth <login | logout | whoami | rotate>`
Manage the locally-stored identity. **The token is never printed** — presence only (`<set>`/`<unset>`);
the 0600 config file is the only place the plaintext token is written.

- **`login`** — persist `endpoint`/`tenant`/`token`/`cache_dir` to `~/.clw/config.toml` (0600, parent 0700),
  preserving unrelated keys.
- **`logout [--all] [--yes]`** — remove the stored token (or `--all` clears the whole file). `--all` is
  destructive: needs `--yes` non-interactively; a declined prompt exits non-zero.
- **`whoami`** — print the resolved identity, redacted, offline (to stdout).
- **`rotate`** — v1 directs you to `clw auth login` with the new token (one-step rotate is post-launch).

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
before any config/login. Shells: `bash`, `zsh`, `fish`, `powershell`, `elvish`.

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
| `CLW_TOKEN` / `CORELINK_TOKEN` | Bearer PAT | — (required) | Redacted everywhere. Not read on the C2c runner path. |
| `CLW_CACHE_DIR` | Local blob cache | `~/.clw/cache` | No `CORELINK_` alias. |
| `CLW_REF_DOMAIN` | Ref keyspace | `user` | `user` \| `runner`; other values → exit 2. |
| `CLW_OP_DEADLINE_SECS` | Overall per-op deadline (across all retries) | `300` (5 min) | Clamped to 24h; `0`/invalid → default. |
| `CLW_CRED_TICKET` + `CLW_LEASE_ID` + `CLW_FABRIC_ENDPOINT` | C2c single-use runner credential trio | — | Only in `runner` ref-domain; ticket is redeemed once at boot. |
| `NO_COLOR` | Disable color | unset | Presence only. |
| `RUST_LOG` | Log level/filter (stderr) | unset | **Overrides** `-v`/`-q`. |
| `HOME` / `USERPROFILE` | Home dir (config path, `~` expansion) | — | |

*(Internal-only: `PATH`/`PATHEXT` for `run` tool resolution; `CLW_FUZZ_GATE`/`CLW_TEST_INTERNAL_PANIC` are test seams, not user knobs.)*

### The config file `~/.clw/config.toml`
- Written by `clw init` / `clw auth login`, **0600** (parent `~/.clw` is **0700**) — it holds the plaintext PAT.
- A minimal `key = "value"` format (flat keys: `endpoint`, `tenant`, `token`, `cache_dir`, `ref_domain`).
- Missing file → silent. Unreadable/typo'd/over-permissive file → a **warning** (and a `chmod 600` hint on
  unix) but clw **falls back** and does not fail. Writes are atomic (temp → fsync → rename), PAT-leak-safe.

---

<a id="9-exit-codes"></a>
## 9. Exit codes & the cross-team contract

`clw` exit codes are a **frozen contract** (the runner that drives clw depends on them — see
`docs/CLI-SURFACE-EXIT-FREEZE.md`).

| Exit | Meaning |
|---|---|
| `0` | Success (incl. a `snapshot` no-op "already up to date"). |
| `1` | `clw status` only: the local tree **differs** from the snapshot (dirty) — not an error. |
| `2` | A clw-internal / config / usage error, or any bad-args (clap) error. For `snapshot`/`hydrate`: "clw-internal failure, the child was never involved." |
| `N` (0–255) | `clw run`: the wrapped **child** ran and returned `N`, passed through **verbatim**. Cached **only if N==0**. |
| `125` | **Reserved** — `clw run` internal failure where **the child never ran** (substrate unreachable, bad config, hydrate/cache error, credential-redeem failure, or a panic on the run path). The runner treats this as `ClwFailed`, **never cached**. Prints `[clw] internal error — child not run: …`. |
| `137` | `clw run`: the child **did run** but was SIGKILL'd for exceeding the **5 MiB** output cap. (`128+9`.) |
| `128 + signal` | `clw run`: a signal-killed child (e.g. SIGTERM → 143) — passed through as the child's code. |

**HTTP 402 is not an exit code** — it's the backend's "monthly usage quota / plan $-ceiling reached"
status. clw maps it to an actionable message (pointing to the pricing page) but still exits `2` (or `125`
on the `run` path). It is a **billing** limit on your CoreLink account, not a clw error.

---

<a id="10-auth--credentials"></a>
## 10. Auth & credentials

- **Model:** `Authorization: Bearer <PAT>` + **tenant-in-URL-path** (`/v1/cas/{tenant}/…`, `/v1/ac/{tenant}`).
  There is *never* an `x-corelink-tenant-id` header. The tenant is validated before it goes into the path.
- **Where the PAT lives:** in `CLW_TOKEN`/config; the config file is 0600; the token is redacted in every
  log, `doctor`, `whoami`, and `Debug`.
- **C2c cred-ticket (runner mode):** instead of a PAT, the runner fabric injects a **single-use**
  `CLW_CRED_TICKET` (+ `CLW_LEASE_ID` + `CLW_FABRIC_ENDPOINT`). At trusted boot — before any untrusted
  `clw run` child execs — clw redeems it **once** (`POST {fabric}/v1/leases/{lease}/cas-cred` → `{cas_pat}`)
  and keeps the PAT in memory only. **Fail-closed:** any non-2xx (410 reused / 401 bad / 404 unknown) or
  transport error fails; there is **no fallback to `CLW_TOKEN`**. A no-network command (doctor, whoami,
  logout, uninstall) never redeems, so it can't burn the ticket.
- **Runner keyspace:** `CLW_REF_DOMAIN=runner` selects `clw/ref/runner/v1/`, **disjoint by construction**
  from the user keyspace `clw/ref/v1/` — a user client can never reach a runner ref, for any name.

### Operational bounds (reliability)
- **Per-op deadline** `CLW_OP_DEADLINE_SECS` (default 300s, 24h clamp) bounds the *whole* operation across
  all retries — a black-hole backend can't hang clw.
- **Retries:** up to 6 attempts, backoff ladder `[200ms, 800ms, 2s, 5s, 10s]`, on connect/transport/5xx/429,
  capped by the remaining deadline; honors a server `Retry-After`.
- **Adaptive concurrency (AIMD):** a 429 halves the in-flight window (floor 1); 8 consecutive successes
  grow it back toward the `--concurrency` ceiling (default 8).
- **Output cap:** `clw run` SIGKILLs a child that exceeds **5 MiB on either stdout or stderr** (each stream is
  capped independently) → exit 137.

---

<a id="11-features-in-depth"></a>
## 11. Features in depth

The full capability inventory, grouped. Everything below is a distinct, shipped behavior you can rely on.

### A · The workspace lifecycle
- **`snapshot`** — content-address + upload a directory and record it under a name.
- **`hydrate`** — materialize a snapshot back onto disk (by name or by content digest).
- **`run`** — memoized command execution (replay stdout/stderr/exit on a hit).
- **`status`** — diff local vs remote (exit 0 clean / 1 dirty / 2 error); a scriptable dirty-check.
- **`ls`** — inspect one ref, or enumerate every ref in the tenant (`--all`).
- **`rm`** — delete a ref by name or by raw `--ref-key` (idempotent; GET-first for a truthful `deleted:`).
- **`prune`** — mark-and-sweep GC of unreferenced CAS blobs (dry-run by default).
- **`erase`** — irreversible full-tenant wipe (GDPR Art.17), triple-guarded + verify-empty.
- **`init` / `auth` / `doctor` / `uninstall` / `completions`** — identity, diagnostics, cleanup, shells.

### B · Content-addressing & efficiency
- **BLAKE3-256 content addressing** — every blob, chunk, and manifest is named by its hash.
- **FastCDC content-defined chunking** (256 KiB / 1 MiB / 4 MiB, frozen) — position-independent boundaries.
- **Cross-file & cross-snapshot dedup** — `exists`-before-`put`; only chunks not already in CAS are uploaded
  (`chunks_uploaded` < `chunks_total` shows the savings). A per-run de-dup set avoids re-handling a digest.
- **Streaming for large files** (> 16 MiB) — chunked with at most one chunk resident in memory at a time.
- **Global upload concurrency** — one shared semaphore across all files, so many small files saturate
  `--concurrency` instead of serializing.
- **Concurrent, in-order chunk fetch on hydrate** — a few large blobs download in parallel, applied in order.
- **Self-verifying hydrate by digest** (`--manifest-digest`) — skip the name lookup; the manifest must hash
  to the digest (a hostile CAS can't substitute content). The seam runners use with `toolchain_ref=<digest>`.
- **Idempotent re-snapshot** — identical content is a clean no-op success; a lost-ACK 409 is reconciled
  (an equivalent occupant is treated as success).
- **`--force` replace + corrupt-ref recovery** — replace a name whose content differs; the replace path can
  recover a present-but-corrupt ref via raw-presence gating.
- **Git-style lineage** — each ref records the prior root as `parent`; `ls` shows the chain.

### C · Local cache
- **Warm content-addressed cache** (`~/.clw/cache`) — a sharded (256-way) L1 store; a hydrate serves chunks
  locally on later runs (`bytes_from_cache`).
- **Self-healing** — every `get` re-hashes; a corrupt blob is removed and treated as a miss.
- **GC-to-budget** — an 8 GiB default budget, LRU(mtime) eviction, checked once per 256 writes so `put`
  stays cheap; in-flight `.tmp-*` files are excluded from the budget.
- **Cache-format versioning** — a sentinel invalidates content safely on an upgrade (never a stale replay).
- **Owner-only perms + cross-process safety** — cache tree 0700, blob files 0600; atomic writes; concurrent
  processes don't clobber each other's in-flight puts.

### D · Integrity & durability
- **BLAKE3 verify on every CAS GET** — mismatched bytes are discarded (`Integrity` error), never trusted.
- **Manifest re-verified before parse**; **each chunk's length verified** against its declared size.
- **fsync-before-rename** on every hydrated file (+ a parent-dir fsync) — durable against a torn file on
  power loss (hydrate never re-hashes on read).
- **Atomic writes** everywhere (temp-in-same-dir → fsync → rename).
- **Size-truth guard** — a File's `size` and the manifest `total_size` derive from the bytes actually
  chunked, so an unrestorable snapshot can't be published.

### E · Security & privacy
- **Token never printed** — redacted to `<set>`/`<unset>` in every log, `doctor`, `whoami`, and `Debug`;
  the 0600 config file is the only place the plaintext PAT is written.
- **Endpoint userinfo redaction** — a `user:pass@host` endpoint is shown as `<redacted>@host` everywhere.
- **Plaintext-PAT / SSRF defenses** — HTTPS-only (exact-host loopback check, no `127.0.0.1.evil.com`
  bypass), reject embedded `@`-userinfo, and a **no-redirect** policy so a 3xx can't chase the token.
- **Tenant validation** — the tenant goes raw into the URL path, so it's validated (path-injection guard).
- **Path & symlink containment** — reject absolute / backslash / `.`/`..` / drive-letter / UNC / NTFS-ADS-colon
  targets, on every platform; symlink targets must resolve within the workspace.
- **Secret env stripping in `run`** — `CLW_TOKEN`/`CORELINK_TOKEN` + the cred-ticket trio are removed before
  the untrusted child execs.
- **Secret-scanning warnings on snapshot** — SSH keys, `*.pem`/`*.key`/`*.p12`/`*.pfx`/`*.keystore`,
  `.env`/`.env.*`, `credentials` are flagged (advisory — the files still upload; clw never silently drops data).
- **Hydrate resource caps** — ≤ 1M entries / 100k symlinks / 1 TiB / 64M total chunks / 16M chunks-per-file,
  budgeted against real summed sizes (anti-OOM / anti-disk-fill / anti-symlink-bomb).
- **Mode hardening** — setuid/setgid/sticky bits stripped; files materialized before symlinks (no
  redirect-a-later-write window).
- **Owner-only footprint** — config + cache under `~/.clw` at 0600/0700; an over-permissive config gets a
  `chmod 600` warning.
- **No telemetry / no phone-home** — clw collects nothing and ships with no mechanism to.

### F · Reliability & transport
- **Overall per-op deadline** (`CLW_OP_DEADLINE_SECS`, default 300s, 24h clamp) — bounds a whole operation
  across all retries; a black-hole backend fails cleanly.
- **Bounded retries + `Retry-After`** — up to 6 attempts on connect/transport/5xx/429, honoring the server's
  backoff hint, capped by the remaining deadline.
- **AIMD adaptive concurrency** — a 429 halves the in-flight window (floor 1); 8 straight successes grow it
  back toward the `--concurrency` ceiling. The governor tolerates a poisoned lock.
- **Pagination caps** — hard page-count + total-entry bounds so a hostile/buggy server cursor can't OOM `ls`/`prune`.
- **Prune fail-safes** — abort on an unresolvable ref (replica-lag 404), refuse an empty-live-set-over-non-empty-CAS
  wipe (even in dry-run), and mark `run` memo blobs live so they're never orphaned; a mid-sweep failure resumes.
- **Erase safety** — dry-run default + `--yes` + typed-tenant confirmation; delete-verify-repeat (≤ 3 passes);
  only claims success once the tenant is confirmed empty; a mid-delete failure is resumable.
- **Bounded `run` output** — each of stdout/stderr capped at 5 MiB (child killed → exit 137); a backgrounded
  grandchild that holds the pipe open is drained under a 2s grace; the post-success store is time-boxed.
- **Frozen exit-code contract** — child code verbatim, `125` reserved (child never ran), `137` oversized,
  `128+signal` — a stable surface a CI runner pins against.

### G · Runner / CI integration
- **Runner ref-domain keyspace** — `CLW_REF_DOMAIN=runner` uses `clw/ref/runner/v1/`, disjoint by
  construction from user refs (a user client can never reach a runner ref).
- **Single-use cred-ticket redemption** (C2c) — a runner boots with `CLW_CRED_TICKET` (not a PAT), redeemed
  once at trusted boot for a CAS PAT; fail-closed, no `CLW_TOKEN` fallback.
- **Env-injected identity / no config file** — a disposable container is fully configured by `CLW_*` env.
- **Exit-transparent wrapping** — a runner classifies `clw run` by `125` (clw-failed, never cache) vs the
  child's own code (cached iff 0).

### H · Observability & UX
- **`--json` on every command** (except `run`, which proxies the child) — a documented machine contract.
- **`doctor`** — a support-ready, redacted diagnostics bundle with a `problems:` advisory list.
- **stdout/stderr discipline** — results to stdout, notices/progress/warnings to stderr (pipe-safe).
- **Human-readable timestamps** (`ls`), **progress reporting** (TTY-gated, never in pipes/CI).
- **Accessibility** — `--no-color` / `NO_COLOR` / non-TTY auto-disable; `-v`/`-vv`/`-q` + `RUST_LOG`.
- **Actionable quota UX** — a `402` shows a plan/upgrade message linking the pricing page; a `429` is shown
  as transient backpressure (retrying), distinct from a quota wall.
- **Advisory notes** — ignore-file exclusions and skipped external symlinks are surfaced (never silent).
- **`User-Agent`** `clw/<ver> (<os>; <arch>)` on every request.
- **Shell completions** for bash / zsh / fish / powershell / elvish (pure, offline).

### I · Config & identity
- **Env-driven config** — `CLW_ENDPOINT/TENANT/TOKEN/CACHE_DIR/REF_DOMAIN` (+ `CORELINK_*` aliases); no
  config file required.
- **Precedence** flag › `CLW_*` › `CORELINK_*` › `~/.clw/config.toml` › default; a corrupt/over-permissive
  file warns and falls back (never fails).
- **Lenient resolution** for offline commands (`doctor`, `auth whoami/logout`, `uninstall`) — they run on a
  half-broken config.
- **`auth`** — login / logout (`--all`) / whoami / rotate, fully offline.

### J · Platform & distribution
- **Five native targets** — macOS (arm64 + x86_64), Linux (x86_64 + arm64), Windows (x86_64-gnu); the suite
  runs on Windows and unix.
- **Windows symlink copy-fallback** + **Windows-aware path validation** on every platform.
- **Signed distribution** — minisign-signed `SHA256SUMS` + committed trust-root + CycloneDX SBOM; Homebrew
  tap + `install.sh` + public mirror; macOS quarantine stripped by the install paths.
- **No self-update** — upgrade via `brew upgrade` or re-running `install.sh`.

---

<a id="12-use-cases--stories"></a>
## 12. Use cases & user stories

1. **Snapshot a workspace.** *"At the end of a session I run `clw snapshot --name myapp-$(git rev-parse --short HEAD)`,
   so my exact working tree — source and build state — is a durable, named, content-addressed object."*
2. **Hydrate on another machine.** *"On a second laptop I run `clw hydrate ~/ws/myapp --name myapp-dev` and get a
   byte/mode/symlink-identical tree in seconds, warmed by the cache on later runs."*
3. **Content-addressed transfer.** *"Re-snapshotting a mostly-unchanged tree, `chunks_uploaded` is far below
   `chunks_total` — I store and transfer only the new bytes."*
4. **Memoize a build/test.** *"I wrap my suite `clw run --input src/ --input tests/ -- cargo test`; the first run
   executes and caches, and every later run with identical inputs replays the cached output and exit code
   instantly — unchanged work never re-runs."*
5. **Dirty-check gate in a script.** *"`if ! clw status --name myapp-dev; then …` blocks on an un-snapshotted
   (dirty) tree, using exit 0=clean / 1=dirty."*
6. **Prune old/orphaned blobs.** *"`clw prune` (dry-run) previews orphaned CAS blobs; `clw prune --yes` reclaims
   them — with a fail-safe that refuses to delete when the live set is empty but CAS isn't."*
7. **GDPR-erase a tenant.** *"Honoring an Art.17 request, `clw erase --yes --confirm-tenant my-tenant`
   irreversibly wipes every ref and blob, guarded by dry-run-default + `--yes` + typed-tenant confirmation,
   then verifies the tenant is empty."*
8. **Runner ephemeral container with a cred-ticket.** *"A disposable container gets `CLW_REF_DOMAIN=runner` and a
   single-use `CLW_CRED_TICKET` (no PAT in the env); clw redeems the ticket at boot for a CAS PAT, then drives
   snapshot/hydrate/run — a second redemption fails closed on the fabric's single-use 410."*
9. **Self-verifying hydrate by digest.** *"A resolver that already holds a toolchain digest runs
   `clw hydrate <DEST> --manifest-digest <HEX>` to materialize directly by content key; clw verifies the
   manifest hashes to the digest."*
10. **Exit-transparent CI wrapping.** *"A runner classifies `clw run …` by: exit `125` → clw-failed (child never
    ran, never cache); any other code → the child's own result (already cached iff 0)."*

---

<a id="13-when-to-use"></a>
## 13. When to use it — and when not

**Sweet spot**
- Capturing / moving / restoring whole workspace trees across machines (CI, agent sandboxes, laptops) with
  content-addressed dedup and a warming cache.
- Memoizing **deterministic**, side-effect-light commands (builds, test suites) whose inputs are enumerable
  as files + declared env vars.
- Ephemeral / disposable compute that treats the workspace as the durable object.
- Tenant-isolated private workspaces (only identical, deterministic content dedups across tenants).

**Limits / handle with care**
- **Manifest single-blob ceiling.** A manifest is one CAS blob capped at **5 MiB**, so a workspace with very
  many entries hits an up-front, actionable "too many entries" error. The exact file limit is byte-derived
  (it depends on path lengths — short paths fit far more; **~32k files** is a rough estimate). Split the
  workspace or trim tracked files (multi-part manifests are a future server capability).
- **`clw run` is not hermetic in v0.1.** The child inherits the full ambient environment (only secret vars are
  stripped). Any env var that affects output **must** be declared with `--env`, or a stale result can replay.
  Ignored files are invisible to the key. Non-deterministic commands are a poor fit.
- **Refs are create-only.** In-place versioning isn't live; a *different* re-snapshot needs `--force`.
- **Windows symlinks become copies** (content round-trips, the link relationship doesn't).
- **Single-operator threat model.** clw is a pure client; tenant isolation / auth enforcement live server-side.
  The PAT is the single credential — leaking it grants full tenant access.
- **Quotas & plan ceilings.** Plans are **flat** (a monthly subscription), and paid tenants default to a very
  high monthly $-ceiling. But there are concrete limits you can hit: a **free tier caps storage (~10 GB) →
  HTTP 429** with a `Retry-After` (clw backs off and retries), and a monthly $-ceiling → **HTTP 402** ("raise
  your plan", not a clw setting). A `429` is transient backpressure; a `402` is a billing wall.

---

<a id="14-security--safety"></a>
## 14. Security & safety model

- **The token is never printed** — everywhere it's redacted to `<set>`/`<unset>`; the only place the plaintext
  PAT is written is the 0600 config file (parent dir 0700; atomic, PAT-leak-safe writes; an over-permissive
  file gets a `chmod 600` warning).
- **No plaintext-PAT smuggling / SSRF.** clw refuses a non-HTTPS endpoint (loopback excepted for tests),
  rejects an endpoint with embedded `user:pass@` userinfo (the `https://host@evil.com` dial-evil vector), and
  uses a **no-redirect** policy so a 3xx can't chase the token to another host. Endpoint userinfo is redacted in
  every message.
- **Tenant validation.** The tenant goes raw into the URL path, so it's validated (ASCII alphanumerics + `-`/`_`/`.`,
  no `/`/`\`/whitespace/control, not `.`/`..`) — closing path-injection / cross-tenant misroute.
- **Path & symlink containment.** `validate_rel_path` rejects absolute/backslash/`.`/`..`/empty components and
  any colon (Windows drive-letter / NTFS ADS). Symlink targets must resolve *within* the workspace. Producer and
  consumer use the same validator, so an accepted manifest always hydrates.
- **Hydrate resource caps** (enforced up-front): ≤ 1,000,000 entries, ≤ 100,000 symlinks, ≤ 1 TiB total (budgeted
  against the *real* summed chunk sizes, not the attacker-controlled `total_size`), ≤ 64,000,000 total chunks,
  ≤ 16,000,000 chunks per single file. Declared `total_size` must equal summed chunk sizes exactly, and each
  chunk's actual length is verified against its declared size.
- **`clw run` strips secret env** before spawning the child (`CLW_TOKEN`, `CORELINK_TOKEN`, the cred-ticket trio,
  and their `CORELINK_` aliases) — an untrusted command can't read them.
- **Prune fail-safes.** Dry-run by default; candidate blobs are snapshotted *before* the mark; a ref that
  enumerated but whose record 404s mid-mark (AC replica lag) **aborts the whole prune** rather than sweeping its
  still-live blobs as false orphans; an empty-live-set-over-non-empty-CAS (a stale ref index) is refused
  unconditionally.
- **Erase safeguards.** Dry-run default + `--yes` + typed-tenant confirmation; delete-verify-repeat up to 3 passes;
  only reports success once enumeration confirms the tenant is empty (else fails loud so you never record a
  completed erasure while data remains).
- **Everything is content-verified.** BLAKE3 re-checked at the transport, at hydrate (manifest + every chunk),
  and in the self-healing cache.

---

<a id="15-distribution--licensing"></a>
## 15. Distribution, signing & licensing

- **Distribution model.** `clw` is **proprietary-licensed** (a commercial EULA governs *use*) but **publicly
  downloadable** — the two are independent. The source tree stays private; signed pre-built binaries are mirrored
  publicly on `HumanGuardrail/clw-releases` (a GitHub Release per tag) and installed via the public Homebrew tap
  `HumanGuardrail/homebrew-clw`. Downloading needs no credential.
- **Signing & trust root.** Every release publishes a **minisign-signed `SHA256SUMS`** (+ `.minisig`), the trust
  root `minisign.pub` (committed in-repo + republished per release), and a CycloneDX **SBOM**. `brew` verifies the
  asset against the `sha256` baked into the formula (derived at bump-time from the minisign-verified manifest);
  independent `minisign -V` against `minisign.pub` is the real cryptographic backstop. Binaries are built + signed
  across 5 targets (macOS ×2, Linux ×2, Windows-gnu); a fail-safe skips publish if any build fails, so there's
  never a partial release. *(Deferred, not yet live: macOS Developer-ID notarization, Windows Authenticode, and
  SLSA build-provenance attestation — the last is gated to public repos and waived while the source repo is
  private. The `install.sh` and `brew` paths strip the macOS quarantine attribute.)*
- **License & status.** Proprietary (`EULA.md`); not open source (all crates `publish = false`). Third-party
  attributions in `NOTICE`; disclosure policy in `SECURITY.md`. **Status: SHIPPED — open beta.** v0.1.0 launched
  2026-06-18; **current release v0.1.11**. Pre-1.0 there's no stability guarantee *except* the explicitly
  **frozen v0.1.0 CLI contract** (verbs, flag names, argument order, the `125` reserved run code, the
  child-passthrough + cache-iff-0 semantics) — a stable surface consumers may pin against.

---

<a id="16-troubleshooting--faq"></a>
## 16. Troubleshooting & FAQ

- **`clw doctor`** is the first stop — a redacted, support-ready diagnostics bundle (it never prints your token).
  Its `problems:` list flags a missing/invalid tenant, a missing token, an empty endpoint, or an invalid
  ref-domain.
- **"missing tenant / token"** (exit 2) — set `CLW_TENANT` + `CLW_TOKEN` (or `clw init` / `clw auth login`).
- **A snapshot says "already exists" (409)** — the name holds *different* content. Re-run with `--force`, or
  `clw rm --name <name> --yes` first. (Identical content is a silent no-op, not a 409.)
- **`hydrate` says "destination not empty"** — hydrate refuses to write into a non-empty directory. Choose a
  new path, or remove/empty the target first.
- **"not found in CAS: <digest>"** — usually a wrong `--name`, a wrong `CLW_TENANT`, or a snapshot that was
  pruned/erased. Check the tenant + name (`clw ls --all`).
- **"config file already exists"** on `clw init` — you already have `~/.clw/config.toml`. Re-run with
  `clw init --force` to overwrite it.
- **HTTP 402** — your CoreLink account hit its plan's monthly usage/$-ceiling. It's a billing limit, not a clw
  error; raise your plan (the message links the pricing page). **HTTP 429** — a rate/storage limit (e.g. the
  free-tier storage cap); clw honors `Retry-After` and backs off automatically.
- **`clw run` replayed a stale result** — an undeclared input changed. Declare every output-affecting env var with
  `--env` and every input path with `--input`; only memoize deterministic commands.
- **"too many entries" on snapshot** — the workspace exceeds the ~32k-file single-blob manifest ceiling. Split it
  or trim tracked files.
- **macOS "cannot be opened / unidentified developer"** on a raw download — install via `brew` or `install.sh`
  (they strip quarantine), or `xattr -d com.apple.quarantine <asset>` after verifying the signature.
- **A hung operation** — clw bounds every operation by `CLW_OP_DEADLINE_SECS` (default 300s); a black-hole backend
  fails cleanly rather than hanging forever.
- **Full cleanup** — `clw uninstall` removes `~/.clw` (config + PAT) and the cache; it prints the `rm <path>` for
  the binary (it won't delete its own running binary).

---

<a id="17-recipes"></a>
## 17. Recipes (cookbook)

**Daily snapshot**
```sh
clw snapshot --name myapp-$(git rev-parse --short HEAD)   # commit-pinned name
clw snapshot --name myapp-dev                              # stable rolling "latest"
```
Each snapshot records the prior root as `parent`; lineage is visible via `clw ls`.

**Re-snapshot: identical vs `--force`**
```sh
clw snapshot --name myapp-dev            # unchanged content → no-op ("already up to date", chunks_uploaded: 0)
clw snapshot --name myapp-dev --force    # content differs → replace (rm-then-write); without --force the server 409s
```

**Resume on another machine**
```sh
clw hydrate ~/workspace/myapp --name myapp-dev   # first hydrate warms ~/.clw/cache; later ones serve locally
cd ~/workspace/myapp && ...work... && clw snapshot --name myapp-dev
clw ls --name myapp-dev                          # inspect root / created_at / parent / files / bytes
```

**`status` as a scripted dirty-check**
```sh
if ! clw status --name myapp-dev; then echo "dirty — snapshot first"; exit 1; fi   # 0 clean / 1 dirty / 2 error
clw --json status --name myapp-dev | jq '.clean'                                   # {"clean":false,"added":[…],…}
```

**`run` — wrapping a test suite**
```sh
clw run --input src/ -- cargo test                                   # 1st = miss, 2nd unchanged = hit
clw run --input src/ --input tests/ -- cargo test --all-targets      # every --input path keys the cache
clw run --input src/ --env RUSTFLAGS --env CARGO_PROFILE -- cargo build --release   # seal env vars into the key
```
Only exit-0 runs are memoized; the cwd is part of the key; **an env var that affects output MUST be `--env`-declared**, and an ignored file (`.gitignore`/`.clwignore`) inside an input dir is invisible to the key — pass it as an explicit `--input <FILE>`.

**Auth management**
```sh
clw auth login          # persist resolved endpoint/tenant/token to ~/.clw/config.toml (0600)
clw auth whoami         # resolved identity, redacted, offline
clw auth logout         # drop just the token line (keep endpoint/tenant/cache_dir)
clw auth logout --all --yes   # clear the whole file
clw --token pat_newtoken auth login   # rotate today (re-authenticate)
```

**Lifecycle**
```sh
clw rm --name myapp-dev --yes           # delete a ref (idempotent; --json → {"deleted":true|false})
clw prune                               # dry-run GC preview; clw prune --yes to reclaim orphans
clw erase --yes --confirm-tenant my-tenant   # GDPR wipe (dry-run default without --yes)
clw uninstall --dry-run                 # preview footprint removal
```

**Completions**
```sh
source <(clw completions bash)                       # current shell
clw completions zsh   > ~/.zfunc/_clw
clw completions fish  > ~/.config/fish/completions/clw.fish
clw completions powershell >> $PROFILE
```

**Throttle / accessibility**
```sh
clw --concurrency 4 snapshot --name myapp-dev   # cap concurrent CAS requests (default 8)
clw --no-color status --name myapp-dev          # no ANSI (also NO_COLOR / non-TTY)
```

**Conformance suite (advanced / verifying a deployment)**
```sh
CLW_TENANT=my-tenant CLW_TOKEN=pat_xxxx cargo test -p clw-conformance --features live
```

**stdout/stderr contract:** query *results* (`whoami`, `doctor`, `ls`, every `--json` payload) go to **stdout**; action *notices* (login/logout/init/rotate confirmations, progress, advisories, warnings) go to **stderr** — so piping stdout into `jq` or a file never captures chatter.

---

<a id="18-outputs-schemas"></a>
## 18. Command outputs & `--json` schemas

`--json` is a **global flag before the subcommand** (`clw --json <cmd>`). Every command supports it **except `run`** (which proxies the child's streams).

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
An identical re-snapshot instead prints `already up to date — 'myapp-main' already points at this content` (`chunks_uploaded: 0`).
`--json`: `{ name, root, files, bytes_total, chunks_total, chunks_uploaded, unchanged(bool), skipped_external_symlinks:[string] }`

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
`--json --name`: `{ name, root, created_at_unix, parent:(hex|null), files, bytes }`  ·  `--json --all`: `{ tenant, count, refs:[{name,root,created_at_unix,parent}], run_cache_entries, skipped, errors:[{ref_key,error}] }`

### run (no `--json`)
```
[clw] cache miss       # then the child's stdout/stderr verbatim, then exits the child's code
[clw] cache hit        # replays cached stdout/stderr/exit without re-running
```

### doctor
```
clw doctor — diagnostics (redacted)
===================================
clw version:    0.1.11
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
`--json`: login `{logged_in,wrote,endpoint,tenant,token}` · whoami `{endpoint,tenant,token}` · logout `{removed:[string],file}` · rotate `{rotated:false,action_required,note}`

### prune / erase / rm / init
```
# prune dry-run:   DRY RUN — would delete 3 orphaned blob(s) (128 live). Re-run with --yes to delete: …
# prune --yes:     pruned 3 orphaned blob(s) (128 live)
# erase dry-run:   DRY RUN — would IRREVERSIBLY erase tenant 'my-tenant': 12 ref(s) + 480 blob(s). …
# erase success:   erased tenant 'my-tenant': 12 ref(s) + 480 blob(s) deleted; tenant verified empty
# rm:              deleted ref 'myapp-dev'   |   no such ref 'x' (nothing to delete)
# init (stderr):   wrote config to /Users/you/.clw/config.toml
```
`--json`: prune `{dry_run,live,orphans?,would_delete?[],deleted?}` · erase `{dry_run,tenant,refs|refs_deleted,blobs|blobs_deleted,complete?}` · rm `{name|ref_key,deleted}` · init `{wrote,token}`. On a server capability regression (405/501) the JSON is `{error:"capability-unavailable",note}` + a non-zero exit.

---

<a id="19-manifest-spec"></a>
## 19. The manifest format (wire spec)

The manifest is the on-the-wire representation of a workspace (source of truth: `clw-types`; spec: `docs/SPEC-manifest.md`).

**Digest primitive.** Every content id is a **BLAKE3-256** digest — the 32-byte hash, on the wire as **exactly 64 lowercase hex chars**. Anything else is rejected.

**Root object** — serialized in declaration order:
```json
{ "version": 1, "total_size": <u64>, "entries": [ <Entry>, … ] }
```
`version` must equal `1`; `total_size` is the sum of all File sizes.

**Entries** — tagged by a `"kind"` discriminant (`"file"` | `"symlink"` | `"dir"`; unknown kinds rejected):
```json
{ "kind":"file",    "path":"<rel>", "mode":<u32>, "size":<u64>, "chunks":[<ChunkRef>,…] }
{ "kind":"symlink", "path":"<rel>", "target":"<string>" }
{ "kind":"dir",     "path":"<rel>" }          // ONLY for empty dirs; non-empty dirs are implied by file paths
```
`mode` = unix permission bits (`420`=`0o644`, `493`=`0o755`); `size` must equal Σ `ChunkRef.size`; the concatenation of chunks (in list order) equals the file content.

**ChunkRef:** `{ "digest":"<64-hex>", "size":<u64> }` — `size` must be `> 0` and `≤ 5 MiB`.

**Path rules** (all entries): relative (no leading `/`), `/`-separated, no `.`/`..` component, valid UTF-8, non-empty.

**Canonical form + digest.** Canonical bytes = compact JSON (no whitespace), fields in declaration order, **`entries` sorted ascending by path (byte order), all paths unique**. `manifest_digest = BLAKE3(canonical_bytes)`. Writers sort+dedup before hashing; readers validate order + uniqueness on parse.

**Chunking (frozen):** FastCDC min 256 KiB / avg 1 MiB / max 4 MiB. Reconstruction is raw bytes — no padding, framing, or compression.

**RefRecord** (the AC value at a ref key) — declaration order:
```json
{ "name":"<name>", "root":"<64-hex>", "manifest_size":<u64>,
  "created_at_unix":<u64>, "parent":"<64-hex>|null", "tool_version":"<string>" }
```
`ref_key(name) = BLAKE3("clw/ref/v1/" ‖ name)` (runner keyspace: `"clw/ref/runner/v1/"`). Concurrency is **last-write-wins** in v1 (the `parent` field is lineage only; compare-and-swap is a future MAY).

**Transport.** CAS `GET|PUT|DELETE /v1/cas/{tenant}/{hash}` + `GET /v1/cas/{tenant}`; AC `GET|PUT|DELETE /v1/ac/{tenant}/{key}` + `GET /v1/ac/{tenant}`. Auth `Authorization: Bearer <PAT>`. Clients **must** verify `BLAKE3(body)` on every CAS GET (mismatch → `Integrity` error); a CAS PUT body must not exceed 5 MiB (else the server returns 413 / the client returns `TooLarge` pre-send).

---

<a id="20-platform-support"></a>
## 20. Platform support

Five signed release targets, one binary each (macOS/Linux ship as bare binaries; Windows as a direct `.exe`). All targets are **gnu** — there is no musl build, and Windows is `-pc-windows-gnu` (not MSVC).

| OS | Arch | Target triple |
|---|---|---|
| macOS (Apple Silicon) | arm64 | `aarch64-apple-darwin` |
| macOS (Intel) | x86_64 | `x86_64-apple-darwin` |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` |
| Linux | arm64 | `aarch64-unknown-linux-gnu` |
| Windows | x86_64 | `x86_64-pc-windows-gnu` |

Each target is **built natively on its own runner** (the release pipeline runs a per-OS matrix — no cross-compilation). The workspace **compiles and its test suite runs on Windows** as well as unix (a dedicated Windows CI lane exercises the runtime).

**Platform caveats**
- **Windows — hydrated symlinks become copies.** No unprivileged symlinks on Windows, so a symlink entry materializes as a byte-for-byte copy (file → copy, dir → copied subtree). Content round-trips; the link relationship does not survive.
- **Path validation is enforced on every platform** — backslash / drive-letter / UNC / NTFS-ADS-colon forms are rejected everywhere, not just on Windows.
- **macOS Gatekeeper quarantine** affects only a raw browser/curl download (the binary isn't Apple-notarized yet); `brew` and `install.sh` strip it. Windows Authenticode isn't live yet — verify via minisign.

---

<a id="21-compliance-legal"></a>
## 21. Compliance, privacy & legal

> Customer-relevant summary. The binding documents are `EULA.md`, `docs/PRIVACY-POLICY.md`, `docs/DPA-subprocessors.md`, `docs/EXPORT-CONTROL.md`, `docs/DR-business-continuity.md`, and `SECURITY.md`. All are **operative for the open beta**, pending full external-counsel review before GA.

**Entity & law.** Licensor / controller / processor is **Human Guardrail LTDA** (Brazil, CNPJ 66.103.400/0001-30). Governing law is Brazil (courts of São Paulo); SCCs may override for restricted transfers. Frameworks addressed: EU/UK GDPR, CCPA/CPRA, Brazil LGPD.

**What data is stored.** As *processor* of your Customer Content: file **contents** (chunked + BLAKE3-addressed in CAS), **file paths / tree structure** (in manifests), **ref names**. clw uploads **only what you explicitly snapshot** — no filesystem scan. As *controller* of Account/Operational data: the PAT + tenant identity, request metadata (bearer PAT, tenant-in-path, **source IP**, timestamp, `User-Agent`), and server operational/security logs. **No client-side encryption** of content before upload — confidentiality in transit is TLS. **No telemetry / no phone-home** — the client contacts only the configured API. *Paths are metadata that may be sensitive — don't snapshot secrets/PII.*

**Subprocessors & residency.** The **sole infrastructure subprocessor is Cloudflare, Inc.** (R2 object storage for CAS + Durable Objects for the write path). No other cloud, no payment/CRM/email/crash-reporting subprocessor at v1. **Residency today is the United States only** (Cloudflare `enam`). Customer-selectable regional residency (incl. EEA/UK in-region) is on the roadmap but **not yet available**; a sign-up resolving to a non-US region is **refused at provisioning** (fail-closed), not silently stored in the US. EEA/UK/Swiss→US transfers rely on the **EU SCCs + UK IDTA**; 30 days' advance notice of any new subprocessor.

**Retention.** Customer Content: life of tenant, then a 30-day export window + deletion within 30 days. Account data: +30 days. Server/IP/request logs: **90 days**. Billing records: 5 years. Backup deletion propagates within 35 days.

**Right to erasure (GDPR Art.17 / LGPD).** Self-service **today** via `clw erase` (irreversibly deletes every ref + every CAS blob for the tenant; dry-run default + `--yes` + typed-tenant confirmation; verified empty). `clw rm` removes one ref; `clw prune` GCs orphaned blobs. Operator-assisted alternative: privacy@humangr.com → deletion within 30 days. Erased data is not resurrected from backups. `clw uninstall` clears the **local** footprint (`~/.clw`: config + PAT + cache).

**Data-subject rights & contacts.** Access / rectification / erasure / restriction / portability / objection (GDPR & LGPD); CCPA/CPRA know/delete/correct/opt-out — HumanGR **does not sell or share** PI. Response windows: GDPR/UK ~1 month, CCPA 45 days, LGPD ~15 days. Contacts: **privacy@ / security@ / support@ / legal@ / sales@humangr.com** (DPO/Encarregado: Gustavo Schneiter).

**Breach notification.** Processor→customer within **72 hours** of awareness (Art. 33-style content); controller→SA within 72 hours where required.

**Export control.** Crypto = standard TLS (rustls) + BLAKE3 hashing; no proprietary crypto, no client-side at-rest encryption. Self-classified **ECCN 5D992.c** (mass-market); no License Exception ENC / CCATS asserted (subject to counsel). Prohibited destinations (comprehensive embargoes as of 2026-06): Cuba, Iran, North Korea, Syria, and the Crimea/Donetsk/Luhansk/Zaporizhzhia/Kherson regions of Ukraine; prohibited parties (OFAC SDN, BIS Entity/Denied, EU/UK/UN analogues) and WMD/military end-uses. Each use carries user reps (not in an embargoed destination, not a restricted party, no prohibited re-export). Automated screening is still being built out (contractual reps + manual review in the interim).

**DR / business continuity (internal objectives, not SLAs unless separately agreed).** RTO/RPO targets 24h across the failure classes (regional outage, CAS data loss, auth-metadata loss, signing-key compromise, download-surface outage). CAS + auth/metadata backed up daily, retained 35 days, encrypted, restore-tested, BLAKE3-verified on restore. **Disclosed concentration risk:** a single critical vendor (Cloudflare) with no hot alternate-provider failover — an explicitly accepted residual risk, mitigated by Cloudflare regional failover + R2 eleven-nines durability. No backup SLA at v1; `~/.clw/cache` is ephemeral and not backed up.

**Security disclosure.** Report privately to **security@humangr.com** (not a public issue). Include `clw --version` + OS/arch, impact, minimal repro, and logs with the PAT + contents redacted. SLAs: ack 2 business days, triage 5, HIGH/CRITICAL fix plan 15. Pre-1.0, only the latest release gets security fixes.

**EULA (use terms).** Licensed, not sold; proprietary + trade secret; binary-only. Non-exclusive, non-transferable, revocable license for **internal business use** on Licensee-controlled devices. No redistribution/hosting/service-bureau; **no reverse engineering** (incl. of the wire/cache formats) except where mandatory law permits after an interoperability request. "AS IS", no warranty; liability cap = fees paid in the prior 12 months (or **USD $100** for no-charge/beta use); the DPA adds a separate data-protection super-cap (2× 12-month fees) + a data-protection indemnity. Termination is automatic on breach and does not by itself delete Service-stored data — use `clw erase`.

---

<a id="22-roadmap-status"></a>
## 22. Roadmap & status

**Status: SHIPPED — open beta.** v0.1.0 launched 2026-06-18; **current release v0.1.11**. Pre-1.0 there is no stability guarantee *except* the frozen v0.1.0 CLI contract (verbs, flag names, argument order, the `125` reserved run code, child-passthrough + cache-iff-0). A SemVer/stability charter + formal EOL policy land with 1.0.

**Recently shipped:** content-addressed hydrate by digest (`--manifest-digest`, v0.1.5); the full ref lifecycle (`rm` / `ls --all` / `prune` / `erase`); GDPR Art.17 self-service erasure; the cross-platform Windows/Linux port (incl. the Windows symlink copy-fallback) + a Windows-runtime CI lane; public distribution (Homebrew tap + public mirror + `install.sh`); minisign-signed checksums + CycloneDX SBOM; the runner ref-domain keyspace; telemetry fully removed (never phones home).

**Planned (committed, not yet in the binary):**
- One-step `auth rotate` (mint-and-revoke) — today rotation = re-run `clw auth login` with the new token; a one-step command can't ship on end-user machines without leaking the internal mint key.
- macOS Developer-ID signing + notarization; Windows Authenticode (Azure Trusted Signing) — until live, verify via minisign.
- Customer-selectable regional residency (incl. EEA/UK in-region) — US-only at launch.
- EU/UK Art. 27 representative appointment; external-counsel legal sign-off; SOC 2 / ISO 27001 reports — all before GA.
- Optimistic concurrency (compare-and-swap) for refs — v1 is last-write-wins.
- `clw run` child-env sandboxing (`env_clear` + allowlist) — v1 doesn't sandbox the child env.
- Billing / paid plans — no payment subprocessor at v1.

**Deferred (logged waivers):** SLSA build-provenance attestation (gated to public repos; the source repo is private); a distinct CAS-PUT created-vs-existed status (so `chunks_uploaded` is best-effort).

**On the manifest ceiling:** the manifest is a single 5 MiB CAS blob, so very large workspaces (~32k+ files, path-length dependent) hit a clean "too many entries" error today; the error notes multi-part manifests as a future server capability. Split the workspace for now.

---

*This guide is generated from the source of record (the `clw` crates + repo docs). Every command, flag, env var,
exit code, invariant, and policy above is drawn from the implementation and the binding documents.
Current version: **v0.1.11 — open beta**.*
