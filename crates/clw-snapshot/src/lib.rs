//! clw-snapshot — walk→chunk→dedupe→upload→ref pipeline.
//!
//! Builds a content-addressed manifest from a workspace directory, uploads
//! chunks to the CoreLink CAS (skipping chunks already present), uploads the
//! manifest itself, and writes a mutable `RefRecord` in the AC namespace so
//! the workspace can be hydrated later.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clw_cache::LocalCache;
use clw_chunk::{chunk_bytes, chunk_stream, to_chunk_refs};
use clw_manifest::{walk, WalkedEntry, WalkedFile};
use clw_types::{
    ref_key_in, AcTransport, CasTransport, ChunkRef, ClwError, Digest, Entry, Manifest, RefDomain,
    RefRecord, Result, CAS_BLOB_CAP_BYTES, MANIFEST_VERSION,
};
use futures::stream::{self, StreamExt};
use tokio::sync::{Mutex, Semaphore};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options for a snapshot operation.
pub struct SnapshotOptions {
    pub workspace_name: String,
    pub concurrency: usize,
    /// Ref keyspace the snapshot's ref is written under (default [`RefDomain::User`]).
    pub ref_domain: RefDomain,
    /// Replace an existing name whose content DIFFERS: because the AC ref PUT is
    /// create-only server-side, an in-place overwrite 409s. With `force`, snapshot
    /// deletes the existing ref then writes the new one (blobs are already uploaded,
    /// so the only gap is a sub-second local round-trip — far safer than the manual
    /// rm-then-snapshot, where a failure leaves NO ref). Re-snapshotting IDENTICAL
    /// content is always a no-op success and needs no `force`.
    pub force: bool,
}

impl SnapshotOptions {
    /// Create options with the given workspace name and default concurrency of 8.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            workspace_name: name.into(),
            concurrency: 8,
            ref_domain: RefDomain::User,
            force: false,
        }
    }

    /// Select the ref keyspace (e.g. [`RefDomain::Runner`] for runner-produced refs).
    #[must_use]
    pub fn with_ref_domain(mut self, ref_domain: RefDomain) -> Self {
        self.ref_domain = ref_domain;
        self
    }

    /// Replace an existing differing ref (rm-then-write). See [`SnapshotOptions::force`].
    #[must_use]
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
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
    /// Relative paths of entries whose name looks like a secret (SSH keys,
    /// `.env`, certificates, keystores, …). These were still uploaded; the
    /// field is purely advisory so the CLI can warn the user. Empty when no
    /// entry matched.
    pub secret_warnings: Vec<String>,
    /// Human strings (`"rel -> target"`) for symlinks that were SKIPPED because
    /// their target points OUTSIDE the workspace (absolute or `..`-escaping) and
    /// so is un-representable/un-restorable. These are NOT in the manifest — the
    /// tree snapshotted fine otherwise. Advisory only, mirrors `secret_warnings`;
    /// the CLI lists them so the user knows exactly which links were dropped.
    pub skipped_external_symlinks: Vec<String>,
    /// True when the name already pointed at this EXACT root: the snapshot was a
    /// no-op (no ref write). The CLI reports "already up to date" instead of a
    /// misleading 409 / a redundant overwrite.
    pub unchanged: bool,
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

/// Guard the small-file in-memory fast path against a TOCTOU size-growth. That path is
/// selected on the WALK-TIME size (`f.size`), but the `std::fs::read` returns the file's
/// CURRENT bytes. If the file grew past the stream threshold between walk and read (or the
/// path was swapped for a large one), a full in-memory read would defeat the threshold and
/// — multiplied by the upload concurrency — could exhaust memory. Fail loudly (a
/// user-facing `Message`, not a mislabeled transport error) rather than slurp it.
fn guard_small_file_size(f: &WalkedFile, read_len: usize) -> Result<()> {
    if read_len as u64 > STREAM_THRESHOLD_BYTES {
        return Err(ClwError::Message(format!(
            "snapshot: '{}' grew from {} to {} bytes during the operation (exceeds the \
             {}-byte stream threshold); re-run the snapshot",
            f.abs.display(),
            f.size,
            read_len,
            STREAM_THRESHOLD_BYTES
        )));
    }
    Ok(())
}

