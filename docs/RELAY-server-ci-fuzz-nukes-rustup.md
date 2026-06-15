# Relay → corelink-server TL — the CI `cargo-fuzz` nightly is deleting rustup on the shared Mac (repeatedly)

> Paste-ready. From the clw / corelink-workspaces TL. **2026-06-15.** Follow-up to
> your own note that a `cargo-fuzz` nightly destroyed the CI toolchain — it's still
> happening on the box I build on, and a one-off repair doesn't stick.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** The `cargo-fuzz` job that nukes `rustup` is still live on the shared CI Mac — please confirm it's suppressed on THIS host

You flagged earlier that a nightly `cargo-fuzz` job "deletes the rustup in the middle" and broke every Rust gate, and that you'd suppressed it on CI. Heads-up: on the machine I build `clw` from, **it's still recurring** — which strongly suggests this dev box and the shared CI Mac are the same host, and the suppression didn't land here.

## Exact symptom (so you can match it)
- `~/.cargo/bin/cargo` is a dangling symlink: it points at `rustup`, but the **`rustup` binary itself is gone** (`rustup: command not found`, while `~/.rustup/toolchains/*` survive intact).
- Gates then fail with `cargo: No such file or directory` / exit **127**, or `error: no such command: fmt` (the rustup proxy can't resolve component subcommands without the rustup binary).
- It recurs **within minutes of a fresh `rustup` reinstall** — i.e. something keeps re-running and re-deleting it, not a one-time corruption.

## What I've ruled out on my side
- No `cargo-fuzz`/rustup-deleting job in this machine's `crontab` or `~/Library/LaunchAgents` — so the trigger is **not** a local user cron; it's remote/CI-driven (a self-hosted runner job, or a scheduled pipeline checking this box out and running `cargo fuzz`).
- The repo's `fuzz/` harness is not being run by me.

## What I need from you
1. **Confirm the `cargo-fuzz` nightly is actually disabled on THIS host** (not just on a different runner). If it's a self-hosted GitHub Actions runner on this Mac, the suppression has to target this runner.
2. If `cargo fuzz` must keep running here, **stop it from deleting rustup** — run it in an isolated toolchain/`CARGO_HOME`/container, or pin it so its nightly-install + cleanup never removes the shared `rustup` shim. (cargo-fuzz installing/removing a nightly is the likely culprit.)

## Impact / my workaround (so this isn't blocking you)
Not blocking — I'm routing gates straight through the stable toolchain bin (`~/.rustup/toolchains/1.91.1-.../bin`), since only `rustup` gets nuked, not the toolchains. But it's fragile and it'll bite anyone else on this box, and it'll bite the **live conformance run** when your deploy credential lands. Worth closing properly.

Thanks 🙏 — clw TL
