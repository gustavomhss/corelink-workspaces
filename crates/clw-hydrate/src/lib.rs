//! clw-hydrate — ref → manifest → cache-first materialize pipeline.
//!
//! Reads a workspace ref from the AC store, fetches the manifest from CAS,
//! then materialises every entry (files, symlinks, empty dirs) into `dest`
//! with file-level concurrency bounded by [`HydrateOptions::concurrency`].
//!
//! All network calls use in-process fakes in tests — never the real endpoint.

pub use clw_types as types;

use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use tokio::task::spawn_blocking;

use clw_cache::LocalCache;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Entry, Manifest, RefRecord, Result};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options that control a single hydrate run.
pub struct HydrateOptions {
    pub workspace_name: String,
    pub concurrency: usize,
}

impl HydrateOptions {
    /// Create options for `name` with the default concurrency (8).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            workspace_name: name.into(),
            concurrency: 8,
        }
    }
}

/// Summary produced by a successful [`hydrate`] call.
#[derive(Debug)]
pub struct HydrateReport {
    /// Root manifest digest (from the RefRecord).
    pub root: Digest,
    /// Number of `File` entries materialised.
    pub files: u64,
    /// `manifest.total_size` (sum of all file sizes).
    pub bytes_total: u64,
    /// Bytes served from the local cache (cache hits).
    pub bytes_from_cache: u64,
    /// Bytes fetched from the remote and written into the cache (cache misses).
    pub bytes_downloaded: u64,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Materialise a workspace snapshot into `dest`.
///
/// # Safety
/// `dest` must either not exist yet, or be an **empty** directory.
/// A non-empty destination returns `Err(ClwError::Io(AlreadyExists))`.
///
/// # Steps
/// 1. Resolve ref → [`RefRecord`] from AC.
/// 2. Fetch manifest blob from CAS; parse as [`Manifest`].
/// 3. Create directories, symlinks, and files (cache-first, bounded concurrency).
/// 4. Return a [`HydrateReport`].
pub async fn hydrate<C>(
    dest: &Path,
    client: &C,
    cache: &LocalCache,
    opts: &HydrateOptions,
) -> Result<HydrateReport>
where
    C: CasTransport + AcTransport,
{
    // ------------------------------------------------------------------
    // 1. Safety: dest must not exist or be empty.
    // ------------------------------------------------------------------
    if dest.exists() {
        // Allow an *empty* directory; reject anything else.
        let is_empty = dest.is_dir()
            && std::fs::read_dir(dest)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(false);
        if !is_empty {
            return Err(ClwError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                "destination not empty",
            )));
        }
    }

    // ------------------------------------------------------------------
    // 2a. Fetch RefRecord from AC.
    // ------------------------------------------------------------------
    let ref_key = clw_types::ref_key(&opts.workspace_name);
    let ref_bytes = AcTransport::get(client, &ref_key)
        .await?
        .ok_or(ClwError::NotFound(ref_key))?;
    let record = RefRecord::from_bytes(&ref_bytes)?;

    // ------------------------------------------------------------------
    // 2b. Fetch manifest blob from CAS; parse.
    // ------------------------------------------------------------------
    let manifest_bytes = CasTransport::get(client, &record.root).await?;
    let manifest = Manifest::from_bytes(&manifest_bytes)?;

    // ------------------------------------------------------------------
    // 3. Ensure dest exists.
    // ------------------------------------------------------------------
    std::fs::create_dir_all(dest)?;

    // Shared state for per-file counters.
    let bytes_from_cache = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let bytes_downloaded = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let files_count = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // ------------------------------------------------------------------
    // 3a. Directories and symlinks are cheap — handle them first,
    //     sequentially (no network I/O needed).
    // ------------------------------------------------------------------
    for entry in &manifest.entries {
        match entry {
            Entry::Dir { path } => {
                let abs = dest.join(path);
                std::fs::create_dir_all(&abs)?;
            }
            Entry::Symlink { path, target } => {
                let abs = dest.join(path);
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(target, &abs)?;
            }
            Entry::File { .. } => {} // handled below
        }
    }

    // ------------------------------------------------------------------
    // 3b. Files — bounded concurrency via buffer_unordered.
    // ------------------------------------------------------------------
    let file_entries: Vec<_> = manifest
        .entries
        .iter()
        .filter_map(|e| {
            if let Entry::File {
                path,
                mode,
                size,
                chunks,
            } = e
            {
                Some((path.clone(), *mode, *size, chunks.clone()))
            } else {
                None
            }
        })
        .collect();

    let dest_arc = Arc::new(dest.to_path_buf());
    let bfc = Arc::clone(&bytes_from_cache);
    let bfd = Arc::clone(&bytes_downloaded);
    let fc = Arc::clone(&files_count);

    // Build a stream of futures — one per file.
    // Each future returns (bytes_from_cache, bytes_downloaded) for that file.
    let stream =
        futures::stream::iter(file_entries.into_iter().map(|(path, mode, _size, chunks)| {
            let dest_arc = Arc::clone(&dest_arc);
            let bfc = Arc::clone(&bfc);
            let bfd = Arc::clone(&bfd);
            let fc = Arc::clone(&fc);
            async move {
                let (from_cache, downloaded) =
                    materialize_file(dest_arc.as_ref(), &path, mode, &chunks, client, cache)
                        .await?;
                bfc.fetch_add(from_cache, std::sync::atomic::Ordering::Relaxed);
                bfd.fetch_add(downloaded, std::sync::atomic::Ordering::Relaxed);
                fc.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok::<(), ClwError>(())
            }
        }));

    // Drive the stream with bounded concurrency.
    let mut buffered = stream.buffer_unordered(opts.concurrency);
    while let Some(result) = buffered.next().await {
        result?;
    }

    Ok(HydrateReport {
        root: record.root,
        files: files_count.load(std::sync::atomic::Ordering::Relaxed),
        bytes_total: manifest.total_size,
        bytes_from_cache: bytes_from_cache.load(std::sync::atomic::Ordering::Relaxed),
        bytes_downloaded: bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed),
    })
}

