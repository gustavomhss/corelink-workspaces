//! clw-hydrate — ref → manifest → cache-first materialize pipeline.
//!
//! Reads a workspace ref from the AC store, fetches the manifest from CAS,
//! then materialises every entry (files, symlinks, empty dirs) into `dest`
//! with file-level concurrency bounded by [`HydrateOptions::concurrency`].
//!
//! All network calls use in-process fakes in tests — never the real endpoint.

pub use clw_types as types;

// The Windows copy-fallback's self-reference guard. Deliberately NOT
// `cfg(windows)`: it is a plain, target-independent predicate so its table test
// runs under a normal `cargo test` on every platform (see the module docs).
mod path_containment;

use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use clw_cache::LocalCache;
use clw_types::{
    AcTransport, CasTransport, ClwError, Digest, Entry, Manifest, RefDomain, RefRecord, Result,
};

// ---------------------------------------------------------------------------
// Resource-safety / abuse bounds (M1.6)
//
// A REMOTE manifest is untrusted input: it may be malicious or pathological.
// Before doing *any* filesystem work we bound what a single hydrate is allowed
// to attempt, so a hostile manifest cannot drive us into OOM, an enumeration
// bomb, a petabyte disk-fill request, or an unbounded symlink chain. These caps
// are intentionally generous (far above any plausible real workspace) but
// finite, and they live in *this* crate as policy — they do NOT touch the
// frozen clw-types contract.
// ---------------------------------------------------------------------------

/// Maximum number of manifest entries a single hydrate will accept.
///
/// Rationale (anti-OOM / anti-enumeration-bomb): every entry is iterated, so the
/// entry count bounds the manifest bookkeeping and number of filesystem
/// operations. One million entries is larger than any real workspace we expect
/// (the Linux kernel tree is ~80k files) yet small enough that the per-entry
/// bookkeeping stays well within memory. A manifest above this cap is rejected
/// up-front, before we allocate the file-entry vector or touch the disk.
const MAX_MANIFEST_ENTRIES: usize = 1_000_000;

/// Maximum total declared bytes (`Manifest::total_size`) a single hydrate will
/// accept.
///
/// Rationale (anti disk-fill): `total_size` is attacker-controlled. Without a
/// ceiling a manifest could *declare* a petabyte and we would happily start
/// writing toward filling the disk. 1 TiB is far above any realistic single
/// workspace snapshot yet bounds the damage a malicious manifest can request.
/// We reject before materialisation; this is a declared-size sanity bound, not
/// a substitute for the OS running out of space mid-write (that still surfaces
/// as a normal `ClwError::Io`).
const MAX_HYDRATE_TOTAL_BYTES: u64 = 1 << 40; // 1 TiB

/// Maximum total number of `ChunkRef`s across the whole manifest (summed over
/// every `File` entry).
///
/// Rationale (anti-OOM): `MAX_MANIFEST_ENTRIES` bounds the entry COUNT but NOT the
/// chunk list WITHIN one entry — a single `File` entry with a billion-element
/// `chunks` Vec defeats the per-entry allocation bound. With the snapshot
/// chunker's minimum chunk size, 1 TiB of content (`MAX_HYDRATE_TOTAL_BYTES`) is on
/// the order of tens of millions of chunks; 64M is generously above any legitimate
/// manifest yet finite. Enforced up-front, before the file-entry vector is built.
const MAX_TOTAL_CHUNKS: u64 = 64_000_000;

/// Maximum number of `ChunkRef`s a SINGLE `File` entry may declare. Bounds the
/// per-file `chunks` clone independently of the manifest-wide `MAX_TOTAL_CHUNKS`,
/// so a 1-entry manifest cannot smuggle an enormous chunk list past the entry cap.
/// A file at the 1 TiB ceiling is far below this in realistic chunkings; 16M is a
/// generous per-file headroom that still bounds a hostile single entry.
const MAX_CHUNKS_PER_FILE: usize = 16_000_000;

/// Enforce the M1.6 abuse caps on an untrusted manifest's declared size and
/// symlink count. Pure (takes counts, not the `Manifest`) so it is testable at
/// the boundary without building a million-entry manifest. Run before any
/// allocation or FS work so a pathological manifest never produces a partial
/// destination.
///
/// `real_total_bytes` is the AUTHORITATIVE size — the sum of every `ChunkRef.size`
/// across every `File` entry — NOT the attacker-controlled declared `total_size`.
/// The declared figure is checked for AGREEMENT separately (see
/// `summed_chunk_bytes` / the integrity check in `hydrate`); the disk-fill budget
/// is enforced against the real number so a manifest declaring `total_size = 0`
/// while its chunks sum to gigabytes cannot bypass the guard.
fn enforce_resource_bounds(
    entry_count: usize,
    real_total_bytes: u64,
    symlink_count: usize,
    total_chunks: u64,
) -> Result<()> {
    if entry_count > MAX_MANIFEST_ENTRIES {
        return Err(ClwError::InvalidManifest(format!(
            "manifest has {entry_count} entries, exceeding the hydrate cap of \
             {MAX_MANIFEST_ENTRIES} (anti-OOM/enumeration-bomb guard); refusing to hydrate"
        )));
    }
    if real_total_bytes > MAX_HYDRATE_TOTAL_BYTES {
        return Err(ClwError::InvalidManifest(format!(
            "manifest's chunks sum to {real_total_bytes} bytes, exceeding the hydrate cap of \
             {MAX_HYDRATE_TOTAL_BYTES} bytes (anti disk-fill guard); refusing to hydrate"
        )));
    }
    if symlink_count > MAX_SYMLINK_ENTRIES {
        return Err(ClwError::InvalidManifest(format!(
            "manifest declares {symlink_count} symlinks, exceeding the hydrate cap of \
             {MAX_SYMLINK_ENTRIES} (anti fan-out/count guard); refusing to hydrate"
        )));
    }
    if total_chunks > MAX_TOTAL_CHUNKS {
        return Err(ClwError::InvalidManifest(format!(
            "manifest declares {total_chunks} chunks total, exceeding the hydrate cap of \
             {MAX_TOTAL_CHUNKS} (anti-OOM chunk-count guard); refusing to hydrate"
        )));
    }
    Ok(())
}

/// Reject a single `File` entry whose `chunks` list exceeds [`MAX_CHUNKS_PER_FILE`]
/// (FIX 6). Pure (takes the count, not the Vec) so it is testable at the boundary
/// without allocating a 16M-element chunk list.
fn enforce_per_file_chunk_cap(path: &str, chunk_count: usize) -> Result<()> {
    if chunk_count > MAX_CHUNKS_PER_FILE {
        return Err(ClwError::InvalidManifest(format!(
            "file entry {path:?} declares {chunk_count} chunks, exceeding the per-file cap of \
             {MAX_CHUNKS_PER_FILE} (anti-OOM chunk-count guard); refusing to hydrate"
        )));
    }
    Ok(())
}

/// Maximum number of symlink entries a single hydrate will create.
///
/// Rationale (anti fan-out / count blow-up): a manifest could declare an
/// enormous number of links. validate_symlink_target already forbids absolute
/// paths, `..`, and drive letters, so a link cannot escape `dest`; this cap
/// additionally bounds the *count* of links a single hydrate will create. It is
/// purely a fan-out/count bound, NOT a cycle guard — hydrate never resolves
/// symlink chains (it only creates the links verbatim), so there is no chain to
/// guard against here. The count is computed up-front in `enforce_resource_bounds`
/// and rejected before any filesystem work, so a pathological manifest never
/// produces a partial destination.
const MAX_SYMLINK_ENTRIES: usize = 100_000;

/// Maximum number of fetched chunks retained for one file before they are
/// handed to its blocking writer. This is deliberately independent of the
/// user-selected file/request concurrency: at most this many 4 MiB chunks are
/// being fetched, plus the one-element writer queue, per materialised file.
const PER_FILE_FETCH_CAP: usize = 8;

/// The producer-to-writer handoff is intentionally tiny. It applies
/// backpressure to fetches while allowing one write to overlap the next fetch.
const FILE_WRITE_QUEUE_CAPACITY: usize = 1;

fn normalized_concurrency(concurrency: usize) -> usize {
    concurrency.max(1)
}

