//! clw-hydrate — ref → manifest → cache-first materialize pipeline.
//!
//! Reads a workspace ref from the AC store, fetches the manifest from CAS,
//! then materialises every entry (files, symlinks, empty dirs) into `dest`
//! with file-level concurrency bounded by [`HydrateOptions::concurrency`].
//!
//! All network calls use in-process fakes in tests — never the real endpoint.

pub use clw_types as types;

use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;

use clw_cache::LocalCache;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Entry, Manifest, RefRecord, Result};

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
/// Rationale (anti-OOM / anti-enumeration-bomb): every entry is iterated, and
/// `File` entries are cloned into an in-memory `Vec` before materialisation, so
/// the entry count bounds our peak allocation and the number of filesystem
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
/// `chunks` Vec defeats the per-entry allocation bound (every `ChunkRef` is cloned
/// into the in-memory file-entry vector before materialisation). With the snapshot
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
    // SECURITY (M1.7): re-verify the whole manifest blob against record.root
    // BEFORE parsing — symmetric to the per-chunk integrity check in
    // fetch_chunk. A hostile or buggy CasTransport must not be able to
    // substitute a different manifest than the one the RefRecord pins.
    let actual = Digest::of_bytes(&manifest_bytes);
    if actual != record.root {
        return Err(ClwError::Integrity {
            expected: record.root,
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

    // ------------------------------------------------------------------
    // 3c. Symlinks — created LAST, after every regular file is materialised
    //     (see the SECURITY note on pass 3a). No network I/O needed.
    // ------------------------------------------------------------------
    // The symlink-count cap (MAX_SYMLINK_ENTRIES) was already enforced up-front
    // in enforce_resource_bounds, before any filesystem work — see pass 2c.
    for entry in &manifest.entries {
        if let Entry::Symlink { path, target } = entry {
            // Defense-in-depth: validate both the link path and its target.
            clw_types::validate_rel_path(path)?;
            clw_types::validate_symlink_target(target)?;
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
            // validated by validate_symlink_target (relative, no `..`, no
            // drive-letter/backslash), and because symlinks are materialised in
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
    let _ = cache.gc_to_budget();

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
    // Defense-in-depth: reject any unsafe path even if from_bytes was bypassed.
    clw_types::validate_rel_path(rel_path)?;
    let abs = dest.join(rel_path);
    let parent = abs
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dest.to_path_buf());
    std::fs::create_dir_all(&parent)?;
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
    let mut bytes_from_cache: u64 = 0;
    let mut bytes_downloaded: u64 = 0;
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp_path)?;
        for chunk in chunks {
            let (data, from_cache) = fetch_chunk(chunk.digest, client, cache).await?;
            if from_cache {
                bytes_from_cache += chunk.size;
            } else {
                bytes_downloaded += chunk.size;
            }
            f.write_all(&data)?;
        }
        f.flush()?;
    }
    std::fs::rename(&tmp_path, &abs)?;
    // SECURITY (M1.7): mask to permission bits only. An untrusted manifest must
    // not be able to create setuid/setgid/sticky files — strip everything above
    // the low 9 permission bits (0o777).
    #[cfg(unix)]
    {
        let perms = std::fs::Permissions::from_mode(mode & 0o777);
        std::fs::set_permissions(&abs, perms)?;
    }
    // On Windows there is no unix mode concept: applying mode bits is a no-op,
    // so we skip set_permissions entirely. `mode` is consumed only on unix.
    #[cfg(windows)]
    let _ = mode;
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
// already validated (relative, no `..`, no drive-letter/backslash) by
// validate_symlink_target. We resolve it within `dest`, then:
//   - regular FILE target  -> copy its bytes (std::fs::copy),
//   - DIRECTORY target      -> recursively copy the subtree,
//   - missing target        -> a dangling symlink can't be copied: clear error.
// File targets are guaranteed to exist because symlinks run in pass 3c, after
// every regular file has been materialised (M1.7 files-before-symlinks order).
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn materialize_symlink_as_copy(dest: &Path, path: &str, target: &str, link: &Path) -> Result<()> {
    // The link's parent within `dest`; the target is resolved relative to it.
    let link_parent = link.parent().unwrap_or(dest);
    // `target` uses '/' separators (backslashes were rejected by validation);
    // join component-by-component so it resolves correctly on Windows.
    let mut resolved = link_parent.to_path_buf();
    for comp in target.split('/') {
        resolved.push(comp);
    }

    let meta = std::fs::symlink_metadata(&resolved).map_err(|_| {
        ClwError::InvalidManifest(format!(
            "symlink target does not exist under dest, cannot copy on Windows: \
             link {path} -> {target}"
        ))
    })?;

    if meta.file_type().is_dir() {
        copy_dir_recursive(&resolved, link)?;
    } else {
        // Regular file (or anything that is not a directory): copy its bytes.
        std::fs::copy(&resolved, link)?;
    }
    Ok(())
}

// Recursively copy the directory subtree at `src` to `dst` (Windows only).
#[cfg(windows)]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
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
}