// ---------------------------------------------------------------------------
// Internal: materialise one file
//
// Returns (bytes_from_cache, bytes_downloaded) tallied across all chunks.
// ---------------------------------------------------------------------------

async fn materialize_file<C>(
    dest: &Path,
    rel_path: &str,
    mode: u32,
    chunks: &[clw_types::ChunkRef],
    client: &C,
    cache: &LocalCache,
) -> Result<(u64, u64)>
where
    C: CasTransport,
{
    let abs = dest.join(rel_path);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Assemble file content from chunks, tracking cache vs network bytes.
    let mut file_bytes: Vec<u8> = Vec::new();
    let mut bytes_from_cache: u64 = 0;
    let mut bytes_downloaded: u64 = 0;

    for chunk in chunks {
        let (data, from_cache) = fetch_chunk(chunk.digest, client, cache).await?;
        if from_cache {
            bytes_from_cache += chunk.size;
        } else {
            bytes_downloaded += chunk.size;
        }
        file_bytes.extend_from_slice(&data);
    }

    // Write atomically: tmp file in the same dir, then rename.
    let parent = abs.parent().unwrap_or(dest).to_path_buf();
    let abs_path = abs.clone();
    let mode_copy = mode;
    let file_bytes = Arc::new(file_bytes);
    let fba = Arc::clone(&file_bytes);

    spawn_blocking(move || -> Result<()> {
        // Create temp file in the same directory.
        let tmp_path = {
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let tid = std::thread::current().id();
            parent.join(format!(".clw-tmp-{tid:?}-{nanos}"))
        };

        std::fs::write(&tmp_path, fba.as_ref())?;
        std::fs::rename(&tmp_path, &abs_path)?;

        // Set permissions.
        let perms = std::fs::Permissions::from_mode(mode_copy);
        std::fs::set_permissions(&abs_path, perms)?;

        Ok(())
    })
    .await
    .map_err(|e| ClwError::Transport(format!("spawn_blocking join: {e}")))??;

    Ok((bytes_from_cache, bytes_downloaded))
}