fn per_file_fetch_concurrency(concurrency: usize) -> usize {
    normalized_concurrency(concurrency).min(PER_FILE_FETCH_CAP)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options that control a single hydrate run.
pub struct HydrateOptions {
    pub workspace_name: String,
    pub concurrency: usize,
    /// Ref keyspace the ref is looked up in (default [`RefDomain::User`]).
    pub ref_domain: RefDomain,
    /// Hydrate DIRECTLY from this manifest digest, skipping the AC name→ref
    /// resolution entirely. When `Some`, `workspace_name`/`ref_domain` are not
    /// used to fetch a [`RefRecord`] — the digest IS the root. The fetched
    /// manifest is still verified to hash to this digest (self-verifying: the
    /// ref equals the content key), so a hostile/buggy CAS cannot substitute a
    /// different manifest. This is the seam a content-addressed caller uses when
    /// it already holds the manifest digest (e.g. `toolchain_ref = <digest>`).
    pub root_override: Option<Digest>,
}

impl HydrateOptions {
    /// Create options for `name` with the default concurrency (8).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            workspace_name: name.into(),
            concurrency: 8,
            ref_domain: RefDomain::User,
            root_override: None,
        }
    }

    /// Create options that hydrate DIRECTLY from a manifest `root` digest,
    /// bypassing the named-ref AC lookup. The fetched manifest is verified to
    /// hash to `root` (the digest is self-verifying).
    pub fn from_manifest_digest(root: Digest) -> Self {
        Self {
            workspace_name: String::new(),
            concurrency: 8,
            ref_domain: RefDomain::User,
            root_override: Some(root),
        }
    }

    /// Select the ref keyspace (e.g. [`RefDomain::Runner`] for runner-produced refs).
    #[must_use]
    pub fn with_ref_domain(mut self, ref_domain: RefDomain) -> Self {
        self.ref_domain = ref_domain;
        self
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
        // Allow an *empty* directory; reject anything else. A real read_dir
        // failure (permission denied, transient IO error on `dest`) must
        // surface AS an IO/permission error — NOT be silently collapsed into
        // "destination not empty", which would misreport the cause. The old
        // `.map(...).unwrap_or(false)` swallowed exactly that error.
        if !dest.is_dir() {
            return Err(ClwError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                "destination not empty",
            )));
        }
        // `?` propagates a genuine open/permission error on `dest`.
        let mut rd = std::fs::read_dir(dest)?;
        match rd.next() {
            None => {} // genuinely-empty directory — proceed.
            Some(Ok(_)) => {
                return Err(ClwError::Io(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    "destination not empty",
                )));
            }
            // An IO error surfacing mid-enumeration is a real error too.
            Some(Err(e)) => return Err(ClwError::Io(e)),
        }
    }

    // ------------------------------------------------------------------
    // 2a. Resolve the root manifest digest.
    //
    // Two paths: either a caller-supplied manifest digest (content-addressed
    // hydrate — skip the AC entirely), or the usual name→RefRecord lookup. Both
    // converge on a single `root` that 2b then fetches + integrity-checks, so
    // the manifest-digest path inherits the SAME verification as the named path.
    // ------------------------------------------------------------------
    let root = match opts.root_override {
        Some(root) => root,
        None => {
            let ref_key = clw_types::ref_key_in(opts.ref_domain, &opts.workspace_name);
            let ref_bytes = AcTransport::get(client, &ref_key)
                .await?
                .ok_or(ClwError::NotFound(ref_key))?;
            RefRecord::from_bytes(&ref_bytes)?.root
        }
    };

    // ------------------------------------------------------------------
    // 2b. Fetch manifest blob from CAS; parse.
    // ------------------------------------------------------------------
    let manifest_bytes = CasTransport::get(client, &root).await?;
    // SECURITY (M1.7): re-verify the whole manifest blob against `root` BEFORE
    // parsing — symmetric to the per-chunk integrity check in fetch_chunk. A
    // hostile or buggy CasTransport must not be able to substitute a different
    // manifest than the one the root pins. For the manifest-digest path this is
    // exactly what makes a caller-supplied `toolchain_ref` self-verifying: the
    // bytes we hydrate provably hash to the digest the caller asked for.
    let actual = Digest::of_bytes(&manifest_bytes);
    if actual != root {
        return Err(ClwError::Integrity {
            expected: root,
            actual,
        });
    }
    let manifest = Manifest::from_bytes(&manifest_bytes)?;

    // ------------------------------------------------------------------
    // 2c. Resource-safety / abuse bounds (M1.6).
    //
    // The manifest is untrusted REMOTE input. Enforce the abuse caps BEFORE we
    // allocate the file-entry vector or touch the filesystem, so a malicious or
    // pathological manifest is rejected with a clear, actionable error instead
    // of driving us into OOM or a disk-fill.
    // ------------------------------------------------------------------
    let symlink_count = manifest
        .entries
        .iter()
        .filter(|e| matches!(e, Entry::Symlink { .. }))
        .count();

    // FIX 1 + FIX 6: derive the AUTHORITATIVE figures from the chunk lists, not
    // from the attacker-controlled `total_size`. The real number of bytes we will
    // actually write is the sum of every ChunkRef.size; the real number of chunk
    // allocations is the total chunk count. A malicious manifest can declare
    // `total_size = 0` while its chunks sum to gigabytes, so we MUST budget against
    // the summed figure. We also bound the per-file chunk count so a single entry
    // cannot smuggle an enormous `chunks` Vec past the entry cap. Saturating adds
    // keep a hostile manifest from wrapping the running total to a small value.
    let mut real_total_bytes: u64 = 0;
    let mut total_chunks: u64 = 0;
    for entry in &manifest.entries {
        if let Entry::File { path, chunks, .. } = entry {
            enforce_per_file_chunk_cap(path, chunks.len())?;
            total_chunks = total_chunks.saturating_add(chunks.len() as u64);
            for c in chunks {
                real_total_bytes = real_total_bytes.saturating_add(c.size);
            }
        }
    }

    // FIX 1: a declared `total_size` that disagrees with the summed chunk sizes is
    // a manifest-integrity error — the two views of "how big is this snapshot" must
    // be consistent, and a divergence is the exact signature of the disk-fill
    // bypass (tiny declared total, huge real chunks). Reject before any FS work.
    //
    // V1 (audit pass-2): the STRICT equality is correct and must NOT be relaxed.
    // The producer (clw-snapshot) sets each `File.size` FROM the summed sizes of
    // that file's chunks, and sets the manifest `total_size` FROM the sum of those
    // per-file sizes over File entries only — so both figures are DERIVED from the
    // same chunk sizes rather than measured independently. Symlink and Dir entries
    // contribute nothing to either side (they have no chunks AND are not added to
    // total_size). Hence `total_size == Σ ChunkRef.size` holds exactly for any
    // legitimately-produced manifest, so `!=` can never spuriously reject a
    // well-formed one; a divergence is a genuine tamper/inconsistency signal.
    if manifest.total_size != real_total_bytes {
        return Err(ClwError::InvalidManifest(format!(
            "manifest total_size ({}) disagrees with the sum of its chunk sizes ({real_total_bytes}); \
             refusing to hydrate an internally inconsistent manifest",
            manifest.total_size
        )));
    }

    enforce_resource_bounds(
        manifest.entries.len(),
        real_total_bytes,
        symlink_count,
        total_chunks,
    )?;

    // ------------------------------------------------------------------
    // 3. Ensure dest exists.
    // ------------------------------------------------------------------
    std::fs::create_dir_all(dest)?;

    // Shared state for per-file counters.
    let bytes_from_cache = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let bytes_downloaded = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let files_count = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // ------------------------------------------------------------------
    // 3a. Directories are cheap — handle them first, sequentially.
    //
    // SECURITY (M1.7): symlinks are materialised in a *separate, later* pass
    // (3c, after all regular files exist). A symlink created before its sibling
    // files could redirect a later file write outside the intended location
    // (e.g. a link `out/sub` → `/etc` followed by a file `out/sub/passwd`).
    // Creating every regular file first removes that window.
    // ------------------------------------------------------------------
    for entry in &manifest.entries {
        if let Entry::Dir { path } = entry {
            // Defense-in-depth: reject unsafe paths even if from_bytes was bypassed.
            clw_types::validate_rel_path(path)?;
            let abs = dest.join(path);
            std::fs::create_dir_all(&abs)?;
        }
    }

    // ------------------------------------------------------------------
    // 3b. Files — bounded concurrency via buffer_unordered.
    //
    // LOW (2× manifest RAM): borrow the File entries straight out of
    // `manifest.entries` instead of cloning each `path` + `chunks` Vec into an
    // owned `file_entries`. The clone doubled peak RSS on a huge manifest (every
    // ChunkRef duplicated alongside the parsed manifest). The stream is driven to
    // completion below, before `manifest` is dropped, so borrowing is sound —
    // nothing is spawned onto another task.
    // ------------------------------------------------------------------
    let dest_arc = Arc::new(dest.to_path_buf());
    let bfc = Arc::clone(&bytes_from_cache);
    let bfd = Arc::clone(&bytes_downloaded);
    let fc = Arc::clone(&files_count);
    // A caller may pass zero. `buffered(0)` makes no progress, so normalize the
    // outer file concurrency at the boundary as well as the per-file fetch
    // concurrency below.
    let concurrency = normalized_concurrency(opts.concurrency);

    // Build a stream of futures — one per file, borrowing from `manifest`.
    // Each future returns (bytes_from_cache, bytes_downloaded) for that file.
    let stream = futures::stream::iter(manifest.entries.iter().filter_map(|e| {
        if let Entry::File {
            path, mode, chunks, ..
        } = e
        {
            Some((path.as_str(), *mode, chunks.as_slice()))
        } else {
            None
        }
    }))
    .map(|(path, mode, chunks)| {
        let dest_arc = Arc::clone(&dest_arc);
        let bfc = Arc::clone(&bfc);
        let bfd = Arc::clone(&bfd);
        let fc = Arc::clone(&fc);
        async move {
            let (from_cache, downloaded) = materialize_file(
                dest_arc.as_ref(),
                path,
                mode,
                chunks,
                concurrency,
                client,
                cache,
            )
            .await?;
            bfc.fetch_add(from_cache, std::sync::atomic::Ordering::Relaxed);
            bfd.fetch_add(downloaded, std::sync::atomic::Ordering::Relaxed);
            fc.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok::<(), ClwError>(())
        }
    });

    // Drive the stream with bounded concurrency.
    let mut buffered = stream.buffer_unordered(concurrency);
    while let Some(result) = buffered.next().await {
        result?;
    }

    // ------------------------------------------------------------------
    // 3c. Symlinks — created LAST, after every regular file is materialised
    //     (see the SECURITY note on pass 3a). No network I/O needed.
    // ------------------------------------------------------------------
    // The symlink-count cap (MAX_SYMLINK_ENTRIES) was already enforced up-front
    // in enforce_resource_bounds, before any filesystem work — see pass 2c.
    for entry in &manifest.entries {
        if let Entry::Symlink { path, target } = entry {
            // Defense-in-depth: validate both the link path and its target. The
            // target is checked with the CONTAINED validator (same as the manifest
            // producer + `Manifest::from_bytes`): an intra-workspace `..` that
            // resolves within the tree (e.g. `bin/x -> ../src/x`) is allowed, while
            // any target that would escape the workspace root is still rejected.
            clw_types::validate_rel_path(path)?;
            clw_types::validate_symlink_target_contained(path, target)?;
            let abs = dest.join(path);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // On unix, create a real symlink.
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, &abs)?;
            // On Windows there are no *unprivileged* symlinks (creating one
            // requires elevated privileges / Developer Mode), so clw copies the
            // target's bytes into the link path instead of linking. This means a
            // snapshot→hydrate round-trip on Windows yields COPIES, not links —
            // a documented, non-silent divergence from the unix behavior, not a
            // skipped entry. Correctness is preserved: the target was already
            // validated by validate_symlink_target_contained (relative, no
            // drive-letter/backslash, and any `..` resolves within the workspace
            // root), and because symlinks are materialised in
            // pass 3c AFTER all files (M1.7 ordering), any file target already
            // exists under `dest`.
            #[cfg(windows)]
            materialize_symlink_as_copy(dest, path, target, &abs)?;
        }
    }

    // FIX 4: a hydrate caches every downloaded chunk; over many hydrates the cache
    // would grow without bound. Force one eviction sweep down to the cache budget
    // at the end of the run. Best-effort: a gc failure must not fail an otherwise
    // successful hydrate (the materialised destination is already complete).
    if let Err(e) = cache.gc_to_budget() {
        tracing::warn!(
            destination = %dest.display(),
            error = %e,
            "cache gc failed after hydrate; retaining successful materialization"
        );
    }

    Ok(HydrateReport {
        root,
        files: files_count.load(std::sync::atomic::Ordering::Relaxed),
        bytes_total: manifest.total_size,
        bytes_from_cache: bytes_from_cache.load(std::sync::atomic::Ordering::Relaxed),
        bytes_downloaded: bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed),
    })
}

// ---------------------------------------------------------------------------
// Internal: RAII cleanup for the per-file temp
//
// A mid-file error (disk-full, a transport error on a LATER chunk, an integrity
// mismatch) must not leave a half-written `.clw-tmp-*` stray under `dest`. This
// guard removes the temp on any early return; on the success path we `disarm` it
// after the temp has been renamed into its final place (so the rename target is
// never touched).
// ---------------------------------------------------------------------------
struct TmpFileGuard {
    path: Option<std::path::PathBuf>,
}

impl TmpFileGuard {
    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for TmpFileGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            // Best-effort: cleanup failure must not mask the original error.
            let _ = std::fs::remove_file(&p);
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: materialise one file
//
// Returns (bytes_from_cache, bytes_downloaded) tallied across all chunks.
// ---------------------------------------------------------------------------

/// Messages accepted by the one blocking writer assigned to a materialised
/// file. In particular, closing the channel is *not* success: only `Finish`
/// authorizes publishing the temporary file.
enum FileWrite {
    Chunk(Vec<u8>),
    Finish,
    #[cfg(test)]
    FailWrite,
    #[cfg(all(test, unix))]
    FailMode,
    #[cfg(test)]
    FailSync,
    #[cfg(all(test, unix))]
    Checkpoint(tokio::sync::oneshot::Sender<()>),
    #[cfg(all(test, unix))]
    BeforePublish {
        ready: tokio::sync::oneshot::Sender<()>,
        resume: tokio::sync::oneshot::Receiver<()>,
    },
    #[cfg(test)]
    BlockFor(std::time::Duration),
}

/// Start a single blocking filesystem consumer for one file.
///
/// The async side retains all borrowed transport/cache state and only sends
/// owned bytes. The closure owns every blocking resource (`PathBuf`, `File`,
/// and `TmpFileGuard`), so cancellation or a failed fetch closes the queue and
/// causes the guard to remove the uncommitted temp. A final path is published
/// only after an explicit [`FileWrite::Finish`].
fn spawn_file_writer(
    tmp_path: PathBuf,
    abs: PathBuf,
    parent: PathBuf,
    mode: u32,
) -> (
    tokio::sync::mpsc::Sender<FileWrite>,
    tokio::task::JoinHandle<Result<()>>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::channel(FILE_WRITE_QUEUE_CAPACITY);
    let writer = tokio::task::spawn_blocking(move || {
        use std::io::Write as _;

        std::fs::create_dir_all(&parent)?;
        // Declare cleanup before the file so errors close the handle before
        // unlinking (also required on Windows). An unsuccessful exclusive open
        // owns nothing: only arm cleanup after we actually created the temp.
        let mut tmp_guard = TmpFileGuard { path: None };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp_path)?;
        tmp_guard.path = Some(tmp_path.clone());
        #[cfg(all(test, unix))]
        let mut fail_mode = false;
        #[cfg(test)]
        let mut fail_sync = false;
        #[cfg(all(test, unix))]
        let mut before_publish: Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )> = None;

        while let Some(message) = rx.blocking_recv() {
            match message {
                FileWrite::Chunk(data) => file.write_all(&data)?,
                FileWrite::Finish => {
                    // Publishing is deliberately a positive protocol message,
                    // never an EOF side effect. Final Unix metadata is committed
                    // to the owned temp before the pathname becomes visible.
                    file.flush()?;
                    #[cfg(all(test, unix))]
                    if fail_mode {
                        return Err(ClwError::Io(std::io::Error::other(
                            "injected pre-publication mode failure",
                        )));
                    }
                    #[cfg(unix)]
                    {
                        // The temp starts owner-only and receives its intended
                        // final metadata while we still own the open file. A
                        // metadata failure therefore aborts before publication,
                        // and the guard removes the uncommitted temp.
                        let perms = std::fs::Permissions::from_mode(mode & 0o777);
                        file.set_permissions(perms)?;
                    }
                    #[cfg(windows)]
                    let _ = mode;
                    // Sync both bytes and the pre-publication metadata before
                    // exposing the final pathname.
                    #[cfg(test)]
                    if fail_sync {
                        return Err(ClwError::Io(std::io::Error::other(
                            "injected pre-publication sync failure",
                        )));
                    }
                    file.sync_all()?;
                    #[cfg(all(test, unix))]
                    if let Some((ready, resume)) = before_publish.take() {
                        let _ = ready.send(());
                        resume.blocking_recv().map_err(|_| {
                            ClwError::Io(std::io::Error::other("publication test cancelled"))
                        })?;
                    }
                    drop(file);
                    std::fs::rename(&tmp_path, &abs)?;
                    tmp_guard.disarm();
                    if let Ok(dir) = std::fs::File::open(&parent) {
                        let _ = dir.sync_all();
                    }
                    return Ok(());
                }
                #[cfg(test)]
                FileWrite::FailWrite => {
                    return Err(ClwError::Io(std::io::Error::other(
                        "injected blocking writer failure",
                    )));
                }
                #[cfg(all(test, unix))]
                FileWrite::FailMode => fail_mode = true,
                #[cfg(test)]
                FileWrite::FailSync => fail_sync = true,
                #[cfg(all(test, unix))]
                FileWrite::Checkpoint(ready) => {
                    let _ = ready.send(());
                }
                #[cfg(all(test, unix))]
                FileWrite::BeforePublish { ready, resume } => {
                    before_publish = Some((ready, resume));
                }
                #[cfg(test)]
                FileWrite::BlockFor(duration) => std::thread::sleep(duration),
            }
        }

        // EOF is an abort, including cancellation and a failed fetch/send. The
        // guard removes the temp as this closure returns; no partial final name
        // can have been published because Finish was never received.
        Err(ClwError::Transport(
            "blocking file writer closed before Finish".into(),
        ))
    });
    (tx, writer)
}

