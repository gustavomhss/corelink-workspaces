//! Example: `clw-run` memoizes a deterministic command.
//!
//! This is runnable, offline API documentation for [`clw_run::run_memoized`].
//! It demonstrates the core promise of the crate — *"never run the same
//! deterministic command twice"* — entirely in-process, against an in-memory
//! fake transport (no network, no real CoreLink endpoint).
//!
//! What you'll see:
//!   1. A FIRST call to `run_memoized` is a cache MISS: the command actually
//!      executes (we prove it by having it create a side-effect file).
//!   2. A SECOND, byte-identical call is a cache HIT: the stored stdout/stderr
//!      are replayed and the command is NOT re-executed (the side-effect file,
//!      which we deleted in between, stays gone).
//!   3. Changing a declared input invalidates the key, producing a MISS again.
//!
//! Run it with:
//!     cargo run -p clw-run --example run_memoization
//!
//! It prints what it did at each step and exits 0 on success.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use clw_run::types::{AcTransport, CasTransport, ClwError, Digest, Result};
use clw_run::{run_memoized, RunOptions};

// ---------------------------------------------------------------------------
// A tiny in-memory transport.
//
// `run_memoized` is generic over `C: CasTransport + AcTransport`. In production
// that `C` is the real CoreLink HTTP client; here we supply a trivial fake that
// keeps two in-memory maps:
//   * CAS (content-addressed store): digest -> blob bytes. Holds the captured
//     stdout/stderr blobs.
//   * AC (action cache): run-key -> RunRecord bytes. Maps "this exact command
//     over these exact inputs" to the memoized result.
//
// The crate's own unit tests use the same shape (`#[cfg(test)]`-only), so we
// rebuild a small inline version here rather than reaching into private test
// code — exactly how a real downstream user would wire their own transport.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MemStore {
    cas: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ac: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

#[async_trait::async_trait]
impl CasTransport for MemStore {
    async fn exists(&self, digest: &Digest) -> Result<bool> {
        Ok(self.cas.lock().unwrap().contains_key(&digest.to_hex()))
    }

    async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
        self.cas
            .lock()
            .unwrap()
            .get(&digest.to_hex())
            .cloned()
            .ok_or(ClwError::NotFound(*digest))
    }

    async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
        self.cas.lock().unwrap().insert(digest.to_hex(), bytes);
        Ok(())
    }

    async fn delete(&self, digest: &Digest) -> Result<()> {
        self.cas.lock().unwrap().remove(&digest.to_hex());
        Ok(())
    }

    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        Ok(self
            .cas
            .lock()
            .unwrap()
            .keys()
            .filter_map(|k| Digest::from_hex(k).ok())
            .collect())
    }
}

#[async_trait::async_trait]
impl AcTransport for MemStore {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
        Ok(self.ac.lock().unwrap().get(&key.to_hex()).cloned())
    }

    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
        self.ac.lock().unwrap().insert(key.to_hex(), value);
        Ok(())
    }

    async fn delete(&self, key: &Digest) -> Result<()> {
        self.ac.lock().unwrap().remove(&key.to_hex());
        Ok(())
    }

    async fn list_refs(&self) -> Result<Vec<Digest>> {
        Ok(self
            .ac
            .lock()
            .unwrap()
            .keys()
            .filter_map(|k| Digest::from_hex(k).ok())
            .collect())
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<()> {
    // A scratch working directory the command runs in. `RunOptions::inputs`,
    // `command` (when relative), and the side-effect file are all resolved
    // against this `cwd`.
    let tmp = tempfile::TempDir::new().map_err(ClwError::Io)?;
    let cwd: &Path = tmp.path();

    // A declared INPUT. Its *content* is digested and folded into the run key,
    // so editing it later changes the key (and forces a re-run). We start with
    // "v1".
    std::fs::write(cwd.join("input.txt"), b"v1").map_err(ClwError::Io)?;

    // The command writes a marker file as an observable SIDE EFFECT. If the
    // command runs, the marker appears; on a cache hit it must NOT appear.
    let marker = cwd.join("ran.marker");
    let marker_str = marker.to_str().unwrap().to_string();
    let shell_cmd = format!("echo 'hello from clw-run' && touch {marker_str}");

    let store = MemStore::default();

    // ---- call 1: expect a MISS (the command actually executes) -------------
    let opts = RunOptions {
        inputs: vec!["input.txt".to_string()],
        command: vec!["/bin/sh".into(), "-c".into(), shell_cmd.clone()],
        env_keys: vec![],
    };
    let first = run_memoized(cwd, &store, &opts).await?;
    assert!(!first.hit, "first call must be a miss");
    assert_eq!(first.exit_code, 0);
    assert!(
        marker.exists(),
        "command should have run and created the marker"
    );
    println!(
        "call 1: MISS  -> executed the command (exit {}, {} bytes stdout); key={}",
        first.exit_code,
        first.stdout.len(),
        &first.key.to_hex()[..16],
    );

    // Delete the marker so we can PROVE the second call does not re-execute.
    std::fs::remove_file(&marker).map_err(ClwError::Io)?;

    // ---- call 2: identical request -> expect a HIT (no re-execution) -------
    let second = run_memoized(cwd, &store, &opts).await?;
    assert!(second.hit, "identical second call must be a hit");
    assert_eq!(
        second.stdout, first.stdout,
        "the hit must replay byte-identical stdout"
    );
    assert!(
        !marker.exists(),
        "on a HIT the command must NOT re-run (marker stays absent)"
    );
    println!(
        "call 2: HIT   -> replayed stored output, command NOT re-run (marker absent: {})",
        !marker.exists(),
    );

    // ---- call 3: change a declared input -> key changes -> MISS again ------
    std::fs::write(cwd.join("input.txt"), b"v2-changed").map_err(ClwError::Io)?;
    let third = run_memoized(cwd, &store, &opts).await?;
    assert!(!third.hit, "changed input must invalidate the memoized key");
    assert_ne!(
        third.key, first.key,
        "a different input digest yields a different run key"
    );
    println!(
        "call 3: MISS  -> input.txt changed, so the run key changed; key={}",
        &third.key.to_hex()[..16],
    );

    println!("\nrun_memoization: OK (miss -> hit -> miss-on-input-change)");
    Ok(())
}

// `run_memoized`'s execution path here uses `/bin/sh`, which is unix-only. On a
// non-unix host the example compiles but simply reports that it was skipped, so
// `cargo build --examples` and a run still succeed everywhere.
#[cfg(not(unix))]
fn main() {
    println!("run_memoization: skipped (requires a unix /bin/sh)");
}
