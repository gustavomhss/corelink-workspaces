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

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `error: authentication failed (401) — check CLW_TOKEN / config PAT` | Missing or invalid token | Verify `CLW_TOKEN` is set and the PAT is correct in your CoreLink dashboard |
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
