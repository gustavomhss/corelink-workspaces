//! Criterion microbenchmarks for the local content-addressed cache hot path.
//!
//! [`LocalCache`] is L1 in front of CoreLink: every chunk fetched or stored on
//! the snapshot path goes through `put` (write to temp file + rename) and `get`
//! (read + BLAKE3 re-hash for self-healing). These benches measure the combined
//! put+get roundtrip over a few representative blob sizes:
//!
//!   * `4KiB`   — small-file / small-chunk case (rename + syscall overhead
//!     dominates; the per-op cost the cache pays on dedupe-friendly trees).
//!   * `256KiB` — a typical content-defined chunk.
//!   * `4MiB`   — large blob (the BLAKE3 re-hash on `get` dominates).
//!
//! Each blob is keyed by its own content digest so `put` actually writes (no
//! idempotent short-circuit) and `get` succeeds with a matching hash.
//!
//! Run with: `cargo bench -p clw-cache`
//! Compile-check only: `cargo bench -p clw-cache --no-run`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use clw_cache::LocalCache;
use clw_types::Digest;
use tempfile::TempDir;

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;

/// Deterministic pseudo-random bytes (xorshift64*) — no rand dependency, and
/// stable across runs so benchmark numbers are comparable. A `seed` lets each
/// size produce distinct content (hence distinct digests / blob paths).
fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut state: u64 = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let v = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn bench_cache_io(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_put_get");

    for &size in &[4 * KIB, 256 * KIB, 4 * MIB] {
        // A fresh cache per size so the put never short-circuits on a pre-existing
        // blob. The TempDir is held for the duration of the measurement loop.
        let dir = TempDir::new().expect("tempdir");
        let cache = LocalCache::new(dir.path()).expect("cache");
        let data = pseudo_random(size, size as u64);
        let digest = Digest::of_bytes(&data);

        let label = if size >= MIB {
            format!("{}MiB", size / MIB)
        } else {
            format!("{}KiB", size / KIB)
        };

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("roundtrip", label),
            &(digest, data),
            |b, (digest, data)| {
                b.iter(|| {
                    cache.put(black_box(digest), black_box(data)).unwrap();
                    let got = cache.get(black_box(digest)).unwrap();
                    black_box(got)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_cache_io);
criterion_main!(benches);
