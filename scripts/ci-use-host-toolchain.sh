#!/usr/bin/env bash
# ci-use-host-toolchain.sh — put the WORKSPACE-PINNED host toolchain on PATH.
#
# WHY THIS EXISTS (do not replace with `dtolnay/rust-toolchain` on self-hosted):
#
# The self-hosted Mac runners (`corelink-builder`) share ONE `$HOME`, so one
# `~/.rustup`. The toolchain action runs, per its own action.yml:
#
#     rustup toolchain install <tc> --profile minimal --no-self-update
#     rustup default <tc>
#
# Both write to that SHARED tree while other jobs are executing binaries out of
# it. Two distinct failure modes, and the second is worse than the first:
#
#   1. CRASH — `~/.cargo/bin/*` are symlinks to the single `rustup` shim, so a
#      concurrent install can make `rustc` unexecutable mid-build. Signature:
#      `could not execute process rustc … No such file or directory (os error 2)`.
#
#   2. SILENT REPRODUCIBILITY DRIFT — `rustup default` rewrites the MACHINE-GLOBAL
#      default. The workspace pins its channel in rust-toolchain.toml precisely so
#      the release is built on one exact toolchain; a job that defaults the host to
#      `stable` moves every OTHER concurrent job off the pin, with no error anywhere.
#
# This script provisions NOTHING. It reads the channel from rust-toolchain.toml
# (so the CI step and the pin can never disagree), points PATH at the pre-installed
# host toolchain, and FAILS LOUDLY if it or a requested target is absent — a missing
# toolchain must stop one job, never silently reshape the fleet. Mirrors the proven
# corelink-server release path (same `corelink-builder` fleet, same 1.91.1 pin).
#
# Usage:  bash scripts/ci-use-host-toolchain.sh [target-triple …]
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

CHANNEL="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml | head -1)"
if [ -z "${CHANNEL:-}" ]; then
    echo "::error::ci-use-host-toolchain: could not parse [toolchain] channel from rust-toolchain.toml." >&2
    exit 2
fi

HOST_TRIPLE="${HOST_TRIPLE:-x86_64-apple-darwin}"
TC="$HOME/.rustup/toolchains/${CHANNEL}-${HOST_TRIPLE}"

if [ ! -x "$TC/bin/cargo" ] || [ ! -x "$TC/bin/rustc" ]; then
    echo "::error::ci-use-host-toolchain: the workspace-pinned toolchain ${CHANNEL}-${HOST_TRIPLE} is not installed on this runner host (looked in $TC)." >&2
    echo "::error::Install it ONCE on the host, while no CI is running: rustup toolchain install ${CHANNEL}" >&2
    echo "::error::Do NOT add a toolchain-provisioning action to a self-hosted job — that is the shared-\$HOME hazard this script exists to prevent." >&2
    exit 1
fi

MISSING=""
for t in "$@"; do
    [ -d "$TC/lib/rustlib/$t" ] || MISSING="${MISSING} $t"
done
if [ -n "${MISSING# }" ]; then
    echo "::error::ci-use-host-toolchain: ${CHANNEL} is missing target(s):${MISSING}" >&2
    echo "::error::Add them ONCE on the host, while no CI is running:  rustup target add --toolchain ${CHANNEL}${MISSING}" >&2
    exit 1
fi

echo "$TC/bin" >> "$GITHUB_PATH"
echo "ci-use-host-toolchain: PATH -> $TC/bin (channel ${CHANNEL} from rust-toolchain.toml)"
[ $# -gt 0 ] && echo "ci-use-host-toolchain: verified target(s): $*"
"$TC/bin/rustc" --version
