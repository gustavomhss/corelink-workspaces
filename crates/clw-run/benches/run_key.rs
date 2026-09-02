//! Criterion benchmark for the public `run_memoized` key/memo boundary.
//!
//! `compute_key` and the tool resolver are intentionally private implementation
//! details. The smallest public surface that exercises them is
//! [`clw_run::run_memoized`], so this benchmark does not widen the production
//! API just for measurement.
//!
//! The fixture contains a deterministic 32 MiB declared input and a 4 MiB tool
//! fixture. Both are created before Criterion starts timing and reread from the
//! same inodes, so the filesystem page cache is warm during measured calls. The
//! AC-miss case uses an in-memory empty transport: key derivation (including
//! input and tool streaming digests) runs, then spawning the deliberately
//! non-executable local fixture fails without invoking a host-provided command.
//! The AC-hit case uses an in-memory AC/CAS record and therefore exercises
//! repeated key/input work followed by a real memo hit. It is *not* labelled as
//! a no-I/O path because declared inputs are still read on every call; any
//! tool-digest reuse is part of the implementation being measured.
//!
//! No baseline number is asserted or invented here. Record Criterion's output
//! from the same revision, machine, and build profile when comparing changes.
//!
//! Run with: `cargo bench -p clw-run --bench run_key` (Criterion output is under
//! `run_key_fs_warm/ac_miss_key_and_tool_digest_fs_warm` and
//! `run_key_fs_warm/ac_hit_recompute_memo_hit_fs_warm`).
//! Compile-check only: `cargo bench -p clw-run --bench run_key --no-run`

use async_trait::async_trait;
use clw_run::{run_memoized, RunOptions};
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::runtime::Runtime;

const INPUT_BYTES: usize = 32 * 1024 * 1024;
const TOOL_BYTES: usize = 4 * 1024 * 1024;

/// Deterministic xorshift bytes keep fixture contents stable without a random
/// dependency or host-specific input. Fixture creation is outside timing.
fn deterministic_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut bytes = Vec::with_capacity(len);
    while bytes.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        bytes.extend_from_slice(&state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn fixture() -> (TempDir, RunOptions, u64) {
    let dir = TempDir::new().expect("temporary benchmark fixture");
    std::fs::write(
        dir.path().join("input.bin"),
        deterministic_bytes(INPUT_BYTES, 0x11),
    )
    .expect("write input fixture");
    // This is data, not a platform executable. It is only read for the tool
    // identity and intentionally fails at spawn in the AC-miss benchmark.
    std::fs::write(
        dir.path().join("tool.bin"),
        deterministic_bytes(TOOL_BYTES, 0x22),
    )
    .expect("write tool fixture");

    let opts = RunOptions {
        inputs: vec!["input.bin".to_owned()],
        command: vec!["./tool.bin".to_owned()],
        env_keys: Vec::new(),
    };
    (dir, opts, (INPUT_BYTES + TOOL_BYTES) as u64)
}

/// An AC/CAS transport that forces an AC miss. The miss reaches the local
/// fixture spawn after the key has been fully derived; no network and no host
/// command are involved.
#[derive(Clone, Copy, Default)]
struct EmptyStore;

#[async_trait]
impl CasTransport for EmptyStore {
    async fn exists(&self, _digest: &Digest) -> Result<bool> {
        Ok(false)
    }

    async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
        Err(ClwError::NotFound(*digest))
    }

    async fn put(&self, _digest: &Digest, _bytes: Vec<u8>) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _digest: &Digest) -> Result<()> {
        Ok(())
    }

    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl AcTransport for EmptyStore {
    async fn get(&self, _key: &Digest) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    async fn put(&self, _key: &Digest, _value: Vec<u8>) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _key: &Digest) -> Result<()> {
        Ok(())
    }

    async fn list_refs(&self) -> Result<Vec<Digest>> {
        Ok(Vec::new())
    }
}

