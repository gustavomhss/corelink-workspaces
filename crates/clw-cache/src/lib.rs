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

/// Filename prefix of an in-flight temp file (the temp-file + atomic-rename `put`
/// path). A committed blob is named by its 64-hex digest and NEVER starts with this;
/// only an in-progress (or crash-stranded) write does. `gc` excludes these from both
/// the budget total and the eviction set — counting a half-written temp toward the
/// budget skews eviction, and evicting one would delete another writer's in-flight
/// blob (a stale temp from a crashed writer is harmless and best left alone here).
const TMP_PREFIX: &str = ".tmp-";

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

    /// Remove every committed cached blob, leaving the shard directories — and any
    /// in-flight `.tmp-*` put — in place.
    ///
    /// Concurrency: a shared cache may be cleared by one process while another is
    /// mid-`put`. Two rules keep that safe, mirroring [`gc`](Self::gc):
    /// * **skip `.tmp-*`** — removing another writer's in-flight temp file would make
    ///   its `rename` fail with `ENOENT`; only digest-named (committed) blobs are wiped.
    /// * **treat a concurrently-vanished entry as benign** — if another process removes
    ///   a blob (or the dir entry) between our `read_dir` and our `metadata`/`remove`,
    ///   `NotFound` is success (the goal — that blob being gone — already holds).
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
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(ClwError::Io(e)),
                };
                // Never clobber another writer's in-flight put (matches gc).
                if entry.file_name().to_string_lossy().starts_with(TMP_PREFIX) {
                    continue;
                }
                let is_file = match entry.metadata() {
                    Ok(m) => m.is_file(),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(ClwError::Io(e)),
                };
                if is_file {
                    match fs::remove_file(entry.path()) {
                        Ok(()) => {}
                        // Another process already removed it — benign.
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(ClwError::Io(e)),
                    }
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
            Err(e) => return Err(io_ctx("get: read", &path, e)),
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
        tmp.file
            .write_all(bytes)
            .map_err(|e| io_ctx("put: write", &tmp.path, e))?;
        tmp.file
            .flush()
            .map_err(|e| io_ctx("put: flush", &tmp.path, e))?;
        // Persist and rename.
        let tmp_path = tmp.path.clone();
        drop(tmp.file); // close before rename on Windows
        fs::rename(&tmp_path, &dest).map_err(|e| {
            // Name BOTH paths: under concurrent shared-cache load an ENOENT here is
            // exactly the failure we want to pinpoint (tmp vs. dest-parent).
            ClwError::Io(io::Error::new(
                e.kind(),
                format!(
                    "cache put: rename {} -> {}: {e}",
                    tmp_path.display(),
                    dest.display()
                ),
            ))
        })?;

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
                let entry = match entry {
                    Ok(e) => e,
                    // A concurrent process removing an entry mid-iteration is benign.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(ClwError::Io(e)),
                };
                // Skip in-flight temp files (`.tmp-*`): they are not committed blobs,
                // so counting them toward the budget would skew eviction, and evicting
                // one would clobber another writer's in-progress put. Only digest-named
                // (committed) blobs participate in the budget + eviction.
                if entry.file_name().to_string_lossy().starts_with(TMP_PREFIX) {
                    continue;
                }
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    // Removed by another process between read_dir and metadata — benign.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(ClwError::Io(e)),
                };
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
            match fs::remove_file(&path) {
                Ok(()) => {}
                // Another process (its own gc) already evicted this blob — benign;
                // the space is freed either way, so still count it toward the target.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(ClwError::Io(e)),
            }
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
///
/// # Cross-process uniqueness
/// The temp name combines the PID, thread-id, subsec-nanos, AND a process-wide
/// monotonic [`AtomicU64`] sequence. Two processes sharing the same cache dir
/// (e.g. concurrent `clw` invocations) can otherwise collide on
/// `ThreadId(1)` + the same `subsec_nanos`, which under `create_new(true)`
/// surfaces as a hard `AlreadyExists` error and a flaky `put`. The PID
/// disambiguates across processes and the sequence disambiguates within a
/// process; we still retry once on the (now astronomically unlikely)
/// `AlreadyExists` to absorb any residual same-nanos/same-seq race. This
/// mirrors the `TMP_SEQ` pattern in clw-hydrate's `materialize_file`.
fn tempfile_in(dir: &Path) -> Result<NamedTmpFile> {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Process-wide monotonic counter: guarantees uniqueness between two temp
    // files created in the same process even when they land on the same
    // `subsec_nanos` (thread-id + nanos alone can collide under concurrency).
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    let pid = std::process::id();
    let tid = std::thread::current().id();

    // Try once; on the vanishingly unlikely `AlreadyExists` (two processes on
    // the same PID-space + same nanos + same seq, which cannot happen on a
    // single host) retry with a fresh nanos + seq before giving up.
    let mut last_err = None;
    for _ in 0..2 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("{TMP_PREFIX}{pid}-{tid:?}-{nanos}-{seq}");
        let path = dir.join(&name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok(NamedTmpFile { file, path }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                last_err = Some(e);
                continue;
            }
            // Name the path: a non-AlreadyExists error here (e.g. ENOENT if the
            // shard dir is unexpectedly absent under concurrent load) must say
            // WHICH file/dir, not just an opaque errno.
            Err(e) => return Err(io_ctx("put: create temp", &path, e)),
        }
    }
    let e = last_err.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::AlreadyExists, "temp file name collision")
    });
    // Name the dir on this (astronomically unlikely) retries-exhausted path too,
    // so every tempfile_in exit is diagnosable — preserves the AlreadyExists kind.
    Err(io_ctx("put: create temp (retries exhausted)", dir, e))
}

