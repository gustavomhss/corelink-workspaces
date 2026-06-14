# corelink-workspaces

**The workspace as a content-addressed object; compute as a disposable cursor.**

`clw` snapshots a directory tree into [CoreLink](https://github.com/humangr-labs/corelink-server)'s
content-addressed store (CAS + AC), hydrates it anywhere in seconds, and memoizes
deterministic commands so they never re-run when the inputs have not changed.

> **Phase 1 / pre-1.0.** The commands documented here (`init`, `snapshot`, `hydrate`,
> `status`, `run`, `ls`) are implemented and tested. No stability guarantee exists
> until 1.0.

---

## What clw does today

| Command      | What it does |
|--------------|--------------|
| `clw init`   | Writes `~/.clw/config.toml` from the resolved config |
| `clw snapshot` | Uploads a workspace directory to CoreLink CAS and records a named ref |
| `clw hydrate`  | Downloads a named ref and materializes the workspace into a directory |
| `clw status`   | Compares the local directory against the remote ref; exits 0 (clean) or 1 (dirty) |
| `clw run`      | Runs a command, memoizing stdout/stderr/exit-code in the AC |
| `clw ls`       | Shows ref metadata (root digest, file count, byte count) for a workspace |

Backed by the live CoreLink API at `https://corelink-api.humangr.com` (`/v1/cas`,
`/v1/ac`). Pure client — zero server changes required.

---

## Install

```sh
git clone https://github.com/humangr-labs/corelink-workspaces
cd corelink-workspaces

cargo build --release

# The binary lands at:
./target/release/clw --version
```

Toolchain pinned to `1.91.1` in `rust-toolchain.toml`, which also declares the
`rustfmt` and `clippy` components — rustup installs the pinned toolchain and its
components automatically on first build.

---

## 5-minute quickstart

### 1. Configure

```sh
export CLW_TENANT=my-tenant
export CLW_TOKEN=pat_xxxxxxxxxxxxxxxx

# Optional: persist to ~/.clw/config.toml
clw init
# wrote config to /Users/you/.clw/config.toml
```

### 2. Snapshot a workspace

```sh
cd ~/projects/myapp
clw snapshot --name myapp-main
# snapshot complete
#   name:            myapp-main
#   root:            af1349b9f5f9a1a6...  (64-char BLAKE3 hex)
#   files:           312
#   bytes_total:     41943040
#   chunks_total:    41
#   chunks_uploaded: 3
```

`chunks_uploaded` < `chunks_total` means deduplication is working — chunks
already in the CAS from a previous snapshot are skipped.

### 3. Hydrate on another machine

```sh
clw hydrate ~/workspace/myapp-copy --name myapp-main
# hydrate complete
#   name:              myapp-main
#   root:              af1349b9f5f9a1a6...
#   files:             312
#   bytes_total:       41943040
#   bytes_from_cache:  0
#   bytes_downloaded:  41943040
```

Subsequent hydrations of the same name populate the local cache at
`~/.clw/cache` so `bytes_from_cache` grows on warm runs.

### 4. Check for local drift

```sh
clw status --name myapp-main
# clean — workspace matches remote ref 'myapp-main'
echo $?   # 0

# After editing a file:
clw status --name myapp-main
# changed:
#   ~ src/main.rs
echo $?   # 1
```

### 5. Run a memoized command

```sh
clw run --input src/ -- cargo test
# [clw] cache miss     (first run — executes cargo test)

clw run --input src/ -- cargo test
# [clw] cache hit      (inputs unchanged — replays cached output instantly)
```

---

## Configuration

Resolution order: **CLI flag > environment variable > `~/.clw/config.toml` > default**.

`tenant` and `token` have no default and produce a friendly error (exit 2) if still
missing after all layers.

| Flag            | Environment variable | Config-file key | Default                              |
|-----------------|----------------------|-----------------|--------------------------------------|
| `--endpoint`    | `CLW_ENDPOINT`       | `endpoint`      | `https://corelink-api.humangr.com`   |
| `--tenant`      | `CLW_TENANT`         | `tenant`        | *(required)*                         |
| `--token`       | `CLW_TOKEN`          | `token`         | *(required)*                         |
| `--cache-dir`   | `CLW_CACHE_DIR`      | `cache_dir`     | `~/.clw/cache`                       |
| `--json`        | —                    | —               | `false`                              |

`~/.clw/config.toml` format (written by `clw init`):

```toml
# clw configuration
endpoint = "https://corelink-api.humangr.com"
tenant   = "my-tenant"
token    = "pat_xxxxxxxxxxxxxxxx"
cache_dir = "~/.clw/cache"
```

All flags are global and can appear before any subcommand.

---

## How memoization works (`clw run`)

1. `clw run` hashes every listed `--input` path (files read byte-for-byte;
   directories walked recursively) plus the command vector and any `--env` variable
   values into a single BLAKE3 key.
2. It looks up that key in the CoreLink AC. A hit replays the cached stdout/stderr
   and exit code without executing anything.
3. On a miss the command runs. If it exits 0, stdout and stderr are uploaded to the
   CAS and the AC key is set; non-zero exits are never cached.
4. The exit code of the original command (or the replayed exit code on a hit) is
   propagated as the process exit code.

---

## Documentation

- [docs/usage.md](docs/usage.md) — Daily recipes, scripting patterns, conformance suite
- [docs/SPEC-manifest.md](docs/SPEC-manifest.md) — Manifest format v1 (normative)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — Crate graph and dataflows

---

## Development

### Gates

```sh
bash scripts/gates.sh
```

Runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, and
`cargo test --workspace`. On pull requests, CI runs a scoped lane (only the crates
the diff touches) unless `clw-types`, `Cargo.toml/lock`, `scripts/`, or `.github/`
are touched — those escalate to the full lane. Docs-only diffs skip the Rust gates.

### DCO

Every commit requires a `Signed-off-by:` trailer:

```
Signed-off-by: Your Name <your@email.com>
```

`bash scripts/dco-check.sh` verifies all non-merge commits in `origin/main..HEAD`.

### Scoped CI lanes

See `scripts/gates.sh` for the exact escalation rules. The short version: touch
`crates/clw-types/` (the frozen contract) and you get a full-workspace gate.

---

## License

UNLICENSED — all rights reserved.
