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
//! # Recency (mtime) and eviction — APPROXIMATE LRU
//! [`LocalCache::gc`] evicts oldest-`mtime`-first, so `mtime` is the cache's
//! recency signal. Both hit paths therefore **refresh** it: a [`LocalCache::get`]
//! hit and a [`LocalCache::put`] of an already-present blob stamp the blob's mtime
//! to "now". Without that refresh the mtime would be frozen at the time of FIRST
//! WRITE and the policy would be FIFO, not LRU — evicting the hottest blobs
//! (fetched earliest, used constantly) first and thrashing the very working set the
//! cache exists to hold.
//!
//! Two deliberate weakenings, both stated so nothing here claims more than it does:
//! * **Approximate.** The stamp is amortised — a blob already refreshed within
//!   [`MTIME_REFRESH_INTERVAL`] is skipped (see [`touch_mtime_if_stale`]), because an
//!   unconditional stamp costs about as much as the read it accompanies. So recency
//!   is resolved to 60s: two blobs used inside the same minute may evict in either
//!   order. It is `approximate LRU`, not exact LRU.
//! * **Best-effort.** A failure to stamp NEVER turns a cache hit into an error; it
//!   only costs that blob some recency credit.
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
/// target — an approximate LRU, because a cache HIT refreshes the blob's mtime (see
/// [`touch_mtime_if_stale`]) rather than leaving it at the write time, to a 60s
/// resolution. It is a target, not a hard write rejection: a single hydrate/snapshot
/// larger than this still completes; we just evict the coldest blobs back under
/// budget afterward.
pub const DEFAULT_CACHE_BUDGET_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB

/// Name of the environment knob that overrides [`DEFAULT_CACHE_BUDGET_BYTES`].
///
/// A raw **byte count** (u64). A developer whose legitimate working set exceeds the
/// 8 GiB default (large monorepos, many parallel snapshots) can raise it so the cache
/// stops evicting warm blobs; a machine tight on disk can lower it. Mirrors how the
/// other `CLW_*` knobs are read from the environment.
pub const CACHE_BUDGET_ENV: &str = "CLW_CACHE_BUDGET_BYTES";

/// Resolve the cache eviction budget (bytes) from a raw `CLW_CACHE_BUDGET_BYTES`
/// value. Unset / empty / unparseable / `0` → [`DEFAULT_CACHE_BUDGET_BYTES`]; an
/// explicit positive value overrides it verbatim (the eviction target is only ever
/// *compared* against summed blob sizes, so there is no overflow to clamp — an
/// intentionally small value simply means aggressive eviction, and eviction is a
/// down-to-budget sweep *after* an op, never a write rejection, so even a tiny budget
/// never breaks a snapshot/hydrate larger than it). Pure over its input so the parse
/// is unit-testable without touching the environment.
fn resolve_cache_budget_bytes(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&b| b > 0)
        .unwrap_or(DEFAULT_CACHE_BUDGET_BYTES)
}