/// Attach the failing operation + path to an io error while preserving its
/// [`io::ErrorKind`] (so `NotFound`-matching callers — see `gc`/`clear_content`
/// — keep working). This turns an opaque, undiagnosable bare errno like
/// `No such file or directory (os error 2)` into a precise
/// `cache put: create temp ".../blobs/ab/.tmp-…": …`, so a rare CI failure on a
/// shared cache becomes an actionable bug report instead of a guess.
fn io_ctx(op: &str, path: &Path, e: io::Error) -> ClwError {
    ClwError::Io(io::Error::new(
        e.kind(),
        format!("cache {op} {}: {e}", path.display()),
    ))
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
    // FIX #3: gc must NOT count `.tmp-*` in-flight temp files toward the budget,
    // and must NEVER evict them. We seed exactly one committed (digest-named) blob
    // that is WITHIN budget, then plant a large `.tmp-*` file in the same shard.
    // If the temp were counted, the total would exceed the budget and gc would
    // start evicting; instead gc must report 0 freed, leave the committed blob in
    // place, and leave the temp file untouched.
    // -----------------------------------------------------------------------
    #[test]
    fn gc_ignores_tmp_files_in_budget_and_eviction() {
        let dir = TempDir::new().unwrap();
        let budget: u64 = 1024;
        let cache = LocalCache::with_budget(dir.path(), budget).unwrap();

        // One committed blob, comfortably within budget (64 bytes ≪ 1 KiB).
        let data = vec![7u8; 64];
        let d = Digest::of_bytes(&data);
        cache.put(&d, &data).unwrap();
        let committed = cache.blob_path(&d);
        assert!(committed.exists(), "committed blob must be on disk");

        // Plant a large `.tmp-*` temp file in the SAME shard. Its size (4 KiB)
        // alone exceeds the budget, so if gc counted it, eviction would trigger.
        let shard = cache.shard_dir(&d);
        let tmp = shard.join(".tmp-planted-inflight");
        fs::write(&tmp, vec![0u8; 4096]).unwrap();

        // gc must see the cache as within budget (temp excluded) → 0 freed.
        let freed = cache.gc(budget).unwrap();
        assert_eq!(
            freed, 0,
            "the .tmp-* file must not count toward the budget, so nothing is evicted"
        );

        // The committed blob is untouched, AND the temp file is never evicted.
        assert!(committed.exists(), "committed blob must survive gc");
        assert!(
            tmp.exists(),
            "an in-flight .tmp-* file must never be evicted by gc"
        );
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
        // GC_CHECK_EVERY_PUTS gate.
        let budget: u64 = 1024;
        let cache = LocalCache::with_budget(dir.path(), budget).unwrap();

        // Helper: total on-disk bytes across all 256 shards (the real footprint).
        let on_disk_total = || -> u64 {
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

        // Pass-2 audit de-vacuous: the prior version called `cache.gc_to_budget()`
        // IMMEDIATELY before measuring, so the assertion would pass even if eviction
        // were NOT wired into `put` at all (the explicit gc masked a missing gate).
        // We instead write EXACTLY `GC_CHECK_EVERY_PUTS` blobs so the per-put gate
        // fires on the LAST put, then assert the cache is back under budget with NO
        // explicit gc() call — proving the eviction is driven by the WIRED write path.
        let n_writes = GC_CHECK_EVERY_PUTS; // gate fires precisely on the final put
        for i in 0..n_writes {
            // Distinct 64-byte payloads → distinct digests → distinct files.
            let mut data = vec![0u8; 64];
            data[..8].copy_from_slice(&i.to_le_bytes());
            let d = Digest::of_bytes(&data);
            cache.put(&d, &data).unwrap();
        }
        // Sanity: we genuinely overflowed the budget before the gate fired
        // (GC_CHECK_EVERY_PUTS * 64 bytes ≫ 1 KiB), so eviction was actually needed.
        assert!(
            GC_CHECK_EVERY_PUTS * 64 > budget,
            "test must write more than the budget for the assertion to be meaningful"
        );

        // The WIRED gate (inside `put`) must have evicted back under budget — with
        // NO explicit gc_to_budget() call here. A regression that unwires eviction
        // from the write path leaves the cache well over budget and FAILS this.
        let total = on_disk_total();
        assert!(
            total <= budget,
            "the wired per-put eviction gate must bring the cache back under budget \
             ({budget}) with no explicit gc call, got {total} bytes"
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
    // Concurrency: a format-mismatch `clear_content` must NOT clobber another
    // process's in-flight `.tmp-*` put. Removing it would make that writer's
    // rename fail with ENOENT — exactly the kind of spurious cross-process
    // failure a shared cache must never inflict. (Matches gc's `.tmp-*` skip.)
    // -----------------------------------------------------------------------
    #[test]
    fn clear_content_preserves_inflight_tmp_but_wipes_committed_blobs() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();
        let data = b"committed blob";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();
        assert!(cache.has(&d));

        // Simulate a concurrent writer's in-flight put: a `.tmp-*` file sitting in
        // the SAME shard as a committed blob, mid-rename.
        let shard = cache.shard_dir(&d);
        let inflight = shard.join(format!("{TMP_PREFIX}other-writer-inflight"));
        fs::write(&inflight, b"another process's half-written put").unwrap();

        // Force a format mismatch, then reopen → check_format_version → clear_content.
        fs::write(
            dir.path().join(VERSION_FILE),
            (CACHE_FORMAT_VERSION + 7).to_string(),
        )
        .unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();

        // Committed blob wiped; the other writer's in-flight temp file survives.
        assert!(!cache.has(&d), "committed blob must be invalidated");
        assert!(
            inflight.exists(),
            "clear_content must NOT remove another writer's in-flight .tmp-* put"
        );
    }

    // -----------------------------------------------------------------------
    // B2 (crash-consistency): a leftover/partial temp file from a crashed `put`
    // is NEVER read as a real blob and does not corrupt a subsequent get/put.
    //
    // The cache writes via tempfile_in + rename, so a crash between the write and
    // the rename leaves a stray `.tmp-*` file in the shard directory. Because the
    // real blob path is the 64-hex digest name (never a `.tmp-*` name), `get`
    // addresses the digest path directly and the stray is simply not on that
    // path — it is ignored. This test simulates that crash and proves the
    // invariant: the real key still reads back, and the stray never masquerades
    // as content.
    // -----------------------------------------------------------------------
    #[test]
    fn stray_partial_temp_file_is_never_read_as_a_blob() {
        let (_dir, cache) = make_cache();
        let data = b"real blob content";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        // Simulate a crashed put: drop a partial temp file in the SAME shard dir
        // as the real blob, named like the cache's own temp files (.tmp-*). Its
        // bytes are garbage that does NOT hash to any requested digest.
        let shard = cache.shard_dir(&d);
        let stray = shard.join(".tmp-Crashed-12345");
        fs::write(&stray, b"half-written-garbage-from-a-crash").unwrap();
        assert!(stray.exists(), "precondition: the stray temp file exists");

        // The real key still reads back intact — the stray does not interfere.
        let got = cache.get(&d).unwrap();
        assert_eq!(
            got.as_deref(),
            Some(data.as_slice()),
            "the real blob must still read back with a stray temp file present"
        );

        // A digest computed from the STRAY's bytes is not addressable as a blob:
        // its on-disk name is `.tmp-*`, not the 64-hex digest, so `get` of that
        // digest finds nothing (the stray is never mistaken for content).
        let stray_digest = Digest::of_bytes(b"half-written-garbage-from-a-crash");
        assert!(
            cache.get(&stray_digest).unwrap().is_none(),
            "a stray temp file must never be readable as a real blob"
        );

        // A subsequent put of a NEW key still succeeds and is unaffected.
        let data2 = b"a second blob after the crash";
        let d2 = Digest::of_bytes(data2);
        cache.put(&d2, data2).unwrap();
        assert_eq!(cache.get(&d2).unwrap().as_deref(), Some(data2.as_slice()));

        // The stray is still sitting there inert (the cache never touched it),
        // confirming it is simply ignored rather than consumed or corrupting state.
        assert!(
            stray.exists(),
            "the inert stray is left untouched, not consumed"
        );
    }

    #[test]
    fn successful_put_leaves_no_temp_file_behind() {
        // The temp+rename write must consume its temp file: after a successful
        // put, NO `.tmp-*` file remains in the shard directory (the rename moved
        // it to the final digest path). A regression that leaks temp files would
        // slowly fill the cache with un-evictable junk.
        let (_dir, cache) = make_cache();
        let data = b"leaves no temp behind";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        let shard = cache.shard_dir(&d);
        let leftover_tmp: Vec<_> = fs::read_dir(&shard)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".tmp-"))
            .collect();
        assert!(
            leftover_tmp.is_empty(),
            "a successful put must leave no temp file, found: {leftover_tmp:?}"
        );

        // Exactly the one final blob (named by its 64-hex digest) is present.
        let blobs: Vec<_> = fs::read_dir(&shard)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(blobs, vec![d.to_hex()], "only the final blob must remain");
    }

    // -----------------------------------------------------------------------
    // temp-file naming: many rapid creations in the same dir never collide.
    //
    // Audit Low (cross-process temp collision): the prior name was
    // `ThreadId + subsec_nanos` with NO counter, opened `create_new(true)`, so
    // two creations on the same thread within the same nanosecond would hit a
    // hard `AlreadyExists`. The name now carries a process-wide AtomicU64
    // sequence (+ PID), so a tight loop of creations on ONE thread — exactly the
    // same-nanos case the old code could not survive — must all succeed with
    // distinct names.
    // -----------------------------------------------------------------------
    #[test]
    fn tempfile_in_is_collision_free_under_rapid_creation() {
        use std::collections::HashSet;

        let dir = TempDir::new().unwrap();
        let mut names = HashSet::new();
        let mut keep = Vec::new();
        // 1000 rapid creations on a single thread: many will share subsec_nanos,
        // so this fails outright (AlreadyExists) without the AtomicU64 sequence.
        for _ in 0..1000 {
            let tmp = tempfile_in(dir.path()).expect("rapid temp creation must not collide");
            let name = tmp.path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                names.insert(name.clone()),
                "duplicate temp file name generated: {name}"
            );
            // Keep the files (and their handles) alive so create_new sees them on
            // disk — proving uniqueness against EXISTING files, not just names.
            keep.push(tmp);
        }
        assert_eq!(names.len(), 1000);
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

    // -----------------------------------------------------------------------
    // Diagnosability: cache fs errors carry the operation + path (so a rare
    // shared-cache failure is an actionable message, not an opaque `os error 2`),
    // while preserving the original ErrorKind for NotFound-matching callers.
    // -----------------------------------------------------------------------
    #[test]
    fn io_ctx_preserves_kind_and_adds_op_and_path() {
        let raw = io::Error::new(io::ErrorKind::NotFound, "No such file or directory");
        let wrapped = io_ctx("put: rename", Path::new("/blobs/ab/deadbeef"), raw);
        match wrapped {
            ClwError::Io(e) => {
                assert_eq!(
                    e.kind(),
                    io::ErrorKind::NotFound,
                    "kind() must be preserved so gc/clear_content NotFound-matching still works"
                );
                let msg = e.to_string();
                assert!(msg.contains("put: rename"), "must name the op: {msg}");
                assert!(
                    msg.contains("/blobs/ab/deadbeef"),
                    "must name the path: {msg}"
                );
            }
            other => panic!("expected ClwError::Io, got {other:?}"),
        }
    }

    #[test]
    fn put_into_missing_shard_reports_the_path() {
        let (_dir, cache) = make_cache();
        let data = b"payload for a removed shard";
        let d = Digest::of_bytes(data);
        // Remove the (empty, fresh) shard dir this digest maps to, so the put's
        // temp-file create hits ENOENT — the exact opaque failure we want named.
        let shard = cache.shard_dir(&d);
        fs::remove_dir(&shard).unwrap();

        let err = cache.put(&d, data).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cache put") && msg.contains("create temp"),
            "put error must name the operation, got: {msg}"
        );
        assert!(
            msg.contains(&shard.display().to_string()),
            "put error must name the failing shard path, got: {msg}"
        );
    }
}
