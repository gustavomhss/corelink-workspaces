//! Example: a full snapshot -> hydrate round-trip, offline and byte-exact.
//!
//! This is runnable, end-to-end API documentation for the two halves of the
//! CoreLink workspace library:
//!   * [`clw_snapshot::snapshot`] — walk a directory, chunk + dedupe every file,
//!     upload the chunks and the manifest to the CAS, and write a mutable ref
//!     in the AC namespace.
//!   * [`clw_hydrate::hydrate`]   — resolve that ref, fetch the manifest, and
//!     materialise the whole tree into a fresh destination directory.
//!
//! Everything runs in-process against an in-memory fake transport — there is no
//! network call and no real CoreLink endpoint. The example builds a small tmp
//! tree, snapshots it, hydrates it to a *new* directory, and asserts the two
//! trees are byte-for-byte identical.
//!
//! Run it with:
//!     cargo run -p clw-hydrate --example snapshot_hydrate_roundtrip
//!
//! It prints what it did at each step and exits 0 on success.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use clw_cache::LocalCache;
use clw_hydrate::types::{AcTransport, CasTransport, ClwError, Digest, Result};
use clw_hydrate::{hydrate, HydrateOptions};
use clw_snapshot::{snapshot, SnapshotOptions};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// A tiny in-memory transport implementing BOTH halves of the contract.
//
// `snapshot` and `hydrate` are each generic over `C: CasTransport + AcTransport`.
// In production that `C` is the real CoreLink HTTP client; here a trivial fake
// backed by two maps lets the whole pipeline run offline:
//   * CAS (content-addressed store): digest -> blob bytes. Holds file chunks and
//     the serialized manifest. `snapshot` writes it; `hydrate` reads it.
//   * AC (action cache): ref-key -> RefRecord bytes. The mutable pointer naming
//     the manifest digest for a workspace. `snapshot` writes it; `hydrate`
//     reads it.
//
// We share the SAME store instance between snapshot and hydrate, which is what
// makes the round-trip work: hydrate sees exactly what snapshot uploaded. The
// crates' own unit tests use this same shape (`#[cfg(test)]`-only), so we
// rebuild a small inline version here — the way a real downstream user would.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MemStore {
    cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
    ac: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
}

#[async_trait::async_trait]
impl CasTransport for MemStore {
    async fn exists(&self, digest: &Digest) -> Result<bool> {
        Ok(self.cas.lock().await.contains_key(digest))
    }

    async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
        self.cas
            .lock()
            .await
            .get(digest)
            .cloned()
            .ok_or(ClwError::NotFound(*digest))
    }

    async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
        self.cas.lock().await.insert(*digest, bytes);
        Ok(())
    }

    async fn delete(&self, digest: &Digest) -> Result<()> {
        self.cas.lock().await.remove(digest);
        Ok(())
    }

    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        Ok(self.cas.lock().await.keys().copied().collect())
    }
}

#[async_trait::async_trait]
impl AcTransport for MemStore {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
        Ok(self.ac.lock().await.get(key).cloned())
    }

    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
        self.ac.lock().await.insert(*key, value);
        Ok(())
    }

    async fn delete(&self, key: &Digest) -> Result<()> {
        self.ac.lock().await.remove(key);
        Ok(())
    }

    async fn list_refs(&self) -> Result<Vec<Digest>> {
        Ok(self.ac.lock().await.keys().copied().collect())
    }
}

// ---------------------------------------------------------------------------
// Build a small source tree: a couple of text files, a binary file large
// enough to span multiple chunks, a nested file, and (on unix) a symlink.
// Returns the relative paths we wrote, so we can compare them after hydrate.
// ---------------------------------------------------------------------------
fn build_source_tree(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root.join("docs")).map_err(ClwError::Io)?;
    std::fs::write(root.join("README.md"), b"# demo workspace\n").map_err(ClwError::Io)?;
    std::fs::write(root.join("docs/notes.txt"), b"nested file content\n").map_err(ClwError::Io)?;

    // A ~2 MiB pseudo-random blob forces FastCDC to emit multiple chunks, so the
    // multi-chunk file path is exercised end-to-end (chunk -> upload -> reassemble).
    let big: Vec<u8> = (0u32..(2 * 1024 * 1024))
        .map(|i| (i.wrapping_mul(0x9e37_79b9)) as u8)
        .collect();
    std::fs::write(root.join("blob.bin"), &big).map_err(ClwError::Io)?;

    // An empty directory — manifests carry these as `Entry::Dir`.
    std::fs::create_dir_all(root.join("empty")).map_err(ClwError::Io)?;

    // A relative symlink (unix only). hydrate recreates it verbatim.
    #[cfg(unix)]
    std::os::unix::fs::symlink("README.md", root.join("link.md")).map_err(ClwError::Io)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Recursively collect (relative-path -> kind+content) for every entry under