/// Read the cache eviction budget (bytes) from the [`CACHE_BUDGET_ENV`] environment
/// knob, mirroring how the other `CLW_*` knobs are read at use time.
fn cache_budget_from_env() -> u64 {
    resolve_cache_budget_bytes(std::env::var(CACHE_BUDGET_ENV).ok().as_deref())
}

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
    ///
    /// The eviction budget is [`DEFAULT_CACHE_BUDGET_BYTES`] unless the
    /// [`CACHE_BUDGET_ENV`] (`CLW_CACHE_BUDGET_BYTES`) knob overrides it. Tests that
    /// need a deterministic budget use [`with_budget`](Self::with_budget), which sets
    /// it explicitly and is unaffected by the environment.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let blobs = root.join("blobs");
        // Pre-create all 256 shard directories.
        for i in 0u16..=255 {
            let shard = format!("{:02x}", i);
            fs::create_dir_all(blobs.join(&shard))?;
        }
        // Workspace content is private: harden the whole cache tree (root + blobs
        // + every shard dir) to owner-only (0700) on EVERY init — not only via the
        // config writers. A user who only ever runs snapshot/hydrate (never `auth
        // login`/`init`, which is where `~/.clw` normally gets hardened) would
        // otherwise leave the cache tree at the umask default (0755), letting
        // another local uid read `~/.clw/cache/blobs/**`. `create_dir_all` above
        // uses umask-derived perms, so we tighten explicitly here.
        // Best-effort — ignore the error on exotic filesystems that reject chmod.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let private = fs::Permissions::from_mode(0o700);
            let _ = fs::set_permissions(&root, private.clone());
            let _ = fs::set_permissions(&blobs, private.clone());
            for i in 0u16..=255 {
                let shard = format!("{:02x}", i);
                let _ = fs::set_permissions(blobs.join(&shard), private.clone());
            }
        }

        let cache = Self {
            root,
            budget_bytes: cache_budget_from_env(),
            puts_since_gc: AtomicU64::new(0),
        };
        cache.check_format_version()?;
        Ok(cache)
    }

    /// Open a cache rooted at `root` with an explicit eviction `budget_bytes`
    /// (used by tests to drive eviction without writing gigabytes, deterministically
    /// and independent of the environment). Production callers use [`new`](Self::new),
    /// which applies [`DEFAULT_CACHE_BUDGET_BYTES`] or the [`CACHE_BUDGET_ENV`] override.
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

    /// Refresh a blob's recency (mtime) after a cache HIT, amortised: a blob stamped
    /// within the last [`MTIME_REFRESH_INTERVAL`] is left alone. See
    /// [`touch_mtime_if_stale`].
    fn touch(&self, path: &Path) {
        touch_mtime_if_stale(path);
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
    ///
    /// A HIT refreshes the blob's mtime so [`gc`](Self::gc)'s oldest-first eviction
    /// is an approximate LRU rather than FIFO (see the crate-level "Recency"
    /// section). The refresh is amortised to [`MTIME_REFRESH_INTERVAL`] and is
    /// best-effort — it can never fail the read.
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

        // Verified HIT ⇒ stamp recency. Best-effort: `touch` swallows every error,
        // so a read-only mount / foreign-owned blob / exotic fs still returns the
        // bytes. (Deliberately AFTER verification: a corrupt blob is removed above
        // and must never be credited with recency.)
        self.touch(&path);

        Ok(Some(bytes))
    }

    /// Store `bytes` under digest `d`.
    ///
    /// Idempotent: if the blob is already present (same digest ⇒ same content),
    /// the content write is skipped — but the blob's mtime is still refreshed, since
    /// re-putting a blob is a use of it and must not leave it looking cold to
    /// [`gc`](Self::gc). Writes atomically via temp-file + rename within the
    /// same shard directory.
    pub fn put(&self, d: &Digest, bytes: &[u8]) -> Result<()> {
        let dest = self.blob_path(d);
        if dest.exists() {
            // Already-present: no content write, but this IS a use — stamp recency
            // (amortised + best-effort; never fails the put).
            self.touch(&dest);
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
        // Create the temp file owner-only (0600) on unix so the committed blob it
        // becomes (via atomic rename, which preserves mode) is never group/world
        // readable — even if the containing dir's 0700 hardening was defeated. The
        // umask can only clear bits, and 0600 has no group/other bits, so the file
        // lands at exactly 0600 regardless of the caller's umask.
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        match opts.open(&path) {
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

/// Recency granularity: a blob whose mtime is already newer than this is NOT
/// re-stamped on a hit.
///
/// This is the amortisation that keeps the hit path cheap. Stamping unconditionally
/// costs an `open(O_WRONLY)` + `futimens` on EVERY hit, which measured at roughly the
/// cost of the whole read+BLAKE3-rehash a `get` already pays (a ~2x hit-path
/// regression on macOS/APFS, where the write-mode `open` dominates). Checking first
/// costs only a `stat`, which measured one to two orders of magnitude cheaper than
/// that `open` (~12us vs ~644us per file on APFS), and in steady
/// state (a blob read many times inside one hydrate/run) the stamp is skipped
/// entirely. The price is that recency is tracked to a 60s granularity — two blobs
/// used within the same minute may evict in either order — which is why the policy
/// is documented as **approximate** LRU. 60s is far finer than the timescale on
/// which an 8 GiB budget actually evicts, and infinitely finer than the FIFO it
/// replaces.
const MTIME_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Stamp `path`'s mtime to *now* **iff** it is older than [`MTIME_REFRESH_INTERVAL`]
/// — the amortised form of [`touch_mtime`] used on the hit path.
///
/// The `stat` is best-effort in both directions: if it fails, or the timestamp is in
/// the future (clock skew / a restored file), we fall through to the unconditional
/// stamp, which pulls the blob back onto a sane "now".
fn touch_mtime_if_stale(path: &Path) {
    let already_fresh = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        // `duration_since` errors when the mtime is in the FUTURE (clock skew, a
        // restored file); that is not "fresh", so it falls through to the stamp.
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < MTIME_REFRESH_INTERVAL);
    if already_fresh {
        return; // skip the write-mode open entirely — this is the amortisation
    }
    touch_mtime(path);
}

/// Stamp `path`'s mtime to *now* — the cache's recency signal for
/// [`LocalCache::gc`]'s oldest-first eviction.
///
/// # Why this exists
/// `gc` sorts by mtime, but nothing else in the cache ever writes a blob twice
/// (`get` only reads; a repeat `put` skips the content write), so without this the
/// mtime would be frozen at first-write time and eviction would be FIFO — throwing
/// out the hottest blobs first.
///
/// # Best-effort by construction
/// Every failure is swallowed. The blob is already read/committed at the call site;
/// losing a timestamp update only costs that blob some recency credit, whereas
/// propagating the error would turn a perfectly good cache HIT into a hard failure.
/// Real cases that fail here: a read-only mount, a blob owned by another uid in a
/// shared cache, a filesystem without settable timestamps, or a concurrent evictor
/// removing the blob between our read and this call.
///
/// Uses `std::fs::File::set_modified` (stable since 1.75) deliberately — the
/// `filetime` crate stays a DEV-dependency only, so this adds no runtime
/// supply-chain surface. The handle is opened `write(true)` (no truncate, no
/// create) because POSIX `futimens` with explicit times needs write access.
fn touch_mtime(path: &Path) {
    if let Ok(f) = fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
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
    // CLW_CACHE_BUDGET_BYTES — the resolver (pure, no env) + one guarded wiring
    // test proving `new()` actually reads the knob
    // -----------------------------------------------------------------------
    #[test]
    fn cache_budget_unset_or_invalid_falls_back_to_default() {
        // Unset, empty, whitespace, non-numeric, negative, and 0 all → default.
        for raw in [
            None,
            Some(""),
            Some("   "),
            Some("lots"),
            Some("-1"),
            Some("0"),
        ] {
            assert_eq!(
                resolve_cache_budget_bytes(raw),
                DEFAULT_CACHE_BUDGET_BYTES,
                "raw {raw:?} should resolve to the default budget",
            );
        }
    }

    #[test]
    fn cache_budget_positive_value_overrides_verbatim() {
        // A raise (20 GiB) and a lower (100 MiB) are both honored exactly.
        assert_eq!(
            resolve_cache_budget_bytes(Some("21474836480")),
            20 * 1024 * 1024 * 1024,
        );
        assert_eq!(
            resolve_cache_budget_bytes(Some("104857600")),
            100 * 1024 * 1024
        );
        // Surrounding whitespace is trimmed before parsing.
        assert_eq!(resolve_cache_budget_bytes(Some("  1024  ")), 1024);
        // A value distinct from the default proves it isn't silently defaulting.
        assert_ne!(
            resolve_cache_budget_bytes(Some("1024")),
            DEFAULT_CACHE_BUDGET_BYTES
        );
    }

    /// Save/restore `CLW_CACHE_BUDGET_BYTES` around a test that mutates it, so the
    /// process env is left as it was found (mirrors `config.rs`'s `HomeEnvGuard`).
    struct BudgetEnvGuard(Option<std::ffi::OsString>);
    impl BudgetEnvGuard {
        fn new() -> Self {
            Self(std::env::var_os(CACHE_BUDGET_ENV))
        }
    }
    impl Drop for BudgetEnvGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var(CACHE_BUDGET_ENV, v),
                None => std::env::remove_var(CACHE_BUDGET_ENV),
            }
        }
    }

    #[test]
    fn new_reads_the_budget_env_knob() {
        // Proves the wiring: `new()` must actually resolve `CLW_CACHE_BUDGET_BYTES`,
        // not hard-code the default (kills the `budget_bytes: DEFAULT` mutation).
        // We use a LARGE distinctive value (12 GiB): even if the runner interleaves
        // another `new()`-based test while the var is set, a 12 GiB budget triggers no
        // eviction, so the leak is inert — and no other test reads/writes this var, so
        // our own assertion is deterministic. The guard restores the env on drop.
        const OVERRIDE: u64 = 12 * 1024 * 1024 * 1024; // 12 GiB, ≠ the 8 GiB default
        let _guard = BudgetEnvGuard::new();
        std::env::set_var(CACHE_BUDGET_ENV, OVERRIDE.to_string());

        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();
        assert_eq!(
            cache.budget_bytes, OVERRIDE,
            "LocalCache::new must apply the CLW_CACHE_BUDGET_BYTES override, not the default",
        );
        assert_ne!(
            cache.budget_bytes, DEFAULT_CACHE_BUDGET_BYTES,
            "the override must be distinct from the default for this test to be meaningful",
        );
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
    // LOW-cache-perms: a committed blob file is owner-only (0600) and the cache
    // root is 0700 — so a snapshot/hydrate-only user (who never runs `auth
    // login`/`init`) does NOT leak content-addressed workspace file content to
    // another local uid. Without the fix, blob temp files inherit the umask
    // default (typically 0644, world-readable) and the root stays 0755.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn put_blob_is_0600_and_cache_root_is_0700() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new().unwrap();
        // Start from a deliberately loose root mode so the assertion proves `new`
        // tightens it rather than merely inheriting a private TempDir.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let cache = LocalCache::new(dir.path()).unwrap();

        let data = b"secret content-addressed workspace bytes";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        // The committed blob must be owner-only 0600 (no group/other bits).
        let blob = cache.blob_path(&d);
        let blob_mode = fs::metadata(&blob).unwrap().permissions().mode();
        assert_eq!(
            blob_mode & 0o777,
            0o600,
            "committed blob must be owner-only 0600, got {:o}",
            blob_mode & 0o777
        );
        assert_eq!(
            blob_mode & 0o077,
            0,
            "committed blob must not be group/world readable"
        );

        // The cache root must be hardened to 0700 on the snapshot/hydrate path.
        let root_mode = fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(
            root_mode & 0o777,
            0o700,
            "cache root must be owner-only 0700, got {:o}",
            root_mode & 0o777
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
    // M22 (LRU, not FIFO): the recency helpers.
    //
    // FLAKINESS NOTE: `entries.sort_by_key` is STABLE, so blobs sharing an mtime
    // tie-break on directory-walk order across the 256 shards — arbitrary. On a
    // coarse-granularity fs (HFS+ 1s, some overlays, NFS) a tight write loop hands
    // several blobs the SAME mtime and any "which one got evicted" assertion becomes
    // a coin flip. So these tests NEVER rely on write order or on the clock: every
    // mtime is set EXPLICITLY, 60s apart, via `filetime::set_file_mtime` (a
    // dev-dependency; production code uses `File::set_modified`).
    // -----------------------------------------------------------------------

    /// Seed `n` distinct 10-byte blobs with mtimes 60s apart, oldest first.
    /// Returns the digests in age order (index 0 = oldest).
    fn seed_aged_blobs(cache: &LocalCache, n: usize) -> Vec<Digest> {
        const BASE_UNIX: i64 = 1_600_000_000; // 2020-09-13, safely in the past
        let mut digests = Vec::with_capacity(n);
        for i in 0..n {
            let data = format!("aged-blob-{i:03}").into_bytes();
            assert_eq!(data.len(), 13, "payloads must be uniform for the size math");
            let d = Digest::of_bytes(&data);
            cache.put(&d, &data).unwrap();
            // 60s spacing dwarfs any filesystem timestamp granularity.
            let ft = FileTime::from_unix_time(BASE_UNIX + 60 * i as i64, 0);
            set_file_mtime(cache.blob_path(&d), ft).unwrap();
            digests.push(d);
        }
        digests
    }

    fn mtime_of(cache: &LocalCache, d: &Digest) -> std::time::SystemTime {
        fs::metadata(cache.blob_path(d))
            .unwrap()
            .modified()
            .unwrap()
    }

    #[test]
    fn get_refreshes_mtime_so_eviction_is_lru_not_fifo() {
        // The M22 regression: `get` used to read without touching the file, so mtime
        // stayed the FIRST-WRITE time and eviction was FIFO — the hottest blob (the
        // one fetched earliest and read constantly) was evicted FIRST. Here the
        // oldest blob is the one being USED; after a `get` it must be the SAFEST.
        let (_dir, cache) = make_cache();
        let digests = seed_aged_blobs(&cache, 3); // 13 bytes each → 39 total

        let before = mtime_of(&cache, &digests[0]);

        // USE the oldest blob.
        let got = cache.get(&digests[0]).unwrap();
        assert_eq!(
            got.as_deref(),
            Some(b"aged-blob-000".as_slice()),
            "the get itself must still return the bytes"
        );

        // Proof the bump actually happened (not survival for some unrelated reason).
        let after = mtime_of(&cache, &digests[0]);
        assert!(
            after > before,
            "get() on a HIT must strictly advance the blob's mtime: {before:?} -> {after:?}"
        );

        // 39 bytes total, cap at 30 → gc must free exactly one 13-byte blob.
        let freed = cache.gc(30).unwrap();
        assert_eq!(freed, 13, "gc should free exactly one blob");

        // LRU: the ACCESSED blob survives; the next-oldest (untouched) is evicted.
        assert!(
            cache.has(&digests[0]),
            "the recently-read blob must SURVIVE (FIFO would have evicted it)"
        );
        assert!(
            !cache.has(&digests[1]),
            "the next-oldest, unaccessed blob must be the one evicted"
        );
        assert!(cache.has(&digests[2]), "the newest blob must survive");
    }

    #[test]
    fn put_of_present_blob_refreshes_mtime() {
        // Symmetric case: a repeat `put` skips the content write (idempotent), but it
        // is still a USE — it must not leave the blob looking cold to gc.
        let (_dir, cache) = make_cache();
        let digests = seed_aged_blobs(&cache, 3);

        let before = mtime_of(&cache, &digests[0]);

        // Re-put the oldest blob with identical bytes (the early-return path).
        cache.put(&digests[0], b"aged-blob-000").unwrap();

        let after = mtime_of(&cache, &digests[0]);
        assert!(
            after > before,
            "put() of an already-present blob must strictly advance its mtime: \
             {before:?} -> {after:?}"
        );

        // And it drives the same LRU outcome as a get.
        let freed = cache.gc(30).unwrap();
        assert_eq!(freed, 13);
        assert!(cache.has(&digests[0]), "the re-put blob must survive");
        assert!(!cache.has(&digests[1]), "the next-oldest must be evicted");
        assert!(cache.has(&digests[2]));
    }

    #[test]
    fn mtime_refresh_is_amortised_within_the_interval() {
        // The stamp is APPROXIMATE LRU by design: a blob refreshed inside
        // MTIME_REFRESH_INTERVAL is left alone, so the hot path pays a `stat` instead
        // of an `open(O_WRONLY)`+`futimens` on every single hit. This pins that
        // contract in both directions — stale blob bumps, fresh blob does not — so a
        // future change to the interval cannot silently make the docs a lie.
        let (_dir, cache) = make_cache();
        let data = b"amortised";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();

        // 1) Force STALE (an hour old, ≫ the 60s interval) → the hit must bump.
        let stale = FileTime::from_unix_time(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                - 3600,
            0,
        );
        set_file_mtime(cache.blob_path(&d), stale).unwrap();
        let before = mtime_of(&cache, &d);
        cache.get(&d).unwrap().unwrap();
        let after_stale = mtime_of(&cache, &d);
        assert!(
            after_stale > before,
            "a stale blob must be re-stamped on a hit: {before:?} -> {after_stale:?}"
        );

        // 2) Now it is FRESH (just stamped) → further hits must NOT re-stamp.
        for _ in 0..3 {
            cache.get(&d).unwrap().unwrap();
            cache.put(&d, data).unwrap();
        }
        assert_eq!(
            mtime_of(&cache, &d),
            after_stale,
            "a blob stamped within MTIME_REFRESH_INTERVAL must not be re-stamped \
             (this is the hot-path amortisation)"
        );
    }

    #[test]
    fn touch_mtime_never_fails_on_a_missing_path() {
        // The helper is best-effort by contract: it must be a silent no-op for any
        // path it cannot stamp (missing file, read-only mount, foreign-owned blob),
        // because a cache HIT must never degrade into an error.
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("definitely-not-here");
        touch_mtime(&missing);

        // And a `get` of a blob whose mtime cannot be advanced still returns bytes.
        let cache = LocalCache::new(dir.path()).unwrap();
        let data = b"still readable";
        let d = Digest::of_bytes(data);
        cache.put(&d, data).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // 0400: no write bit → the write(true) open inside touch_mtime fails.
            fs::set_permissions(cache.blob_path(&d), fs::Permissions::from_mode(0o400)).unwrap();
        }
        assert_eq!(
            cache.get(&d).unwrap().as_deref(),
            Some(data.as_slice()),
            "a failed mtime refresh must NEVER turn a cache hit into an error"
        );
        // LOAD-BEARING: the stamp must never CREATE. `touch_mtime` opens with
        // `.write(true)` and deliberately NOT `.create(true)`, which is the only
        // reason a stamp racing `gc`'s unlink cannot resurrect an evicted blob as
        // a zero-byte file named after a digest. A future "fix" that adds
        // `.create(true)` to silence the swallowed ENOENT would keep every other
        // assertion here green, so pin it explicitly.
        assert!(
            !missing.exists(),
            "touch_mtime must never create the path — that would resurrect an evicted blob"
        );
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