async fn materialize_file<C>(
    dest: &Path,
    rel_path: &str,
    mode: u32,
    chunks: &[clw_types::ChunkRef],
    concurrency: usize,
    client: &C,
    cache: &LocalCache,
) -> Result<(u64, u64)>
where
    C: CasTransport,
{
    // Defense-in-depth: reject any unsafe path even if from_bytes was bypassed.
    clw_types::validate_rel_path(rel_path)?;
    let abs = dest.join(rel_path);
    let parent = abs
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dest.to_path_buf());
    let tmp_path = {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        // Process-wide monotonic counter guarantees uniqueness even when two
        // tasks on the same thread hit the same subsec_nanos value (the thread-id
        // + nanos pair alone can collide under concurrency).
        static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let tid = std::thread::current().id();
        parent.join(format!(".clw-tmp-{tid:?}-{nanos}-{seq}"))
    };
    let (tx, writer) = spawn_file_writer(tmp_path, abs, parent, mode);

    // Fetches remain entirely on the async side, preserving their manifest
    // order through `buffered`. P independently caps buffered chunk payloads;
    // the Q=1 channel adds at most one more 4 MiB payload per file.
    let fetch_concurrency = per_file_fetch_concurrency(concurrency);
    let producer_result = async {
        let mut bytes_from_cache: u64 = 0;
        let mut bytes_downloaded: u64 = 0;
        let mut fetches = futures::stream::iter(chunks.iter().map(|chunk| {
            let digest = chunk.digest;
            let size = chunk.size;
            async move {
                let (data, from_cache) = fetch_chunk(digest, client, cache).await?;
                Ok::<(Vec<u8>, bool, Digest, u64), ClwError>((data, from_cache, digest, size))
            }
        }))
        .buffered(fetch_concurrency);

        while let Some(res) = fetches.next().await {
            let (data, from_cache, digest, size) = res?;
            // The digest was verified by fetch_chunk; the declared length is a
            // separate manifest invariant and must be checked before enqueueing.
            if data.len() as u64 != size {
                return Err(ClwError::InvalidManifest(format!(
                    "chunk {} for file {rel_path:?} returned {} byte(s) but the manifest \
                     declares {} byte(s); refusing to hydrate a length-mismatched chunk",
                    digest.to_hex(),
                    data.len(),
                    size
                )));
            }
            tx.send(FileWrite::Chunk(data)).await.map_err(|_| {
                ClwError::Transport("blocking file writer stopped before Finish".into())
            })?;
            if from_cache {
                bytes_from_cache += size;
            } else {
                bytes_downloaded += size;
            }
        }
        tx.send(FileWrite::Finish).await.map_err(|_| {
            ClwError::Transport("blocking file writer stopped before Finish".into())
        })?;
        Ok::<(u64, u64), ClwError>((bytes_from_cache, bytes_downloaded))
    }
    .await;

    // On any producer failure (including cancellation), closing Q tells the
    // writer to abort and its guard removes the temp. We still await the handle
    // on ordinary error paths so cleanup is complete before returning.
    drop(tx);
    let writer_result = writer.await.map_err(|error| {
        ClwError::Transport(format!("blocking file writer task failed: {error}"))
    })?;
    let (bytes_from_cache, bytes_downloaded) = producer_result?;
    writer_result?;
    // Diagnostic channel (M5.1): per-file cache-hit vs download split on stderr.
    tracing::debug!(
        path = rel_path,
        bytes_from_cache,
        bytes_downloaded,
        "materialized file"
    );
    Ok((bytes_from_cache, bytes_downloaded))
}

// ---------------------------------------------------------------------------
// Internal (Windows only): materialise a symlink entry as a byte/subtree copy.
//
// Windows has no unprivileged symlinks, so clw copies the target instead of
// linking. The `target` is relative to the LINK's parent directory and was
// already validated (relative, no drive-letter/backslash, and any `..` resolves
// within the workspace root) by validate_symlink_target_contained. Joining it
// component-by-component onto `link_parent` therefore resolves to a path inside
// `dest`. We resolve it within `dest`, then:
//   - regular FILE target  -> copy its bytes (std::fs::copy),
//   - DIRECTORY target      -> recursively copy the subtree,
//   - missing target        -> a dangling symlink can't be copied: clear error.
// File targets are guaranteed to exist because symlinks run in pass 3c, after
// every regular file has been materialised (M1.7 files-before-symlinks order).
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn materialize_symlink_as_copy(dest: &Path, path: &str, target: &str, link: &Path) -> Result<()> {
    let resolved = path_containment::resolve_symlink_target(dest, target, link);

    let meta = std::fs::symlink_metadata(&resolved).map_err(|_| {
        ClwError::InvalidManifest(format!(
            "symlink target does not exist under dest, cannot copy on Windows: \
             link {path} -> {target}"
        ))
    })?;

    if meta.file_type().is_dir() {
        // M17: a DIRECTORY target that contains the link itself (canonically
        // `d/up -> ..`) would make copy_dir_recursive walk into the tree it is
        // writing, re-reading what it just wrote, until it dies on MAX_PATH or
        // fills the disk. The manifest is untrusted remote input, so reject it
        // here with a clear error instead. This is a MATERIALIZER rule, not a
        // format rule: the same manifest stays valid (and materialises as a real
        // symlink) on unix.
        //
        // The decision is the UNION of two independent checks — a lexical model
        // of Win32 folding, and `fs::canonicalize` on both sides — and a positive
        // from EITHER refuses the copy. Not a pre-filter plus an authority:
        // neither layer subsumes the other, and the lexical one short-circuits,
        // so for a trailing space or dot (`.. ` IS `..` to Win32) and for Unicode
        // case (`Ä`/`ä`) it is OUR MODEL that answers, not the OS. What
        // canonicalize adds is everything the model does not enumerate — 8.3
        // aliases, junctions, NTFS's own case table. See the `path_containment`
        // module docs, which are the authority on this split.
        //
        // FAIL-CLOSED: if either side cannot be canonicalised we refuse the entry
        // rather than copy blind — an unresolvable path is precisely the case
        // where the copy cannot be proven safe.
        let would_recurse = path_containment::copy_would_recurse(&resolved, link).map_err(|e| {
            ClwError::InvalidManifest(format!(
                "cannot verify that the symlink target directory does not contain the \
                     link itself ({e}); refusing to copy on Windows: link {path} -> {target}"
            ))
        })?;
        if would_recurse {
            return Err(ClwError::InvalidManifest(format!(
                "symlink target directory contains the link itself; copying it on \
                 Windows would recurse without bound: link {path} -> {target}"
            )));
        }
        copy_dir_recursive(&resolved, link, 0)?;
    } else {
        // Regular file (or anything that is not a directory): copy its bytes.
        std::fs::copy(&resolved, link)?;
    }
    Ok(())
}

/// Maximum directory nesting the Windows symlink copy-fallback will follow.
///
/// This is a FAIL-SAFE BACKSTOP, not the guard. `copy_would_recurse` is supposed
/// to have refused every self-referential copy before we get here — but that
/// check is a single oracle over untrusted input (and a TOCTOU snapshot: a
/// concurrent writer could re-point a directory between the check and this
/// walk). Without a bound, any miss is UNBOUNDED rather than merely wrong. With
/// one, a miss costs 64 levels and a clean error.
///
/// 64 is far deeper than any real workspace. It is NOT, however, generally
/// reached before MAX_PATH: 64 levels of `up\` plus a real `dest` prefix is well
/// past ~260 characters, so on a normal Windows system the runaway copy dies on
/// MAX_PATH first and this cap only bites on short paths or where long paths are
/// enabled. It is a guaranteed *upper* bound on the damage, not the first thing
/// to fire.
///
/// BEHAVIOUR CHANGE (Windows only): a legitimate manifest whose symlinked
/// directory target nests deeper than this now fails to hydrate, where it
/// previously copied. No real workspace is anywhere near it; raise the constant
/// if one ever is. Noted in CHANGELOG.md.
const MAX_COPY_DEPTH: u32 = 64;

