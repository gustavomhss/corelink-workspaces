//! Scale / stress E2E — "every molecule STRESSED".
//!
//! These push the real snapshot -> hydrate lifecycle to magnitude: a GB-scale file,
//! 100k files, a deep+wide tree, and high N-way concurrency against ONE store. They
//! assert the pipeline COMPLETES (no hang / no OOM within the runner+timeout) and stays
//! byte-CORRECT at scale — exercising the streaming path, the manifest-size caps, the
//! upload/hydrate concurrency, and store consistency under contention.
//!
//! `#[ignore]` by default (too slow for the per-PR gate); the weekly `stress.yml` lane
//! runs them with `--ignored` and AGGRESSIVE sizes via env. Defaults here are moderate
//! so a local `--ignored` run is feasible; the lane overrides:
//!   CLW_STRESS_BIG_BYTES (default 256 MiB; lane 2 GiB)
//!   CLW_STRESS_FILES     (default 10_000;  lane 100_000)
//!   CLW_STRESS_DEPTH     (default 100;     lane 500)
//!   CLW_STRESS_WIDE      (default 5_000;   lane 20_000)
//!   CLW_STRESS_CONC      (default 16;      lane 64)
//!
//! A disk-backed store (DiskTransport) holds blobs on disk, so the harness itself does
//! not cap the scale in RAM — what completes (or OOMs) is clw's own streaming client.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use clw_cache::LocalCache;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use tempfile::TempDir;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// Disk-backed CAS + AC store (scales to GB / 100k without holding it all in RAM).
// ---------------------------------------------------------------------------
struct DiskTransport {
    cas: PathBuf,
    ac: PathBuf,
}

impl DiskTransport {
    fn new(base: &Path) -> Self {
        let cas = base.join("cas");
        let ac = base.join("ac");
        std::fs::create_dir_all(&cas).unwrap();
        std::fs::create_dir_all(&ac).unwrap();
        Self { cas, ac }
    }
}

#[async_trait::async_trait]
impl CasTransport for DiskTransport {
    async fn exists(&self, d: &Digest) -> Result<bool> {
        Ok(self.cas.join(d.to_hex()).exists())
    }
    async fn get(&self, d: &Digest) -> Result<Vec<u8>> {
        std::fs::read(self.cas.join(d.to_hex())).map_err(|_| ClwError::NotFound(*d))
    }
    async fn put(&self, d: &Digest, bytes: Vec<u8>) -> Result<()> {
        std::fs::write(self.cas.join(d.to_hex()), bytes)?;
        Ok(())
    }
    async fn delete(&self, d: &Digest) -> Result<()> {
        let _ = std::fs::remove_file(self.cas.join(d.to_hex()));
        Ok(())
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(&self.cas)? {
            if let Some(s) = e?.file_name().to_str() {
                if let Ok(d) = Digest::from_hex(s) {
                    v.push(d);
                }
            }
        }
        Ok(v)
    }
}

#[async_trait::async_trait]
impl AcTransport for DiskTransport {
    async fn get(&self, k: &Digest) -> Result<Option<Vec<u8>>> {
        Ok(std::fs::read(self.ac.join(k.to_hex())).ok())
    }
    async fn put(&self, k: &Digest, value: Vec<u8>) -> Result<()> {
        std::fs::write(self.ac.join(k.to_hex()), value)?;
        Ok(())
    }
    async fn delete(&self, k: &Digest) -> Result<()> {
        let _ = std::fs::remove_file(self.ac.join(k.to_hex()));
        Ok(())
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(&self.ac)? {
            if let Some(s) = e?.file_name().to_str() {
                if let Ok(d) = Digest::from_hex(s) {
                    v.push(d);
                }
            }
        }
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write `len` pseudo-random (seeded, xorshift) bytes to `path` in blocks, WITHOUT
/// holding the whole file in RAM. Pseudo-random ⇒ no trivial dedupe, so a large file
/// produces many real chunk PUTs (exercises the streaming path).
fn write_random_file(path: &Path, len: u64, seed: u64) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut state = seed | 1;
    let mut remaining = len;
    let mut block = vec![0u8; 8 * 1024 * 1024];
    while remaining > 0 {
        let n = remaining.min(block.len() as u64) as usize;
        for b in block[..n].iter_mut() {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = (state & 0xff) as u8;
        }
        f.write_all(&block[..n]).unwrap();
        remaining -= n as u64;
    }
    f.flush().unwrap();
}

/// Stream-compare two files for byte-equality without loading either fully.
fn assert_files_identical(a: &Path, b: &Path) {
    let (fa, fb) = (
        std::fs::File::open(a).unwrap(),
        std::fs::File::open(b).unwrap(),
    );
    assert_eq!(
        fa.metadata().unwrap().len(),
        fb.metadata().unwrap().len(),
        "size mismatch between {a:?} and {b:?}"
    );
    let mut ra = std::io::BufReader::new(fa);
    let mut rb = std::io::BufReader::new(fb);
    let (mut ba, mut bb) = (vec![0u8; 1024 * 1024], vec![0u8; 1024 * 1024]);
    loop {
        let na = ra.read(&mut ba).unwrap();
        let nb = read_full(&mut rb, &mut bb[..na]);
        assert_eq!(na, nb, "length diverged mid-stream");
        if na == 0 {
            break;
        }
        assert_eq!(
            ba[..na],
            bb[..na],
            "byte mismatch in the round-tripped file"
        );
    }
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> usize {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]).unwrap() {
            0 => break,
            n => filled += n,
        }
    }
    filled
}

