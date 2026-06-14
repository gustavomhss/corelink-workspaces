//! Criterion microbenchmark for the ignore-aware tree walk hot path.
//!
//! [`walk`] is on the snapshot critical path: every `clw` snapshot enumerates
//! the workspace through it (FastCDC chunking and manifest building both consume
//! its output). The walk applies `.gitignore`/`.clwignore` matching, stats every
//! entry, and sorts the result — cost that scales with the file count and the
//! directory fan-out of the tree.
//!
//! These benches build a synthetic tree in a tempdir and measure `walk` over it:
//!
//!   * `flat/1000`   — 1000 files in a single directory (worst case for the
//!     final sort and for one directory's read fan-out).
//!   * `nested/1000` — 1000 files spread across 100 subdirectories of 10 files
//!     each (representative of a real source tree's directory structure).
//!
//! Run with: `cargo bench -p clw-manifest`
//! Compile-check only: `cargo bench -p clw-manifest --no-run`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs;
use std::hint::black_box;

use clw_manifest::walk;
use tempfile::TempDir;

/// Build a flat tree of `n` files directly under `root`.
fn build_flat(root: &std::path::Path, n: usize) {
    for i in 0..n {
        fs::write(root.join(format!("file_{i:05}.txt")), b"corelink").unwrap();
    }
}

/// Build a nested tree of `n` files spread across `dirs` subdirectories.
fn build_nested(root: &std::path::Path, n: usize, dirs: usize) {
    let per_dir = n.div_ceil(dirs);
    let mut written = 0;
    for d in 0..dirs {
        let sub = root.join(format!("dir_{d:03}"));
        fs::create_dir_all(&sub).unwrap();
        for f in 0..per_dir {
            if written >= n {
                break;
            }
            fs::write(sub.join(format!("file_{f:03}.txt")), b"corelink").unwrap();
            written += 1;
        }
    }
}

fn bench_walk(c: &mut Criterion) {
    let mut group = c.benchmark_group("walk");

    const N: usize = 1000;

    let flat_dir = TempDir::new().expect("tempdir");
    build_flat(flat_dir.path(), N);
    group.bench_with_input(BenchmarkId::new("flat", N), &flat_dir, |b, dir| {
        b.iter(|| walk(black_box(dir.path())).unwrap())
    });

    let nested_dir = TempDir::new().expect("tempdir");
    build_nested(nested_dir.path(), N, 100);
    group.bench_with_input(BenchmarkId::new("nested", N), &nested_dir, |b, dir| {
        b.iter(|| walk(black_box(dir.path())).unwrap())
    });

    group.finish();
}

criterion_group!(benches, bench_walk);
criterion_main!(benches);
