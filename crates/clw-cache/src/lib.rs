//! clw-cache — synchronous local content-addressed cache (L1 in front of CoreLink).
//!
//! # Design
//! Blobs are stored under `<root>/blobs/<first-2-hex-chars>/<full-64-hex>` (fan-out
//! sharding to avoid large flat directories).
//!
//! All I/O uses the synchronous `std::fs` API intentionally.  Async callers **must**
//! wrap cache operations in [`tokio::task::spawn_blocking`] (or equivalent).  This
//! keeps the implementation free of async runtimes and usable from sync contexts such
//! as `clap` commands and test harnesses.
//!
//! # Atomicity
//! [`LocalCache::put`] writes to a temporary file **in the same directory** as the
//! final blob path and then renames it into place.  Rename is atomic on POSIX; on
//! Windows it may clobber (acceptable for a local cache).
//!
//! # Self-healing
//! [`LocalCache::get`] re-hashes every blob it reads.  If the hash does not match the
//! requested [`Digest`] (disk corruption, partial write, etc.) the corrupt file is
//! removed and `Ok(None)` is returned so the caller can fetch the blob from the remote.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use clw_types::{ClwError, Digest, Result};

// ---------------------------------------------------------------------------
// LocalCache
// ---------------------------------------------------------------------------

/// A synchronous, content-addressed on-disk blob store.
///
/// See the crate-level documentation for layout, atomicity, and self-healing
/// guarantees.
pub struct LocalCache {
    root: PathBuf,
}

impl LocalCache {
    /// Open (or create) a cache rooted at `root`.
    ///
    /// Creates `<root>/blobs` and the 256 two-char shard directories on first
    /// call so that subsequent operations never need to mkdir at write time.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        // Pre-create all 256 shard directories.
        for i in 0u16..=255 {
            let shard = format!("{:02x}", i);
            fs::create_dir_all(root.join("blobs").join(&shard))?;
        }
        // Workspace content is private: keep the cache root owner-only (0700).
        // Best-effort — ignore the error on exotic filesystems that reject chmod.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(&root, fs::Permissions::from_mode(0o700));
        }
        Ok(Self { root })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn blob_path(&self, d: &Digest) -> PathBuf {
        let hex = d.to_hex();
        self.root.join("blobs").join(&hex[..2]).join(&hex)
    }

    fn shard_dir(&self, d: &Digest) -> PathBuf {
        let hex = d.to_hex();
        self.root.join("blobs").join(&hex[..2])
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Returns `true` iff the blob exists on disk (no integrity check — fast path).
    pub fn has(&self, d: &Digest) -> bool {
        self.blob_path(d).exists()
    }

    /// Read the blob for `d`.
    ///
    /// Returns `Ok(None)` when not cached.  If the file exists but its content
    /// does not hash to `d` (corruption), the file is removed and `Ok(None)` is
    /// returned (self-healing).
    pub fn get(&self, d: &Digest) -> Result<Option<Vec<u8>>> {
        let path = self.blob_path(d);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ClwError::Io(e)),
        };

        let actual = Digest::of_bytes(&bytes);
        if actual != *d {
            // Self-heal: remove the corrupt blob.
            let _ = fs::remove_file(&path);
            return Ok(None);
        }

        Ok(Some(bytes))
    }

    /// Store `bytes` under digest `d`.
    ///
    /// Idempotent: if the blob is already present (same digest ⇒ same content),
    /// this is a no-op.  Writes atomically via temp-file + rename within the
    /// same shard directory.
    pub fn put(&self, d: &Digest, bytes: &[u8]) -> Result<()> {
        let dest = self.blob_path(d);
        if dest.exists() {
            return Ok(());
        }

        let dir = self.shard_dir(d);
        // Write to a named temp file in the same directory so rename is atomic.
        let mut tmp = tempfile_in(&dir)?;
        tmp.file.write_all(bytes)?;
        tmp.file.flush()?;
        // Persist and rename.
        let tmp_path = tmp.path.clone();
        drop(tmp.file); // close before rename on Windows
        fs::rename(&tmp_path, &dest)?;
        Ok(())
    }

    /// Evict blobs until the total cached size is ≤ `max_total_bytes`.
    ///
    /// Blobs are deleted oldest-`mtime`-first.  Returns the number of bytes freed.
    /// If the store is already within budget, returns 0.
    pub fn gc(&self, max_total_bytes: u64) -> Result<u64> {
        // Collect all blob paths with their sizes and mtimes.
        let blobs_root = self.root.join("blobs");
        let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total: u64 = 0;

        for i in 0u16..=255 {
            let shard = format!("{:02x}", i);
            let shard_path = blobs_root.join(&shard);
            let rd = match fs::read_dir(&shard_path) {
                Ok(rd) => rd,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(ClwError::Io(e)),
            };
            for entry in rd {
                let entry = entry?;
                let meta = entry.metadata()?;
                if !meta.is_file() {
                    continue;
                }
                let size = meta.len();
                let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                total += size;
                entries.push((entry.path(), size, mtime));
            }
        }

        if total <= max_total_bytes {
            return Ok(0);
        }

        // Sort oldest-first.
        entries.sort_by_key(|(_, _, mtime)| *mtime);

        let mut freed: u64 = 0;
        for (path, size, _) in entries {
            if total <= max_total_bytes {
                break;
            }
            fs::remove_file(&path)?;
            total -= size;
            freed += size;
        }

        Ok(freed)
    }
}