async fn snapshot_then_hydrate(store: &DiskTransport, src: &Path, out: &Path, ws: &str) {
    let cache1 = TempDir::new().unwrap();
    let lc1 = LocalCache::new(cache1.path()).unwrap();
    clw_snapshot::snapshot(src, store, &lc1, &clw_snapshot::SnapshotOptions::new(ws))
        .await
        .unwrap_or_else(|e| panic!("stress snapshot '{ws}' failed: {e}"));
    let cache2 = TempDir::new().unwrap();
    let lc2 = LocalCache::new(cache2.path()).unwrap();
    clw_hydrate::hydrate(out, store, &lc2, &clw_hydrate::HydrateOptions::new(ws))
        .await
        .unwrap_or_else(|e| panic!("stress hydrate '{ws}' failed: {e}"));
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// GB-scale single file must stream through snapshot -> hydrate byte-identical.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "stress: run via stress.yml or `--ignored`"]
async fn gb_file_roundtrips_byte_identical() {
    let big = env_usize("CLW_STRESS_BIG_BYTES", 256 * 1024 * 1024) as u64;
    let store_dir = TempDir::new().unwrap();
    let store = DiskTransport::new(store_dir.path());
    let src = TempDir::new().unwrap();
    let src_file = src.path().join("big.bin");
    write_random_file(&src_file, big, 0xB16_5EED);

    let dest = TempDir::new().unwrap();
    let out = dest.path().join("hydrated");
    snapshot_then_hydrate(&store, src.path(), &out, "stress-gb").await;

    assert_files_identical(&src_file, &out.join("big.bin"));
}

/// A huge FILE COUNT must snapshot -> hydrate with the full set intact.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "stress: run via stress.yml or `--ignored`"]
async fn many_files_roundtrip() {
    let n = env_usize("CLW_STRESS_FILES", 10_000);
    let store_dir = TempDir::new().unwrap();
    let store = DiskTransport::new(store_dir.path());
    let src = TempDir::new().unwrap();
    // Spread across subdirs (100k files in one dir is pathological on some FS).
    for i in 0..n {
        let sub = src.path().join(format!("d{:04}", i / 1000));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join(format!("f{i}.txt")),
            format!("content-{i}").as_bytes(),
        )
        .unwrap();
    }

    let dest = TempDir::new().unwrap();
    let out = dest.path().join("hydrated");
    snapshot_then_hydrate(&store, src.path(), &out, "stress-manyfiles").await;

    // Count matches + spot-check a sample end-to-end.
    let count = walk_count(&out);
    assert_eq!(count, n, "hydrated file count must equal the source count");
    for i in [0, n / 2, n - 1] {
        let p = out
            .join(format!("d{:04}", i / 1000))
            .join(format!("f{i}.txt"));
        assert_eq!(
            std::fs::read(&p).unwrap(),
            format!("content-{i}").into_bytes()
        );
    }
}

/// The OTHER side of the file-count boundary: a workspace with TOO MANY entries to fit
/// the single-blob manifest (`CAS_BLOB_CAP_BYTES = 5 MiB`; the gate is a byte cap, not a file count) must fail with a
/// CLEAN, actionable `InvalidManifest` — never a crash/hang/corruption. `many_files_*`
/// proves the supported max works; this proves overflowing it degrades gracefully.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "stress: run via stress.yml or `--ignored`"]
async fn too_many_files_over_manifest_cap_fails_cleanly() {
    let n = env_usize("CLW_STRESS_FILES_OVERCAP", 40_000); // ~162 B/entry × 40k ≈ 6.5 MiB > 5 MiB cap
    let store_dir = TempDir::new().unwrap();
    let store = DiskTransport::new(store_dir.path());
    let src = TempDir::new().unwrap();
    for i in 0..n {
        let sub = src.path().join(format!("d{:04}", i / 1000));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f{i}.txt")), b"x").unwrap();
    }
    let cache = TempDir::new().unwrap();
    let lc = LocalCache::new(cache.path()).unwrap();
    let res = clw_snapshot::snapshot(
        src.path(),
        &store,
        &lc,
        &clw_snapshot::SnapshotOptions::new("stress-overcap"),
    )
    .await;
    match res {
        Err(clw_types::ClwError::InvalidManifest(m)) => assert!(
            m.contains("manifest") && (m.contains("cap") || m.contains("too many")),
            "over-cap must be a clear, actionable manifest error, got: {m}"
        ),
        Err(other) => panic!("expected a clean InvalidManifest over-cap failure, got: {other:?}"),
        Ok(_) => panic!("a workspace over the single-blob manifest cap must FAIL, not succeed"),
    }
}

