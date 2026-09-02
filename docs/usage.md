# clw Usage Recipes

Deeper recipes for daily use, scripting, and conformance testing.
For installation and the 5-minute quickstart, see [README.md](../README.md).

---

## Daily snapshot of a project

Snapshot the current directory at the end of a work session so it can be resumed
anywhere:

```sh
# From inside the project directory:
clw snapshot --name myapp-$(git rev-parse --short HEAD)
```

Use a stable name (no SHA) if you want a rolling "latest" ref that any machine
can hydrate without knowing the current commit:

```sh
clw snapshot --name myapp-dev
```

Each snapshot records the previous root digest as `parent`, so the lineage is
always visible via `clw ls`.

### Re-snapshotting a name: identical content vs. `--force`

The ref store is **create-only** (in-place versioning isn't live yet), so the two
re-snapshot cases differ:

- **Identical content → no-op success.** Re-snapshotting a name that already
  points at the *same* content does nothing and succeeds (exit 0). It prints
  `already up to date` rather than `snapshot complete`, and `chunks_uploaded` is
  `0` (unless the server had lost/pruned blobs that were re-uploaded). No `--force`
  is needed.

  ```sh
  clw snapshot --name myapp-dev
  # already up to date — 'myapp-dev' already points at this content (no change)
  #   name:            myapp-dev
  #   root:            af1349b9f5f9...
  #   files:           312
  #   bytes_total:     41943040
  #   chunks_uploaded: 0
  ```

- **Different content → needs `--force`.** Re-snapshotting a name to *different*
  content is rejected by the server (HTTP 409) unless you pass `--force`, which
  does an rm-then-write (delete the old ref, then write the new one).

  ```sh
  clw snapshot --name myapp-dev --force
  # snapshot complete
  #   name:            myapp-dev
  #   ...
  ```

  Without `--force`, clw surfaces the actionable message naming the `--force` flag
  (or the `clw rm --name <ws> --yes` then re-snapshot workaround).

---

## Resume on another machine

### Step 1: hydrate

```sh
clw hydrate ~/workspace/myapp --name myapp-dev
# hydrate complete
#   name:              myapp-dev
#   root:              af1349b9f5f9...
#   files:             312
#   bytes_total:       41943040
#   bytes_from_cache:  0
#   bytes_downloaded:  41943040
```

The local cache (`~/.clw/cache` by default, overridable with `--cache-dir` /
`CLW_CACHE_DIR`) is populated on first hydration. Subsequent hydrations of the
same content are faster because chunks are served locally.

### Step 2: work, then snapshot back

```sh
cd ~/workspace/myapp
# ... do work ...
clw snapshot --name myapp-dev
```

### Step 3: inspect ref metadata

```sh
clw ls --name myapp-dev
# name:         myapp-dev
# root:         af1349b9f5f9...
# created_at:   2025-06-04 00:00:00 UTC
# parent:       c3d7e2a1b4f8...
# files:        312
# bytes_total:  41943040
```

The human-readable `created_at` is a UTC datetime; the raw epoch is preserved as
`created_at_unix` in the `--json` form (and `ls --all --json` adds a
`run_cache_entries` count of memoized `clw run` records).

---

## `clw status` as a dirty-check in scripts

`clw status` exits **0** when the local directory matches the remote ref exactly,
and **1** when there is any diff (added, removed, or changed files). Exit **2**
means an error (network, auth, etc.).

```sh
if ! clw status --name myapp-dev; then
  echo "workspace is dirty — run 'clw snapshot --name myapp-dev' before continuing"
  exit 1
fi
```

Human-readable output (default):

```
clean — workspace matches remote ref 'myapp-dev'
```

Or, when dirty:

```
added:
  + new-file.rs
changed:
  ~ src/main.rs
removed:
  - old-file.rs
```

Machine-readable JSON (`--json` global flag):

```sh
clw --json status --name myapp-dev
# {"clean":false,"added":["new-file.rs"],"removed":["old-file.rs"],"changed":["src/main.rs"]}
```

Use `jq '.clean'` to extract the boolean in scripts.

---

## `clw run` — wrapping a test suite

### Basic: hash the source tree, run tests

```sh
clw run --input src/ -- cargo test
```

- First run: cache miss, executes `cargo test`, stores stdout/stderr in CAS.
- Second run (no changes to `src/`): cache hit, replays output instantly.

### Multiple inputs

```sh
clw run --input src/ --input tests/ -- cargo test --all-targets
```

All listed `--input` paths contribute to the cache key. A change in any of them
produces a miss.

### Sealing environment variables into the key

Some commands depend on env vars that are not reflected in the source tree.
List them with `--env`:

```sh
clw run --input src/ --env RUSTFLAGS --env CARGO_PROFILE -- cargo build --release
```

Only the variables listed with `--env` are included in the key. Unlisted variables
are ignored — they do not affect the cache key.

> **Caller responsibility — undeclared env vars.** The wrapped command inherits
> the full ambient environment; `clw` does **not** sandbox it in v0.1. So if your
> command reads an environment variable that changes its output, you **must** list
> it with `--env` — otherwise that variable is invisible to the cache key and a run
> can replay a **stale** result. Declaring it is on you; clw cannot infer it.
> (Sandboxing the child env to an explicit allowlist is a tracked post-beta
> hardening, intentionally out of scope for v0.1.)

### Working directory is part of the key

The cache key also folds in the (canonicalized, absolute) **working directory** the
command runs in. Two runs with identical `--input` paths, command, and `--env` but
in **different directories** therefore get **different** keys — one workspace can
never replay another's cached result.

### Exit code propagation

`clw run` exits with the child's exit code, making it drop-in safe in any
pipeline. Only successful runs (exit 0) are memoized; failed runs are never
cached and always re-execute.

### Caveat: ignored files are invisible to the cache key

When an `--input` is a directory, `clw run` walks it the same way `snapshot`
does — **respecting `.gitignore` and `.clwignore`**. A file inside an input
directory that is ignored by those rules is therefore invisible to BOTH the
snapshot and the cache key. If your command depends on such a file (a local
`.env`, a generated config, a secret excluded from version control), changing it
will **not** invalidate the cache, and a run can replay a stale result.

To make an ignored file part of the key, either pass it as an explicit
`--input <FILE>` (a directly-named file is digested regardless of ignore rules)
or un-ignore it. This is by design — `clw` respects ignore files so build
artifacts and caches do not pollute the key — but it is a sharp edge worth
knowing.

---

## Conformance suite

The `clw-conformance` crate exercises the live CoreLink API surfaces directly.
It is gated behind the `live` feature flag and requires a valid tenant and token.

```sh
CLW_TENANT=my-tenant \
CLW_TOKEN=pat_xxxxxxxxxxxxxxxx \
  cargo test -p clw-conformance --features live
```

The `live` feature is never enabled in the default CI lane. Run it manually
before a release or when verifying a new CoreLink deployment.

---

## `clw auth` — managing the stored identity

`clw auth` manages the locally-stored CoreLink identity in `~/.clw/config.toml`.
It is **fully offline** — no subcommand makes a network call. The personal access
token is never printed; its presence is reported as `<set>` / `<unset>`.

```sh
# Persist the resolved endpoint/tenant/token to ~/.clw/config.toml (forced 0600):
clw auth login
# logged in — wrote identity to /Users/you/.clw/config.toml
#   endpoint: https://corelink-api.humangr.com
#   tenant:   my-tenant
#   token:    <set>

# Print the resolved identity (redacted, offline):
clw auth whoami
# endpoint: https://corelink-api.humangr.com
# tenant:   my-tenant
# token:    <set>

# Remove just the token line (keeps endpoint/tenant/cache_dir):
clw auth logout

# Clear the entire config file (confirm prompt unless --yes):
clw auth logout --all
clw auth logout --all --yes
```

`clw auth login` resolves the identity through the usual precedence (CLI flag >
env > config > default), so `clw --tenant t --token pat_… auth login` persists
those values. `whoami` works with `--json` for scripting.

**Rotating your token (v1).** In v1 you rotate by **re-authenticating**: run
`clw auth login` with the new token to atomically replace the stored credential.
`clw auth rotate` exists only to point you at that supported path (it exits 0 and
changes nothing on its own); a dedicated one-step rotate (mint-new + revoke-old)
is a post-launch nicety that needs a server endpoint.

```sh
clw auth rotate
# clw auth rotate: to rotate your token, run `clw auth login` with the new
# token — this replaces the stored credential. (v1 has no one-step rotate;
# a dedicated mint-and-revoke command is planned post-launch.)

# Rotate today:
clw --token pat_newtoken auth login
```

---

## `clw doctor` — redacted diagnostics for support

`clw doctor` prints a support-ready diagnostics bundle: clw version, os/arch, the
config file path / existence / mode, the resolved endpoint, tenant + token
**presence** (never the token value), and the cache dir. It always exits 0 and is
safe to paste into a bug report. Use `--json` for a machine-readable form.

```sh
clw doctor
# clw doctor — diagnostics (redacted)
# ===================================
# clw version:    0.1.0
# os/arch:        macos/aarch64
# config file:    /Users/you/.clw/config.toml
# config exists:  true
# config mode:    0o600
# endpoint:       https://corelink-api.humangr.com
# tenant:         <set>
# tenant value:   my-tenant
# token:          <set>
# cache dir:      /Users/you/.clw/cache
# cache exists:   true
```

For security issues, follow the disclosure process in `SECURITY.md` rather than
pasting diagnostics into a public issue.

---

## `clw rm` — deleting a workspace ref

`clw rm --name <ws>` performs a **real, idempotent** `DELETE /v1/ac` against the
live CoreLink server: it removes the named ref. Deleting a name that no longer
exists succeeds quietly (idempotent).

This is a destructive operation, so it is gated by a confirmation:

- By default it prompts `delete ref '<ws>'? this cannot be undone [y/N]` and
  proceeds only on `y`/`yes`.
- `--yes` skips the prompt.
- In non-interactive contexts (no TTY) and with `--json`, `clw rm` **refuses to
  delete without `--yes`** — it never silently deletes in a pipeline.

`clw rm` reports **honestly** whether the ref actually existed. Deleting a name
that exists prints `deleted ref '<ws>'`; deleting one that never existed still
succeeds (idempotent, exit 0) but prints `no such ref '<ws>' (nothing to delete)`.
Under `--json` this is `{"deleted":true}` vs `{"deleted":false}`.

```sh
clw rm --name myapp-dev
# delete ref 'myapp-dev'? this cannot be undone [y/N]: y
# deleted ref 'myapp-dev'

# Non-interactive / scripted:
clw rm --name myapp-dev --yes
# deleted ref 'myapp-dev'

# Removing a name that does not exist — still exit 0, but says so honestly:
clw rm --name never-existed --yes
# no such ref 'never-existed' (nothing to delete)

# JSON form reflects whether the ref actually existed:
clw --json rm --name myapp-dev --yes
# {"name":"myapp-dev","deleted":true}
clw --json rm --name never-existed --yes
# {"name":"never-existed","deleted":false}
```

`clw rm` removes **only the ref** (the named pointer in the AC); it does **not**
GC the underlying CAS blobs. To reclaim unreferenced blobs, use
[`clw prune`](#clw-prune--garbage-collecting-unreferenced-cas-blobs); to
irreversibly erase **all** of a tenant's data, use
[`clw erase`](#clw-erase--gdpr-full-tenant-erasure).

---

## `clw prune` — garbage-collecting unreferenced CAS blobs

`clw prune` runs a live **mark-and-sweep** garbage collection over the tenant's
CAS: it computes the live set (every manifest root reachable from a ref, plus
every chunk those manifests reference) and deletes CAS blobs outside it. It
removes blobs, never refs.

It is **dry-run by default** — without `--yes` it reports exactly what it *would*
delete and changes nothing. Pass `--yes` to actually delete.

```sh
# Preview (default) — deletes nothing:
clw prune
# DRY RUN — would delete 3 orphaned blob(s) (128 live). Re-run with --yes to delete:
#   1a2b3c…
#   4d5e6f…
#   7890ab…

# Actually delete the orphans:
clw prune --yes
# pruned 3 orphaned blob(s) (128 live)
```

As a safety floor, `prune --yes` **refuses** to delete when the live set is empty
but CAS is non-empty (that pattern usually means a transiently-empty/stale ref
index, not a genuinely orphaned tenant). Verify with `clw ls --all`; if the
tenant truly is empty, use `clw erase` (which requires typed-tenant confirmation).

---

## `clw erase` — GDPR full-tenant erasure

`clw erase` is the GDPR Art.17 "right to erasure" path: it irreversibly deletes
**every ref and every CAS blob** for the tenant. It is the single most destructive
command in the CLI, with three independent safety rails:

1. **Dry-run by default** — without `--yes` it only reports the blast radius
   (ref + blob counts) and changes nothing.
2. **`--yes`** — explicit intent to actually delete.
3. **Typed-tenant confirmation** — you must reproduce the exact tenant id, either
   at the interactive prompt or via `--confirm-tenant <TENANT>` for
   non-interactive / `--json` use. A mismatch (or a non-TTY without the flag)
   aborts and exits non-zero.

```sh
# Dry run (default) — reports the blast radius, deletes nothing:
clw erase
# DRY RUN — would IRREVERSIBLY erase tenant 'my-tenant': 12 ref(s) + 480 blob(s).
# Re-run with --yes and confirm the tenant id to proceed.

# Execute interactively (prompts for the tenant id):
clw erase --yes
# type the tenant id 'my-tenant' to IRREVERSIBLY erase it: my-tenant
# erased tenant 'my-tenant': 12 ref(s) + 480 blob(s) deleted

# Non-interactive / scripted (typed-tenant confirmation via flag):
clw erase --yes --confirm-tenant my-tenant
```

---

## `clw uninstall` — removing the local footprint

`clw uninstall` removes the local `clw` footprint: the `~/.clw` directory (the
config file, **including the stored PAT**) and the local cache. It prompts for
confirmation by default.

```sh
# Prompt before removing anything:
clw uninstall

# Skip the confirmation prompt (for scripts / non-interactive runs):
clw uninstall --yes

# Preview exactly what would be removed — deletes nothing:
clw uninstall --dry-run
```

The cache is removed along with the config: if you set a custom cache location
(`--cache-dir` / `CLW_CACHE_DIR`), that directory is the one removed.

`clw uninstall` **cannot delete its own running executable.** It prints the binary
path so you can remove it manually, e.g.:

```sh
rm ./target/release/clw
```

---

## `clw completions` — shell completions

`clw completions <shell>` prints a completion script to stdout for one of `bash`,
`zsh`, `fish`, `powershell`, or `elvish`. Redirect or source it per your shell's
convention.

```sh
# bash — source into the current shell (add to ~/.bashrc to persist):
source <(clw completions bash)

# zsh — write to a directory on your $fpath, then restart the shell:
clw completions zsh > ~/.zfunc/_clw

# fish:
clw completions fish > ~/.config/fish/completions/clw.fish

# powershell — append to your profile:
clw completions powershell >> $PROFILE

# elvish:
clw completions elvish > ~/.config/elvish/lib/clw-completions.elv
```

---

## Accessibility — color and `--json`

- **`--no-color`** disables ANSI color in human-readable output. Color is also
  auto-disabled when the `NO_COLOR` env var is set (https://no-color.org) or when
  stdout is not a TTY (a pipe, a file, or a `--json` consumer).
- **`--json`** emits machine-readable JSON on stdout for every command **except
  `clw run`**. `run` has no `--json` form by design: the wrapped command owns
  stdout, so clw must not interleave its own JSON there. Errors always go to
  stderr regardless of `--json`.

```sh
clw --no-color status --name myapp-dev
clw --json ls --name myapp-dev
```

---

## No telemetry

clw collects **no usage data** and ships with **no mechanism** to send any — there
is no telemetry sink, no opt-in flag, and no phone-home path in the binary. The
only data clw transmits is the workspace content you explicitly `snapshot`/`run`
and the auth token needed to reach the API. Nothing is reported about how you use
the tool.

---

## `--concurrency` — throttling CoreLink requests

`--concurrency` (default `8`; `0` is clamped to `1`) sets the per-invocation
ceiling for concurrent CoreLink requests in `snapshot`, `hydrate`, `prune`, and
`erase`. It is a CLI flag only: there is no `CLW_CONCURRENCY` environment variable
or config-file setting. Lower it if you are hitting rate limits; the client also
honors `Retry-After` and adapts concurrency downward (AIMD) on HTTP 429 on its own.

```sh
clw --concurrency 4 snapshot --name myapp-dev
clw --concurrency 4 hydrate ~/workspace/myapp --name myapp-dev
```

---

## Windows — hydrated symlinks become copies

Windows has no unprivileged symlinks, so when `clw hydrate` materializes a symlink
entry on Windows it **copies the target** instead of creating a link: a file
target is copied byte-for-byte, a directory target is copied as a recursive
subtree. Content round-trips bit-for-bit, but the link relationship does not
survive a Windows hydrate. This is a documented, non-silent divergence.

Path validation (rejecting backslash, drive-letter, and UNC traversal forms) is
enforced on **every** platform, not just Windows.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `error: authentication failed — check CLW_TOKEN or ~/.clw/config.toml` | Missing or invalid token | Verify `CLW_TOKEN` is set and the PAT is correct in your CoreLink dashboard (or run `clw auth whoami` to see the resolved identity) |
| `error: not found in CAS: <digest>` on `clw hydrate` | Wrong workspace name or wrong tenant | Check `--name` matches what was passed to `clw snapshot`; verify `CLW_TENANT` is the same tenant that created the snapshot |
| `error: transport error: config file already exists at ...; use --force to overwrite` | `clw init` run on a machine that already has a config | Run `clw init --force` to overwrite, or edit `~/.clw/config.toml` manually |
| `error: missing tenant: ...` | `CLW_TENANT` not set and not in config | Set `CLW_TENANT=<your-tenant>` (env or `--tenant`), then run `clw init` to persist it — `clw init` reads the resolved config, so the value must be supplied first |
| `error: missing token: ...` | `CLW_TOKEN` not set and not in config | Set `CLW_TOKEN=<pat>` (env or `--token`), then run `clw init` to persist it — `clw init` reads the resolved config, so the token must be supplied first |
| `clw hydrate` fails with a non-empty destination directory | Destination directory exists and is not empty | Choose a new (non-existent) path, or remove the existing directory first |
| `[clw] cache miss` on every run despite no source changes | An `--env` variable is fluctuating (e.g., `TMPDIR`, timestamps) | Audit `--env` flags and remove any variables that change between runs |


## Rate limits (free tier, as of 2026-06-05)

- **Storage:** 10 GB (enforced; exceeding → 429 + `Retry-After`).
- **Requests:** 1M/month on paper; enforcement currently deferred server-side.
- **Burst throttling:** parallel chunk uploads can be throttled at the CDN edge
  (HTTP 429). `clw` retries automatically with backoff (200ms → 10s ladder);
  long snapshots of many small files may simply take longer under throttle.