// ---------------------------------------------------------------------------
// Internal: fetch one chunk (cache-first)
//
// Returns (data, was_from_cache).
// ---------------------------------------------------------------------------

async fn fetch_chunk<C>(digest: Digest, client: &C, cache: &LocalCache) -> Result<(Vec<u8>, bool)>
where
    C: CasTransport,
{
    // Try local cache first (blocking I/O).
    let cache_ptr = cache as *const LocalCache as usize;
    let cached = spawn_blocking(move || {
        // SAFETY: `cache` is borrowed for the lifetime of `hydrate`, which
        // awaits every future before returning. The raw pointer is valid.
        let cache_ref = unsafe { &*(cache_ptr as *const LocalCache) };
        cache_ref.get(&digest)
    })
    .await
    .map_err(|e| ClwError::Transport(format!("spawn_blocking join: {e}")))??;

    if let Some(data) = cached {
        return Ok((data, true));
    }

    // Cache miss — fetch from remote.
    let data = CasTransport::get(client, &digest).await?;

    // Populate cache (blocking I/O).
    let data_clone = data.clone();
    let cache_ptr2 = cache as *const LocalCache as usize;
    spawn_blocking(move || {
        let cache_ref = unsafe { &*(cache_ptr2 as *const LocalCache) };
        cache_ref.put(&digest, &data_clone)
    })
    .await
    .map_err(|e| ClwError::Transport(format!("spawn_blocking join: {e}")))??;

    Ok((data, false))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{
        AcTransport, CasTransport, ChunkRef, ClwError, Digest, Entry, Manifest, RefRecord, Result,
        MANIFEST_VERSION,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // In-process fake transport
    // -----------------------------------------------------------------------

    struct FakeStore {
        cas: Mutex<HashMap<Digest, Vec<u8>>>,
        ac: Mutex<HashMap<Digest, Vec<u8>>>,
    }

    impl FakeStore {
        fn new() -> Self {
            Self {
                cas: Mutex::new(HashMap::new()),
                ac: Mutex::new(HashMap::new()),
            }
        }

        fn cas_put_raw(&self, digest: Digest, data: Vec<u8>) {
            self.cas.lock().unwrap().insert(digest, data);
        }

        fn ac_put_raw(&self, key: Digest, data: Vec<u8>) {
            self.ac.lock().unwrap().insert(key, data);
        }
    }

    #[async_trait::async_trait]
    impl CasTransport for FakeStore {
        async fn exists(&self, digest: &Digest) -> Result<bool> {
            Ok(self.cas.lock().unwrap().contains_key(digest))
        }

        async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
            self.cas
                .lock()
                .unwrap()
                .get(digest)
                .cloned()
                .ok_or(ClwError::NotFound(*digest))
        }

        async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
            self.cas.lock().unwrap().insert(*digest, bytes);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl AcTransport for FakeStore {
        async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
            Ok(self.ac.lock().unwrap().get(key).cloned())
        }

        async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
            self.ac.lock().unwrap().insert(*key, value);
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_chunk(data: &[u8]) -> (Digest, Vec<u8>) {
        let d = Digest::of_bytes(data);
        (d, data.to_vec())
    }

    /// Build a FakeStore + LocalCache + RefRecord for a manifest; seed stores.
    fn setup_store(
        manifest: &Manifest,
        all_chunks: &[(Digest, Vec<u8>)],
    ) -> (FakeStore, TempDir, LocalCache) {
        let store = FakeStore::new();

        // Put all chunks referenced by the manifest into CAS.
        for entry in &manifest.entries {
            if let Entry::File { chunks, .. } = entry {
                for chunk in chunks {
                    let bytes = all_chunks
                        .iter()
                        .find(|(d, _)| *d == chunk.digest)
                        .map(|(_, b)| b.clone())
                        .expect("chunk data must be provided in all_chunks");
                    store.cas_put_raw(chunk.digest, bytes);
                }
            }
        }

        // Put manifest in CAS.
        let manifest_bytes = manifest.canonical_bytes().unwrap();
        let manifest_digest = Digest::of_bytes(&manifest_bytes);
        store.cas_put_raw(manifest_digest, manifest_bytes.clone());

        // Put RefRecord in AC.
        let record = RefRecord {
            name: "test-ws".into(),
            root: manifest_digest,
            manifest_size: manifest_bytes.len() as u64,
            created_at_unix: 0,
            parent: None,
            tool_version: "0.1.0".into(),
        };
        let rk = clw_types::ref_key("test-ws");
        store.ac_put_raw(rk, record.to_bytes().unwrap());

        // Local cache dir.
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();

        (store, cache_dir, cache)
    }

    // -----------------------------------------------------------------------
    // Test 1: full hydrate (2 files + 1 multi-chunk file + symlink + empty dir)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn full_hydrate_two_files_multichunk_symlink_empty_dir() {
        let data_a = b"hello world".as_ref();
        let data_b = b"second file content".as_ref();
        let chunk1 = b"chunk-one-data".as_ref();
        let chunk2 = b"chunk-two-data".as_ref();

        let (d_a, bytes_a) = make_chunk(data_a);
        let (d_b, bytes_b) = make_chunk(data_b);
        let (d_c1, bytes_c1) = make_chunk(chunk1);
        let (d_c2, bytes_c2) = make_chunk(chunk2);

        let mut entries = vec![
            Entry::Dir {
                path: "empty-dir".into(),
            },
            Entry::File {
                path: "a.txt".into(),
                mode: 0o644,
                size: data_a.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d_a,
                    size: data_a.len() as u64,
                }],
            },
            Entry::File {
                path: "b.txt".into(),
                mode: 0o644,
                size: data_b.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d_b,
                    size: data_b.len() as u64,
                }],
            },
            Entry::File {
                path: "multi.bin".into(),
                mode: 0o755,
                size: (chunk1.len() + chunk2.len()) as u64,
                chunks: vec![
                    ChunkRef {
                        digest: d_c1,
                        size: chunk1.len() as u64,
                    },
                    ChunkRef {
                        digest: d_c2,
                        size: chunk2.len() as u64,
                    },
                ],
            },
            Entry::Symlink {
                path: "link.txt".into(),
                target: "a.txt".into(),
            },
        ];
        entries.sort_by(|a, b| a.path().cmp(b.path()));

        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data_a.len() as u64
                + data_b.len() as u64
                + chunk1.len() as u64
                + chunk2.len() as u64,
            entries,
        };

        let all_chunks = vec![
            (d_a, bytes_a),
            (d_b, bytes_b),
            (d_c1, bytes_c1.clone()),
            (d_c2, bytes_c2.clone()),
        ];

        let (store, _cache_dir, cache) = setup_store(&manifest, &all_chunks);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let report = hydrate(&dest, &store, &cache, &opts).await.unwrap();

        // Verify files.
        assert_eq!(
            std::fs::read(dest.join("a.txt")).unwrap(),
            data_a,
            "a.txt content"
        );
        assert_eq!(
            std::fs::read(dest.join("b.txt")).unwrap(),
            data_b,
            "b.txt content"
        );
        let mut expected_multi = chunk1.to_vec();
        expected_multi.extend_from_slice(chunk2);
        assert_eq!(
            std::fs::read(dest.join("multi.bin")).unwrap(),
            expected_multi,
            "multi.bin content"
        );

        // Verify mode on 0o755 file.
        let meta = std::fs::metadata(dest.join("multi.bin")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o755, "multi.bin mode");

        // Verify symlink.
        let link_target = std::fs::read_link(dest.join("link.txt")).unwrap();
        assert_eq!(link_target.to_str().unwrap(), "a.txt");

        // Verify empty dir.
        assert!(dest.join("empty-dir").is_dir());

        // Verify report counters.
        assert_eq!(report.files, 3, "3 file entries");
        assert_eq!(report.bytes_total, manifest.total_size);
        assert_eq!(report.root, {
            let mb = manifest.canonical_bytes().unwrap();
            Digest::of_bytes(&mb)
        });
    }

    // -----------------------------------------------------------------------
    // Test 2: cache-first — pre-populate some chunks; verify split counters
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cache_first_bytes_from_cache_and_downloaded_split() {
        let data_cached = b"pre-cached blob";
        let data_miss = b"fetched from remote";

        let (d_cached, bytes_cached) = make_chunk(data_cached);
        let (d_miss, bytes_miss) = make_chunk(data_miss);

        let mut entries = vec![
            Entry::File {
                path: "cached.bin".into(),
                mode: 0o644,
                size: data_cached.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d_cached,
                    size: data_cached.len() as u64,
                }],
            },
            Entry::File {
                path: "remote.bin".into(),
                mode: 0o644,
                size: data_miss.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d_miss,
                    size: data_miss.len() as u64,
                }],
            },
        ];
        entries.sort_by(|a, b| a.path().cmp(b.path()));

        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: (data_cached.len() + data_miss.len()) as u64,
            entries,
        };

        let all_chunks = vec![(d_cached, bytes_cached.clone()), (d_miss, bytes_miss)];
        let (store, _cache_dir, cache) = setup_store(&manifest, &all_chunks);

        // Pre-populate cache with the first chunk.
        cache.put(&d_cached, &bytes_cached).unwrap();

        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let report = hydrate(&dest, &store, &cache, &opts).await.unwrap();

        assert_eq!(report.bytes_from_cache, data_cached.len() as u64);
        assert_eq!(report.bytes_downloaded, data_miss.len() as u64);

        // The downloaded chunk must now be in cache.
        assert!(
            cache.has(&d_miss),
            "downloaded chunk must be in cache after hydrate"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3a: non-empty dest → AlreadyExists
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn non_empty_dest_returns_already_exists() {
        let data = b"any";
        let (d, bytes) = make_chunk(data);
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data.len() as u64,
            entries: vec![Entry::File {
                path: "f.txt".into(),
                mode: 0o644,
                size: data.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d,
                    size: data.len() as u64,
                }],
            }],
        };
        let all_chunks = vec![(d, bytes)];
        let (store, _cache_dir, cache) = setup_store(&manifest, &all_chunks);

        // Create a non-empty dest.
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("existing.txt"), b"block").unwrap();

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Io(ref e) if e.kind() == ErrorKind::AlreadyExists),
            "expected AlreadyExists, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3b: missing ref → NotFound
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn missing_ref_returns_not_found() {
        let store = FakeStore::new(); // empty
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("nonexistent-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::NotFound(_)),
            "expected NotFound, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: corrupt manifest bytes → Json/InvalidManifest error surfaces
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn corrupt_manifest_bytes_returns_json_error() {
        let store = FakeStore::new();

        // Put garbage JSON under a valid-but-arbitrary digest.
        let garbage = b"not-valid-json{{{";
        let garbage_digest = Digest::of_bytes(garbage);
        store.cas_put_raw(garbage_digest, garbage.to_vec());

        // Build a RefRecord pointing to the garbage digest.
        let record = RefRecord {
            name: "bad-ws".into(),
            root: garbage_digest,
            manifest_size: garbage.len() as u64,
            created_at_unix: 0,
            parent: None,
            tool_version: "0.1.0".into(),
        };
        let rk = clw_types::ref_key("bad-ws");
        store.ac_put_raw(rk, record.to_bytes().unwrap());

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("bad-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Json(_) | ClwError::InvalidManifest(_)),
            "expected Json or InvalidManifest, got: {err}"
        );
    }
}