/// A deep + wide tree must round-trip (deep nesting + a wide fan-out dir).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "stress: run via stress.yml or `--ignored`"]
async fn deep_and_wide_tree_roundtrip() {
    let depth = env_usize("CLW_STRESS_DEPTH", 100);
    let wide = env_usize("CLW_STRESS_WIDE", 5_000);
    let store_dir = TempDir::new().unwrap();
    let store = DiskTransport::new(store_dir.path());
    let src = TempDir::new().unwrap();

    // Deep chain deep/a/a/.../a/leaf.txt
    let mut deep = src.path().join("deep");
    for _ in 0..depth {
        deep = deep.join("a");
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("leaf.txt"), b"bottom").unwrap();
    let rel_deep = deep.strip_prefix(src.path()).unwrap().to_path_buf();
    // Wide fan-out in one dir.
    let widedir = src.path().join("wide");
    std::fs::create_dir_all(&widedir).unwrap();
    for i in 0..wide {
        std::fs::write(widedir.join(format!("w{i}.txt")), format!("{i}").as_bytes()).unwrap();
    }

    let dest = TempDir::new().unwrap();
    let out = dest.path().join("hydrated");
    snapshot_then_hydrate(&store, src.path(), &out, "stress-deepwide").await;

    assert_eq!(
        std::fs::read(out.join(&rel_deep).join("leaf.txt")).unwrap(),
        b"bottom",
        "deep-nested leaf must round-trip"
    );
    assert_eq!(
        walk_count(&out.join("wide")),
        wide,
        "wide dir must round-trip fully"
    );
}

/// N concurrent snapshot+hydrate flows against ONE store: no corruption, no cross-talk.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "stress: run via stress.yml or `--ignored`"]
async fn nway_concurrent_no_corruption() {
    let conc = env_usize("CLW_STRESS_CONC", 16);
    let store_dir = TempDir::new().unwrap();
    let store_path = store_dir.path().to_path_buf();
    DiskTransport::new(&store_path); // create the shared store dirs once

    // Each flow OWNS a DiskTransport over the SAME on-disk store (shared state via the
    // filesystem). Run them concurrently via join_all (interleaved on the runtime — the
    // same shape as the existing 2-way `tokio::join!` concurrency test, and where a
    // store-consistency race would surface). spawn is avoided: its `Send` bound trips a
    // higher-ranked-lifetime limitation on the generic async_trait transport future.
    let flows = (0..conc).map(|k| {
        let store_path = store_path.clone();
        async move {
            let store = DiskTransport::new(&store_path);
            let src = TempDir::new().unwrap();
            let payload = format!("flow-{k}-payload").repeat(1000);
            std::fs::write(src.path().join("data.bin"), payload.as_bytes()).unwrap();
            let dest = TempDir::new().unwrap();
            let out = dest.path().join("hydrated");
            let ws = format!("stress-conc-{k}");
            let cache1 = TempDir::new().unwrap();
            let lc1 = LocalCache::new(cache1.path()).unwrap();
            clw_snapshot::snapshot(
                src.path(),
                &store,
                &lc1,
                &clw_snapshot::SnapshotOptions::new(&ws),
            )
            .await
            .unwrap();
            let cache2 = TempDir::new().unwrap();
            let lc2 = LocalCache::new(cache2.path()).unwrap();
            clw_hydrate::hydrate(&out, &store, &lc2, &clw_hydrate::HydrateOptions::new(&ws))
                .await
                .unwrap();
            // Each flow's output must equal ITS OWN payload (no cross-contamination).
            assert_eq!(
                std::fs::read(out.join("data.bin")).unwrap(),
                payload.into_bytes()
            );
        }
    });
    futures::future::join_all(flows).await;
    drop(store_dir);
}

fn walk_count(dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if std::fs::symlink_metadata(&p).unwrap().file_type().is_dir() {
                n += walk_count(&p);
            } else {
                n += 1;
            }
        }
    }
    n
}