// `root`, so two trees can be compared for byte-identical equality. Symlinks
// are compared by their target string, files by their bytes, dirs by presence.
// ---------------------------------------------------------------------------
#[derive(PartialEq, Eq, Debug)]
enum Node {
    File(Vec<u8>),
    Symlink(String),
    Dir,
}

fn collect_tree(root: &Path) -> Result<HashMap<String, Node>> {
    fn walk(base: &Path, dir: &Path, out: &mut HashMap<String, Node>) -> Result<()> {
        for entry in std::fs::read_dir(dir).map_err(ClwError::Io)? {
            let entry = entry.map_err(ClwError::Io)?;
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let meta = std::fs::symlink_metadata(&path).map_err(ClwError::Io)?;
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&path).map_err(ClwError::Io)?;
                out.insert(rel, Node::Symlink(target.to_string_lossy().into_owned()));
            } else if meta.is_dir() {
                out.insert(rel.clone(), Node::Dir);
                walk(base, &path, out)?;
            } else {
                out.insert(rel, Node::File(std::fs::read(&path).map_err(ClwError::Io)?));
            }
        }
        Ok(())
    }
    let mut out = HashMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

#[tokio::main]
async fn main() -> Result<()> {
    // The single shared store both halves talk to (offline; no network).
    let store = MemStore::default();

    // --- 1. Build a source tree in a scratch directory ----------------------
    let src_dir = tempfile::TempDir::new().map_err(ClwError::Io)?;
    let src = src_dir.path();
    build_source_tree(src)?;
    let before = collect_tree(src)?;
    println!(
        "built source tree: {} entries under {}",
        before.len(),
        src.display()
    );

    // --- 2. SNAPSHOT: walk -> chunk -> dedupe -> upload -> write ref --------
    // The snapshot uses its own local chunk cache (a temp dir). The shared
    // `store` receives the chunks, the manifest blob, and the ref record.
    let snap_cache_dir = tempfile::TempDir::new().map_err(ClwError::Io)?;
    let snap_cache = LocalCache::new(snap_cache_dir.path())?;
    let snap_opts = SnapshotOptions::new("demo-ws");
    let report = snapshot(src, &store, &snap_cache, &snap_opts).await?;
    println!(
        "snapshot: {} files, {} bytes, {} chunks ({} uploaded, {} deduped); root={}",
        report.files,
        report.bytes_total,
        report.chunks_total,
        report.chunks_uploaded,
        report.chunks_total - report.chunks_uploaded,
        &report.root.to_hex()[..16],
    );

    // --- 3. HYDRATE: ref -> manifest -> materialize into a NEW dir ----------
    // A fresh (empty) destination and a fresh local cache: hydrate must pull
    // every chunk from the shared store, proving the data really round-tripped
    // through the transport rather than leaking via the snapshot's cache.
    let dst_dir = tempfile::TempDir::new().map_err(ClwError::Io)?;
    let dst = dst_dir.path().join("hydrated"); // must not pre-exist (or be empty)
    let hyd_cache_dir = tempfile::TempDir::new().map_err(ClwError::Io)?;
    let hyd_cache = LocalCache::new(hyd_cache_dir.path())?;
    let hyd_opts = HydrateOptions::new("demo-ws");
    let hreport = hydrate(&dst, &store, &hyd_cache, &hyd_opts).await?;
    println!(
        "hydrate:  {} files, {} bytes ({} downloaded, {} from cache); root={}",
        hreport.files,
        hreport.bytes_total,
        hreport.bytes_downloaded,
        hreport.bytes_from_cache,
        &hreport.root.to_hex()[..16],
    );

    // --- 4. ASSERT the round-trip is byte-identical -------------------------
    // The ref the hydrate resolved must be the manifest the snapshot wrote.
    assert_eq!(
        hreport.root, report.root,
        "hydrate must resolve the exact manifest snapshot produced"
    );
    let after = collect_tree(&dst)?;
    assert_eq!(
        before, after,
        "hydrated tree must be byte-for-byte identical to the source tree"
    );
    println!(
        "verified: hydrated tree is byte-identical to the source ({} entries match)",
        after.len(),
    );

    println!("\nsnapshot_hydrate_roundtrip: OK (snapshot -> hydrate -> byte-identical)");
    Ok(())
}
