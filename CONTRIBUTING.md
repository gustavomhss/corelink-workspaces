# Contributing to `clw` (CoreLink Workspaces)

`clw` is a **proprietary, commercial** client of the CoreLink API. There is no
public repository; this guide is for authorized contributors. It is a **pure
client** — it never edits `corelink-server` (server work is coordinated through
the dependency ledger in `docs/ROADMAP-go-live.md`).

For reviewing a change, see **`REVIEWER-BRIEF.md`**.

---

## Toolchain

The toolchain is **pinned to Rust `1.91.1`** in `rust-toolchain.toml`, with
`rustfmt` and `clippy` declared as components, so a clean machine builds correctly:

```toml
[toolchain]
channel = "1.91.1"
components = ["rustfmt", "clippy"]
```

`rustup` selects it automatically — **no PATH hack, no manual toolchain juggling.**
(The old founder-Mac rustup hack was removed in M2.0; if you see one anywhere,
it is a bug.)

## Build

Plain `cargo`, nothing custom:

```bash
cargo build              # debug build of the workspace
cargo build --release    # release profile (strip + codegen-units=1)
cargo test --workspace   # the full test suite
```

Windows is a release target. If you touch any OS-sensitive code (file perms,
symlinks, home-dir resolution, the `run` shell, path joins), prove it still
compiles for Windows:

```bash
cargo check --workspace --target x86_64-pc-windows-gnu
```

## Run the gate before every commit

`scripts/gates.sh` is the load-bearing gate — **green or it doesn't merge**:

```bash
bash scripts/gates.sh
```

It runs `cargo fmt --check`, `clippy -D warnings`, the workspace tests, and (on
the full lane) `cargo deny check` + `cargo audit`. The gate is **change-scoped**:
a PR diff is gated to the crates it touches, docs-only diffs skip the Rust gates,
and any touch to **`clw-types`** (the frozen contract), `Cargo.toml`/`Cargo.lock`,
`scripts/`, or `.github/` escalates back to the **full** workspace lane. `main`
is **always** full-lane.

## DCO sign-off (required)

Every non-merge commit must carry a `Signed-off-by:` trailer
([Developer Certificate of Origin](https://developercertificate.org/)):

```bash
git commit -s -m "fix(clw-cli): ..."
```

`-s` adds `Signed-off-by: Your Name <you@example.com>` from your git identity.
CI enforces it via `scripts/dco-check.sh`; an unsigned commit fails the DCO gate.

## Commit messages — conventional commits

```
feat(scope): add a user-facing capability
fix(scope):  fix a bug
docs:        documentation only
chore:       tooling / housekeeping
```

Use the crate or area as the `scope` (`clw-cli`, `clw-client`, `clw-hydrate`, …).
Keep the subject imperative and scoped to one logical change. Contract changes to
`clw-types` ship in their **own** commit, separate from behavior changes, with a
decision record.

## Never `--no-verify`

Do **not** bypass commit/push hooks with `--no-verify`. The hooks (commit-msg
lint, DCO, the gate) are the contract. If a hook is wrong, fix the hook — don't
skip it. A skipped gate is a merge-then-rot risk, and `main` will catch it
full-lane anyway, more expensively.

## Parallel work — worktree-isolated lanes

Multiple changes proceed in parallel on **isolated git worktrees** (one branch
per worktree, off a shared baseline), so lanes never share a working tree and
stay conflict-disjoint. Each lane runs its own scoped gate and merges green
independently. Keep a lane's diff inside its declared scope; if two lanes must
touch the same file (e.g. the frozen `clw-types` contract), serialize them rather
than racing. Clean up the worktree when the lane merges.
