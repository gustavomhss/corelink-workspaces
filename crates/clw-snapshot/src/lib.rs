//! clw-snapshot — walk→chunk→dedupe→upload→ref pipeline.
//!
//! Builds a content-addressed manifest from a workspace directory, uploads
//! chunks to the CoreLink CAS (skipping chunks already present), uploads the
//! manifest itself, and writes a mutable `RefRecord` in the AC namespace so
//! the workspace can be hydrated later.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clw_cache::LocalCache;
use clw_chunk::{chunk_bytes, chunk_stream, to_chunk_refs};
use clw_manifest::{walk, WalkedEntry};
use clw_types::{
    ref_key, AcTransport, CasTransport, ChunkRef, Digest, Entry, Manifest, RefRecord, Result,
    MANIFEST_VERSION,
};
use futures::stream::{self, StreamExt};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options for a snapshot operation.
pub struct SnapshotOptions {
    pub workspace_name: String,
    pub concurrency: usize,
}

impl SnapshotOptions {
    /// Create options with the given workspace name and default concurrency of 8.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            workspace_name: name.into(),
            concurrency: 8,
        }
    }
}

/// Summary of a completed snapshot.
pub struct SnapshotReport {
    /// CAS digest of the root `Manifest`.
    pub root: Digest,
    /// Number of `File` entries in the manifest.
    pub files: u64,
    /// Sum of all file sizes (== `manifest.total_size`).
    pub bytes_total: u64,
    /// Total number of chunks across all files.
    pub chunks_total: u64,
    /// Chunks that were actually PUT to the remote (not already present).
    pub chunks_uploaded: u64,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Files at or below this size keep the in-memory fast path (read whole →
/// `chunk_bytes`); larger files are streamed (`chunk_stream`) so that at most
/// one chunk's bytes are resident at a time. Streaming and in-memory chunking
/// are byte-for-byte dedupe-identical (see clw-chunk's `equivalence` proptest),
/// so the manifest/digest is unaffected by which path a file takes.
const STREAM_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;

/// Upload one chunk: write it into the local cache, then PUT it to the remote
/// CAS only if it is not already present. Returns `true` iff a remote PUT was
/// performed (i.e. the chunk was missing), so the caller can advance the
/// `chunks_uploaded` counter. This preserves the existing dedupe behaviour:
/// `exists` is consulted and `put` happens only on a miss.
async fn upload_chunk<C>(
    client: &C,
    cache: &LocalCache,
    digest: &Digest,
    data: Vec<u8>,
) -> Result<bool>
where
    C: CasTransport,
{
    // Local cache: temp-file + atomic rename(2); fast enough to call directly.
    cache.put(digest, &data)?;

    // Remote: check existence, then PUT if missing.
    let already_there = CasTransport::exists(client, digest).await?;
    if already_there {
        Ok(false)
    } else {
        CasTransport::put(client, digest, data).await?;
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// build_manifest_local
// ---------------------------------------------------------------------------

/// Walk `root` and build a [`Manifest`] by chunking every file.
///
/// Symlinks and empty directories are included as-is.  File bytes are read
/// with `tokio::task::spawn_blocking` so this function is safe to `.await`
/// inside a Tokio runtime.
pub async fn build_manifest_local(root: &Path) -> Result<Manifest> {
    // Walk is sync; run it on the blocking thread pool.
    let root_owned = root.to_path_buf();
    let walked = tokio::task::spawn_blocking(move || walk(&root_owned))
        .await
        .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

    let mut entries: Vec<Entry> = Vec::with_capacity(walked.len());
    let mut total_size: u64 = 0;

    for we in walked {
        match we {
            WalkedEntry::File(f) => {
                // Same threshold as `snapshot`: small files chunk in memory,
                // large files stream (bounded to one chunk's bytes at a time).
                // Both paths are dedupe-identical, so the manifest produced here
                // matches `snapshot`'s regardless of which path a file takes.
                let chunk_refs: Vec<ChunkRef> = if f.size <= STREAM_THRESHOLD_BYTES {
                    let abs = f.abs.clone();
                    let data = tokio::task::spawn_blocking(move || std::fs::read(&abs))
                        .await
                        .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

                    let chunks_raw = chunk_bytes(&data);
                    to_chunk_refs(&chunks_raw)
                } else {
                    let abs = f.abs.clone();
                    let file = tokio::task::spawn_blocking(move || std::fs::File::open(&abs))
                        .await
                        .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

                    let mut refs: Vec<ChunkRef> = Vec::new();
                    for item in chunk_stream(file) {
                        let sc = item.map_err(|e| clw_types::ClwError::Transport(e.to_string()))?;
                        refs.push(ChunkRef {
                            digest: sc.digest,
                            size: sc.len,
                        });
                    }
                    refs
                };

                total_size += f.size;
                entries.push(Entry::File {
                    path: f.rel,
                    mode: f.mode,
                    size: f.size,
                    chunks: chunk_refs,
                });
            }
            WalkedEntry::Symlink { rel, target } => {
                entries.push(Entry::Symlink { path: rel, target });
            }
            WalkedEntry::EmptyDir { rel } => {
                entries.push(Entry::Dir { path: rel });
            }
        }
    }

    // walk already returns sorted output; preserve order.
    Ok(Manifest {
        version: MANIFEST_VERSION,
        total_size,
        entries,
    })
}

// ---------------------------------------------------------------------------
// snapshot
// ---------------------------------------------------------------------------

/// Walk `root`, chunk every file, upload missing chunks to `client`, upload
/// the manifest, and write a mutable ref record in the AC namespace.
///
/// Returns a [`SnapshotReport`] with counters.
pub async fn snapshot<C>(
    root: &Path,
    client: &C,
    cache: &LocalCache,
    opts: &SnapshotOptions,
) -> Result<SnapshotReport>
where
    C: CasTransport + AcTransport,
{
    // --- 1. Walk (sync, blocking thread) ------------------------------------
    let root_owned = root.to_path_buf();
    let walked = tokio::task::spawn_blocking(move || walk(&root_owned))
        .await
        .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

    // --- 2. Read/stream files, chunk, and upload as we go -------------------
    //
    // Memory is kept bounded: we NEVER buffer the whole workspace's chunk
    // bytes. Small files take the in-memory fast path (their chunks are
    // uploaded with bounded concurrency, then their bytes are dropped before
    // moving on); large files are streamed so at most one chunk's bytes are
    // resident at a time. Only the small `ChunkRef`s and the counters survive
    // across files. Streaming and in-memory chunking are dedupe-identical, so
    // the resulting manifest/digest is independent of which path each file
    // took.
    let mut entries: Vec<Entry> = Vec::with_capacity(walked.len());
    let mut total_size: u64 = 0;
    let mut chunks_total: u64 = 0;
    let mut files_count: u64 = 0;

    // Shared across all files so `chunks_uploaded` aggregates the whole tree.
    let uploaded_counter = Arc::new(AtomicU64::new(0));

    for we in walked {
        match we {
            WalkedEntry::File(f) => {
                let chunk_refs: Vec<ChunkRef> = if f.size <= STREAM_THRESHOLD_BYTES {
                    // ---- small file: in-memory fast path -------------------
                    let abs = f.abs.clone();
                    let data = tokio::task::spawn_blocking(move || std::fs::read(&abs))
                        .await
                        .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

                    let chunks_raw = chunk_bytes(&data);
                    let chunk_refs = to_chunk_refs(&chunks_raw);
                    chunks_total += chunks_raw.len() as u64;

                    // Upload THIS file's chunks (bounded concurrency), then let
                    // `data` / the per-chunk copies drop at the end of the block.
                    let concurrency = opts.concurrency.max(1);
                    let results: Vec<Result<()>> = stream::iter(chunks_raw.into_iter().map(|c| {
                        let counter = Arc::clone(&uploaded_counter);
                        // Copy out this chunk's bytes (bounded by CHUNK_MAX_BYTES).
                        let bytes = data[c.offset as usize..(c.offset + c.len) as usize].to_vec();
                        async move {
                            if upload_chunk(client, cache, &c.digest, bytes).await? {
                                counter.fetch_add(1, Ordering::Relaxed);
                            }
                            Ok(())
                        }
                    }))
                    .buffer_unordered(concurrency)
                    .collect()
                    .await;
                    for r in results {
                        r?;
                    }

                    chunk_refs
                } else {
                    // ---- large file: stream, one chunk's bytes at a time ----
                    // Open the handle on the blocking pool (mirrors the
                    // `std::fs::read` pattern); chunking + uploads then proceed
                    // sequentially so peak memory stays at ~one chunk.
                    let abs = f.abs.clone();
                    let file = tokio::task::spawn_blocking(move || std::fs::File::open(&abs))
                        .await
                        .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

                    let mut chunk_refs: Vec<ChunkRef> = Vec::new();
                    for item in chunk_stream(file) {
                        let sc = item.map_err(|e| clw_types::ClwError::Transport(e.to_string()))?;
                        chunks_total += 1;
                        chunk_refs.push(ChunkRef {
                            digest: sc.digest,
                            size: sc.len,
                        });
                        if upload_chunk(client, cache, &sc.digest, sc.data).await? {
                            uploaded_counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    chunk_refs
                };

                total_size += f.size;
                files_count += 1;

                entries.push(Entry::File {
                    path: f.rel,
                    mode: f.mode,
                    size: f.size,
                    chunks: chunk_refs,
                });
            }
            WalkedEntry::Symlink { rel, target } => {
                entries.push(Entry::Symlink { path: rel, target });
            }
            WalkedEntry::EmptyDir { rel } => {
                entries.push(Entry::Dir { path: rel });
            }
        }
    }

    // --- 3. Assemble manifest -----------------------------------------------
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        total_size,
        entries,
    };

    // --- 4. Chunk uploads already happened inline above ---------------------
    let chunks_uploaded = uploaded_counter.load(Ordering::Relaxed);

    // --- 5. Upload manifest -------------------------------------------------
    let manifest_bytes = manifest.canonical_bytes()?;
    let manifest_digest = Digest::of_bytes(&manifest_bytes);
    let manifest_size = manifest_bytes.len() as u64;

    let already_there = CasTransport::exists(client, &manifest_digest).await?;
    if !already_there {
        CasTransport::put(client, &manifest_digest, manifest_bytes).await?;
    }

    // --- 6. Read existing ref for parent lineage ----------------------------
    let rk = ref_key(&opts.workspace_name);
    let parent: Option<Digest> = match AcTransport::get(client, &rk).await? {
        Some(bytes) => match RefRecord::from_bytes(&bytes) {
            Ok(rec) => Some(rec.root),
            Err(_) => None, // corrupt → treat as absent
        },
        None => None,
    };

    // --- 7. Write new ref record --------------------------------------------
    let created_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let ref_record = RefRecord {
        name: opts.workspace_name.clone(),
        root: manifest_digest,
        manifest_size,
        created_at_unix,
        parent,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    AcTransport::put(client, &rk, ref_record.to_bytes()?).await?;

    // --- 8. Return report ---------------------------------------------------
    Ok(SnapshotReport {
        root: manifest_digest,
        files: files_count,
        bytes_total: total_size,
        chunks_total,
        chunks_uploaded,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    // -----------------------------------------------------------------------
    // In-crate fake transport (CasTransport + AcTransport over in-memory maps)
    // -----------------------------------------------------------------------

    #[derive(Clone, Default)]
    struct FakeTransport {
        cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        ac: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
    }

    #[async_trait::async_trait]
    impl CasTransport for FakeTransport {
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
    }

    #[async_trait::async_trait]
    impl AcTransport for FakeTransport {
        async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
            Ok(self.ac.lock().await.get(key).cloned())
        }

        async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
            self.ac.lock().await.insert(*key, value);
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Helper: build a tempdir tree with a small file, a large file (>1 MiB),
    // a symlink, and an empty directory.
    // -----------------------------------------------------------------------

    fn build_tree(root: &Path) {
        use std::fs;

        // Small file
        fs::write(root.join("small.txt"), b"hello snapshot").unwrap();

        // Large file (>1 MiB → forces multi-chunk with FastCDC avg=1 MiB)
        let big: Vec<u8> = (0u32..=(1024 * 1024 + 100))
            .map(|i| (i.wrapping_mul(0x9e37_79b9u32)) as u8)
            .collect();
        fs::write(root.join("big.bin"), &big).unwrap();

        // Symlink
        std::os::unix::fs::symlink("small.txt", root.join("link.txt")).unwrap();

        // Empty directory
        fs::create_dir(root.join("empty_dir")).unwrap();
    }

    // -----------------------------------------------------------------------
    // Test 1: full pipeline on a tempdir tree
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn full_pipeline_basic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        build_tree(root);

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("my-workspace");

        let report = snapshot(root, &client, &cache, &opts).await.unwrap();

        // files = 2 (small.txt + big.bin); symlink and empty dir are not counted
        assert_eq!(report.files, 2, "expected 2 file entries");

        // bytes_total = small + big sizes
        let small_len = b"hello snapshot".len() as u64;
        let big_len = (1024 * 1024 + 101) as u64;
        assert_eq!(report.bytes_total, small_len + big_len);

        // chunks_total >= 2 (big.bin forces multi-chunk)
        assert!(
            report.chunks_total >= 2,
            "expected at least 2 chunks total (big file forces multi-chunk), got {}",
            report.chunks_total
        );

        // First snapshot: all chunks uploaded
        assert_eq!(
            report.chunks_uploaded, report.chunks_total,
            "first snapshot: all chunks should be uploaded"
        );

        // Manifest digest present in fake CAS
        let cas = client.cas.lock().await;
        assert!(
            cas.contains_key(&report.root),
            "manifest digest should be in CAS"
        );
        drop(cas);

        // Ref record present with correct root and parent=None
        let rk = ref_key("my-workspace");
        let ac = client.ac.lock().await;
        let ref_bytes = ac.get(&rk).expect("ref record should be in AC");
        let rec = RefRecord::from_bytes(ref_bytes).unwrap();
        assert_eq!(rec.root, report.root);
        assert_eq!(rec.name, "my-workspace");
        assert!(
            rec.parent.is_none(),
            "first snapshot should have parent=None"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: second snapshot of unchanged tree → chunks_uploaded == 0
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn second_snapshot_unchanged_no_uploads() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        build_tree(root);

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-unchanged");

        let report1 = snapshot(root, &client, &cache, &opts).await.unwrap();
        let report2 = snapshot(root, &client, &cache, &opts).await.unwrap();

        // Same tree → same root
        assert_eq!(report2.root, report1.root);

        // No new chunks should have been PUT
        assert_eq!(
            report2.chunks_uploaded, 0,
            "second snapshot of unchanged tree should upload 0 chunks"
        );

        // Parent lineage: second ref's parent == first root
        let rk = ref_key("ws-unchanged");
        let ac = client.ac.lock().await;
        let ref_bytes = ac.get(&rk).unwrap();
        let rec = RefRecord::from_bytes(ref_bytes).unwrap();
        assert_eq!(
            rec.parent,
            Some(report1.root),
            "second snapshot parent should be first root"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: changed one file → only its chunks uploaded; lineage correct
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn third_snapshot_changed_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        build_tree(root);

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-changed");

        // First snapshot
        let report1 = snapshot(root, &client, &cache, &opts).await.unwrap();

        // Modify small.txt only — its chunk is new; big.bin chunks unchanged.
        std::fs::write(root.join("small.txt"), b"changed content here!").unwrap();

        let report2 = snapshot(root, &client, &cache, &opts).await.unwrap();

        // Root must differ (content changed)
        assert_ne!(report2.root, report1.root);

        // Only small.txt's chunk(s) uploaded (it's < CHUNK_MIN so 1 chunk)
        assert!(
            report2.chunks_uploaded >= 1,
            "at least the new small.txt chunk should be uploaded"
        );
        // big.bin chunks were already present → not uploaded again.
        assert!(
            report2.chunks_uploaded < report2.chunks_total,
            "big.bin chunks should not be re-uploaded; uploaded={} total={}",
            report2.chunks_uploaded,
            report2.chunks_total
        );

        // Parent lineage: new ref's parent == first root
        let rk = ref_key("ws-changed");
        let ac = client.ac.lock().await;
        let ref_bytes = ac.get(&rk).unwrap();
        let rec = RefRecord::from_bytes(ref_bytes).unwrap();
        assert_eq!(
            rec.parent,
            Some(report1.root),
            "parent should point to first snapshot root"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: build_manifest_local digest matches snapshot() root
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn build_manifest_local_matches_snapshot() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        build_tree(root);

        let manifest_local = build_manifest_local(root).await.unwrap();
        let digest_local = manifest_local.digest().unwrap();

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-digest-check");

        let report = snapshot(root, &client, &cache, &opts).await.unwrap();

        assert_eq!(
            digest_local, report.root,
            "build_manifest_local and snapshot() must produce the same manifest digest"
        );
    }
}