/// Upload one chunk: write it into the local cache, then PUT it to the remote
/// CAS only if it is not already present. Returns `true` iff a remote PUT was
/// performed (i.e. the chunk was missing), so the caller can advance the
/// `chunks_uploaded` counter. This preserves the existing dedupe behaviour:
/// `exists` is consulted and `put` happens only on a miss.
///
/// H4 (global upload concurrency): a permit is acquired from the shared
/// `sem` before any cache/network work, so the number of chunk uploads in
/// flight is bounded GLOBALLY across every file — not per-file. Because every
/// file's chunks contend on the same semaphore, a tree of many small files
/// (one chunk each) now parallelises up to `concurrency`, where before the
/// outer per-file `for` serialised them.
///
/// LOW (per-run dedup): `seen` is the set of chunk digests already handled this
/// snapshot. A digest already present is skipped entirely — no cache write, no
/// remote `exists`/`put` — so vendored/duplicated files that repeat the same
/// chunk pay the round-trip only once. The check+insert is done under the lock
/// so two concurrent tasks for the same digest can't both proceed; whichever
/// wins the insert does the upload (and it completes before the snapshot moves
/// on to the manifest, since all chunk tasks are awaited first).
async fn upload_chunk<C>(
    client: &C,
    cache: &LocalCache,
    sem: &Semaphore,
    seen: &Mutex<HashSet<Digest>>,
    digest: &Digest,
    data: Vec<u8>,
) -> Result<bool>
where
    C: CasTransport,
{
    // LOW: per-run dedup — skip a digest already handled this snapshot.
    {
        let mut set = seen.lock().await;
        if !set.insert(*digest) {
            return Ok(false);
        }
    }

    // H4: bound in-flight uploads GLOBALLY (across files), not per file.
    let _permit = sem
        .acquire()
        .await
        .map_err(|e| ClwError::Transport(e.to_string()))?;

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

/// Chunk one file and upload its chunks, returning its manifest [`Entry::File`].
///
/// H1 (size truth): the entry's `size` is the sum of the `ChunkRef.size`s that
/// were ACTUALLY produced from the read bytes — NOT the walk-time metadata size.
/// If the file changed size between walk and read, the manifest still satisfies
/// `entry.size == Σ chunk sizes` by construction, so a later hydrate cannot
/// hard-reject on `total_size != Σ ChunkRef.size`. `mode` still comes from walk.
///
/// H4: every chunk upload contends on the shared `sem`, so uploads parallelise
/// globally when the caller drives multiple files concurrently. Large files are
/// still streamed one chunk at a time (peak ~one chunk's bytes resident per
/// in-flight large file); small files chunk in memory then drop their bytes.
async fn process_file<C>(
    client: &C,
    cache: &LocalCache,
    sem: &Semaphore,
    seen: &Mutex<HashSet<Digest>>,
    counter: &AtomicU64,
    concurrency: usize,
    f: WalkedFile,
) -> Result<Entry>
where
    C: CasTransport,
{
    // TOCTOU NOTE (audit Low): the walker classified `f.abs` as a regular file with
    // `follow_links(false)`, but the reads below follow symlinks. A concurrent hostile
    // writer on a SHARED checkout could swap the path. The CONTENT-swap correctness angle
    // is out of the v0.1 threat model (single-operator local tool; deferred hardening:
    // O_NOFOLLOW open + fstat-after-open). The MEMORY-size angle IS guarded: the small-file
    // branch below rejects a file that grew past the stream threshold between walk and read
    // (`guard_small_file_size`), so a mid-operation growth can't OOM instead of streaming.
    let chunk_refs: Vec<ChunkRef> = if f.size <= STREAM_THRESHOLD_BYTES {
        // ---- small file: in-memory fast path -----------------------------
        let abs = f.abs.clone();
        let data = tokio::task::spawn_blocking(move || std::fs::read(&abs))
            .await
            .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;
        guard_small_file_size(&f, data.len())?;

        let chunks_raw = chunk_bytes(&data);
        let chunk_refs = to_chunk_refs(&chunks_raw);

        // Upload THIS file's chunks; each contends on the GLOBAL semaphore, so
        // the real parallelism ceiling is `sem`'s permit count regardless of
        // how many files are in flight. `data` drops at the end of the block.
        let results: Vec<Result<()>> = stream::iter(chunks_raw.iter().map(|c| {
            let bytes = data[c.offset as usize..(c.offset + c.len) as usize].to_vec();
            let digest = c.digest;
            async move {
                if upload_chunk(client, cache, sem, seen, &digest, bytes).await? {
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
        // ---- large file: stream, one chunk's bytes at a time -------------
        let abs = f.abs.clone();
        let file = tokio::task::spawn_blocking(move || std::fs::File::open(&abs))
            .await
            .map_err(|e| clw_types::ClwError::Transport(e.to_string()))??;

        let mut chunk_refs: Vec<ChunkRef> = Vec::new();
        for item in chunk_stream(file) {
            let sc = item.map_err(|e| clw_types::ClwError::Transport(e.to_string()))?;
            // Preserve file byte-order in the manifest (H4 preservation (a)).
            chunk_refs.push(ChunkRef {
                digest: sc.digest,
                size: sc.len,
            });
            if upload_chunk(client, cache, sem, seen, &sc.digest, sc.data).await? {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        chunk_refs
    };

    // H1: derive `size` from the bytes we ACTUALLY chunked, not walk metadata.
    let size: u64 = chunk_refs.iter().map(|c| c.size).sum();
    Ok(Entry::File {
        path: f.rel,
        mode: f.mode,
        size,
        chunks: chunk_refs,
    })
}

/// Return `true` if `rel` (a manifest-relative path) has a basename or
/// extension that matches a common secret-file pattern. This is a heuristic
/// for an advisory warning only — it never affects which files are uploaded.
///
/// Matched: SSH private keys (`id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`),
/// `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.keystore`, `.env` / `.env.*`,
/// and a basename of exactly `credentials`.
fn looks_like_secret(rel: &str) -> bool {
    // Manifest paths are always '/'-separated; take the trailing component.
    let base = rel.rsplit('/').next().unwrap_or(rel);

    // Exact-basename matches (SSH keys + `credentials`).
    const EXACT: &[&str] = &["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", "credentials"];
    if EXACT.contains(&base) {
        return true;
    }

    // `.env` and `.env.*` (e.g. `.env.local`, `.env.production`).
    if base == ".env" || base.starts_with(".env.") {
        return true;
    }

    // Extension matches (case-insensitive on the extension only).
    const EXT: &[&str] = &["pem", "key", "p12", "pfx", "keystore"];
    if let Some((_, ext)) = base.rsplit_once('.') {
        let ext_lower = ext.to_ascii_lowercase();
        if EXT.contains(&ext_lower.as_str()) {
            return true;
        }
    }

    false
}

/// FIX F2: a manifest is serialized to a SINGLE CAS blob and uploaded under one
/// digest. The CAS contract caps any blob at `CAS_BLOB_CAP_BYTES` (5 MiB), so a
/// workspace with very many entries can produce a manifest blob that exceeds the
/// cap — which would otherwise surface as an opaque server 413 LATE in the
/// snapshot, after every chunk has already been uploaded. Check the serialized
/// size UP FRONT and fail with a clear, actionable error naming the size and the
/// cap. Pure over the byte length so it is unit-testable without a real upload.
fn check_manifest_blob_fits(manifest_bytes_len: usize) -> Result<()> {
    if manifest_bytes_len > CAS_BLOB_CAP_BYTES {
        return Err(ClwError::InvalidManifest(format!(
            "the workspace manifest serializes to {manifest_bytes_len} bytes, exceeding the \
             single-blob CAS cap of {CAS_BLOB_CAP_BYTES} bytes — this workspace has too many \
             entries to fit one manifest blob. Split the workspace or reduce the number of \
             tracked files (a server-side multi-part manifest is a future capability)."
        )));
    }
    Ok(())
}

/// M6: does a create-only AC ref PUT error look like a 409/AlreadyExists
/// conflict? The real HTTP transport maps a 409 to `ClwError::Http { status:
/// 409, .. }`; a fake/create-only transport may instead signal it in a
/// `Transport`/`Message` string. Match both so the idempotent-retry recovery
/// fires regardless of which layer produced the conflict.
fn is_conflict(e: &ClwError) -> bool {
    match e {
        ClwError::Http { status: 409, .. } => true,
        ClwError::Transport(m) | ClwError::Message(m) => {
            let l = m.to_ascii_lowercase();
            l.contains("409") || l.contains("already exists") || l.contains("conflict")
        }
        _ => false,
    }
}

/// M6: is the ref that already OCCUPIES the key equivalent to the one we were
/// about to write? Byte-equality is the strict case; otherwise a semantic match
/// on `root` + `manifest_size` (the fields that make a ref restore the same
/// workspace) counts — created-at/parent may legitimately differ across a
/// lost-ACK retry. A corrupt/unparseable occupant is NOT equivalent.
fn existing_ref_matches(existing_bytes: &[u8], want_bytes: &[u8], want: &RefRecord) -> bool {
    if existing_bytes == want_bytes {
        return true;
    }
    match RefRecord::from_bytes(existing_bytes) {
        Ok(cur) => cur.root == want.root && cur.manifest_size == want.manifest_size,
        Err(_) => false,
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
                    // Same TOCTOU size-growth guard as `snapshot` (see the note there).
                    guard_small_file_size(&f, data.len())?;

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

                // H1: derive size from the ACTUAL chunked bytes (Σ ChunkRef.size),
                // not walk metadata, so this stays byte-identical to `snapshot`'s
                // manifest even under a walk/read size divergence — the
                // content-addressed `hydrate --manifest-digest` path relies on the
                // two agreeing.
                let size: u64 = chunk_refs.iter().map(|c| c.size).sum();
                total_size += size;
                entries.push(Entry::File {
                    path: f.rel,
                    mode: f.mode,
                    size,
                    chunks: chunk_refs,
                });
            }
            WalkedEntry::Symlink { rel, target } => {
                entries.push(Entry::Symlink { path: rel, target });
            }
            WalkedEntry::EmptyDir { rel } => {
                entries.push(Entry::Dir { path: rel });
            }
            // A symlink whose target escapes the workspace produces NO manifest
            // entry (kept in lockstep with `snapshot`), so a manifest built here
            // never carries an external/escaping symlink and its digest matches
            // the one `snapshot` produced for the same tree.
            WalkedEntry::SkippedExternalSymlink { .. } => {}
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
    // uploaded, then their bytes are dropped); large files are streamed so at
    // most one chunk's bytes are resident per in-flight large file. Streaming
    // and in-memory chunking are dedupe-identical, so the resulting
    // manifest/digest is independent of which path each file took.
    //
    // H4 (global upload concurrency): file processing is driven concurrently
    // (`buffer_unordered`), and every chunk upload contends on ONE shared
    // `Semaphore`. So a tree of many small files (one chunk each) now saturates
    // `concurrency` uploads globally, where before the per-file `for` awaited
    // each file's chunks before starting the next (zero cross-file
    // parallelism). Scheduling is order-INDEPENDENT: each file is tagged with
    // its walk index and re-sorted below, so the manifest — and thus its digest
    // — is byte-identical to the old serial output for identical input.
    let concurrency = opts.concurrency.max(1);

    // ONE semaphore shared by every chunk upload across every file (H4).
    let sem = Arc::new(Semaphore::new(concurrency));
    // Per-run chunk-digest dedup set (LOW): a digest is uploaded once per run.
    let seen: Arc<Mutex<HashSet<Digest>>> = Arc::new(Mutex::new(HashSet::new()));
    // Shared across all files so `chunks_uploaded` aggregates the whole tree.
    let uploaded_counter = Arc::new(AtomicU64::new(0));

    // Partition off symlinks whose target escapes the workspace: they produce NO
    // manifest `Entry` (their bytes aren't in the snapshot, so they can't be
    // restored) and are collected as advisory warnings instead. The manifest
    // therefore never carries an external/escaping symlink. Everything else
    // (files/dirs/contained symlinks) flows through the concurrent pipeline.
    let mut skipped_external_symlinks: Vec<String> = Vec::new();
    let mut representable: Vec<WalkedEntry> = Vec::with_capacity(walked.len());
    for we in walked {
        match we {
            WalkedEntry::SkippedExternalSymlink { rel, target } => {
                skipped_external_symlinks.push(format!("{rel} -> {target}"));
            }
            other => representable.push(other),
        }
    }

    // Process files (and pass symlinks/dirs through) concurrently, tagging each
    // with its walk index so we can restore walk order for the manifest.
    let indexed: Vec<(usize, WalkedEntry)> = representable.into_iter().enumerate().collect();
    let results: Vec<Result<(usize, Entry)>> =
        stream::iter(indexed.into_iter().map(|(idx, we)| {
            let sem = Arc::clone(&sem);
            let seen = Arc::clone(&seen);
            let counter = Arc::clone(&uploaded_counter);
            async move {
                let entry = match we {
                    WalkedEntry::File(f) => {
                        process_file(client, cache, &sem, &seen, &counter, concurrency, f).await?
                    }
                    WalkedEntry::Symlink { rel, target } => Entry::Symlink { path: rel, target },
                    WalkedEntry::EmptyDir { rel } => Entry::Dir { path: rel },
                    // Skimmed off above; the stream never sees an external symlink.
                    WalkedEntry::SkippedExternalSymlink { .. } => unreachable!(
                        "external symlinks are partitioned out before the upload stream"
                    ),
                };
                Ok::<(usize, Entry), ClwError>((idx, entry))
            }
        }))
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // Restore walk order: the manifest (and its digest) MUST be independent of
    // the concurrent upload schedule.
    let mut indexed_entries: Vec<(usize, Entry)> = Vec::with_capacity(results.len());
    for r in results {
        indexed_entries.push(r?);
    }
    indexed_entries.sort_by_key(|(i, _)| *i);
    let entries: Vec<Entry> = indexed_entries.into_iter().map(|(_, e)| e).collect();

    // --- 3. Assemble manifest -----------------------------------------------
    //
    // H1: derive `total_size` (and the per-entry sizes, done in `process_file`)
    // from the chunk sizes we ACTUALLY produced. `total_size == Σ ChunkRef.size`
    // now holds BY CONSTRUCTION, so a walk/read size divergence can no longer
    // yield a manifest that every future hydrate hard-rejects.
    let mut total_size: u64 = 0;
    let mut chunks_total: u64 = 0;
    let mut files_count: u64 = 0;
    for e in &entries {
        if let Entry::File { size, chunks, .. } = e {
            total_size += *size;
            chunks_total += chunks.len() as u64;
            files_count += 1;
        }
    }

    let manifest = Manifest {
        version: MANIFEST_VERSION,
        total_size,
        entries,
    };

    // Advisory scan: collect any entries whose name looks like a secret. This
    // never drops files — everything was already uploaded above; we only warn.
    let secret_warnings: Vec<String> = manifest
        .entries
        .iter()
        .map(|e| e.path())
        .filter(|p| looks_like_secret(p))
        .map(|p| p.to_string())
        .collect();

    // --- 4. Chunk uploads already happened inline above ---------------------
    let chunks_uploaded = uploaded_counter.load(Ordering::Relaxed);

    // Diagnostic channel (M5.1): dedupe effectiveness at a glance, on stderr.
    tracing::info!(
        files = files_count,
        chunks_total,
        chunks_uploaded,
        chunks_deduped = chunks_total.saturating_sub(chunks_uploaded),
        "snapshot chunk upload complete"
    );

    // --- 5. Upload manifest -------------------------------------------------
    let manifest_bytes = manifest.canonical_bytes()?;
    // FIX F2: reject an oversized manifest BEFORE the upload, so the failure is a
    // clear "too many entries" error instead of an opaque 413 from the server.
    check_manifest_blob_fits(manifest_bytes.len())?;
    let manifest_digest = Digest::of_bytes(&manifest_bytes);
    let manifest_size = manifest_bytes.len() as u64;

    let already_there = CasTransport::exists(client, &manifest_digest).await?;
    if !already_there {
        CasTransport::put(client, &manifest_digest, manifest_bytes).await?;
    }

    // --- 6. Read existing ref (parent lineage + idempotency) ----------------
    let rk = ref_key_in(opts.ref_domain, &opts.workspace_name);
    // Capture RAW presence separately from a successful parse. A present-but-corrupt
    // ref still OCCUPIES the create-only AC key, so a force-replace must delete it on
    // RAW presence — gating the delete on `existing.is_some()` (a successful parse)
    // would skip the delete for a corrupt ref and the create-only PUT below would 409,
    // making `--force` unable to recover a corrupt-but-present ref. `existing` (the
    // parsed record) is still used for the no-op short-circuit and the parent lineage.
    let raw_present;
    let existing: Option<RefRecord> = match AcTransport::get(client, &rk).await? {
        Some(bytes) => {
            raw_present = true;
            RefRecord::from_bytes(&bytes).ok() // corrupt → treat as absent for lineage
        }
        None => {
            raw_present = false;
            None
        }
    };

    // Idempotent no-op: the name already points at this EXACT root, so there is
    // nothing to write. This makes "save again, nothing changed" (and a runner's
    // idempotent re-snapshot per job) a clean SUCCESS instead of the create-only
    // AC PUT's 409. Blobs were still re-verified/deduped above (chunks_uploaded
    // reflects that), so the content is provably present.
    if let Some(ex) = &existing {
        if ex.root == manifest_digest {
            return Ok(SnapshotReport {
                root: manifest_digest,
                files: files_count,
                bytes_total: total_size,
                chunks_total,
                chunks_uploaded,
                secret_warnings,
                skipped_external_symlinks,
                unchanged: true,
            });
        }
    }

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
        parent: existing.as_ref().map(|e| e.root),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    // The AC ref PUT is create-only server-side, so REPLACING an existing name whose
    // content DIFFERS needs an explicit rm-first. `--force` does the rm+PUT here: the
    // blobs are already uploaded, so the only gap is this sub-second local round-trip
    // (vs the manual rm-then-snapshot, where a crash between the two leaves NO ref).
    // Without `force`, the PUT 409s and the CLI surfaces the actionable guidance.
    // Gate on RAW presence, NOT a successful parse: a present-but-corrupt ref must
    // also be deleted so the create-only PUT does not 409. (A no-op identical re-
    // snapshot already returned above, so reaching here with `force` means we intend
    // to replace whatever currently occupies the key — parseable or not.)
    if opts.force && raw_present {
        AcTransport::delete(client, &rk).await?;
    }
    let ref_bytes = ref_record.to_bytes()?;
    // M6 (idempotent 409): the create-only PUT can 409 spuriously — a retried
    // PUT after a LOST ACK re-hits the now-occupied key (or an eventual-
    // consistency gap let step-6's GET miss a ref that a prior run already
    // wrote). On a conflict, GET the occupant: if it already IS what we meant to
    // write (same root + manifest_size, or byte-equal), the write is a no-op
    // SUCCESS. Only a genuinely DIFFERENT occupant is a real conflict — re-raise
    // the original error so the CLI still renders the "already exists — re-run
    // with --force" guidance. This does NOT weaken `--force`: force already
    // deleted any present ref above, so reaching here under force + conflict
    // means a concurrent writer, and an identical one is still a safe no-op.
    let mut unchanged = false;
    match AcTransport::put(client, &rk, ref_bytes.clone()).await {
        Ok(()) => {}
        Err(e) if is_conflict(&e) => match AcTransport::get(client, &rk).await? {
            Some(cur) if existing_ref_matches(&cur, &ref_bytes, &ref_record) => {
                unchanged = true;
            }
            _ => return Err(e),
        },
        Err(e) => return Err(e),
    }

    // --- 8. Return report ---------------------------------------------------
    Ok(SnapshotReport {
        root: manifest_digest,
        files: files_count,
        bytes_total: total_size,
        chunks_total,
        chunks_uploaded,
        secret_warnings,
        skipped_external_symlinks,
        unchanged,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{ref_key, AcTransport, CasTransport, ClwError, Digest, Result};
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

        async fn delete(&self, digest: &Digest) -> Result<()> {
            self.cas.lock().await.remove(digest);
            Ok(())
        }

        async fn list_blobs(&self) -> Result<Vec<Digest>> {
            Ok(self.cas.lock().await.keys().copied().collect())
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

        async fn delete(&self, key: &Digest) -> Result<()> {
            self.ac.lock().await.remove(key);
            Ok(())
        }

        async fn list_refs(&self) -> Result<Vec<Digest>> {
            Ok(self.ac.lock().await.keys().copied().collect())
        }
    }

    #[tokio::test]
    async fn fake_list_refs_and_blobs_return_map_keys() {
        let client = FakeTransport::default();
        let r = Digest::of_bytes(b"r");
        let b = Digest::of_bytes(b"b");
        AcTransport::put(&client, &r, b"v".to_vec()).await.unwrap();
        CasTransport::put(&client, &b, b"data".to_vec())
            .await
            .unwrap();
        assert_eq!(AcTransport::list_refs(&client).await.unwrap(), vec![r]);
        assert_eq!(CasTransport::list_blobs(&client).await.unwrap(), vec![b]);
        // CAS delete removes the blob.
        CasTransport::delete(&client, &b).await.unwrap();
        assert!(CasTransport::list_blobs(&client).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fake_delete_removes_key_and_is_idempotent() {
        let client = FakeTransport::default();
        let key = Digest::of_bytes(b"some-ref-key");
        AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap();
        assert!(AcTransport::get(&client, &key).await.unwrap().is_some());
        AcTransport::delete(&client, &key).await.unwrap();
        assert!(AcTransport::get(&client, &key).await.unwrap().is_none());
        // Idempotent: deleting an absent key is still Ok.
        AcTransport::delete(&client, &key).await.unwrap();
    }

    // -----------------------------------------------------------------------
    // Helper: build a tempdir tree with a small file, a large file (>1 MiB),
    // a symlink, and an empty directory.
    // -----------------------------------------------------------------------

    // Builds a fixture tree containing a unix symlink (`link.txt`), so every
    // test that uses it is unix-only. Gated to unix (Windows symlink creation
    // needs privileges + the manifest would omit the symlink entry).
    #[cfg(unix)]
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

    #[cfg(unix)] // uses build_tree (unix symlink fixture)
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
    // Empty-tree base case: snapshotting an EMPTY workspace root (zero entries)
    // must SUCCEED with a minimal manifest (0 files, total_size 0), a stable
    // non-panicking root digest, and a written ref. The degenerate base case.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn snapshot_empty_tree_succeeds_with_minimal_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path(); // brand-new tempdir: zero entries

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-empty");

        let report = snapshot(root, &client, &cache, &opts)
            .await
            .expect("snapshotting an empty workspace must succeed, not panic/error");

        // Report counters reflect the empty tree.
        assert_eq!(report.files, 0, "empty tree has zero files");
        assert_eq!(report.bytes_total, 0, "empty tree has zero bytes");
        assert_eq!(report.chunks_total, 0, "empty tree has zero chunks");
        assert_eq!(report.chunks_uploaded, 0, "no chunks to upload");
        assert!(!report.unchanged, "the FIRST snapshot is a real write");
        assert!(report.secret_warnings.is_empty());
        assert!(report.skipped_external_symlinks.is_empty());

        // The manifest blob is in CAS and decodes to a 0-file, 0-byte manifest.
        let cas = client.cas.lock().await;
        let manifest = Manifest::from_bytes(cas.get(&report.root).expect("manifest in CAS"))
            .expect("the empty-tree manifest must decode");
        assert!(manifest.entries.is_empty(), "empty tree → no entries");
        assert_eq!(manifest.total_size, 0);
        drop(cas);

        // The root digest is STABLE/deterministic: the local-build path over the
        // same empty tree yields the identical digest (no panic on the empty walk).
        let manifest_local = build_manifest_local(root).await.unwrap();
        assert_eq!(
            manifest_local.digest().unwrap(),
            report.root,
            "empty-tree root digest must be stable across the two build paths"
        );

        // A ref was written and points at the empty-tree root, parent=None.
        let rk = ref_key("ws-empty");
        let ac = client.ac.lock().await;
        let rec = RefRecord::from_bytes(ac.get(&rk).expect("ref must be written")).unwrap();
        assert_eq!(rec.root, report.root);
        assert_eq!(rec.name, "ws-empty");
        assert!(rec.parent.is_none(), "first snapshot has no parent");
    }

    // -----------------------------------------------------------------------
    // Test 2: second snapshot of unchanged tree → chunks_uploaded == 0
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses build_tree (unix symlink fixture)
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

        // Re-snapshot of IDENTICAL content is a NO-OP: it reports `unchanged` and does
        // NOT write a new ref version (writing a v2-identical-to-v1 would 409 against the
        // create-only server anyway). So the stored ref is untouched — still the first
        // snapshot's record with parent == None (there is no v2; nothing changed).
        assert!(
            report2.unchanged,
            "re-snapshot of identical content must be a no-op"
        );
        let rk = ref_key("ws-unchanged");
        let ac = client.ac.lock().await;
        let rec = RefRecord::from_bytes(ac.get(&rk).unwrap()).unwrap();
        assert_eq!(
            rec.root, report1.root,
            "the ref still points at the same root"
        );
        assert_eq!(
            rec.parent, None,
            "a no-op re-snapshot must NOT write a new ref version (parent stays None)"
        );
        assert!(
            !report1.unchanged,
            "the FIRST snapshot is a real write, not a no-op"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: changed one file → only its chunks uploaded; lineage correct
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses build_tree (unix symlink fixture)
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

    #[cfg(unix)] // uses build_tree (unix symlink fixture)
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

    // -----------------------------------------------------------------------
    // Test 5: looks_like_secret pattern matching (unit)
    // -----------------------------------------------------------------------

    #[test]
    fn looks_like_secret_matches_expected() {
        // Positives — basenames, extensions, and nested paths.
        for p in [
            "id_rsa",
            "id_dsa",
            "id_ecdsa",
            "id_ed25519",
            ".ssh/id_rsa",
            ".env",
            ".env.local",
            ".env.production",
            "server.pem",
            "tls/server.key",
            "bundle.p12",
            "cert.PFX",
            "store.keystore",
            "aws/credentials",
            "credentials",
        ] {
            assert!(looks_like_secret(p), "{p} should be flagged as a secret");
        }

        // Negatives — ordinary files that must not trip the heuristic.
        for p in [
            "README.md",
            "src/main.rs",
            "environment.txt",
            "keys.json",
            "credentials.md",
            "id_rsa_backup.txt",
            "notes.env.bak",
        ] {
            assert!(!looks_like_secret(p), "{p} should NOT be flagged");
        }
    }

    // -----------------------------------------------------------------------
    // Test 6: a tree with .env + id_rsa yields those in secret_warnings
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn snapshot_collects_secret_warnings() {
        use std::fs;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("README.md"), b"hello").unwrap();
        fs::write(root.join(".env"), b"SECRET=1").unwrap();
        fs::write(root.join("id_rsa"), b"-----BEGIN KEY-----").unwrap();

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-secrets");

        let report = snapshot(root, &client, &cache, &opts).await.unwrap();

        let mut warnings = report.secret_warnings.clone();
        warnings.sort();
        assert_eq!(
            warnings,
            vec![".env".to_string(), "id_rsa".to_string()],
            "expected .env and id_rsa flagged, got {:?}",
            report.secret_warnings
        );
    }

    // -----------------------------------------------------------------------
    // Test 7: a clean tree yields no secret_warnings
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Test 8 (FIX F2): the manifest-blob size check rejects a serialized manifest
    // that exceeds CAS_BLOB_CAP_BYTES with a clear, actionable error — exercised
    // at the pure-helper boundary so no real (5 MiB+) manifest/upload is needed.
    // -----------------------------------------------------------------------

    #[test]
    fn manifest_blob_size_check_rejects_oversized() {
        // At-or-below the cap is accepted.
        assert!(check_manifest_blob_fits(0).is_ok());
        assert!(check_manifest_blob_fits(CAS_BLOB_CAP_BYTES).is_ok());
        // One byte over the cap is rejected with a clear, named error.
        let err = check_manifest_blob_fits(CAS_BLOB_CAP_BYTES + 1).unwrap_err();
        match err {
            ClwError::InvalidManifest(ref m) => {
                assert!(
                    m.contains("too many entries"),
                    "error must explain the entry-count cause, got: {m}"
                );
                assert!(
                    m.contains(&(CAS_BLOB_CAP_BYTES + 1).to_string())
                        && m.contains(&CAS_BLOB_CAP_BYTES.to_string()),
                    "error must name both the size and the cap, got: {m}"
                );
            }
            other => panic!("expected InvalidManifest, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // FIX #1: a create-only AC (mirrors the server: a PUT to an OCCUPIED key
    // 409s). `snapshot(--force)` must DELETE the present-but-corrupt ref first
    // so the create-only PUT succeeds, rather than skipping the delete (because
    // the corrupt bytes don't parse) and then 409-ing.
    // -----------------------------------------------------------------------

    /// CAS behaves normally; AC is create-only: `put` to an existing key errors
    /// (like the server's 409) unless `delete` clears it first.
    #[derive(Clone, Default)]
    struct CreateOnlyAc {
        cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        ac: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
    }

    #[async_trait::async_trait]
    impl CasTransport for CreateOnlyAc {
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
    impl AcTransport for CreateOnlyAc {
        async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
            Ok(self.ac.lock().await.get(key).cloned())
        }
        async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
            // Create-only: a PUT to an occupied key conflicts (server 409).
            let mut ac = self.ac.lock().await;
            if ac.contains_key(key) {
                return Err(ClwError::Transport("409 ref already exists".into()));
            }
            ac.insert(*key, value);
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

    #[tokio::test]
    async fn force_recovers_a_present_but_corrupt_ref() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.txt"), b"content").unwrap();

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = CreateOnlyAc::default();

        // Plant a present-but-CORRUPT ref under this name's key (unparseable bytes).
        let rk = ref_key("ws-corrupt");
        client
            .ac
            .lock()
            .await
            .insert(rk, b"this is not a valid RefRecord".to_vec());

        // Without --force, the create-only PUT would 409 (delete is skipped).
        let opts_noforce = SnapshotOptions::new("ws-corrupt");
        let res = snapshot(root, &client, &cache, &opts_noforce)
            .await
            .map(|_| ()); // discard the non-Debug SnapshotReport so we can match cleanly
        match res {
            Err(ClwError::Transport(m)) if m.contains("409") => {}
            other => panic!("without --force a corrupt-but-present ref must 409, got: {other:?}"),
        }

        // With --force, snapshot deletes the corrupt ref then recreates it: success.
        let opts_force = SnapshotOptions::new("ws-corrupt").with_force(true);
        let report = snapshot(root, &client, &cache, &opts_force)
            .await
            .expect("--force must recover a present-but-corrupt ref (delete + recreate)");
        assert!(!report.unchanged);

        // The ref now parses (the corrupt bytes were replaced).
        let ac = client.ac.lock().await;
        let rec = RefRecord::from_bytes(ac.get(&rk).unwrap())
            .expect("the ref must be a valid RefRecord after --force");
        assert_eq!(rec.root, report.root);
        // A corrupt prior ref yields no parent lineage (it never parsed).
        assert_eq!(rec.parent, None);
    }

    #[cfg(unix)] // uses build_tree (unix symlink fixture)
    #[tokio::test]
    async fn snapshot_clean_tree_no_warnings() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        build_tree(root); // small.txt, big.bin, link.txt, empty_dir

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-clean");

        let report = snapshot(root, &client, &cache, &opts).await.unwrap();

        assert!(
            report.secret_warnings.is_empty(),
            "clean tree should have no secret warnings, got {:?}",
            report.secret_warnings
        );
    }

    // -----------------------------------------------------------------------
    // H1 (size truth): Entry::File.size derives from the bytes we ACTUALLY
    // chunked (Σ ChunkRef.size), NOT the walk-time metadata size. Proven by
    // handing `process_file` a WalkedFile with a deliberately WRONG `size`; the
    // returned entry must reflect the real byte length on BOTH the in-memory and
    // the streaming path. RED-on-revert: restoring `size: f.size` fails this.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn file_entry_size_derives_from_chunked_bytes_not_walk_metadata() {
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let sem = Semaphore::new(4);
        let seen = Mutex::new(HashSet::new());
        let counter = AtomicU64::new(0);

        let tmp = TempDir::new().unwrap();

        // --- small (in-memory) path: bogus size stays under the threshold. ---
        let small = tmp.path().join("small.txt");
        std::fs::write(&small, b"hello snapshot").unwrap(); // 14 bytes
        let wf_small = WalkedFile {
            abs: small.clone(),
            rel: "small.txt".into(),
            mode: 0o644,
            size: 1, // WRONG on purpose (real is 14); still <= threshold → small path
        };
        let entry = process_file(&client, &cache, &sem, &seen, &counter, 4, wf_small)
            .await
            .unwrap();
        if let Entry::File { size, chunks, .. } = entry {
            let sum: u64 = chunks.iter().map(|c| c.size).sum();
            assert_eq!(size, 14, "small-path size must be the real byte length");
            assert_eq!(size, sum, "size must equal Σ chunk sizes (H1 invariant)");
            assert_ne!(size, 1, "size must NOT be the bogus walk-metadata size");
        } else {
            panic!("expected a File entry");
        }

        // --- large (streaming) path: bogus size ABOVE the threshold forces the
        // stream path even though the real file is tiny (no huge file needed). --
        let big = tmp.path().join("big.dat");
        std::fs::write(&big, vec![7u8; 100]).unwrap(); // 100 bytes
        let wf_big = WalkedFile {
            abs: big.clone(),
            rel: "big.dat".into(),
            mode: 0o644,
            size: STREAM_THRESHOLD_BYTES + 1, // WRONG (real is 100) → forces streaming path
        };
        let entry = process_file(&client, &cache, &sem, &seen, &counter, 4, wf_big)
            .await
            .unwrap();
        if let Entry::File { size, chunks, .. } = entry {
            let sum: u64 = chunks.iter().map(|c| c.size).sum();
            assert_eq!(size, 100, "stream-path size must be the real byte length");
            assert_eq!(size, sum, "size must equal Σ chunk sizes (H1 invariant)");
            assert_ne!(
                size,
                STREAM_THRESHOLD_BYTES + 1,
                "size must NOT be the bogus walk-metadata size"
            );
        } else {
            panic!("expected a File entry");
        }
    }

    #[tokio::test]
    async fn small_path_rejects_file_grown_past_threshold() {
        // TOCTOU size-growth: the walk-time size says "small" (selects the in-memory fast
        // path), but the file on disk is actually larger than the stream threshold. The
        // guard must REJECT it (a user-facing Message), not slurp the whole thing into RAM.
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let sem = Semaphore::new(4);
        let seen = Mutex::new(HashSet::new());
        let counter = AtomicU64::new(0);
        let tmp = TempDir::new().unwrap();

        let grown = tmp.path().join("grown.bin");
        std::fs::write(&grown, vec![0u8; STREAM_THRESHOLD_BYTES as usize + 1]).unwrap();
        let wf = WalkedFile {
            abs: grown.clone(),
            rel: "grown.bin".into(),
            mode: 0o644,
            size: 1, // WRONG-small on purpose → selects the in-memory fast path
        };
        let err = process_file(&client, &cache, &sem, &seen, &counter, 4, wf)
            .await
            .unwrap_err();
        match err {
            ClwError::Message(m) => {
                assert!(
                    m.contains("grew"),
                    "expected a size-growth message, got: {m}"
                );
                assert!(
                    m.contains("re-run"),
                    "message should be actionable, got: {m}"
                );
            }
            other => panic!("expected a Message size-growth rejection, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // H1 (whole-manifest observable): the hydrate invariant
    // `total_size == Σ ChunkRef.size` (and per-file `size == Σ its chunks`)
    // holds by construction on a real multi-file, multi-chunk snapshot. This is
    // the nearest observable of the end-to-end invariant; the RED-on-revert
    // proof is the process_file test above (a real walk/read size divergence is
    // a TOCTOU race not reproducible deterministically in-process).
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses build_tree (unix symlink fixture)
    #[tokio::test]
    async fn manifest_total_size_equals_sum_of_all_chunk_sizes() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        build_tree(root); // multi-file; big.bin is multi-chunk

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-h1");
        let report = snapshot(root, &client, &cache, &opts).await.unwrap();

        let cas = client.cas.lock().await;
        let manifest = Manifest::from_bytes(cas.get(&report.root).unwrap()).unwrap();
        let mut grand: u64 = 0;
        for e in &manifest.entries {
            if let Entry::File { size, chunks, .. } = e {
                let sum: u64 = chunks.iter().map(|c| c.size).sum();
                assert_eq!(*size, sum, "each File.size must equal Σ its chunk sizes");
                grand += sum;
            }
        }
        assert_eq!(
            manifest.total_size, grand,
            "total_size must equal Σ ALL ChunkRef.size (hydrate invariant)"
        );
        assert_eq!(report.bytes_total, grand);
    }

    // -----------------------------------------------------------------------
    // H4 (global concurrency, digest stability): with uploads driven
    // concurrently across files, the manifest — and its digest — MUST be
    // byte-identical to the sequential `build_manifest_local` output for the
    // same input (walk order preserved, schedule-independent). RED-on-revert:
    // dropping the index-tag + re-sort lets completion order leak into the
    // manifest and the bytes/digest diverge.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn manifest_digest_is_stable_across_concurrent_upload_schedule() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Many small files (1 chunk each) exercise cross-file scheduling…
        for i in 0..25u32 {
            std::fs::write(root.join(format!("f{i:03}.txt")), format!("payload-{i}")).unwrap();
        }
        // …plus a multi-chunk big file.
        let big: Vec<u8> = (0u32..=(1024 * 1024 + 100))
            .map(|i| (i.wrapping_mul(0x9e37_79b9u32)) as u8)
            .collect();
        std::fs::write(root.join("big.bin"), &big).unwrap();

        let expected = build_manifest_local(root).await.unwrap();
        let expected_bytes = expected.canonical_bytes().unwrap();
        let expected_digest = expected.digest().unwrap();

        // Two independent runs at the default concurrency (8) must both
        // reproduce the manifest byte-for-byte.
        for _ in 0..2 {
            let cache_dir = TempDir::new().unwrap();
            let cache = LocalCache::new(cache_dir.path()).unwrap();
            let client = FakeTransport::default();
            let opts = SnapshotOptions::new("ws-h4");
            let report = snapshot(root, &client, &cache, &opts).await.unwrap();
            assert_eq!(
                report.root, expected_digest,
                "digest must match the sequential build regardless of upload schedule"
            );
            let cas = client.cas.lock().await;
            let got = cas.get(&report.root).unwrap();
            assert_eq!(
                got, &expected_bytes,
                "manifest bytes must be identical (walk order preserved)"
            );
        }

        // Entries stay in sorted path order regardless of upload completion order.
        let paths: Vec<&str> = expected.entries.iter().map(|e| e.path()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(
            paths, sorted,
            "manifest entries must stay in sorted path order"
        );
    }

    // -----------------------------------------------------------------------
    // LOW (per-run dedup): a chunk digest repeated across files is handled ONCE
    // per run. To make this a DETERMINISTIC RED-on-revert we use a CAS whose
    // `exists` ALWAYS misses and that COUNTS every `put` — so the ONLY thing
    // that can prevent the 2nd/3rd identical chunk from re-uploading is the
    // in-run `seen` set. WITH the fix: exactly one content PUT + one counted
    // upload. WITHOUT it: three of each.
    // -----------------------------------------------------------------------

    /// CAS that never reports a blob present (`exists` → false) and counts every
    /// PUT. AC is a normal in-memory map so the snapshot still completes.
    #[derive(Clone, Default)]
    struct AlwaysMissCas {
        cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        ac: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        puts: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl CasTransport for AlwaysMissCas {
        async fn exists(&self, _digest: &Digest) -> Result<bool> {
            Ok(false) // never present → the ONLY dedup is the in-run `seen` set
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
            self.puts.fetch_add(1, Ordering::SeqCst);
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
    impl AcTransport for AlwaysMissCas {
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

    #[tokio::test]
    async fn per_run_dedup_uploads_repeated_chunk_once() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Three vendored copies with byte-identical content → same chunk digest.
        for name in ["a.txt", "vendor/b.txt", "vendor/nested/c.txt"] {
            let p = root.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"identical vendored payload").unwrap();
        }

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = AlwaysMissCas::default();
        let opts = SnapshotOptions::new("ws-dedup");
        let report = snapshot(root, &client, &cache, &opts).await.unwrap();

        assert_eq!(report.files, 3, "three files snapshotted");
        // The repeated chunk is counted as uploaded exactly once.
        assert_eq!(
            report.chunks_uploaded, 1,
            "the repeated chunk must be uploaded exactly once per run"
        );
        // Exactly one content PUT + one manifest PUT = two remote PUTs total.
        // (Without the `seen` set this would be 3 content PUTs + 1 manifest = 4.)
        assert_eq!(
            client.puts.load(Ordering::SeqCst),
            2,
            "one content chunk + one manifest blob should be PUT"
        );
    }

    // -----------------------------------------------------------------------
    // M6 (idempotent 409): a create-only ref PUT can 409 spuriously (a retried
    // PUT after a lost ACK, or an eventual-consistency gap that let step-6's GET
    // miss a ref a prior run already wrote). If the occupant already IS what we
    // meant to write, that is an idempotent SUCCESS; only a genuinely DIFFERENT
    // occupant is a real conflict.
    // -----------------------------------------------------------------------

    /// AC that ALWAYS 409s on PUT and reveals the occupant only AFTER the first
    /// GET (step-6 sees nothing → no no-op short-circuit; the M6 recovery GET
    /// then reads it). CAS is a normal in-memory map.
    #[derive(Clone)]
    struct Idem409Ac {
        cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        stored: Arc<Vec<u8>>,
        get_calls: Arc<AtomicU64>,
    }

    impl Idem409Ac {
        fn new(stored: Vec<u8>) -> Self {
            Self {
                cas: Arc::new(Mutex::new(HashMap::new())),
                stored: Arc::new(stored),
                get_calls: Arc::new(AtomicU64::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl CasTransport for Idem409Ac {
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
    impl AcTransport for Idem409Ac {
        async fn get(&self, _key: &Digest) -> Result<Option<Vec<u8>>> {
            let n = self.get_calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(None) // step-6 GET: no ref visible yet
            } else {
                Ok(Some((*self.stored).clone())) // M6 recovery GET: the occupant
            }
        }
        async fn put(&self, _key: &Digest, _value: Vec<u8>) -> Result<()> {
            // Occupied key → the create-only server 409s (real transport shape).
            Err(ClwError::Http {
                status: 409,
                context: "PUT ac".into(),
            })
        }
        async fn delete(&self, _key: &Digest) -> Result<()> {
            Ok(())
        }
        async fn list_refs(&self) -> Result<Vec<Digest>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn idempotent_409_identical_ref_is_success() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.txt"), b"idempotent").unwrap();

        // Learn the manifest digest + size this tree WILL produce.
        let manifest = build_manifest_local(root).await.unwrap();
        let want_root = manifest.digest().unwrap();
        let want_size = manifest.canonical_bytes().unwrap().len() as u64;

        // A byte-DIFFERENT occupant (different created_at) that nonetheless
        // restores the SAME workspace (same root + manifest_size).
        let occupant = RefRecord {
            name: "ws-idem".into(),
            root: want_root,
            manifest_size: want_size,
            created_at_unix: 1,
            parent: None,
            tool_version: "test".into(),
        };
        let client = Idem409Ac::new(occupant.to_bytes().unwrap());

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let opts = SnapshotOptions::new("ws-idem");

        let report = snapshot(root, &client, &cache, &opts)
            .await
            .expect("an identical occupant on a 409 must be an idempotent SUCCESS");
        assert_eq!(report.root, want_root);
        assert!(
            report.unchanged,
            "idempotent 409 recovery reports unchanged"
        );
        assert!(
            client.get_calls.load(Ordering::SeqCst) >= 2,
            "M6 must GET the occupant after the 409"
        );
    }

    #[tokio::test]
    async fn idempotent_409_differing_ref_errors() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.txt"), b"idempotent").unwrap();

        // Occupant restores a DIFFERENT workspace → a genuine conflict.
        let occupant = RefRecord {
            name: "ws-idem".into(),
            root: Digest::of_bytes(b"some-other-root"),
            manifest_size: 999,
            created_at_unix: 1,
            parent: None,
            tool_version: "test".into(),
        };
        let client = Idem409Ac::new(occupant.to_bytes().unwrap());

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let opts = SnapshotOptions::new("ws-idem");

        let res = snapshot(root, &client, &cache, &opts).await.map(|_| ());
        match res {
            Err(ClwError::Http { status: 409, .. }) => {}
            other => {
                panic!("a DIFFERING occupant on a 409 must surface the conflict, got: {other:?}")
            }
        }
    }

    // -----------------------------------------------------------------------
    // Option C — a tree containing a symlink whose target ESCAPES the workspace
    // (absolute `/etc/passwd`) must SUCCEED: the escaping link is skipped (and
    // reported), the manifest contains the normal file but NO symlink entry for
    // it, and the security invariant holds — no external symlink is ever in the
    // manifest. RED-on-revert: restoring the walk's
    // `validate_symlink_target_contained(..)?` `?` makes `walk` (and thus this
    // snapshot) error instead of skipping.
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn snapshot_skips_external_symlink_and_reports_it() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("real.txt"), b"content").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();

        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path()).unwrap();
        let client = FakeTransport::default();
        let opts = SnapshotOptions::new("ws-external-symlink");

        let report = snapshot(root, &client, &cache, &opts)
            .await
            .expect("an external symlink must NOT fail the whole snapshot");

        // The escaping link is reported as skipped, listing its path -> target.
        assert_eq!(
            report.skipped_external_symlinks,
            vec!["link -> /etc/passwd".to_string()],
            "the external symlink must be reported as skipped, got {:?}",
            report.skipped_external_symlinks
        );

        // The built manifest carries the normal file but NO symlink entry for
        // `link` — the security invariant: the manifest never contains an
        // external/escaping symlink.
        let cas = client.cas.lock().await;
        let manifest = Manifest::from_bytes(cas.get(&report.root).unwrap()).unwrap();
        assert!(
            manifest
                .entries
                .iter()
                .any(|e| matches!(e, Entry::File { path, .. } if path == "real.txt")),
            "the normal file must be in the manifest: {:?}",
            manifest.entries
        );
        assert!(
            !manifest
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Symlink { path, .. } if path == "link")),
            "the manifest must NEVER contain the external symlink: {:?}",
            manifest.entries
        );
        assert_eq!(report.files, 1, "only the one real file is counted");
    }
}
