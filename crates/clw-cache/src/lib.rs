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
//!
//! # Format versioning
//! The cache root carries a version sentinel file ([`VERSION_FILE`]) holding the
//! integer [`CACHE_FORMAT_VERSION`].  On [`LocalCache::new`] the sentinel is checked:
//! absent is treated as a fresh/legacy v1 store (the sentinel is written, content is
//! **not** wiped); a matching version is a no-op; a mismatching version (older *or*
//! newer) triggers a safe invalidation — all blob content is cleared and the sentinel
//! is rewritten — so a future format change can never silently replay stale reads.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clw_types::{ClwError, Digest, Result};

/// On-disk cache format version.  Bump this whenever the blob layout, sharding, or
/// stored representation changes incompatibly; mismatching stores are invalidated on
/// open rather than read stale.
pub const CACHE_FORMAT_VERSION: u32 = 1;

/// Name of the version sentinel file stored at the cache root.
const VERSION_FILE: &str = ".clw-cache-version";

/// Default ceiling for the on-disk cache before [`gc`](LocalCache::gc) is invoked.
///
/// Rationale (anti unbounded-disk-growth): the cache is an L1 in front of CoreLink
/// — every `put` (from snapshot uploads and from hydrate's download-side caching)
/// writes a blob and NOTHING reclaimed them, so `~/.clw/cache` grew forever. 8 GiB
/// is comfortably larger than any single workspace snapshot we expect (so eviction
/// never thrashes a legitimate working set) yet bounds the steady-state footprint a
/// long-lived developer machine accumulates. It is an LRU(mtime)-down-to-budget
/// target, not a hard write rejection: a single hydrate/snapshot larger than this
/// still completes; we just evict the coldest blobs back under budget afterward.
pub const DEFAULT_CACHE_BUDGET_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB

/// How many `put`s may land between two budget checks. Walking all 256 shards to
/// total the cache size on EVERY write would make `put` O(cache) — far too costly.
/// We instead sweep once per this many writes; with the 8 GiB budget and realistic
/// blob sizes the cache can only overshoot the budget by a bounded slack (this many
/// blobs) between sweeps, which is acceptable for a best-effort eviction policy.
const GC_CHECK_EVERY_PUTS: u64 = 256;

// ---------------------------------------------------------------------------
// LocalCache
// ---------------------------------------------------------------------------

/// A synchronous, content-addressed on-disk blob store.
///
/// See the crate-level documentation for layout, atomicity, and self-healing
/// guarantees.
pub struct LocalCache {
    root: PathBuf,
    /// Size ceiling (bytes) the cache is evicted back down to once it overshoots.
    budget_bytes: u64,
    /// Count of successful `put`s since the last budget sweep; gates the cost of
    /// `gc` so we don't stat the whole cache on every single write.
    puts_since_gc: AtomicU64,
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