// ---------------------------------------------------------------------------
// Internal: named temp file in a specific directory
// ---------------------------------------------------------------------------

struct NamedTmpFile {
    file: fs::File,
    path: PathBuf,
}

/// Create a uniquely named temporary file inside `dir`.  The file is NOT
/// deleted on drop — the caller is responsible for renaming or removing it.
fn tempfile_in(dir: &Path) -> Result<NamedTmpFile> {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Use a combination of time + thread-id for uniqueness without external deps.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tid = std::thread::current().id();
    let name = format!(".tmp-{:?}-{}", tid, nanos);
    let path = dir.join(&name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(ClwError::Io)?;
    Ok(NamedTmpFile { file, path })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use tempfile::TempDir;

    fn make_cache() -> (TempDir, LocalCache) {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();
        (dir, cache)
    }

    // -----------------------------------------------------------------------
    // cache root is private (0700): workspace content must not be world-readable
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn cache_root_is_0700() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new().unwrap();
        // Start from a deliberately loose mode so the test proves `new` tightens it
        // rather than merely inheriting whatever TempDir created.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        let _cache = LocalCache::new(dir.path()).unwrap();

        let mode = fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o700, 0o700, "cache root must be owner-rwx");
        assert_eq!(
            mode & 0o077,
            0,
            "cache root must not be group/world accessible"
        );
    }

    // -----------------------------------------------------------------------
    // put / get roundtrip
    // -----------------------------------------------------------------------
    #[test]
    fn put_get_roundtrip() {
        let (_dir, cache) = make_cache();
        let data = b"hello corelink";
        let d = Digest::of_bytes(data);

        cache.put(&d, data).unwrap();
        let got = cache.get(&d).unwrap();
        assert_eq!(got.as_deref(), Some(data.as_slice()));
    }

    // -----------------------------------------------------------------------
    // has
    // -----------------------------------------------------------------------
    #[test]
    fn has_returns_false_then_true() {
        let (_dir, cache) = make_cache();
        let data = b"has test";
        let d = Digest::of_bytes(data);

        assert!(!cache.has(&d));
        cache.put(&d, data).unwrap();
        assert!(cache.has(&d));
    }

    // -----------------------------------------------------------------------
    // idempotent double-put
    // -----------------------------------------------------------------------
    #[test]
    fn double_put_is_idempotent() {
        let (_dir, cache) = make_cache();
        let data = b"idempotent";
        let d = Digest::of_bytes(data);

        cache.put(&d, data).unwrap();
        // Second put must not error or change anything.
        cache.put(&d, data).unwrap();

        let got = cache.get(&d).unwrap().unwrap();
        assert_eq!(got, data);
    }

    // -----------------------------------------------------------------------
    // corruption: get returns Ok(None) and removes the file
    // -----------------------------------------------------------------------
    #[test]
    fn corruption_self_heals() {
        let (_dir, cache) = make_cache();
        let data = b"good data";
        let d = Digest::of_bytes(data);

        cache.put(&d, data).unwrap();

        // Overwrite with garbage.
        let path = cache.blob_path(&d);
        fs::write(&path, b"garbage!garbage!garbage!").unwrap();

        // get must return None (not an error) and remove the file.
        let result = cache.get(&d).unwrap();
        assert!(result.is_none(), "corrupted blob should yield None");
        assert!(
            !path.exists(),
            "corrupted blob file should have been removed"
        );
    }

    // -----------------------------------------------------------------------
    // gc: deletes oldest blobs first, respects cap
    // -----------------------------------------------------------------------
    #[test]
    fn gc_deletes_oldest_first() {
        let (_dir, cache) = make_cache();

        let blobs: &[&[u8]] = &[b"alpha-blob", b"beta--blob", b"gamma-blob"];
        let mut digests = Vec::new();
        for &data in blobs {
            let d = Digest::of_bytes(data);
            cache.put(&d, data).unwrap();
            digests.push(d);
        }

        // Assign distinct mtimes: alpha oldest, gamma newest.
        let base = FileTime::from_unix_time(1_000_000, 0);
        for (i, d) in digests.iter().enumerate() {
            let ft = FileTime::from_unix_time(1_000_000 + i as i64, 0);
            set_file_mtime(cache.blob_path(d), ft).unwrap();
        }
        // Verify baseline: all 3 present.
        assert!(cache.has(&digests[0]));
        assert!(cache.has(&digests[1]));
        assert!(cache.has(&digests[2]));

        // Each blob is 10 bytes → total = 30.  Cap at 20 → free 10 (oldest).
        let freed = cache.gc(20).unwrap();
        assert_eq!(freed, 10, "should have freed exactly one 10-byte blob");

        // alpha (oldest) should be gone; beta and gamma should remain.
        assert!(!cache.has(&digests[0]), "oldest blob should be evicted");
        assert!(cache.has(&digests[1]), "second blob should remain");
        assert!(cache.has(&digests[2]), "newest blob should remain");

        // Suppress unused variable warning.
        let _ = base;
    }

    // -----------------------------------------------------------------------
    // gc: under-cap store frees 0
    // -----------------------------------------------------------------------
    #[test]
    fn gc_under_cap_frees_nothing() {
        let (_dir, cache) = make_cache();
        let data = b"small";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        // 5 bytes total, cap at 1 MiB → nothing evicted.
        let freed = cache.gc(1024 * 1024).unwrap();
        assert_eq!(freed, 0);
        assert!(cache.has(&d));
    }
}