// Recursively copy the directory subtree at `src` to `dst`, refusing to descend
// past MAX_COPY_DEPTH.
//
// Only the Windows copy-fallback calls this, but — like the containment guard —
// it is NOT `cfg(windows)`, so its backstop is exercised by a normal `cargo test`
// on unix instead of being a bound nobody ever runs.
#[cfg_attr(not(windows), allow(dead_code))]
fn copy_dir_recursive(src: &Path, dst: &Path, depth: u32) -> Result<()> {
    // `>=`, not `>`: `depth` is 0-based, so `depth == MAX_COPY_DEPTH` is already
    // the 65th level. The doc says 64 and the code must mean 64.
    if depth >= MAX_COPY_DEPTH {
        return Err(ClwError::InvalidManifest(format!(
            "symlink copy-fallback exceeded the maximum directory depth of \
             {MAX_COPY_DEPTH} while copying {}; refusing to descend further",
            src.display()
        )));
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to, depth + 1)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
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
    // Direct sync cache access (LocalCache is just a PathBuf; calling it directly
    // here avoids the 'static requirement of spawn_blocking — same approach as
    // clw-snapshot). NO unsafe, NO raw pointers.
    if let Some(data) = cache.get(&digest)? {
        // Local-cache path is trusted (we only ever cache verified bytes).
        return Ok((data, true));
    }
    let data = CasTransport::get(client, &digest).await?;
    // SECURITY (M1.7): re-verify the fetched bytes against the expected digest
    // before caching/writing. The CasTransport contract requires this, but an
    // untrusted or buggy transport could return mismatched bytes; hydrate must
    // not write or cache content that does not hash to what the manifest pins.
    let actual = Digest::of_bytes(&data);
    if actual != digest {
        return Err(ClwError::Integrity {
            expected: digest,
            actual,
        });
    }
    cache.put(&digest, &data)?;
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

        async fn delete(&self, digest: &Digest) -> Result<()> {
            self.cas.lock().unwrap().remove(digest);
            Ok(())
        }

        async fn list_blobs(&self) -> Result<Vec<Digest>> {
            Ok(self.cas.lock().unwrap().keys().copied().collect())
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

        async fn delete(&self, key: &Digest) -> Result<()> {
            self.ac.lock().unwrap().remove(key);
            Ok(())
        }

        async fn list_refs(&self) -> Result<Vec<Digest>> {
            Ok(self.ac.lock().unwrap().keys().copied().collect())
        }
    }

    #[tokio::test]
    async fn fake_list_refs_and_blobs_return_map_keys() {
        let store = FakeStore::new();
        let r = Digest::of_bytes(b"r");
        let b = Digest::of_bytes(b"b");
        store.ac_put_raw(r, b"v".to_vec());
        store.cas_put_raw(b, b"data".to_vec());
        assert_eq!(AcTransport::list_refs(&store).await.unwrap(), vec![r]);
        assert_eq!(CasTransport::list_blobs(&store).await.unwrap(), vec![b]);
        CasTransport::delete(&store, &b).await.unwrap();
        assert!(CasTransport::list_blobs(&store).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fake_delete_removes_key_and_is_idempotent() {
        let store = FakeStore::new();
        let key = Digest::of_bytes(b"some-ref-key");
        AcTransport::put(&store, &key, b"value".to_vec())
            .await
            .unwrap();
        assert!(AcTransport::get(&store, &key).await.unwrap().is_some());
        AcTransport::delete(&store, &key).await.unwrap();
        assert!(AcTransport::get(&store, &key).await.unwrap().is_none());
        // Idempotent: deleting an absent key is still Ok.
        AcTransport::delete(&store, &key).await.unwrap();
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

    // Asserts unix permission bits via PermissionsExt::mode(); gate to unix so
    // the test build compiles on Windows (production code is already cfg-gated).
    #[cfg(unix)]
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
    // Test 1b: hydrate DIRECTLY from a manifest digest — with an EMPTY AC.
    // Proves `HydrateOptions::from_manifest_digest` performs zero named-ref
    // lookup (the content-addressed seam behind `clw hydrate --manifest-digest`,
    // e.g. a runner hydrating `toolchain_ref = <digest>`).
    // -----------------------------------------------------------------------
    // Asserts the unix exec bit via PermissionsExt::mode(); gate to unix.
    #[cfg(unix)]
    #[tokio::test]
    async fn hydrate_by_manifest_digest_needs_no_ref() {
        let data = b"toolchain-payload".as_ref();
        let (d, bytes) = make_chunk(data);
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data.len() as u64,
            entries: vec![Entry::File {
                path: "bin/rustc".into(),
                mode: 0o755,
                size: data.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d,
                    size: data.len() as u64,
                }],
            }],
        };
        let manifest_bytes = manifest.canonical_bytes().unwrap();
        let root = Digest::of_bytes(&manifest_bytes);

        // CAS holds the manifest + its chunk; the AC is left EMPTY (no RefRecord).
        let store = FakeStore::new();
        store.cas_put_raw(d, bytes);
        store.cas_put_raw(root, manifest_bytes);

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::from_manifest_digest(root);
        let report = hydrate(&dest, &store, &cache, &opts).await.unwrap();

        assert_eq!(
            std::fs::read(dest.join("bin/rustc")).unwrap(),
            data,
            "materialised content"
        );
        assert_eq!(
            std::fs::metadata(dest.join("bin/rustc"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "exec bit preserved"
        );
        assert_eq!(report.files, 1);
        assert_eq!(
            report.root, root,
            "report echoes the requested manifest digest"
        );
    }

    // -----------------------------------------------------------------------
    // Test 1c: a manifest-digest hydrate is self-verifying — if the CAS bytes at
    // the requested key do NOT hash to it, the pre-parse integrity check rejects.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn hydrate_by_manifest_digest_rejects_mismatched_bytes() {
        // The digest of the REAL manifest bytes, but seed the CAS at that key
        // with different (tampered) bytes.
        let (root, _real) = make_chunk(b"the real manifest bytes");
        let store = FakeStore::new();
        store.cas_put_raw(root, b"tampered-not-the-manifest".to_vec());

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::from_manifest_digest(root);
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Integrity { .. }),
            "mismatched manifest bytes must be rejected as Integrity, got: {err:?}"
        );
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
    // B2 (crash-consistency): hydrate is idempotent / crash-safe via re-run.
    //
    // Model: a hydrate is interrupted, leaving a PARTIAL destination (some files
    // missing, some truncated). The documented recovery (hydrate requires an
    // EMPTY dest — see `non_empty_dest_returns_already_exists`) is to clear the
    // partial dest and re-run. This test proves the re-run HEALS: the final tree
    // is BYTE-IDENTICAL to the source, regardless of the partial state left by
    // the crash. It also proves the per-file temp+rename leaves NO `.clw-tmp-*`
    // stray behind on success (so a real materialized file is always complete,
    // never a half-written temp under the destination name).
    // -----------------------------------------------------------------------

    /// Recursively collect (relative-path -> bytes) for every regular file under
    /// `root`, so two trees can be compared byte-for-byte.
    fn read_tree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let ft = entry.file_type().unwrap();
                if ft.is_dir() {
                    walk(base, &path, out);
                } else if ft.is_file() {
                    let rel = path
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
                // symlinks intentionally skipped here (compared separately).
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    #[tokio::test]
    async fn hydrate_rerun_heals_a_partial_destination() {
        // A multi-file, multi-chunk tree so a partial dest is meaningful.
        let f1 = b"first file body".as_ref();
        let f2a = b"second-file-chunk-A".as_ref();
        let f2b = b"second-file-chunk-B".as_ref();
        let f3 = b"nested third file".as_ref();

        let (d1, b1) = make_chunk(f1);
        let (d2a, b2a) = make_chunk(f2a);
        let (d2b, b2b) = make_chunk(f2b);
        let (d3, b3) = make_chunk(f3);

        let mut entries = vec![
            Entry::File {
                path: "a.txt".into(),
                mode: 0o644,
                size: f1.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d1,
                    size: f1.len() as u64,
                }],
            },
            Entry::File {
                path: "multi.bin".into(),
                mode: 0o644,
                size: (f2a.len() + f2b.len()) as u64,
                chunks: vec![
                    ChunkRef {
                        digest: d2a,
                        size: f2a.len() as u64,
                    },
                    ChunkRef {
                        digest: d2b,
                        size: f2b.len() as u64,
                    },
                ],
            },
            Entry::File {
                path: "dir/c.txt".into(),
                mode: 0o644,
                size: f3.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d3,
                    size: f3.len() as u64,
                }],
            },
        ];
        entries.sort_by(|a, b| a.path().cmp(b.path()));

        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: (f1.len() + f2a.len() + f2b.len() + f3.len()) as u64,
            entries,
        };
        let all_chunks = vec![(d1, b1), (d2a, b2a), (d2b, b2b), (d3, b3)];

        let (store, _cache_dir, cache) = setup_store(&manifest, &all_chunks);
        let opts = HydrateOptions::new("test-ws");

        // --- Reference hydrate: the SOURCE tree we must converge to. ----------
        let ref_dir = TempDir::new().unwrap();
        let ref_dest = ref_dir.path().join("out");
        hydrate(&ref_dest, &store, &cache, &opts).await.unwrap();
        let source_tree = read_tree(&ref_dest);
        assert_eq!(source_tree.len(), 3, "source tree must have 3 files");

        // The per-file temp+rename must have left NO `.clw-tmp-*` stray under the
        // destination on a clean hydrate (every materialized file is complete).
        fn no_tmp_strays(root: &Path) -> bool {
            fn walk(dir: &Path) -> bool {
                for e in std::fs::read_dir(dir).unwrap() {
                    let e = e.unwrap();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with(".clw-tmp-") {
                        return false;
                    }
                    if e.file_type().unwrap().is_dir() && !walk(&e.path()) {
                        return false;
                    }
                }
                true
            }
            walk(root)
        }
        assert!(
            no_tmp_strays(&ref_dest),
            "a clean hydrate must leave no per-file temp (.clw-tmp-*) stray"
        );

        // --- Simulate an INTERRUPTED hydrate: a PARTIAL destination. ----------
        // Re-materialize the same tree into a fresh dir, then corrupt it the way a
        // crash would: delete one file entirely and TRUNCATE another mid-write.
        let work_dir = TempDir::new().unwrap();
        let dest = work_dir.path().join("out");
        hydrate(&dest, &store, &cache, &opts).await.unwrap();

        std::fs::remove_file(dest.join("a.txt")).unwrap(); // file never written
        std::fs::write(dest.join("multi.bin"), b"trunc").unwrap(); // partial write
                                                                   // Confirm the destination is genuinely partial (diverges from source).
        assert_ne!(
            read_tree(&dest),
            source_tree,
            "precondition: the simulated partial dest must diverge from the source"
        );

        // --- Re-run hydrate to HEAL. ------------------------------------------
        // hydrate requires an EMPTY dest (the documented recovery precondition —
        // see non_empty_dest_returns_already_exists), so clear the partial dest
        // and re-run into the SAME path. A buggy hydrate that was not idempotent
        // would produce a tree that differs from the source.
        std::fs::remove_dir_all(&dest).unwrap();
        hydrate(&dest, &store, &cache, &opts).await.unwrap();

        let healed_tree = read_tree(&dest);
        assert_eq!(
            healed_tree, source_tree,
            "re-running hydrate must heal the partial destination to a byte-identical tree"
        );
        assert!(
            no_tmp_strays(&dest),
            "the healed hydrate must also leave no per-file temp stray"
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

    // -----------------------------------------------------------------------
    // Test 4b (M1.7): a fetched MANIFEST blob whose bytes don't hash to
    // record.root must surface ClwError::Integrity — never be parsed or used.
    // Mirrors the per-chunk integrity check at the whole-manifest level.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn substituted_manifest_returns_integrity_error() {
        // Build a legitimate manifest and seed the store so record.root points
        // at its digest.
        let data = b"file-bytes";
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
        let (store, _cache_dir, cache) = setup_store(&manifest, &[(d, bytes)]);

        // Recover record.root, then overwrite the CAS blob at that key with a
        // DIFFERENT (still well-formed) manifest — a hostile transport
        // substituting the whole manifest. Its bytes no longer hash to root.
        let root = {
            let mb = manifest.canonical_bytes().unwrap();
            Digest::of_bytes(&mb)
        };
        let other = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0,
            entries: vec![Entry::Dir {
                path: "evil".into(),
            }],
        };
        store.cas_put_raw(root, other.canonical_bytes().unwrap());

        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Integrity { expected, .. } if expected == root),
            "expected Integrity for manifest root {root}, got: {err}"
        );
        // The substituted manifest must not have driven any filesystem work.
        assert!(
            !dest.join("evil").exists(),
            "substituted manifest must not produce any output"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5 (M1.7): a fetched chunk whose bytes don't hash to the expected
    // digest must surface ClwError::Integrity — never be written or cached.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn corrupt_fetched_chunk_returns_integrity_error() {
        let honest = b"the real chunk bytes";
        let (digest, _real_bytes) = make_chunk(honest);

        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: honest.len() as u64,
            entries: vec![Entry::File {
                path: "f.bin".into(),
                mode: 0o644,
                size: honest.len() as u64,
                chunks: vec![ChunkRef {
                    digest,
                    size: honest.len() as u64,
                }],
            }],
        };

        // Seed the store but deliberately store CORRUPT bytes under the digest.
        // (The fake transport does not self-verify, mimicking a hostile/buggy
        // CAS endpoint; hydrate must catch the mismatch itself.)
        let corrupt = b"TAMPERED payload of a different length!!".to_vec();
        let (store, _cache_dir, cache) = setup_store(&manifest, &[(digest, honest.to_vec())]);
        store.cas_put_raw(digest, corrupt);

        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Integrity { expected, .. } if expected == digest),
            "expected Integrity for {digest}, got: {err}"
        );

        // The corrupt bytes must NOT have been written or cached.
        assert!(
            !dest.join("f.bin").exists(),
            "corrupt file must not be written"
        );
        assert!(!cache.has(&digest), "corrupt chunk must not be cached");
    }

    // -----------------------------------------------------------------------
    // Test 5b (FIX F1): a fetched chunk whose ACTUAL byte length disagrees with
    // its DECLARED `ChunkRef.size` must be rejected as an integrity error, even
    // when its digest verifies. We seed CAS with the REAL bytes (so the digest
    // check passes) but declare a `ChunkRef.size` that does NOT match those
    // bytes' length — the length guard must fire and no file must be written.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn chunk_length_mismatch_returns_integrity_error() {
        let real = b"the-honest-chunk-bytes";
        let (digest, bytes) = make_chunk(real);

        // Declared sizes deliberately LIE: total_size and the ChunkRef.size both
        // claim a length different from the real bytes. They agree with each
        // other (so the pass-2c declared-vs-summed check passes), but disagree
        // with the bytes actually returned for the chunk.
        let declared_len = real.len() as u64 + 7;
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: declared_len,
            entries: vec![Entry::File {
                path: "f.bin".into(),
                mode: 0o644,
                size: declared_len,
                chunks: vec![ChunkRef {
                    digest,
                    size: declared_len,
                }],
            }],
        };

        // Seed CAS with the REAL bytes under the (correct) digest, so the digest
        // re-verification in fetch_chunk passes and the LENGTH guard is what fires.
        let (store, _cache_dir, cache) = setup_store(&manifest, &[(digest, bytes)]);

        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("length-mismatched")),
            "expected InvalidManifest about a length-mismatched chunk, got: {err}"
        );
        assert!(
            !dest.join("f.bin").exists(),
            "a length-mismatched chunk must not produce a file"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6 (M1.7): setuid/setgid/sticky bits in a manifest mode must be
    // stripped — only the low 9 permission bits survive.
    // -----------------------------------------------------------------------

    // Asserts unix permission bits via PermissionsExt::mode(); gate to unix so
    // the test build compiles on Windows (production code is already cfg-gated).
    #[cfg(unix)]
    #[tokio::test]
    async fn manifest_mode_setuid_bit_is_stripped() {
        let data = b"executable-ish";
        let (d, bytes) = make_chunk(data);

        // 0o4755 = setuid + rwxr-xr-x. After masking we expect 0o755 only.
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data.len() as u64,
            entries: vec![Entry::File {
                path: "suid.bin".into(),
                mode: 0o4755,
                size: data.len() as u64,
                chunks: vec![ChunkRef {
                    digest: d,
                    size: data.len() as u64,
                }],
            }],
        };

        let (store, _cache_dir, cache) = setup_store(&manifest, &[(d, bytes)]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        hydrate(&dest, &store, &cache, &opts).await.unwrap();

        let meta = std::fs::metadata(dest.join("suid.bin")).unwrap();
        let mode = meta.permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "permission bits preserved");
        assert_eq!(mode & 0o7000, 0, "setuid/setgid/sticky bits stripped");
    }

    // -----------------------------------------------------------------------
    // Test 7 (M1.7): regular files are materialised BEFORE symlinks are
    // created. We place the symlink first in the manifest's entry order; a
    // symlink that pre-empted a sibling directory would corrupt where the
    // file lands. Files-first guarantees the real directory wins.
    // -----------------------------------------------------------------------

    // Unix asserts real symlink semantics (read_link → target). On Windows there are no
    // unprivileged symlinks, so hydrate copies instead — asserted by its sibling below.
    #[cfg(unix)]
    #[tokio::test]
    async fn regular_files_materialized_before_symlinks() {
        let data = b"inner-file-data";
        let (d, bytes) = make_chunk(data);

        // Entry order (NOT sorted): symlink FIRST, then the file under `sub/`.
        // `mylink` -> `sub/inner.txt`. If the link were created before the file
        // exists it would dangle; files-first means the target already exists.
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data.len() as u64,
            entries: vec![
                Entry::Symlink {
                    path: "mylink".into(),
                    target: "sub/inner.txt".into(),
                },
                Entry::File {
                    path: "sub/inner.txt".into(),
                    mode: 0o644,
                    size: data.len() as u64,
                    chunks: vec![ChunkRef {
                        digest: d,
                        size: data.len() as u64,
                    }],
                },
            ],
        };

        let (store, _cache_dir, cache) = setup_store(&manifest, &[(d, bytes)]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        hydrate(&dest, &store, &cache, &opts).await.unwrap();

        // The real file exists with correct content.
        assert_eq!(std::fs::read(dest.join("sub/inner.txt")).unwrap(), data);

        // The symlink exists and, because the file was materialised first,
        // resolves through to the real content (no dangling link).
        let link = dest.join("mylink");
        assert_eq!(
            std::fs::read_link(&link).unwrap().to_str().unwrap(),
            "sub/inner.txt"
        );
        assert_eq!(
            std::fs::read(&link).unwrap(),
            data,
            "symlink resolves to the already-materialised file"
        );
    }

    /// Windows counterpart: there are no unprivileged symlinks on Windows, so hydrate
    /// materialises a symlink entry as a COPY of the (already-materialised) target — a
    /// documented, non-silent divergence. Assert the entry is a REGULAR FILE (not a link)
    /// carrying the target's bytes. (Surfaced by the Windows runtime lane; validates the
    /// `materialize_symlink_as_copy` path the unix test can't exercise.)
    #[cfg(windows)]
    #[tokio::test]
    async fn symlink_materialized_as_copy_on_windows() {
        let data = b"inner-file-data";
        let (d, bytes) = make_chunk(data);
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data.len() as u64,
            entries: vec![
                Entry::Symlink {
                    path: "mylink".into(),
                    target: "sub/inner.txt".into(),
                },
                Entry::File {
                    path: "sub/inner.txt".into(),
                    mode: 0o644,
                    size: data.len() as u64,
                    chunks: vec![ChunkRef {
                        digest: d,
                        size: data.len() as u64,
                    }],
                },
            ],
        };

        let (store, _cache_dir, cache) = setup_store(&manifest, &[(d, bytes)]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");
        hydrate(&dest, &store, &cache, &HydrateOptions::new("test-ws"))
            .await
            .unwrap();

        assert_eq!(std::fs::read(dest.join("sub/inner.txt")).unwrap(), data);
        let link = dest.join("mylink");
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(
            !meta.file_type().is_symlink(),
            "on Windows the symlink entry is materialised as a COPY, not a link"
        );
        assert_eq!(
            std::fs::read(&link).unwrap(),
            data,
            "the copy carries the target's bytes"
        );
    }

    // -----------------------------------------------------------------------
    // Test 7c (M17): the SELF-REFERENTIAL symlink `d/up -> ..`.
    //
    // The manifest is VALID (validate_symlink_target_contained accepts it: the
    // `..` resolves to the workspace root, well inside the tree) and it stays
    // valid — the unix half below hydrates it into a real symlink. Only the
    // Windows COPY fallback has to refuse it, because copying the link's own
    // ancestor into the link recurses until MAX_PATH.
    // -----------------------------------------------------------------------

    /// The three self-referential shapes, as (name, file entry, link, target).
    ///
    /// All three are ACCEPTED by `validate_symlink_target_contained` — verified,
    /// and asserted below — so all three are reachable from a real manifest:
    ///
    /// * `plain`   — the base M17 case, `d/up -> ..`.
    /// * `trailing`— BLOCKER 1: Win32 strips a trailing SPACE, so `.. ` IS `..`.
    /// * `unicode` — BLOCKER 2: NTFS folds `Ä`/`ä` to one directory.
    ///
    /// The last two were let through by the first version of this guard.
    const SELF_REF_SHAPES: &[(&str, &str, &str, &str)] = &[
        ("plain", "d/f.txt", "d/up", ".."),
        ("trailing-space", "d/sub/f.txt", "d/sub/up", ".. "),
        ("unicode-case", "Ä/sub/f.txt", "Ä/sub/up", "../../ä/sub"),
    ];

    fn self_referential_symlink_fixture(
        file_path: &str,
        link: &str,
        target: &str,
    ) -> (Manifest, Digest, Vec<u8>) {
        let data = b"payload";
        let (d, bytes) = make_chunk(data);
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: data.len() as u64,
            entries: vec![
                Entry::File {
                    path: file_path.into(),
                    mode: 0o644,
                    size: data.len() as u64,
                    chunks: vec![ChunkRef {
                        digest: d,
                        size: data.len() as u64,
                    }],
                },
                Entry::Symlink {
                    path: link.into(),
                    target: target.into(),
                },
            ],
        };
        (manifest, d, bytes)
    }

    /// Every shape the Windows materializer refuses is a VALID manifest. If this
    /// ever fails, the rejection has leaked into the format and the invariant
    /// ("no manifest valid on unix becomes invalid") is broken.
    #[test]
    fn every_self_referential_shape_is_a_valid_manifest() {
        for (name, _file, link, target) in SELF_REF_SHAPES {
            clw_types::validate_rel_path(link)
                .unwrap_or_else(|e| panic!("{name}: link {link:?} must be a valid path: {e}"));
            clw_types::validate_symlink_target_contained(link, target).unwrap_or_else(|e| {
                panic!("{name}: {link:?} -> {target:?} must be a VALID manifest entry: {e}")
            });
        }
    }

    /// The materializer's ACTUAL decision, for all three shapes, on a real tree —
    /// runnable on unix, where the `cfg(windows)` materializer itself is not.
    ///
    /// This drives the same two functions the materializer calls, in the same
    /// order, on the same paths (`resolve_symlink_target` is shared code, not a
    /// re-derivation), so a `true` here is precisely the refusal Windows makes.
    ///
    /// `Ok(true)` is asserted deliberately rather than "refuses somehow": the
    /// fail-closed path would ALSO refuse the trailing-space shape on unix (the
    /// literal directory `.. ` does not exist here), which would mask a
    /// regression in the lexical layer. Demanding a positive DETECTION keeps
    /// every row red-on-revert.
    ///
    /// What this does NOT prove: on unix the `true` comes from layer 1, our model
    /// of Win32 folding. Layer 2 confirms it independently only on a real NTFS,
    /// where `canonicalize` resolves `.. ` and `Ä` for us.
    #[test]
    fn materializer_decision_refuses_every_self_referential_shape() {
        for (name, file, link_path, target) in SELF_REF_SHAPES {
            let td = TempDir::new().unwrap();
            let dest = td.path();
            // Materialise the tree the file pass would have created (pass 3a runs
            // before symlinks — M1.7), so the target really exists.
            let file_parent = std::path::Path::new(file).parent().unwrap();
            std::fs::create_dir_all(dest.join(file_parent)).unwrap();
            std::fs::write(dest.join(file), b"payload").unwrap();

            let link = dest.join(link_path);
            let resolved = path_containment::resolve_symlink_target(dest, target, &link);

            assert_eq!(
                path_containment::copy_would_recurse(&resolved, &link).ok(),
                Some(true),
                "{name}: {link_path} -> {target:?} must be DETECTED as self-referential \
                 (resolved={resolved:?}, link={link:?})"
            );
        }
    }

    /// The mirror image: the ordinary shapes the guard must let through, driven
    /// the same way. Without this, "reject everything" would pass the test above.
    #[test]
    fn materializer_decision_allows_ordinary_directory_links() {
        // (name, dirs to create, link, target)
        let ok_shapes: &[(&str, &str, &str, &str)] = &[
            ("sibling-dir", "src/x", "bin/link", "../src"),
            ("nested-dir", "a/b/c", "a/link", "b/c"),
            ("unicode-sibling", "ä/x", "Ö/link", "../ä"),
        ];
        for (name, dir, link_path, target) in ok_shapes {
            let td = TempDir::new().unwrap();
            let dest = td.path();
            std::fs::create_dir_all(dest.join(dir)).unwrap();
            let link = dest.join(link_path);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();

            let resolved = path_containment::resolve_symlink_target(dest, target, &link);
            assert_eq!(
                path_containment::copy_would_recurse(&resolved, &link).ok(),
                Some(false),
                "{name}: {link_path} -> {target:?} is an ordinary copy and must be ALLOWED"
            );
        }
    }

    /// The FAIL-SAFE BACKSTOP: even if the containment guard were bypassed
    /// entirely, the copy cannot run away — it stops at MAX_COPY_DEPTH with a
    /// clean error instead of marching into MAX_PATH.
    ///
    /// This is the second oracle. A single lexical/canonical check over untrusted
    /// input is not enough, and it is a TOCTOU snapshot besides; without a bound,
    /// any miss is unbounded rather than merely wrong.
    #[test]
    fn copy_dir_recursive_is_depth_bounded() {
        let td = TempDir::new().unwrap();

        // Build a chain of `levels` nested directories under a fresh src, and
        // return the copy's result. Pins the BOUNDARY, so an off-by-one in the
        // cap check (`>` vs `>=`) cannot slip back in.
        let attempt = |name: &str, levels: u32| -> Result<()> {
            let src = td.path().join(name);
            let mut deep = src.clone();
            for i in 0..levels {
                deep = deep.join(format!("l{i}"));
            }
            std::fs::create_dir_all(&deep).unwrap();
            copy_dir_recursive(&src, &td.path().join(format!("{name}-dst")), 0)
        };

        // Exactly at the cap: allowed.
        attempt("at-cap", MAX_COPY_DEPTH - 1).expect("a tree at the depth cap must still copy");

        // One level past it: refused, cleanly.
        let err = attempt("over-cap", MAX_COPY_DEPTH)
            .expect_err("a tree deeper than the cap must be refused");
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("maximum directory depth")),
            "expected the depth-cap refusal, got: {err}"
        );

        // Far past it: still refused, not a stack overflow.
        assert!(attempt("way-over", MAX_COPY_DEPTH + 8).is_err());

        // A tree within the cap still copies normally.
        let shallow_src = td.path().join("ok-src");
        std::fs::create_dir_all(shallow_src.join("a/b/c")).unwrap();
        std::fs::write(shallow_src.join("a/b/c/f.txt"), b"hello").unwrap();
        let shallow_dst = td.path().join("ok-dst");
        copy_dir_recursive(&shallow_src, &shallow_dst, 0).unwrap();
        assert_eq!(
            std::fs::read(shallow_dst.join("a/b/c/f.txt")).unwrap(),
            b"hello"
        );
    }

    /// INVARIANT: no manifest that is valid on unix becomes invalid. All three
    /// shapes are legitimate relative intra-workspace links and still hydrate
    /// here, as real symlinks with their target preserved verbatim.
    #[cfg(unix)]
    #[tokio::test]
    async fn self_referential_symlinks_still_hydrate_on_unix() {
        for (name, file, link_path, target) in SELF_REF_SHAPES {
            let (manifest, d, bytes) = self_referential_symlink_fixture(file, link_path, target);
            let (store, _cache_dir, cache) = setup_store(&manifest, &[(d, bytes)]);
            let dest_dir = TempDir::new().unwrap();
            let dest = dest_dir.path().join("out");
            hydrate(&dest, &store, &cache, &HydrateOptions::new("test-ws"))
                .await
                .unwrap_or_else(|e| panic!("{name}: must still hydrate on unix: {e}"));
            let link = dest.join(link_path);
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{name}: still a real symlink on unix"
            );
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                std::path::Path::new(target),
                "{name}: target preserved verbatim"
            );
        }
    }

    /// Windows counterpart: the copy-fallback REJECTS all three with a clear
    /// error instead of recursing, and leaves no runaway tree behind.
    #[cfg(windows)]
    #[tokio::test]
    async fn self_referential_symlinks_rejected_on_windows() {
        // Sum of every file under `p` — an unbounded copy shows up here.
        fn tree_bytes(p: &std::path::Path) -> u64 {
            let mut total = 0;
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    match e.file_type() {
                        Ok(t) if t.is_dir() => total += tree_bytes(&e.path()),
                        _ => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                    }
                }
            }
            total
        }

        for (name, file, link_path, target) in SELF_REF_SHAPES {
            let (manifest, d, bytes) = self_referential_symlink_fixture(file, link_path, target);
            let (store, _cache_dir, cache) = setup_store(&manifest, &[(d, bytes)]);
            let dest_dir = TempDir::new().unwrap();
            let dest = dest_dir.path().join("out");
            let err = hydrate(&dest, &store, &cache, &HydrateOptions::new("test-ws"))
                .await
                .unwrap_err();
            // Either the containment guard fired, or it could not be proven safe
            // and we failed CLOSED. Both are refusals; neither is a copy.
            assert!(
                matches!(err, ClwError::InvalidManifest(ref m)
                    if m.contains("contains the link itself")
                        || m.contains("cannot verify")),
                "{name}: expected an M17 refusal, got: {err}"
            );
            assert!(
                tree_bytes(&dest) < 1024 * 1024,
                "{name}: the aborted hydrate must not have copied a runaway tree"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Test 8 (M1.6): a manifest with more entries than MAX_MANIFEST_ENTRIES is
    // rejected up-front with InvalidManifest, before any filesystem work.
    // We build the entry list directly (bypassing canonical_bytes round-trip)
    // so the test stays cheap — the guard runs on the parsed in-memory manifest.
    // -----------------------------------------------------------------------

    #[test]
    fn oversized_entry_count_rejected() {
        // The cap is a pure guard — exercise it at the boundary directly instead
        // of building a million-entry manifest (which made this test ~minute-long
        // via canonical_bytes serialization). The guard runs before any FS work.
        assert!(enforce_resource_bounds(MAX_MANIFEST_ENTRIES, 0, 0, 0).is_ok());
        let err = enforce_resource_bounds(MAX_MANIFEST_ENTRIES + 1, 0, 0, 0).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("entries")),
            "expected InvalidManifest about entry count, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 8b (M1.6): the symlink-count cap is enforced up-front in
    // enforce_resource_bounds, before any filesystem work.
    // -----------------------------------------------------------------------

    #[test]
    fn oversized_symlink_count_rejected() {
        assert!(enforce_resource_bounds(0, 0, MAX_SYMLINK_ENTRIES, 0).is_ok());
        let err = enforce_resource_bounds(0, 0, MAX_SYMLINK_ENTRIES + 1, 0).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("symlink")),
            "expected InvalidManifest about symlinks, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 9 (M1.6 / FIX 1): a manifest whose REAL size (summed chunk sizes)
    // exceeds MAX_HYDRATE_TOTAL_BYTES is rejected up-front with InvalidManifest.
    // CHANGED from the original (which used a bare Dir + an inflated declared
    // total_size): the budget is now enforced against the summed chunk sizes, so
    // the guard is exercised with a File whose ChunkRef.size is over the cap and a
    // declared total_size that AGREES (so the FIX-1 agreement check passes and the
    // disk-fill budget guard is what fires).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn oversized_total_bytes_rejected() {
        // A file declaring a single chunk whose size is (cap + 1) bytes. We never
        // actually fetch the chunk — the budget guard fires before any FS work.
        let fake_digest = Digest::of_bytes(b"never-fetched");
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: MAX_HYDRATE_TOTAL_BYTES + 1,
            entries: vec![Entry::File {
                path: "huge.bin".into(),
                mode: 0o644,
                size: MAX_HYDRATE_TOTAL_BYTES + 1,
                chunks: vec![ChunkRef {
                    digest: fake_digest,
                    size: MAX_HYDRATE_TOTAL_BYTES + 1,
                }],
            }],
        };

        // setup_store would try to find the chunk bytes; build the store inline.
        let store = FakeStore::new();
        let manifest_bytes = manifest.canonical_bytes().unwrap();
        let manifest_digest = Digest::of_bytes(&manifest_bytes);
        store.cas_put_raw(manifest_digest, manifest_bytes.clone());
        let record = RefRecord {
            name: "test-ws".into(),
            root: manifest_digest,
            manifest_size: manifest_bytes.len() as u64,
            created_at_unix: 0,
            parent: None,
            tool_version: "0.1.0".into(),
        };
        store.ac_put_raw(clw_types::ref_key("test-ws"), record.to_bytes().unwrap());
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();

        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("chunks sum")),
            "expected InvalidManifest about summed chunk bytes, got: {err}"
        );
        assert!(
            !dest.join("huge.bin").exists(),
            "guard must fire before any file is created"
        );
    }

    // -----------------------------------------------------------------------
    // Test 10 (FIX 1): the disk-fill guard is enforced against the REAL summed
    // chunk sizes, NOT the attacker-declared total_size. A manifest declaring a
    // tiny (or zero) total_size while its chunks sum huge must be REJECTED before
    // any write — both as an integrity disagreement and (had they agreed) as an
    // over-budget request. Here the declared total is deliberately tiny, so the
    // agreement check catches the bypass.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn declared_total_size_disagreeing_with_chunks_is_rejected() {
        // Two real chunks summing to a known size, but total_size declared as 0 —
        // the classic disk-fill bypass. We seed the chunks so this would otherwise
        // hydrate; the guard must abort BEFORE writing.
        let (d1, b1) = make_chunk(b"chunk-aaaa");
        let (d2, b2) = make_chunk(b"chunk-bbbbbb");
        let real = (b1.len() + b2.len()) as u64;

        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0, // LIE: real chunks sum to `real` bytes.
            entries: vec![Entry::File {
                path: "f.bin".into(),
                mode: 0o644,
                size: real,
                chunks: vec![
                    ChunkRef {
                        digest: d1,
                        size: b1.len() as u64,
                    },
                    ChunkRef {
                        digest: d2,
                        size: b2.len() as u64,
                    },
                ],
            }],
        };

        let (store, _cache_dir, cache) =
            setup_store(&manifest, &[(d1, b1.clone()), (d2, b2.clone())]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("disagrees")),
            "expected InvalidManifest about declared/real disagreement, got: {err}"
        );
        assert!(
            !dest.join("f.bin").exists(),
            "disk-fill bypass must abort before any file is written"
        );
    }

    // -----------------------------------------------------------------------
    // Test 11 (FIX 6): the per-file and manifest-wide chunk-count caps reject a
    // chunk Vec larger than the bound. The caps are pure guards (they take counts)
    // so we exercise them at the boundary directly — building 16M/64M ChunkRefs in
    // a unit test is infeasible, and the guards run on the count before any FS work.
    // -----------------------------------------------------------------------

    #[test]
    fn oversized_chunk_counts_rejected() {
        // Per-file cap.
        assert!(enforce_per_file_chunk_cap("f.bin", MAX_CHUNKS_PER_FILE).is_ok());
        let err = enforce_per_file_chunk_cap("f.bin", MAX_CHUNKS_PER_FILE + 1).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("per-file cap")),
            "expected InvalidManifest about per-file chunk count, got: {err}"
        );

        // Manifest-wide total-chunk cap.
        assert!(enforce_resource_bounds(1, 0, 0, MAX_TOTAL_CHUNKS).is_ok());
        let err = enforce_resource_bounds(1, 0, 0, MAX_TOTAL_CHUNKS + 1).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(ref m) if m.contains("chunks total")),
            "expected InvalidManifest about total chunk count, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 11b (FIX 6 regression): a LEGITIMATE multi-chunk file (well under both
    // caps) must still hydrate — the new guards don't reject honest input.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn legitimate_multichunk_file_still_hydrates_under_new_caps() {
        let (d1, b1) = make_chunk(b"part-one-");
        let (d2, b2) = make_chunk(b"part-two!");
        let real = (b1.len() + b2.len()) as u64;
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: real,
            entries: vec![Entry::File {
                path: "multi.bin".into(),
                mode: 0o644,
                size: real,
                chunks: vec![
                    ChunkRef {
                        digest: d1,
                        size: b1.len() as u64,
                    },
                    ChunkRef {
                        digest: d2,
                        size: b2.len() as u64,
                    },
                ],
            }],
        };
        let (store, _cache_dir, cache) =
            setup_store(&manifest, &[(d1, b1.clone()), (d2, b2.clone())]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");
        let opts = HydrateOptions::new("test-ws");
        hydrate(&dest, &store, &cache, &opts).await.unwrap();
        let mut expected = b1.clone();
        expected.extend_from_slice(&b2);
        assert_eq!(std::fs::read(dest.join("multi.bin")).unwrap(), expected);
    }

    #[tokio::test]
    async fn zero_file_concurrency_is_normalized_to_one() {
        let (digest, bytes) = make_chunk(b"zero-is-one");
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: bytes.len() as u64,
            entries: vec![Entry::File {
                path: "one.bin".into(),
                mode: 0o644,
                size: bytes.len() as u64,
                chunks: vec![ChunkRef {
                    digest,
                    size: bytes.len() as u64,
                }],
            }],
        };
        let (store, _cache_dir, cache) = setup_store(&manifest, &[(digest, bytes.clone())]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");
        let mut opts = HydrateOptions::new("test-ws");
        opts.concurrency = 0;

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            hydrate(&dest, &store, &cache, &opts),
        )
        .await
        .expect("N=0 must not stall buffer_unordered")
        .unwrap();
        assert_eq!(std::fs::read(dest.join("one.bin")).unwrap(), bytes);
    }

    // -----------------------------------------------------------------------
    // Test 12 (M9): a multi-chunk file fetches its chunks with BOUNDED
    // CONCURRENCY but writes them IN ORDER. The CAS below has INVERTED per-chunk
    // latency (earlier chunks are SLOWER), so under an *unordered* buffer the
    // later chunks would complete — and be written — first, scrambling the file.
    // A byte-exact concat match proves the ordered `buffered` stream preserved
    // offset order. RED-on-revert: swapping `buffered` → `buffer_unordered`
    // reorders the writes and this concat assertion fails.
    // -----------------------------------------------------------------------

    /// A CAS fake that sleeps a per-digest delay before returning bytes, so chunk
    /// completion order can be forced to DIFFER from submission order.
    struct DelayStore {
        cas: Mutex<HashMap<Digest, (Vec<u8>, u64)>>, // digest -> (bytes, delay_ms)
    }

    #[async_trait::async_trait]
    impl CasTransport for DelayStore {
        async fn exists(&self, digest: &Digest) -> Result<bool> {
            Ok(self.cas.lock().unwrap().contains_key(digest))
        }
        async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
            let (bytes, delay) = self
                .cas
                .lock()
                .unwrap()
                .get(digest)
                .cloned()
                .ok_or(ClwError::NotFound(*digest))?;
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            Ok(bytes)
        }
        async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
            self.cas.lock().unwrap().insert(*digest, (bytes, 0));
            Ok(())
        }
        async fn delete(&self, digest: &Digest) -> Result<()> {
            self.cas.lock().unwrap().remove(digest);
            Ok(())
        }
        async fn list_blobs(&self) -> Result<Vec<Digest>> {
            Ok(self.cas.lock().unwrap().keys().copied().collect())
        }
    }

    /// A CAS fake that holds every fetch until the test releases it and records
    /// the true maximum in-flight count. This makes the per-file P cap
    /// observable rather than merely testing a helper constant.
    struct CappedFetchStore {
        cas: Mutex<HashMap<Digest, Vec<u8>>>,
        in_flight: std::sync::atomic::AtomicUsize,
        max_in_flight: std::sync::atomic::AtomicUsize,
        reached_cap: tokio::sync::Notify,
        release: tokio::sync::Notify,
        released: std::sync::atomic::AtomicBool,
    }

    impl CappedFetchStore {
        fn new() -> Self {
            Self {
                cas: Mutex::new(HashMap::new()),
                in_flight: std::sync::atomic::AtomicUsize::new(0),
                max_in_flight: std::sync::atomic::AtomicUsize::new(0),
                reached_cap: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
                released: std::sync::atomic::AtomicBool::new(false),
            }
        }

        async fn wait_until_released(&self) {
            use std::sync::atomic::Ordering;

            while !self.released.load(Ordering::Acquire) {
                let wake = self.release.notified();
                if self.released.load(Ordering::Acquire) {
                    break;
                }
                wake.await;
            }
        }

        fn release_all(&self) {
            use std::sync::atomic::Ordering;

            self.released.store(true, Ordering::Release);
            self.release.notify_waiters();
        }
    }

    #[async_trait::async_trait]
    impl CasTransport for CappedFetchStore {
        async fn exists(&self, digest: &Digest) -> Result<bool> {
            Ok(self.cas.lock().unwrap().contains_key(digest))
        }

        async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
            use std::sync::atomic::Ordering;

            let bytes = self
                .cas
                .lock()
                .unwrap()
                .get(digest)
                .cloned()
                .ok_or(ClwError::NotFound(*digest))?;
            let in_flight = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_in_flight.fetch_max(in_flight, Ordering::AcqRel);
            if in_flight == PER_FILE_FETCH_CAP {
                self.reached_cap.notify_one();
            }
            self.wait_until_released().await;
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            Ok(bytes)
        }

        async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
            self.cas.lock().unwrap().insert(*digest, bytes);
            Ok(())
        }

        async fn delete(&self, digest: &Digest) -> Result<()> {
            self.cas.lock().unwrap().remove(digest);
            Ok(())
        }

        async fn list_blobs(&self) -> Result<Vec<Digest>> {
            Ok(self.cas.lock().unwrap().keys().copied().collect())
        }
    }

    #[tokio::test]
    async fn multichunk_file_fetches_concurrent_but_writes_in_order() {
        const N: usize = 8;
        // Distinct per-chunk payloads so ANY reorder is detectable in the concat.
        let payloads: Vec<Vec<u8>> = (0..N)
            .map(|i| format!("<<chunk-{i:02}-distinct-payload>>").into_bytes())
            .collect();

        let store = DelayStore {
            cas: Mutex::new(HashMap::new()),
        };
        let mut chunks = Vec::new();
        let mut expected = Vec::new();
        for (i, p) in payloads.iter().enumerate() {
            let d = Digest::of_bytes(p);
            // INVERT latency: earlier chunks are SLOWER, so under an unordered
            // buffer the later chunks would complete (and be written) FIRST.
            let delay_ms = ((N - i) as u64) * 10;
            store.cas.lock().unwrap().insert(d, (p.clone(), delay_ms));
            chunks.push(ChunkRef {
                digest: d,
                size: p.len() as u64,
            });
            expected.extend_from_slice(p);
        }

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path();

        // Concurrency >= N so every chunk is in flight at once (max reorder pressure).
        let (from_cache, downloaded) =
            materialize_file(dest, "multi.bin", 0o644, &chunks, N + 4, &store, &cache)
                .await
                .unwrap();

        assert_eq!(
            std::fs::read(dest.join("multi.bin")).unwrap(),
            expected,
            "chunks must be written in manifest order despite inverted fetch latency"
        );
        assert_eq!(from_cache, 0, "all chunks were cache misses");
        assert_eq!(downloaded, expected.len() as u64, "all bytes downloaded");
    }

    #[tokio::test]
    async fn per_file_fetches_never_exceed_cap_under_huge_requested_concurrency() {
        use std::sync::atomic::Ordering;

        const CHUNK_COUNT: usize = PER_FILE_FETCH_CAP + 5;
        let store = CappedFetchStore::new();
        let mut chunks = Vec::with_capacity(CHUNK_COUNT);
        let mut expected = Vec::new();
        for i in 0..CHUNK_COUNT {
            let bytes = format!("cap-probe-{i:02}").into_bytes();
            let digest = Digest::of_bytes(&bytes);
            store.cas.lock().unwrap().insert(digest, bytes.clone());
            chunks.push(ChunkRef {
                digest,
                size: bytes.len() as u64,
            });
            expected.extend_from_slice(&bytes);
        }

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let materialize = materialize_file(
            dest_dir.path(),
            "capped.bin",
            0o644,
            &chunks,
            usize::MAX,
            &store,
            &cache,
        );
        tokio::pin!(materialize);
        // Register before driving materialize so the Notify cannot lose the
        // cap-reaching event. With raw requested concurrency this records >P
        // before control returns here, making the test red on that regression.
        let reached_cap = store.reached_cap.notified();
        tokio::pin!(reached_cap);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::select! {
                _ = &mut reached_cap => {},
                result = &mut materialize => panic!("materialize completed before reaching P: {result:?}"),
            }
        })
        .await
        .expect("P fetches should become in-flight promptly");
        assert_eq!(
            store.max_in_flight.load(Ordering::Acquire),
            PER_FILE_FETCH_CAP,
            "P, not the caller's huge N, must bound real in-flight fetches"
        );

        store.release_all();
        materialize.await.unwrap();
        assert_eq!(
            std::fs::read(dest_dir.path().join("capped.bin")).unwrap(),
            expected
        );
    }

    // -----------------------------------------------------------------------
    // WP42: the blocking writer's protocol is commit-on-Finish. These direct
    // tests keep failure/cancellation timing deterministic without exposing a
    // production payload or transport seam.
    // -----------------------------------------------------------------------

    fn assert_no_tmp_or_final(parent: &Path, final_path: &Path) {
        assert!(
            !final_path.exists(),
            "an aborted writer must not publish a partial final file"
        );
        assert!(
            std::fs::read_dir(parent).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".clw-tmp-")),
            "an aborted writer must eventually remove its temporary file"
        );
    }

    #[cfg(unix)]
    async fn writer_checkpoint(tx: &tokio::sync::mpsc::Sender<FileWrite>) {
        let (ready, observed) = tokio::sync::oneshot::channel();
        tx.send(FileWrite::Checkpoint(ready)).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), observed)
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writer_temp_is_private_during_writes_and_mode_precedes_publication() {
        for requested_mode in [0o600, 0o640, 0o644, 0o755] {
            let dir = TempDir::new().unwrap();
            let parent = dir.path().to_path_buf();
            let final_path = parent.join("out.bin");
            let tmp_path = parent.join(".clw-tmp-private");
            let (tx, writer) =
                spawn_file_writer(tmp_path.clone(), final_path.clone(), parent, requested_mode);
            // The actual writer acknowledges each boundary, not a timer.
            writer_checkpoint(&tx).await;
            assert_eq!(
                std::fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            tx.send(FileWrite::Chunk(b"private-".to_vec()))
                .await
                .unwrap();
            writer_checkpoint(&tx).await;
            assert_eq!(std::fs::read(&tmp_path).unwrap(), b"private-");
            assert_eq!(
                std::fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            tx.send(FileWrite::Chunk(b"bytes".to_vec())).await.unwrap();
            writer_checkpoint(&tx).await;
            assert_eq!(std::fs::read(&tmp_path).unwrap(), b"private-bytes");
            assert_eq!(
                std::fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(!final_path.exists());

            let (ready, observed) = tokio::sync::oneshot::channel();
            let (resume, paused) = tokio::sync::oneshot::channel();
            tx.send(FileWrite::BeforePublish {
                ready,
                resume: paused,
            })
            .await
            .unwrap();
            tx.send(FileWrite::Finish).await.unwrap();
            drop(tx);
            tokio::time::timeout(std::time::Duration::from_secs(10), observed)
                .await
                .unwrap()
                .unwrap();
            // Only the intended final permissions may be applied. The final
            // pathname is still absent while metadata and bytes are synced.
            assert!(!final_path.exists());
            assert_eq!(
                std::fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777,
                requested_mode
            );
            resume.send(()).unwrap();
            writer.await.unwrap().unwrap();
            assert!(!tmp_path.exists());
            assert_eq!(
                std::fs::metadata(&final_path).unwrap().permissions().mode() & 0o777,
                requested_mode
            );
            assert_eq!(std::fs::read(&final_path).unwrap(), b"private-bytes");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writer_empty_file_is_private_until_final_metadata() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let final_path = parent.join("empty.bin");
        let tmp_path = parent.join(".clw-tmp-empty-private");
        let (tx, writer) = spawn_file_writer(tmp_path.clone(), final_path.clone(), parent, 0o640);
        writer_checkpoint(&tx).await;
        assert_eq!(
            std::fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!final_path.exists());
        tx.send(FileWrite::Finish).await.unwrap();
        drop(tx);
        writer.await.unwrap().unwrap();
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"");
        assert_eq!(
            std::fs::metadata(&final_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[tokio::test]
    async fn writer_exclusive_open_preserves_preexisting_temp() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let tmp_path = parent.join(".clw-tmp-already-owned");
        let final_path = parent.join("out.bin");
        std::fs::write(&tmp_path, b"existing owned bytes").unwrap();
        let (tx, writer) = spawn_file_writer(tmp_path.clone(), final_path.clone(), parent, 0o600);
        drop(tx);
        let error = writer.await.unwrap().unwrap_err();
        assert!(matches!(error, ClwError::Io(ref e) if e.kind() == ErrorKind::AlreadyExists));
        assert_eq!(std::fs::read(&tmp_path).unwrap(), b"existing owned bytes");
        assert!(!final_path.exists());
    }

    #[tokio::test]
    async fn sync_failure_aborts_before_publication_and_cleans_owned_temp() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let tmp_path = parent.join(".clw-tmp-sync-fail");
        let final_path = parent.join("out.bin");
        let (tx, writer) = spawn_file_writer(tmp_path, final_path.clone(), parent.clone(), 0o600);
        tx.send(FileWrite::Chunk(b"synthetic".to_vec()))
            .await
            .unwrap();
        tx.send(FileWrite::FailSync).await.unwrap();
        tx.send(FileWrite::Finish).await.unwrap();
        drop(tx);
        assert!(matches!(
            writer.await.unwrap().unwrap_err(),
            ClwError::Io(_)
        ));
        assert_no_tmp_or_final(&parent, &final_path);
    }

    #[tokio::test]
    async fn writer_eof_without_finish_aborts_and_cleans_temp() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let final_path = parent.join("out.bin");
        let tmp_path = parent.join(".clw-tmp-eof");
        let (tx, writer) = spawn_file_writer(tmp_path, final_path.clone(), parent.clone(), 0o644);

        tx.send(FileWrite::Chunk(b"partial".to_vec()))
            .await
            .unwrap();
        drop(tx); // cancellation/fetch failure closes Q without the commit message
        let err = writer.await.unwrap().unwrap_err();
        assert!(matches!(err, ClwError::Transport(_)));
        assert_no_tmp_or_final(&parent, &final_path);
    }

    #[tokio::test]
    async fn injected_writer_failure_leaves_no_final_or_temp() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let final_path = parent.join("out.bin");
        let tmp_path = parent.join(".clw-tmp-fail");
        let (tx, writer) = spawn_file_writer(tmp_path, final_path.clone(), parent.clone(), 0o644);

        tx.send(FileWrite::Chunk(b"partial".to_vec()))
            .await
            .unwrap();
        tx.send(FileWrite::FailWrite).await.unwrap();
        drop(tx);
        let err = writer.await.unwrap().unwrap_err();
        assert!(matches!(err, ClwError::Io(_)));
        assert_no_tmp_or_final(&parent, &final_path);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mode_failure_aborts_before_publication_and_cleans_temp() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let final_path = parent.join("out.bin");
        let tmp_path = parent.join(".clw-tmp-mode-fail");
        let (tx, writer) = spawn_file_writer(tmp_path, final_path.clone(), parent.clone(), 0o600);

        tx.send(FileWrite::Chunk(b"private".to_vec()))
            .await
            .unwrap();
        tx.send(FileWrite::FailMode).await.unwrap();
        tx.send(FileWrite::Finish).await.unwrap();
        drop(tx);
        let err = writer.await.unwrap().unwrap_err();
        assert!(matches!(err, ClwError::Io(_)));
        assert_no_tmp_or_final(&parent, &final_path);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rename_failure_does_not_publish_and_cleans_temp() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let final_path = parent.join("occupied");
        std::fs::create_dir(&final_path).unwrap();
        let tmp_path = parent.join(".clw-tmp-rename-fail");
        let (tx, writer) =
            spawn_file_writer(tmp_path.clone(), final_path.clone(), parent.clone(), 0o600);

        tx.send(FileWrite::Chunk(b"private".to_vec()))
            .await
            .unwrap();
        tx.send(FileWrite::Finish).await.unwrap();
        drop(tx);
        let err = writer.await.unwrap().unwrap_err();
        assert!(matches!(err, ClwError::Io(_)));
        assert!(
            final_path.is_dir(),
            "existing destination directory must remain"
        );
        assert!(
            !tmp_path.exists(),
            "failed rename must clean the private temp"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_writer_keeps_current_thread_runtime_responsive() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let dir = TempDir::new().unwrap();
        let parent = dir.path().to_path_buf();
        let final_path = parent.join("out.bin");
        let tmp_path = parent.join(".clw-tmp-heartbeat");
        let (tx, writer) = spawn_file_writer(tmp_path, final_path, parent, 0o644);
        let done = Arc::new(AtomicBool::new(false));
        let beats = Arc::new(AtomicUsize::new(0));
        let heartbeat_done = Arc::clone(&done);
        let heartbeat_beats = Arc::clone(&beats);
        let heartbeat = tokio::spawn(async move {
            while !heartbeat_done.load(Ordering::Relaxed) {
                heartbeat_beats.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        });

        tx.send(FileWrite::BlockFor(std::time::Duration::from_millis(100)))
            .await
            .unwrap();
        tx.send(FileWrite::Finish).await.unwrap();
        drop(tx);
        writer.await.unwrap().unwrap();
        done.store(true, Ordering::Relaxed);
        heartbeat.await.unwrap();
        assert!(
            beats.load(Ordering::Relaxed) > 1,
            "a blocking file operation must not starve a current-thread runtime"
        );
    }

    #[test]
    fn file_and_fetch_concurrency_are_normalized_and_bounded() {
        assert_eq!(normalized_concurrency(0), 1, "N=0 must make progress");
        assert_eq!(per_file_fetch_concurrency(0), 1);
        assert_eq!(per_file_fetch_concurrency(usize::MAX), PER_FILE_FETCH_CAP);
        assert_eq!(FILE_WRITE_QUEUE_CAPACITY, 1, "Q must remain one chunk");
    }

    // -----------------------------------------------------------------------
    // Test 13 (LOW dest-guard): a real read_dir/permission error on `dest` must
    // surface AS an IO/permission error — NOT be misreported as "destination not
    // empty" (the old `.unwrap_or(false)` collapsed the two). Unix-only: we make
    // `dest` mode-000 so opendir returns PermissionDenied. If the process can
    // still read it (e.g. running as root, which bypasses perms) the guard cannot
    // observe the error, so we skip.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_dest_surfaces_io_error_not_not_empty() {
        let store = FakeStore::new();
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();

        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o000)).unwrap();

        // Root bypasses permission bits — the guard can't fail; restore + skip.
        if std::fs::read_dir(&dest).is_ok() {
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        // Restore perms so the TempDir can clean up regardless of the outcome.
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();

        match err {
            ClwError::Io(ref e) => {
                assert_eq!(
                    e.kind(),
                    ErrorKind::PermissionDenied,
                    "a read_dir permission error must surface as PermissionDenied, got: {e}"
                );
                assert!(
                    !e.to_string().contains("destination not empty"),
                    "an IO/permission error must NOT be misreported as 'destination not empty': {e}"
                );
            }
            other => panic!("expected ClwError::Io(PermissionDenied), got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Test 14 (LOW temp-cleanup): a mid-file failure (a LATER chunk missing from
    // CAS) must leave NO `.clw-tmp-*` stray under `dest`. We seed a two-chunk file
    // but only the FIRST chunk, so the second fetch fails after the temp already
    // holds chunk one — the drop-guard must remove it on the error path.
    // RED-on-revert: dropping the guard leaks the temp and this scan finds it.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn mid_file_failure_leaves_no_tmp_stray() {
        let (d1, b1) = make_chunk(b"first-chunk-bytes");
        let second = b"second-chunk-never-seeded".as_ref();
        let (d2, _b2) = make_chunk(second);
        let declared = (b1.len() + second.len()) as u64;

        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: declared,
            entries: vec![Entry::File {
                path: "big.bin".into(),
                mode: 0o644,
                size: declared,
                chunks: vec![
                    ChunkRef {
                        digest: d1,
                        size: b1.len() as u64,
                    },
                    ChunkRef {
                        digest: d2,
                        size: second.len() as u64,
                    },
                ],
            }],
        };

        // Build the store inline: manifest + ref + ONLY the first chunk.
        let store = FakeStore::new();
        let manifest_bytes = manifest.canonical_bytes().unwrap();
        let manifest_digest = Digest::of_bytes(&manifest_bytes);
        store.cas_put_raw(manifest_digest, manifest_bytes.clone());
        store.cas_put_raw(d1, b1.clone());
        let record = RefRecord {
            name: "test-ws".into(),
            root: manifest_digest,
            manifest_size: manifest_bytes.len() as u64,
            created_at_unix: 0,
            parent: None,
            tool_version: "0.1.0".into(),
        };
        store.ac_put_raw(clw_types::ref_key("test-ws"), record.to_bytes().unwrap());

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let err = hydrate(&dest, &store, &cache, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::NotFound(_)),
            "expected NotFound for the unseeded second chunk, got: {err}"
        );

        fn has_tmp_stray(dir: &Path) -> bool {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                if e.file_name().to_string_lossy().starts_with(".clw-tmp-") {
                    return true;
                }
                if e.file_type().unwrap().is_dir() && has_tmp_stray(&e.path()) {
                    return true;
                }
            }
            false
        }
        assert!(
            !has_tmp_stray(&dest),
            "a mid-file failure must leave no .clw-tmp-* stray under dest"
        );
        assert!(
            !dest.join("big.bin").exists(),
            "the failed file must not be materialised"
        );
    }

    // -----------------------------------------------------------------------
    // Test 15 (base case): an EMPTY snapshot — a manifest with ZERO entries — is
    // a legal empty workspace. hydrate must CREATE the destination and report
    // files=0 / bytes_total=0, NOT trip the dest-empty guard or error on the
    // empty entry list. Previously untested (the empty-snapshot journey gap).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn empty_snapshot_hydrates_to_empty_dest() {
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0,
            entries: vec![],
        };
        // No chunks referenced — setup_store seeds only the manifest blob + ref.
        let (store, _cache_dir, cache) = setup_store(&manifest, &[]);
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");

        let opts = HydrateOptions::new("test-ws");
        let report = hydrate(&dest, &store, &cache, &opts).await.unwrap();

        assert!(
            dest.is_dir(),
            "an empty snapshot must still create the dest"
        );
        assert_eq!(
            std::fs::read_dir(&dest).unwrap().count(),
            0,
            "the materialised dest of an empty snapshot must be empty"
        );
        assert_eq!(report.files, 0, "no file entries");
        assert_eq!(report.bytes_total, 0, "no declared bytes");
        assert_eq!(report.bytes_from_cache, 0);
        assert_eq!(report.bytes_downloaded, 0);
        assert_eq!(report.root, {
            let mb = manifest.canonical_bytes().unwrap();
            Digest::of_bytes(&mb)
        });
    }

    // -----------------------------------------------------------------------
    // Test 16 (containment defense-in-depth): hydrate re-validates every entry
    // path with its OWN `validate_rel_path` re-check, so a hostile path that
    // somehow bypassed `Manifest::from_bytes` is STILL rejected before any
    // filesystem work. That re-check LOOKS redundant precisely because
    // `from_bytes` (via `validate_canonical`) blocks a hostile path first on the
    // whole-hydrate path — so it is unreachable through `hydrate()`. We exercise
    // the internal `materialize_file` guard directly (same seam the concurrency
    // test uses) with in-code hostile paths (absolute + `..` escapes) it would
    // otherwise never see. RED-on-revert: deleting the `validate_rel_path(rel_path)?`
    // line in `materialize_file` lets these paths reach `dest.join(..)` and escape.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn materialize_file_re_rejects_hostile_rel_path() {
        let store = FakeStore::new();
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();

        // Absolute, single `..`, and an interior `..` that escapes the root.
        for hostile in ["/etc/clw-escape", "../escape.txt", "a/../../b"] {
            let err = materialize_file(&dest, hostile, 0o644, &[], 4, &store, &cache)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ClwError::InvalidManifest(_)),
                "hostile path {hostile:?} must be rejected by hydrate's own re-check, got: {err:?}"
            );
        }

        // The guard fires before any FS work, so nothing escaped the destination.
        assert!(
            !dest_dir.path().join("escape.txt").exists(),
            "a `..` path must not have written outside dest"
        );
        assert!(
            std::fs::read_dir(&dest).unwrap().next().is_none(),
            "no partial output under dest for a rejected path"
        );
    }

    // -----------------------------------------------------------------------
    // Test 17 (mid-WRITE IO cleanup): the fetch-side failure path is covered by
    // `mid_file_failure_leaves_no_tmp_stray`; this covers the WRITE side. A file's
    // chunk is fully fetched + written to the per-file temp, then the FINAL
    // rename fails because a directory already occupies the destination name
    // (`rename(file, dir)` → an IO error on unix). The drop-guard must remove the
    // `.clw-tmp-*` temp and the IO error must surface. RED-on-revert: dropping the
    // TmpFileGuard leaks the written temp and this scan finds it.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[tokio::test]
    async fn mid_write_io_failure_leaves_no_tmp_stray_and_surfaces_io() {
        let (d, bytes) = make_chunk(b"payload-that-actually-gets-written");
        let store = FakeStore::new();
        store.cas_put_raw(d, bytes.clone());

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let dest_dir = TempDir::new().unwrap();
        let dest = dest_dir.path().to_path_buf();

        // Occupy the destination file name with a NON-EMPTY directory: the temp
        // is created + written first (dest is writable), and only the trailing
        // rename onto this name fails — exercising the post-write cleanup.
        let blocker = dest.join("f.bin");
        std::fs::create_dir_all(&blocker).unwrap();
        std::fs::write(blocker.join("resident"), b"keep-me").unwrap();

        let chunks = vec![ChunkRef {
            digest: d,
            size: bytes.len() as u64,
        }];
        let err = materialize_file(&dest, "f.bin", 0o644, &chunks, 4, &store, &cache)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClwError::Io(_)),
            "a failed rename after the write must surface as an IO error, got: {err:?}"
        );

        // The write-side failure must have cleaned up its temp: NO stray remains.
        fn has_tmp_stray(dir: &Path) -> bool {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                if e.file_name().to_string_lossy().starts_with(".clw-tmp-") {
                    return true;
                }
            }
            false
        }
        assert!(
            !has_tmp_stray(&dest),
            "a mid-write IO failure must leave no .clw-tmp-* stray under dest"
        );
        // The pre-existing directory entry is untouched by the failed materialise.
        assert!(
            blocker.join("resident").exists(),
            "the failed materialise must not disturb the resident dir entry"
        );
    }

    // Note (M7 fsync durability): the `sync_all()` before rename + the best-effort
    // parent-directory fsync harden hydrate against a torn file on power loss.
    // That is a crash/power-loss durability property and is NOT observable from an
    // in-process unit test (nothing here can drop write-back cache blocks while
    // keeping rename metadata). The `full_hydrate_*`, `hydrate_rerun_heals_*`, and
    // `multichunk_*` tests assert the file materialises byte-exactly; we honestly
    // do NOT fake a power-loss assertion for the fsync itself.
}
