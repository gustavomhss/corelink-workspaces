# clw-fuzz — fuzz harnesses for the untrusted-input parsers

These [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html) targets
exercise the **parse + validate** path of every value `clw-types` accepts from a
remote the client must NOT trust — the manifest and `RefRecord` come from a
remote CAS/AC. Each target's only assertion is *"arbitrary bytes never panic / never
hit UB"*: the parsers may only ever return `Ok` (a fully validated value) or
`Err`, never crash.

## Why this is a separate, out-of-workspace package

cargo-fuzz needs the **nightly** toolchain (it links libFuzzer and drives
`-Z build-std` / sanitizers). The main workspace is pinned to **stable 1.91.1**
(`rust-toolchain.toml`). To keep `bash scripts/gates.sh` (stable, whole-workspace)
green and free of the `libfuzzer-sys` dependency, this `fuzz/` package is **not**
a member of the root workspace (`members` is an explicit list). It depends on
`clw-types` by path only.

## Prerequisites

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Run

```sh
cargo +nightly fuzz run fuzz_manifest_from_bytes
cargo +nightly fuzz run fuzz_validate_rel_path
cargo +nightly fuzz run fuzz_validate_symlink_target
cargo +nightly fuzz run fuzz_digest_from_hex
```

A target runs until it finds a crash or you stop it (Ctrl-C). To bound a CI run,
add a time/iteration cap, e.g.:

```sh
cargo +nightly fuzz run fuzz_manifest_from_bytes -- -max_total_time=60
```

Any reproducer is written under `fuzz/artifacts/<target>/`; replay it with:

```sh
cargo +nightly fuzz run fuzz_manifest_from_bytes fuzz/artifacts/fuzz_manifest_from_bytes/<crash-file>
```

## Targets

| target | API under test | untrusted source |
| --- | --- | --- |
| `fuzz_manifest_from_bytes` | `Manifest::from_bytes` (JSON parse + path/symlink/drive-letter validation) | remote CAS manifest blob |
| `fuzz_validate_rel_path` | `validate_rel_path` | each manifest entry path |
| `fuzz_validate_symlink_target` | `validate_symlink_target` | symlink entry targets |
| `fuzz_digest_from_hex` | `Digest::from_hex` | manifest / RefRecord / URL digests |
