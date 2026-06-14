//! Criterion microbenchmarks for manifest canonicalization + digest.
//!
//! Every snapshot serializes its `Manifest` to canonical bytes and BLAKE3-hashes
//! them (`canonical_bytes` / `digest`), and that cost grows with the number of
//! entries. Workspaces with thousands of files are routine, so these benches
//! sweep synthetic manifests of 1k and 10k entries and measure both:
//!
//!   * `canonical_bytes` — the serde_json serialization (the bulk of the work),
//!   * `digest` — canonicalization followed by the BLAKE3 hash of the result.
//!
//! Run with: `cargo bench -p clw-types`
//! Compile-check only: `cargo bench -p clw-types --no-run`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use clw_types::{ChunkRef, Digest, Entry, Manifest, MANIFEST_VERSION};

/// Build a manifest with `n` `File` entries, each with a couple of chunks.
/// Paths are zero-padded so the generated order is already the byte-sorted
/// order `canonical_bytes`'s invariant requires (no duplicates).
fn synthetic_manifest(n: usize) -> Manifest {
    let mut entries = Vec::with_capacity(n);
    let mut total_size: u64 = 0;
    for i in 0..n {
        // Deterministic per-entry digests so runs are comparable.
        let mut a = [0u8; 32];
        a[..8].copy_from_slice(&(i as u64).to_le_bytes());
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&((i as u64) ^ 0xFFFF_FFFF).to_le_bytes());

        let size = 1024 + (i as u64 % 4096);
        total_size += size;
        entries.push(Entry::File {
            path: format!("dir{:03}/file{:08}.bin", i % 256, i),
            mode: 0o644,
            size,
            chunks: vec![
                ChunkRef {
                    digest: Digest(a),
                    size: size / 2,
                },
                ChunkRef {
                    digest: Digest(b),
                    size: size - size / 2,
                },
            ],
        });
    }
    // Enforce the sorted, duplicate-free invariant regardless of generation.
    entries.sort_by(|x, y| x.path().cmp(y.path()));

    Manifest {
        version: MANIFEST_VERSION,
        total_size,
        entries,
    }
}

fn bench_manifest(c: &mut Criterion) {
    let mut group = c.benchmark_group("manifest");

    for &n in &[1_000usize, 10_000usize] {
        let manifest = synthetic_manifest(n);
        // Use the serialized length as the throughput denominator so the
        // numbers reflect bytes processed, not just entry count.
        let bytes = manifest
            .canonical_bytes()
            .expect("synthetic manifest serializes")
            .len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::new("canonical_bytes", n), &manifest, |b, m| {
            b.iter(|| black_box(m).canonical_bytes().unwrap())
        });
        group.bench_with_input(BenchmarkId::new("digest", n), &manifest, |b, m| {
            b.iter(|| black_box(m).digest().unwrap())
        });
    }

    group.finish();
}

criterion_group!(benches, bench_manifest);
criterion_main!(benches);
