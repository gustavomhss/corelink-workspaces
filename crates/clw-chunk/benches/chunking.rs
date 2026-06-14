//! Criterion microbenchmarks for the content-defined chunking hot path.
//!
//! `chunk_bytes` is on the snapshot critical path: every byte of every file
//! flows through FastCDC boundary detection + a BLAKE3 hash per chunk. These
//! benches measure throughput over representative inputs:
//!
//!   * `random/1MiB`, `random/16MiB` — incompressible, dedupe-unfriendly data
//!     (worst case: every chunk is unique, so the BLAKE3 cost is fully paid).
//!   * `repetitive/16MiB` — a small pattern tiled to size. Content-defined
//!     chunking cuts it into recurring boundaries, so it stands in for the
//!     dedupe-friendly case the tool is built to exploit.
//!
//! Run with: `cargo bench -p clw-chunk`
//! Compile-check only: `cargo bench -p clw-chunk --no-run`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use clw_chunk::chunk_bytes;

const MIB: usize = 1024 * 1024;

/// Deterministic pseudo-random bytes (xorshift64*) — no rand dependency, and
/// stable across runs so benchmark numbers are comparable.
fn pseudo_random(len: usize) -> Vec<u8> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
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

/// A repetitive buffer: a small pattern tiled to `len`. Stands in for the
/// dedupe-friendly content content-defined chunking is designed to exploit.
fn repetitive(len: usize) -> Vec<u8> {
    let pattern: Vec<u8> = (0u32..4096).map(|i| (i % 251) as u8).collect();
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let take = pattern.len().min(len - out.len());
        out.extend_from_slice(&pattern[..take]);
    }
    out
}

fn bench_chunking(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunk_bytes");

    for &size in &[MIB, 16 * MIB] {
        let data = pseudo_random(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("random", format!("{}MiB", size / MIB)),
            &data,
            |b, data| b.iter(|| chunk_bytes(black_box(data))),
        );
    }

    {
        let size = 16 * MIB;
        let data = repetitive(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("repetitive", format!("{}MiB", size / MIB)),
            &data,
            |b, data| b.iter(|| chunk_bytes(black_box(data))),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_chunking);
criterion_main!(benches);