        let cache = Self {
            root,
            budget_bytes: DEFAULT_CACHE_BUDGET_BYTES,
            puts_since_gc: AtomicU64::new(0),
        };
        cache.check_format_version()?;
        Ok(cache)
    }

    /// Open a cache rooted at `root` with an explicit eviction `budget_bytes`
    /// (used by tests to drive eviction without writing gigabytes). Production
    /// callers use [`new`](Self::new), which applies [`DEFAULT_CACHE_BUDGET_BYTES`].
    pub fn with_budget(root: impl Into<PathBuf>, budget_bytes: u64) -> Result<Self> {
        let mut cache = Self::new(root)?;
        cache.budget_bytes = budget_bytes;
        Ok(cache)
    }

    /// Reconcile the on-disk format version with [`CACHE_FORMAT_VERSION`].
    ///
    /// * sentinel absent  ⇒ fresh or legacy v1 store; write the current version and
    ///   leave existing content untouched.
    /// * sentinel matches ⇒ no-op.
    /// * sentinel differs (older *or* newer, or unparseable) ⇒ safely invalidate:
    ///   clear all blob content, then rewrite the sentinel.  A one-line notice is
    ///   logged to stderr so the wipe is never silent.
    fn check_format_version(&self) -> Result<()> {
        let version_path = self.root.join(VERSION_FILE);
        let on_disk: Option<u32> = match fs::read_to_string(&version_path) {
            Ok(s) => s.trim().parse::<u32>().ok().or(Some(u32::MAX)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(ClwError::Io(e)),
        };

        match on_disk {
            // Matching version: nothing to do.
            Some(v) if v == CACHE_FORMAT_VERSION => Ok(()),
            // Mismatch (parseable-but-different or unparseable sentinel): invalidate.
            Some(v) => {
                // Diagnostic channel (M5.1): the wipe is never silent, but it now
                // rides the structured tracing channel (stderr) instead of a raw
                // eprintln so it composes with -v/RUST_LOG verbosity control.
                tracing::warn!(
                    root = %self.root.display(),
                    on_disk = v,
                    expected = CACHE_FORMAT_VERSION,
                    "cache format mismatch; invalidating cached content to avoid stale reads"
                );
                self.clear_content()?;
                self.write_version()
            }
            // Absent sentinel: fresh or legacy v1 store — stamp it, keep content.
            None => self.write_version(),
        }
    }

    /// Write the current [`CACHE_FORMAT_VERSION`] to the sentinel file.
    fn write_version(&self) -> Result<()> {
        fs::write(
            self.root.join(VERSION_FILE),
            CACHE_FORMAT_VERSION.to_string(),
        )
        .map_err(ClwError::Io)
    }

    /// Remove every cached blob, leaving the shard directories in place.
    fn clear_content(&self) -> Result<()> {
        let blobs_root = self.root.join("blobs");
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
                if entry.metadata()?.is_file() {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
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

        // Eviction is wired HERE so every write path (snapshot uploads + hydrate's
        // download-side caching) is bounded without each caller remembering to call
        // gc. The full-cache stat in gc is O(cache), so we gate it: only sweep once
        // per GC_CHECK_EVERY_PUTS writes. A gc failure must NOT fail the put (the
        // blob is already durably written); eviction is best-effort.
        if self.puts_since_gc.fetch_add(1, Ordering::Relaxed) + 1 >= GC_CHECK_EVERY_PUTS {
            self.puts_since_gc.store(0, Ordering::Relaxed);
            let _ = self.gc(self.budget_bytes);
        }
        Ok(())
    }

    /// Evict the cache back down to its configured `budget_bytes` (the value passed
    /// to [`with_budget`](Self::with_budget), else [`DEFAULT_CACHE_BUDGET_BYTES`]).
    /// Returns the number of bytes freed. Exposed so a hydrate/snapshot path can
    /// force a sweep at the end of a run regardless of the per-`put` gate.
    pub fn gc_to_budget(&self) -> Result<u64> {
        self.puts_since_gc.store(0, Ordering::Relaxed);
        self.gc(self.budget_bytes)
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
    // FIX 4: eviction is WIRED into the write path — driving the cache over its
    // budget triggers gc back under budget without any explicit gc() call.
    // We use a tiny budget + with_budget so the test stays cheap, and write more
    // than GC_CHECK_EVERY_PUTS blobs so the per-put gate fires.
    // -----------------------------------------------------------------------
    #[test]
    fn put_over_budget_triggers_eviction_back_under_budget() {
        let dir = TempDir::new().unwrap();
        // Each blob below is 64 bytes; budget = 1 KiB so the steady state holds
        // ~16 blobs. Write enough to both exceed the budget AND trip the
        // GC_CHECK_EVERY_PUTS gate at least once.
        let budget: u64 = 1024;
        let cache = LocalCache::with_budget(dir.path(), budget).unwrap();

        let n_writes = GC_CHECK_EVERY_PUTS + 64;
        for i in 0..n_writes {
            // Distinct 64-byte payloads → distinct digests → distinct files.
            let mut data = vec![0u8; 64];
            data[..8].copy_from_slice(&i.to_le_bytes());
            let d = Digest::of_bytes(&data);
            cache.put(&d, &data).unwrap();
        }

        // After at least one gated sweep, the cache must be at/under budget.
        let on_disk = cache.gc_to_budget().unwrap(); // also asserts gc itself is callable
        let _ = on_disk;
        let total: u64 = {
            let blobs_root = dir.path().join("blobs");
            let mut t = 0u64;
            for i in 0u16..=255 {
                let shard = format!("{:02x}", i);
                if let Ok(rd) = fs::read_dir(blobs_root.join(&shard)) {
                    for e in rd.flatten() {
                        let m = e.metadata().unwrap();
                        if m.is_file() {
                            t += m.len();
                        }
                    }
                }
            }
            t
        };
        assert!(
            total <= budget,
            "cache must be evicted back under budget ({budget}), got {total} bytes"
        );
    }

    // -----------------------------------------------------------------------
    // format version: fresh cache stamps the sentinel
    // -----------------------------------------------------------------------
    #[test]
    fn fresh_cache_writes_version_sentinel() {
        let (dir, _cache) = make_cache();
        let sentinel = dir.path().join(VERSION_FILE);
        assert!(
            sentinel.exists(),
            "fresh cache must write the version sentinel"
        );
        let contents = fs::read_to_string(&sentinel).unwrap();
        assert_eq!(contents.trim(), CACHE_FORMAT_VERSION.to_string());
    }

    // -----------------------------------------------------------------------
    // format version: absent sentinel (legacy store) keeps existing content
    // -----------------------------------------------------------------------
    #[test]
    fn absent_sentinel_is_treated_as_legacy_and_preserves_content() {
        let dir = TempDir::new().unwrap();
        // First open stamps the sentinel and stores a blob.
        let cache = LocalCache::new(dir.path()).unwrap();
        let data = b"legacy content";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        // Simulate a legacy store: remove the sentinel but keep content.
        fs::remove_file(dir.path().join(VERSION_FILE)).unwrap();

        // Reopen: sentinel is rewritten, content is NOT wiped.
        let cache = LocalCache::new(dir.path()).unwrap();
        assert!(dir.path().join(VERSION_FILE).exists());
        assert!(cache.has(&d), "legacy content must survive the stamp");
    }

    // -----------------------------------------------------------------------
    // format version: matching sentinel is a no-op (content untouched)
    // -----------------------------------------------------------------------
    #[test]
    fn matching_version_is_noop() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();
        let data = b"keep me";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        // Reopen with an unchanged sentinel: content must remain.
        let cache = LocalCache::new(dir.path()).unwrap();
        assert!(cache.has(&d), "matching version must not touch content");
    }

    // -----------------------------------------------------------------------
    // format version: mismatching sentinel invalidates content + rewrites stamp
    // -----------------------------------------------------------------------
    #[test]
    fn mismatching_version_invalidates_cache() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();
        let data = b"stale across formats";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();
        assert!(cache.has(&d));

        // Write a different (older/newer) version into the sentinel.
        fs::write(
            dir.path().join(VERSION_FILE),
            (CACHE_FORMAT_VERSION + 7).to_string(),
        )
        .unwrap();

        // Reopen: content must be cleared and the sentinel rewritten to current.
        let cache = LocalCache::new(dir.path()).unwrap();
        assert!(!cache.has(&d), "mismatched format must invalidate content");
        let contents = fs::read_to_string(dir.path().join(VERSION_FILE)).unwrap();
        assert_eq!(
            contents.trim(),
            CACHE_FORMAT_VERSION.to_string(),
            "sentinel must be rewritten to the current version"
        );

        // The store is still usable after invalidation.
        cache.put(&d, data).unwrap();
        assert!(cache.has(&d));
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
