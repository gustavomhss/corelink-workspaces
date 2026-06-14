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
# created_at:   1748995200
# parent:       c3d7e2a1b4f8...
# files:        312
# bytes_total:  41943040
```

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

### Exit code propagation

`clw run` exits with the child's exit code, making it drop-in safe in any
pipeline. Only successful runs (exit 0) are memoized; failed runs are never
cached and always re-execute.

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

**Rotation is not yet supported.** `clw auth rotate` is a stub pending a server
token-rotation endpoint:

```sh
clw auth rotate
# clw auth rotate: not yet supported — tracked (server dependency).
# Use `clw auth login` with a new --token to replace the stored token.
```

To rotate today, run `clw auth login` with a fresh `--token`.

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

## `clw rm` — deleting a workspace ref (server-pending)

`clw rm --name <ws>` deletes a workspace ref (AC erasure). The client is shipped,
but it depends on a server `DELETE /v1/ac` endpoint that is **not yet deployed**.
Until that endpoint ships, `clw rm` is **optimistic**: it prints a clear "not yet
deployed" message and does **not** touch live data.

```sh
clw rm --name myapp-dev
# clw rm: ref deletion is not yet deployed — tracked (server dependency).
```

`clw rm` only removes the **ref** (the named pointer in the AC); it does not GC the
underlying CAS blobs. CAS blob garbage-collection / right-to-erasure is a separate,
also-unshipped track. When the server endpoint lands, this page will be updated
with the live success/confirmation behavior.

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

## Telemetry — off by default, with kill-switches

Telemetry is **off by default** and currently has **no network backend** — events
route to a documented no-op sink. It activates only on explicit opt-in. Resolution
precedence (first match wins):

1. **Kill-switches, always.** `DO_NOT_TRACK=1` or `CLW_TELEMETRY=0` force it OFF
   and cannot be overridden.
2. `CLW_TELEMETRY=1` opts in.
3. `telemetry = "on"|"off"` recorded in `~/.clw/config.toml`.
4. Otherwise OFF.

clw **never prompts** for consent in CI / non-interactive / non-TTY contexts. The
event type is a fixed enum carrying only a coarse subcommand/outcome/duration — by
construction it cannot carry a PAT, a path, a workspace name, or file contents.

```sh
# Force telemetry off regardless of config (cross-vendor standard):
export DO_NOT_TRACK=1
# or
export CLW_TELEMETRY=0
```

---

## `--concurrency` — throttling CAS requests

`--concurrency` (default `8`) caps the number of concurrent CAS requests for
`snapshot` and `hydrate`. Lower it if you are hitting rate limits; the client also
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
| `error: missing tenant: ...` | `CLW_TENANT` not set and not in config | Set `CLW_TENANT=<your-tenant>` or run `clw init` |
| `error: missing token: ...` | `CLW_TOKEN` not set and not in config | Set `CLW_TOKEN=<pat>` or run `clw init` |
| `clw hydrate` fails with a non-empty destination directory | Destination directory exists and is not empty | Choose a new (non-existent) path, or remove the existing directory first |
| `[clw] cache miss` on every run despite no source changes | An `--env` variable is fluctuating (e.g., `TMPDIR`, timestamps) | Audit `--env` flags and remove any variables that change between runs |


## Rate limits (free tier, as of 2026-06-05)

- **Storage:** 10 GB (enforced; exceeding → 429 + `Retry-After`).
- **Requests:** 1M/month on paper; enforcement currently deferred server-side.
- **Burst throttling:** parallel chunk uploads can be throttled at the CDN edge
  (HTTP 429). `clw` retries automatically with backoff (200ms → 10s ladder);
  long snapshots of many small files may simply take longer under throttle.
