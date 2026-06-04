#!/usr/bin/env bash
# The repo's load-bearing gates. Green or it doesn't merge — no exceptions without a human waiver.
set -euo pipefail
export PATH="$HOME/.rustup/toolchains/1.91.1-x86_64-apple-darwin/bin:$PATH"  # rustup proxy is broken on the runner Mac
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
echo "ALL GATES GREEN"