/// In-memory AC/CAS transport used for a genuine public memo-hit path.
#[derive(Clone, Default)]
struct WarmStore {
    cas: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ac: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl WarmStore {
    fn seed_for(&self, key: &Digest) -> Vec<u8> {
        let stdout = b"benchmark stdout";
        let stderr = b"benchmark stderr";
        let stdout_digest = Digest::of_bytes(stdout);
        let stderr_digest = Digest::of_bytes(stderr);
        self.cas
            .lock()
            .expect("CAS mutex")
            .insert(stdout_digest.to_hex(), stdout.to_vec());
        self.cas
            .lock()
            .expect("CAS mutex")
            .insert(stderr_digest.to_hex(), stderr.to_vec());
        let record = serde_json::json!({
            "exit_code": 0,
            "stdout": stdout_digest.to_hex(),
            "stderr": stderr_digest.to_hex(),
        });
        let bytes = serde_json::to_vec(&record).expect("serialize benchmark record");
        self.ac
            .lock()
            .expect("AC mutex")
            .insert(key.to_hex(), bytes.clone());
        bytes
    }
}

#[async_trait]
impl CasTransport for WarmStore {
    async fn exists(&self, digest: &Digest) -> Result<bool> {
        Ok(self
            .cas
            .lock()
            .expect("CAS mutex")
            .contains_key(&digest.to_hex()))
    }

    async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
        self.cas
            .lock()
            .expect("CAS mutex")
            .get(&digest.to_hex())
            .cloned()
            .ok_or(ClwError::NotFound(*digest))
    }

    async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
        self.cas
            .lock()
            .expect("CAS mutex")
            .insert(digest.to_hex(), bytes);
        Ok(())
    }

    async fn delete(&self, digest: &Digest) -> Result<()> {
        self.cas.lock().expect("CAS mutex").remove(&digest.to_hex());
        Ok(())
    }

    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        self.cas
            .lock()
            .expect("CAS mutex")
            .keys()
            .map(|key| Digest::from_hex(key))
            .collect()
    }
}

#[async_trait]
impl AcTransport for WarmStore {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
        if let Some(record) = self
            .ac
            .lock()
            .expect("AC mutex")
            .get(&key.to_hex())
            .cloned()
        {
            return Ok(Some(record));
        }

        // Seed once before timing, then return the same record as a normal AC
        // lookup. `seed_for` is only a fixture convenience, not production API.
        let record = self.seed_for(key);
        Ok(Some(record))
    }

    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
        self.ac
            .lock()
            .expect("AC mutex")
            .insert(key.to_hex(), value);
        Ok(())
    }

    async fn delete(&self, key: &Digest) -> Result<()> {
        self.ac.lock().expect("AC mutex").remove(&key.to_hex());
        Ok(())
    }

    async fn list_refs(&self) -> Result<Vec<Digest>> {
        self.ac
            .lock()
            .expect("AC mutex")
            .keys()
            .map(|key| Digest::from_hex(key))
            .collect()
    }
}

fn bench_run_key_fs_warm(c: &mut Criterion) {
    // Keep the fixtures separate so a process-local tool-digest cache warmed by
    // the AC-hit setup cannot turn the AC-miss case into an AC-hit tool lookup.
    let (ac_miss_dir, ac_miss_opts, bytes) = fixture();
    let (warm_dir, warm_opts, _) = fixture();
    let runtime = Runtime::new().expect("Tokio runtime");
    let ac_miss = EmptyStore;
    let warm = WarmStore::default();

    // Keep lazy fixture seeding and all filesystem setup out of the measured
    // warm path. This also proves the public call really returns a memo hit.
    let seeded = runtime.block_on(run_memoized(warm_dir.path(), &warm, &warm_opts));
    assert!(seeded.expect("seed warm memo").hit);

    let mut group = c.benchmark_group("run_key_fs_warm");
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("ac_miss_key_and_tool_digest_fs_warm", |b| {
        b.iter(|| {
            let result = runtime.block_on(run_memoized(
                black_box(ac_miss_dir.path()),
                &ac_miss,
                black_box(&ac_miss_opts),
            ));
            assert!(result.is_err(), "non-executable fixture must not run");
            black_box(result)
        })
    });
    group.bench_function("ac_hit_recompute_memo_hit_fs_warm", |b| {
        b.iter(|| {
            let result = runtime
                .block_on(run_memoized(
                    black_box(warm_dir.path()),
                    &warm,
                    black_box(&warm_opts),
                ))
                .expect("seeded memo hit");
            assert!(result.hit);
            black_box(result)
        })
    });
    group.finish();
}

criterion_group!(benches, bench_run_key_fs_warm);
criterion_main!(benches);
