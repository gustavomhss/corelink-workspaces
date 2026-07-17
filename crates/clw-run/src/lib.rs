//! clw-run — AC-backed command memoization.
//!
//! The core primitive: "never run the same deterministic command twice."
//! Keys are computed from input file/directory digests, command vector, and
//! env vars. Results (stdout + stderr blobs) are stored in the CAS; the AC
//! maps the run key → RunRecord.
//!
//! # Memoization contract (v0.1)
//!
//! `clw run` memoizes on, and ONLY on:
//! - the digests (and unix mode bits) of the declared `--input` paths,
//! - the declared `--env` variable NAMES (their current values + set/unset
//!   presence are folded in; their *names* define the scope),
//! - the command vector + the resolved identity of `command[0]` (the tool), and
//! - the canonicalized absolute working directory the child runs in.
//!
//! **UNDECLARED environment variables are the CALLER's responsibility.** clw
//! does **not** sandbox the child's environment in v0.1: the child inherits the
//! orchestrator's full ambient environment (there is no `env_clear()`). An
//! undeclared env var that changes the command's output will NOT change the key,
//! so a caller whose command reads such a var MUST declare it via `--env` or risk
//! a stale replay. Sandboxing the child environment (`env_clear` + an explicit
//! allowlist) is a deliberate semantic change tracked as a post-beta hardening,
//! reserved for owner approval; it is intentionally out of scope for v0.1.

pub use clw_types as types;

use clw_manifest::walk;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result, CAS_BLOB_CAP_BYTES};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Public constants + types (frozen anchor)
// ---------------------------------------------------------------------------

pub const RUN_KEY_DOMAIN: &str = "clw/run/v3/";

/// H3 (credential leak): the credential-bearing environment variables that `clw`
/// itself reads (see `clw-cli/src/config.rs`) and that MUST be stripped from the
/// UNTRUSTED child's environment before it is spawned. Without this the child
/// (arbitrary CI code) inherits the CAS bearer token / redemption ticket and can
/// exfiltrate it for cross-tenant theft.
///
/// The list mirrors every secret `clw` resolves: the PAT (`CLW_TOKEN` and its
/// CoreLink-wide `CORELINK_TOKEN` alias) and the C2c credential-broker inputs
/// (`CLW_CRED_TICKET` = the single-use redemption auth, `CLW_LEASE_ID`,
/// `CLW_FABRIC_ENDPOINT`). The `CORELINK_*` namesakes of the broker vars are
/// stripped defensively too (clw reads only the `CLW_*` spellings today, but a
/// stray `CORELINK_*` credential in the ambient env must not reach the child
/// either). We strip ONLY these secrets — NOT `env_clear()` — so the child keeps
/// its `PATH`/`HOME`/locale and the non-secret `CLW_*` config (`CLW_ENDPOINT`,
/// `CLW_TENANT`, `CLW_REF_DOMAIN`, `CLW_CACHE_DIR`) a build may legitimately need.
const SECRET_ENV_VARS: &[&str] = &[
    "CLW_TOKEN",
    "CORELINK_TOKEN",
    "CLW_CRED_TICKET",
    "CORELINK_CRED_TICKET",
    "CLW_LEASE_ID",
    "CORELINK_LEASE_ID",
    "CLW_FABRIC_ENDPOINT",
    "CORELINK_FABRIC_ENDPOINT",
];

pub struct RunOptions {
    pub inputs: Vec<String>,
    pub command: Vec<String>,
    pub env_keys: Vec<String>,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub key: Digest,
    pub hit: bool,
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Internal record stored in the AC
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct RunRecord {
    exit_code: i32,
    stdout: Digest,
    stderr: Digest,
}

/// Returns `true` if `bytes` is a `clw run` memoization record (AC-backed run cache).
///
/// `clw run` stores its memo records in the SAME AC tenant keyspace as workspace refs
/// (both are AC PUTs under an opaque `BLAKE3` key), so a tenant-wide enumeration
/// (`clw ls --all` → `list_refs`) surfaces run-cache entries alongside refs. Those
/// records are NOT `RefRecord`s, so a naive parse fails and they look "corrupt". This
/// lets a caller tell a legitimate run-cache entry apart from a genuinely-corrupt ref:
/// a `RunRecord` and a `RefRecord` have disjoint required fields (`exit_code`/`stdout`/
/// `stderr` vs `name`/`root`/…), so neither parses as the other.
pub fn is_run_record(bytes: &[u8]) -> bool {
    serde_json::from_slice::<RunRecord>(bytes).is_ok()
}

/// If `bytes` is a `clw run` memoization record, return the `(stdout, stderr)` CAS
/// digests it points at; otherwise return `None`.
///
/// `clw run` stores its memo records in the SAME AC tenant keyspace as workspace refs,
/// and each `RunRecord` references two CAS blobs (the captured stdout/stderr). A
/// tenant-wide GC (`clw prune`) that walks `list_refs` must NOT treat those blobs as
/// orphans — deleting them would corrupt the run cache (a later cache HIT would fail
/// to fetch its output). This lets the GC pull a run-cache entry's live blobs into the
/// live set instead of mistaking the unparseable-as-`RefRecord` entry for corruption.
/// Returns `None` for a `RefRecord` or any non-`RunRecord` bytes (disjoint required
/// fields, so a `RefRecord` never parses as a `RunRecord`).
pub fn run_record_blobs(bytes: &[u8]) -> Option<(Digest, Digest)> {
    serde_json::from_slice::<RunRecord>(bytes)
        .ok()
        .map(|r| (r.stdout, r.stderr))
}

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Canonical JSON payload used to derive the run key.
#[derive(Serialize)]
struct KeyPayload {
    inputs: Vec<[String; 2]>,
    command: Vec<String>,
    env: Vec<[String; 2]>,
    /// Identity of the tool being invoked (`command[0]`), resolved to a content
    /// digest when the executable can be located + read, else a literal marker.
    /// Folding this into the key prevents a stale replay after a toolchain
    /// upgrade (e.g. `cargo` upgraded in place would otherwise be a false hit).
    tool: String,
    /// The canonicalized absolute working directory the child is spawned in
    /// (`.current_dir(cwd)`). Folding this into the key prevents a stale cache
    /// replay ACROSS workspaces: two invocations with identical inputs / command
    /// / env but different cwds (e.g. the same relative `src/` in two checkouts)
    /// would otherwise collide on one key and replay each other's output. This is
    /// purely safer — distinct cwds now yield distinct keys, never a false hit.
    cwd: String,
}

/// FIX 3: normalize a path string to `/` separators so the run key is stable
/// regardless of how the path was discovered. A top-level input is recorded
/// verbatim while the dir-walk joins nested entries with `/`; on Windows the
/// two forms differ (`a\b` vs `a/b`), spuriously splitting the cache and risking
/// a cross-platform key mismatch. Normalizing every path that feeds the key to
/// `/` makes it separator-stable. On unix `MAIN_SEPARATOR` is already `/`, so
/// this is a no-op there (a literal `\` in a unix filename is left untouched).
fn norm_sep(p: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        p.to_string()
    } else {
        p.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// FIX 2: the unix permission bits (masked to `0o777`) of an input file, folded
/// into the key so a `chmod +x` invalidates a stale memoized result even though
/// the content is byte-identical. On non-unix there is no mode concept, so a
/// stable constant is returned (matching how `clw-manifest::walk` treats it).
fn file_mode(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0o644
    }
}

/// True if `cmd0` is a path (resolve directly) rather than a bare name to look up
/// on `PATH`. Cross-platform: matches a forward slash (every platform), the OS
/// path separator (`\` on Windows), and absolute paths incl. Windows drive-letters
/// (`C:\tool.exe`, `C:/tool.exe`). Without the Windows arms, a backslash/drive
/// path would fall into the PATH branch, miss, and silently degrade the tool key.
fn looks_like_path(cmd0: &str) -> bool {
    cmd0.contains('/') || cmd0.contains(std::path::MAIN_SEPARATOR) || Path::new(cmd0).is_absolute()
}

/// Search `PATH` for a bare command name. On Windows a bare `cargo` is launched as
/// `cargo.exe`, so we also try each `PATHEXT` extension — otherwise the lookup
/// misses and the tool key degrades to a content-free literal.
fn search_path(cmd0: &str) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let direct = dir.join(cmd0);
        if direct.is_file() {
            return Some(direct);
        }
        #[cfg(windows)]
        {
            // PATHEXT is `;`-separated (".COM;.EXE;.BAT;..."), which is exactly the
            // Windows PATH separator, so `split_paths` parses it; each entry keeps
            // its leading dot.
            if let Some(exts) = std::env::var_os("PATHEXT") {
                for ext in std::env::split_paths(&exts) {
                    let ext = ext.to_string_lossy();
                    if ext.is_empty() {
                        continue;
                    }
                    let candidate = dir.join(format!("{cmd0}{ext}"));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

/// Resolve `command[0]` to a stable tool-identity string.
///
/// If `cmd0` is a path (see [`looks_like_path`]) it is resolved against the RUN
/// cwd; otherwise `PATH` is searched (with `PATHEXT` on Windows). When the
/// executable can be read, the identity is its content digest; if it cannot be
/// located or read, we fall back to a literal marker so the key is still
/// well-defined.
async fn resolve_tool_id(cwd: &Path, cmd0: &str) -> String {
    // A relative tool path must resolve where the command actually RUNS, not
    // against the orchestrator process cwd (else the digest is unstable / wrong).
    // `Path::join` returns cmd0 unchanged when it is already absolute.
    let candidate: Option<std::path::PathBuf> = if looks_like_path(cmd0) {
        Some(cwd.join(cmd0))
    } else {
        search_path(cmd0)
    };
    // Not locatable on PATH/cwd → nothing to hash; a literal marker keeps the key
    // well-defined (the command string itself is already in the payload).
    let Some(path) = candidate else {
        return format!("literal:{cmd0}");
    };
    // Read via tokio::fs so the (potentially large) toolchain binary is not read
    // with a BLOCKING std::fs call on the async worker (matches how the inputs
    // above are digested).
    match tokio::fs::read(&path).await {
        Ok(bytes) => format!("digest:{}", Digest::of_bytes(&bytes).to_hex()),
        Err(_) => {
            // The file EXISTS but is unreadable (e.g. exec-only / no read bit —
            // exec and read are independent perms on unix). A content-free
            // `literal:` would make two different builds of the tool collide and
            // replay stale output across an in-place upgrade. Fold in a non-content
            // identity (len + mtime) that still changes when the binary does;
            // fall back to literal only if even the metadata is unavailable.
            match tokio::fs::metadata(&path).await {
                Ok(m) => {
                    let mtime_ns = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    format!("meta:{}:{}", m.len(), mtime_ns)
                }
                Err(_) => format!("literal:{cmd0}"),
            }
        }
    }
}

/// Compute the run key for the given options and working directory.
///
/// The key folds in the declared `--input` digests (+ mode bits), the declared
/// `--env` variable names (+ their current presence/value), the command vector,
/// the resolved tool identity (`command[0]`), and the canonicalized absolute
/// `cwd`. See the crate-level "Memoization contract (v0.1)" doc for the full
/// boundary — notably that UNDECLARED env vars are NOT in the key and are the
/// caller's responsibility (clw does not sandbox the child env in v0.1).
///
/// Returns `Err(ClwError::Io(...))` if any input path is missing, or
/// `Err(ClwError::InvalidManifest(...))` for walk errors.
async fn compute_key(cwd: &Path, opts: &RunOptions) -> Result<Digest> {
    // --- inputs ---
    // NOTE (cache-key change): this key now folds in (a) empty input directories
    // by path, (b) each input file's unix mode (exec/perm bits), and (c) `/`-
    // normalized path separators. All three CORRECTLY invalidate pre-existing
    // cache entries — that is expected, not a regression.
    let mut input_pairs: Vec<[String; 2]> = Vec::new();

    for rel in &opts.inputs {
        // Normalize the top-level input path so a key is stable regardless of the
        // separator the caller used (`\` on Windows) and identical to the form the
        // dir-walk emits for nested entries (which join with `/`). See `norm_sep`.
        let rel = norm_sep(rel);
        let abs = cwd.join(&rel);
        let meta = tokio::fs::symlink_metadata(&abs).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("input not found: {}", abs.display()),
                )
            } else {
                e
            }
        })?;

        if meta.file_type().is_symlink() {
            // Symlink: contribute target string.
            let target = tokio::fs::read_link(&abs).await?;
            let target_str = norm_sep(&target.to_string_lossy());
            input_pairs.push([rel.clone(), format!("symlink:{}", target_str)]);
        } else if meta.file_type().is_dir() {
            // Directory: walk it, digest each file, collect entries sorted by rel path.
            let walked = walk(&abs)?;
            // FIX 1: a directly-declared input directory that is ITSELF empty walks
            // to zero entries (`walk` skips its own root), so without this it would
            // contribute nothing and adding/removing the dir would be a false hit.
            // Anchor its path so its presence/absence moves the key.
            if walked.is_empty() {
                input_pairs.push([rel.clone(), "emptydir".to_string()]);
            }
            for entry in walked {
                match entry {
                    clw_manifest::WalkedEntry::File(f) => {
                        let content = tokio::fs::read(&f.abs).await?;
                        let digest = Digest::of_bytes(&content);
                        let entry_rel = format!("{}/{}", rel, norm_sep(&f.rel));
                        // FIX 2: fold the file's mode (perm bits) into the value so a
                        // `chmod +x` on a declared input changes the key even though
                        // the content is byte-identical. `walk` already masks to 0o777.
                        input_pairs.push([entry_rel, format!("{}:{:o}", digest.to_hex(), f.mode)]);
                    }
                    clw_manifest::WalkedEntry::Symlink {
                        rel: sym_rel,
                        target,
                    } => {
                        let entry_rel = format!("{}/{}", rel, norm_sep(&sym_rel));
                        input_pairs.push([entry_rel, format!("symlink:{}", norm_sep(&target))]);
                    }
                    clw_manifest::WalkedEntry::EmptyDir { rel: dir_rel } => {
                        // FIX 1: an empty input directory must still contribute its
                        // path to the key — otherwise a build that depends on the
                        // dir's presence/absence gets a false hit when it appears or
                        // disappears. Content-free, but path-bearing.
                        let entry_rel = format!("{}/{}", rel, norm_sep(&dir_rel));
                        input_pairs.push([entry_rel, "emptydir".to_string()]);
                    }
                    clw_manifest::WalkedEntry::SkippedExternalSymlink {
                        rel: sym_rel,
                        target,
                    } => {
                        // A symlink whose target escapes the workspace is un-snapshottable
                        // (snapshot skips it), but for a `run` INPUT-cache key its
                        // presence/target must still move the key — anchor it path-and-
                        // target-bearing so a build that depends on such a link appearing,
                        // disappearing, or being repointed does not get a false cache hit.
                        let entry_rel = format!("{}/{}", rel, norm_sep(&sym_rel));
                        input_pairs
                            .push([entry_rel, format!("external-symlink:{}", norm_sep(&target))]);
                    }
                }
            }
        } else {
            // Regular file: digest bytes + fold in the unix mode (FIX 2).
            let content = tokio::fs::read(&abs).await?;
            let digest = Digest::of_bytes(&content);
            input_pairs.push([
                rel.clone(),
                format!("{}:{:o}", digest.to_hex(), file_mode(&meta)),
            ]);
        }
    }

    // Sort by path (first element of pair) — canonical ordering.
    input_pairs.sort_by(|a, b| a[0].cmp(&b[0]));

    // --- env ---
    // Presence-distinguishing encoding: an UNSET var ("unset") and a var SET to
    // "" ("set:") now produce DIFFERENT bytes. Previously both collapsed to ""
    // and yielded identical key bytes, breaking hermeticity (unset ≠ empty).
    let mut env_pairs: Vec<[String; 2]> = Vec::new();
    let mut env_keys = opts.env_keys.clone();
    env_keys.sort();
    for key in &env_keys {
        let value = match std::env::var(key) {
            Ok(v) => format!("set:{v}"),
            Err(_) => "unset".to_string(),
        };
        env_pairs.push([key.clone(), value]);
    }

    // --- tool identity ---
    // Resolve command[0] to a content digest (or literal marker) so an in-place
    // toolchain upgrade invalidates stale memoized results instead of replaying
    // them. `run_memoized` guarantees a non-empty command before reaching here,
    // but guard defensively in case compute_key is ever called directly.
    let tool = if opts.command.is_empty() {
        String::new()
    } else {
        resolve_tool_id(cwd, &opts.command[0]).await
    };

    // --- working directory ---
    // Fold the canonicalized absolute cwd into the key so two invocations with
    // identical inputs/command/env in DIFFERENT directories yield different keys
    // (no stale cross-workspace replay). Canonicalize so equivalent spellings of
    // the same dir (symlinks, `..`, relative) collapse to one identity; fall back
    // to the `/`-normalized lossy path if the dir cannot be canonicalized (e.g. a
    // permission quirk) so the key stays well-defined rather than failing.
    let cwd_key = match tokio::fs::canonicalize(cwd).await {
        Ok(abs) => norm_sep(&abs.to_string_lossy()),
        Err(_) => norm_sep(&cwd.to_string_lossy()),
    };

    // --- canonical JSON ---
    let payload = KeyPayload {
        inputs: input_pairs,
        command: opts.command.clone(),
        env: env_pairs,
        tool,
        cwd: cwd_key,
    };
    let json_bytes = serde_json::to_vec(&payload)?;

    // --- BLAKE3(domain || json) ---
    let mut hasher = blake3::Hasher::new();
    hasher.update(RUN_KEY_DOMAIN.as_bytes());
    hasher.update(&json_bytes);
    Ok(Digest(*hasher.finalize().as_bytes()))
}

// ---------------------------------------------------------------------------
// FIX 3: bounded child-output capture
// ---------------------------------------------------------------------------

/// Error from [`read_capped`]: either the stream exceeded the cap (carrying the
/// projected size that tripped it) or an underlying I/O error.
enum CapErr {
    TooLarge(u64),
    Io(std::io::Error),
}

impl CapErr {
    fn into_clw(self) -> ClwError {
        match self {
            // A TooLarge here is handled by the caller before this is reached, but
            // keep a faithful mapping so the conversion is total.
            CapErr::TooLarge(size) => ClwError::TooLarge {
                size,
                cap: CAS_BLOB_CAP_BYTES as u64,
            },
            CapErr::Io(e) => ClwError::Io(e),
        }
    }
}

/// Read an optional async pipe into a `Vec` with a HARD running ceiling. Reads in
/// fixed-size blocks and aborts with [`CapErr::TooLarge`] the moment the buffer
/// would exceed `cap`, so we never retain more than ~`cap` + one block of bytes.
/// A `None` pipe (stream not captured) yields an empty buffer.
///
/// H2 (backgrounded-grandchild hang): the read races a `stop` signal. Normally we
/// read to EOF; but a grandchild that inherited the pipe's write end (e.g.
/// `sh -c 'sleep 300 & exit 0'`, or a build spawning a daemon) keeps that write
/// end OPEN after the direct child exits, so EOF NEVER comes and an unbounded read
/// would wedge forever. The caller therefore arms a BOUNDED post-exit grace and
/// flips `stop`; on `stop` we return whatever we have BUFFERED so far (the output
/// the child actually wrote before exiting — it was already readable, so it is in
/// `buf` by the time the grace elapses) instead of blocking on an EOF that a
/// still-living orphan will never deliver. Returning drops the reader, closing our
/// read end → the orphaned writer gets SIGPIPE on its next write.
async fn read_capped<R>(
    reader: Option<R>,
    cap: usize,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> std::result::Result<Vec<u8>, CapErr>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt as _;
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut buf: Vec<u8> = Vec::new();
    let mut block = [0u8; 64 * 1024];
    loop {
        tokio::select! {
            res = reader.read(&mut block) => {
                let n = res.map_err(CapErr::Io)?;
                if n == 0 {
                    break; // clean EOF — the write end closed.
                }
                let projected = buf.len() as u64 + n as u64;
                if projected > cap as u64 {
                    // Stop reading immediately; the caller will kill the child. We do
                    // NOT keep extending `buf`, so memory stays bounded at ~cap + one
                    // block.
                    return Err(CapErr::TooLarge(projected));
                }
                buf.extend_from_slice(&block[..n]);
            }
            _ = stop.changed() => {
                // Post-exit grace elapsed while EOF is still outstanding (a grandchild
                // holds the write end). Stop draining and return what the child already
                // wrote — do NOT wait for an EOF that will never arrive.
                break;
            }
        }
    }
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run `opts.command` in `cwd`, memoized on the content-addressed run key.
///
/// On a key HIT the stored stdout/stderr/exit are replayed without re-executing;
/// on a MISS the child runs (inheriting the orchestrator's full ambient
/// environment — clw does NOT `env_clear()` in v0.1) and a zero-exit result is
/// memoized. The key is derived by [`compute_key`] from the declared `--input`
/// digests, the declared `--env` variable NAMES, the command + tool identity,
/// and the canonicalized `cwd`. See the crate-level "Memoization contract (v0.1)"
/// doc: UNDECLARED env vars that affect the command's output are NOT in the key
/// and are the CALLER's responsibility to declare via `--env`; child env
/// sandboxing is a tracked post-beta hardening, not in scope here.
pub async fn run_memoized<C: CasTransport + AcTransport>(
    cwd: &Path,
    client: &C,
    opts: &RunOptions,
) -> Result<RunOutcome> {
    if opts.command.is_empty() {
        return Err(ClwError::InvalidManifest("empty command".to_string()));
    }

    let key = compute_key(cwd, opts).await?;

    // --- AC lookup ---
    if let Some(record_bytes) = AcTransport::get(client, &key).await? {
        // Try to parse the record and fetch blobs; treat any failure as a miss.
        if let Ok(record) = serde_json::from_slice::<RunRecord>(&record_bytes) {
            match (
                CasTransport::get(client, &record.stdout).await,
                CasTransport::get(client, &record.stderr).await,
            ) {
                (Ok(stdout), Ok(stderr)) => {
                    // Diagnostic channel (M5.1): cache hit + key on stderr.
                    tracing::debug!(key = %key.to_hex(), hit = true, "run cache hit");
                    return Ok(RunOutcome {
                        key,
                        hit: true,
                        exit_code: record.exit_code,
                        stdout,
                        stderr,
                    });
                }
                _ => {
                    // Corrupt/unfetchable → treat as miss; fall through.
                }
            }
        }
        // Corrupt record → treat as miss; fall through.
    }

    // --- MISS: execute the command ---
    // Diagnostic channel (M5.1): cache miss + key on stderr.
    tracing::debug!(key = %key.to_hex(), hit = false, "run cache miss; executing");
    let mut command = tokio::process::Command::new(&opts.command[0]);
    command
        .args(&opts.command[1..])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // H3 (credential leak): strip clw's secret-bearing env vars so the UNTRUSTED
    // child cannot read the CAS bearer token / redemption ticket and exfiltrate it.
    // We remove ONLY the known secrets (NOT env_clear) so PATH/HOME/locale and the
    // non-secret CLW_* config survive for the build. See `SECRET_ENV_VARS`.
    for var in SECRET_ENV_VARS {
        command.env_remove(var);
    }
    let mut child = command.spawn()?;

    // FIX 3: enforce the output-size cap WHILE capturing, never buffering the whole
    // child output first. `wait_with_output` reads ALL of stdout/stderr into memory
    // before any size check runs, so a command emitting gigabytes OOMs the process
    // before the cap is consulted. Instead we read each stream with a running
    // ceiling (CAS_BLOB_CAP_BYTES) and KILL the child the moment either exceeds it.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    // CRITICAL (deadlock- AND hang-safe): read BOTH streams concurrently while ALSO
    // observing the DIRECT child's exit in the SAME `select!`. Two failure modes are
    // guarded here:
    //
    // 1. Cap-blow DEADLOCK. A plain `join!` DEADLOCKS: when one `read_capped` returns
    //    (cap blown) and stops draining its pipe, the child blocks on its next write
    //    to that now-unread pipe (the OS pipe buffer fills), starving the OTHER
    //    reader, which never sees EOF. So the MOMENT either stream exceeds the cap we
    //    KILL the child and RETURN immediately — never awaiting the other stream.
    //    Returning drops both read futures, closing our read ends → any orphaned
    //    writer gets SIGPIPE. We reap the direct child to avoid a zombie.
    //
    // 2. Grandchild HANG (H2). Draining only until BOTH streams EOF wedges FOREVER
    //    when a backgrounded grandchild inherited the pipe write end (e.g.
    //    `sh -c 'sleep 300 & exit 0'`, or a build spawning a daemon / a `make`
    //    sub-job): the direct child exits but the write end stays open, so EOF never
    //    comes and there is no timeout. We therefore watch `child.wait()` in the
    //    select too; once the DIRECT child has exited we arm a BOUNDED post-exit
    //    drain grace, and if a stream still has not EOF'd when the grace elapses we
    //    signal the readers (via `stop_tx`) to return what they have BUFFERED and
    //    stop. Dropping the read futures then closes our read ends → the orphan gets
    //    SIGPIPE. The child's real output (written before it exited) was already
    //    readable, so it is still captured; only a never-arriving orphan-held EOF is
    //    abandoned. The grace only ever DELAYS the grandchild case — in the normal
    //    path both streams EOF the instant the child's write end closes, so the loop
    //    breaks with no wait and the fast path is not regressed.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut stdout_fut = std::pin::pin!(read_capped(
        stdout_pipe,
        CAS_BLOB_CAP_BYTES,
        stop_rx.clone()
    ));
    let mut stderr_fut = std::pin::pin!(read_capped(stderr_pipe, CAS_BLOB_CAP_BYTES, stop_rx));
    let mut stdout_res: Option<std::result::Result<Vec<u8>, CapErr>> = None;
    let mut stderr_res: Option<std::result::Result<Vec<u8>, CapErr>> = None;
    // The direct child's exit status, observed via the select so a grandchild holding
    // the pipe can no longer wedge us waiting for an EOF that never comes.
    let mut status: Option<std::process::ExitStatus> = None;
    // Bounded post-exit drain grace: how long we keep collecting already-buffered
    // output after the DIRECT child exits before abandoning an outstanding EOF. A few
    // seconds is ample — the child's own output is already readable the moment it
    // exits; the grace only bounds the wait for a grandchild-held EOF that never
    // arrives. Disabled (`grace_armed = false`) until the child actually exits.
    const POST_EXIT_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
    let grace = tokio::time::sleep(POST_EXIT_DRAIN_GRACE);
    tokio::pin!(grace);
    let mut grace_armed = false;
    loop {
        tokio::select! {
            r = &mut stdout_fut, if stdout_res.is_none() => {
                if let Err(CapErr::TooLarge(n)) = &r {
                    let size = *n;
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ClwError::TooLarge { size, cap: CAS_BLOB_CAP_BYTES as u64 });
                }
                stdout_res = Some(r);
            }
            r = &mut stderr_fut, if stderr_res.is_none() => {
                if let Err(CapErr::TooLarge(n)) = &r {
                    let size = *n;
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ClwError::TooLarge { size, cap: CAS_BLOB_CAP_BYTES as u64 });
                }
                stderr_res = Some(r);
            }
            s = child.wait(), if status.is_none() => {
                status = Some(s?);
                // Direct child has exited — arm the bounded grace. If the streams do
                // not EOF within it, a grandchild is holding the write end; we then
                // stop draining rather than hang.
                grace.as_mut().reset(tokio::time::Instant::now() + POST_EXIT_DRAIN_GRACE);
                grace_armed = true;
            }
            _ = &mut grace, if grace_armed && (stdout_res.is_none() || stderr_res.is_none()) => {
                // Grace elapsed after the direct child exited with a stream still not
                // EOF'd (grandchild holds the pipe). Tell the readers to return their
                // buffered bytes; the read arms then complete and the loop breaks.
                let _ = stop_tx.send(true);
                grace_armed = false; // one-shot
            }
        }
        // Done once BOTH streams have yielded AND the direct child has been reaped.
        if stdout_res.is_some() && stderr_res.is_some() && status.is_some() {
            break;
        }
    }
    let stdout = stdout_res
        .expect("loop breaks only when both streams are Some")
        .map_err(CapErr::into_clw)?;
    let stderr = stderr_res
        .expect("loop breaks only when both streams are Some")
        .map_err(CapErr::into_clw)?;
    let status = status.expect("loop breaks only when the child has been reaped");
    // A signal-killed child has no exit code; map it to the POSIX 128+signal
    // convention so the propagated process exit is meaningful (e.g. SIGTERM → 143)
    // instead of -1, which `process::exit` truncates to a useless 255.
    let exit_code = match status.code() {
        Some(c) => c,
        #[cfg(unix)]
        None => {
            use std::os::unix::process::ExitStatusExt as _;
            status.signal().map(|s| 128 + s).unwrap_or(255)
        }
        #[cfg(not(unix))]
        None => 255,
    };

    // --- Memoize only on success (output is already within cap by construction) ---
    // We cache iff the DIRECT child exited 0. This holds even when the H2 post-exit
    // grace fired (a grandchild kept the pipe open): `status` is the direct child's
    // OWN exit, and the bytes we buffered are exactly what it wrote BEFORE exiting
    // (its output is readable the instant it exits, so it is already in `buf` when the
    // grace elapses) — abandoning a never-arriving orphan-held EOF does NOT truncate
    // the child's result. A cap-blow or a non-zero/signal exit never reaches here (the
    // former returns `TooLarge` above; the latter fails the `== 0` guard), so a torn
    // or unsuccessful run is never cached as a complete success.
    if exit_code == 0 {
        let stdout_digest = Digest::of_bytes(&stdout);
        let stderr_digest = Digest::of_bytes(&stderr);

        let record = RunRecord {
            exit_code,
            stdout: stdout_digest,
            stderr: stderr_digest,
        };
        let record_bytes = serde_json::to_vec(&record).ok();

        // Post-success cache write. The child ALREADY exited 0, so these three
        // stores are pure best-effort caching (a store failure is IGNORED — the
        // outcome is still valid). Two hazards, both closed here:
        //   * SERIAL cost — three awaited `with_retry` stores, each with its own
        //     ~300 s op-deadline, sum to ~15 min if the backend goes black-hole
        //     AFTER the child passed. So run them CONCURRENTLY (`join!`): the phase
        //     is bounded by the SLOWEST single store, not the sum.
        //   * UNBOUNDED wait — a black-hole backend still stalls each concurrent
        //     store for its full op-deadline. So cap the WHOLE phase with one SHORT
        //     grace: on timeout the cache write is simply skipped and we still
        //     return the correct exit 0. A best-effort cache write must never
        //     materially delay an already-successful run.
        const STORE_PHASE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
        let _ = tokio::time::timeout(STORE_PHASE_DEADLINE, async {
            tokio::join!(
                async {
                    let _ = CasTransport::put(client, &stdout_digest, stdout.clone()).await;
                },
                async {
                    let _ = CasTransport::put(client, &stderr_digest, stderr.clone()).await;
                },
                async {
                    if let Some(bytes) = record_bytes {
                        let _ = AcTransport::put(client, &key, bytes).await;
                    }
                },
            );
        })
        .await;
    }

    Ok(RunOutcome {
        key,
        hit: false,
        exit_code,
        stdout,
        stderr,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{AcTransport, CasTransport, Digest, Result};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[test]
    fn is_run_record_distinguishes_run_records_from_refs_and_garbage() {
        let d = "ab".repeat(32);
        // A RunRecord-shaped payload is recognized.
        let run = format!("{{\"exit_code\":0,\"stdout\":\"{d}\",\"stderr\":\"{d}\"}}");
        assert!(is_run_record(run.as_bytes()));
        // A RefRecord-shaped payload is NOT a run record (disjoint required fields).
        let ref_rec = clw_types::RefRecord {
            name: "ws".into(),
            root: Digest::of_bytes(b"r"),
            manifest_size: 1,
            created_at_unix: 0,
            parent: None,
            tool_version: "t".into(),
        }
        .to_bytes()
        .unwrap();
        assert!(!is_run_record(&ref_rec));
        // Non-JSON / garbage is not a run record.
        assert!(!is_run_record(b"not json at all"));
        assert!(!is_run_record(b""));
    }

    #[test]
    fn run_record_blobs_returns_digests_for_run_records_and_none_otherwise() {
        // A RunRecord → Some((stdout, stderr)) with the exact digests it carries.
        let stdout = Digest::of_bytes(b"the stdout blob");
        let stderr = Digest::of_bytes(b"the stderr blob");
        let run = RunRecord {
            exit_code: 0,
            stdout,
            stderr,
        };
        let bytes = serde_json::to_vec(&run).unwrap();
        assert_eq!(run_record_blobs(&bytes), Some((stdout, stderr)));

        // A RefRecord (disjoint required fields) → None.
        let ref_rec = clw_types::RefRecord {
            name: "ws".into(),
            root: Digest::of_bytes(b"r"),
            manifest_size: 1,
            created_at_unix: 0,
            parent: None,
            tool_version: "t".into(),
        }
        .to_bytes()
        .unwrap();
        assert_eq!(run_record_blobs(&ref_rec), None);

        // Garbage / empty → None.
        assert_eq!(run_record_blobs(b"not json at all"), None);
        assert_eq!(run_record_blobs(b""), None);
    }

    // -----------------------------------------------------------------------
    // PART A — proptest: the cache-hit-path `RunRecord` parse
    // (serde_json::from_slice in the replay path) must never panic on arbitrary
    // bytes — only Ok/Err. RunRecord is private; this in-crate test sees it via
    // `use super::*`. A panic = test failure (non-vacuous).
    // -----------------------------------------------------------------------
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        // Case count for the in-crate fuzzers. The deep run (50k) is OPT-IN via
        // the repo's existing fuzz lane signal (CLW_FUZZ_GATE=1, see
        // scripts/gates.sh); the default keeps the per-PR gate fast while still
        // exercising a meaningful 1k cases.
        fn fuzz_cases() -> u32 {
            if std::env::var("CLW_FUZZ_GATE").is_ok() {
                50_000
            } else {
                1_000
            }
        }

        // A RunRecord-shaped JSON value (right keys, hostile field values) so the
        // fuzzer reaches serde's structural decode, not just JSON-syntax reject.
        // Returns the bytes PLUS an INDEPENDENT oracle for "should decode as a
        // RunRecord": a RunRecord requires `exit_code` (any i32 is valid) plus
        // both `stdout`/`stderr` as valid 64-hex `Digest`s. The oracle is computed
        // via `Digest::from_hex` — a DIFFERENT code path than `is_run_record`'s
        // `from_slice::<RunRecord>` — so the assertion is a real cross-check rather
        // than `f(x) == f(x)`.
        fn runrecordish_json() -> impl Strategy<Value = (Vec<u8>, bool)> {
            let digestish = prop_oneof![
                "[0-9a-f]{64}",      // valid 64-hex digest
                "[0-9a-fA-F]{0,80}", // wrong length / uppercase
                ".{0,16}",           // arbitrary garbage
            ];
            (any::<i32>(), digestish.clone(), digestish).prop_map(|(ec, so, se)| {
                let bytes = serde_json::to_vec(&serde_json::json!({
                    "exit_code": ec, "stdout": so, "stderr": se,
                }))
                .unwrap();
                let expected = clw_types::Digest::from_hex(&so).is_ok()
                    && clw_types::Digest::from_hex(&se).is_ok();
                (bytes, expected)
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]

            /// `serde_json::from_slice::<RunRecord>` on ARBITRARY bytes never
            /// panics — only Ok/Err (the cache-hit replay decode path).
            #[test]
            fn run_record_from_slice_never_panics(
                bytes in proptest::collection::vec(any::<u8>(), 0..4096)
            ) {
                let _: std::result::Result<RunRecord, _> = serde_json::from_slice(&bytes);
            }

            /// `is_run_record` (the ls/prune classifier) on ARBITRARY bytes never
            /// panics — only true/false (pure no-panic guard over hostile input;
            /// the structured fuzzer below cross-checks the true/false result).
            #[test]
            fn is_run_record_never_panics(
                bytes in proptest::collection::vec(any::<u8>(), 0..4096)
            ) {
                let _ = is_run_record(&bytes);
            }

            /// `is_run_record` on STRUCTURED RunRecord-shaped JSON never panics AND
            /// agrees with an INDEPENDENT oracle (both digest fields valid 64-hex,
            /// via `Digest::from_hex` — not the `from_slice` `is_run_record` uses).
            /// Exercises the field-decode path (valid/garbage digest hex) rather
            /// than the JSON-syntax reject, and is non-vacuous.
            #[test]
            fn is_run_record_structured_matches_digest_oracle(case in runrecordish_json()) {
                let (bytes, expected) = case;
                prop_assert_eq!(is_run_record(&bytes), expected);
            }

            /// DISJOINTNESS: a well-formed `RefRecord` (the OTHER thing in the AC
            /// keyspace) must NEVER be classified as a run record — the property the
            /// ls/prune classifier relies on to tell run-cache entries from refs.
            #[test]
            fn refrecord_never_misclassified_as_run_record(
                name in ".*",
                created in any::<u64>(),
                msize in any::<u64>(),
            ) {
                let rr = clw_types::RefRecord {
                    name,
                    root: clw_types::Digest::of_bytes(b"root"),
                    manifest_size: msize,
                    created_at_unix: created,
                    parent: None,
                    tool_version: "test".to_string(),
                };
                let bytes = rr.to_bytes().unwrap();
                prop_assert!(!is_run_record(&bytes));
            }
        }
    }

    #[test]
    fn looks_like_path_detects_paths_cross_platform() {
        // Bare names are PATH lookups (false); anything with a separator or an
        // absolute prefix is a direct path (true). These cases hold on every
        // platform; the Windows-only `\`/drive-letter arms are exercised by the
        // cross-compile gate and behave per std::path on a Windows host.
        assert!(!looks_like_path("cargo"));
        assert!(!looks_like_path("rustc"));
        assert!(looks_like_path("./tool"));
        assert!(looks_like_path("../bin/tool"));
        assert!(looks_like_path("/usr/bin/cargo"));
        assert!(looks_like_path("dir/tool"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_only_tool_uses_metadata_not_content_free_literal() {
        // A binary that is executable but NOT readable (no read bit) must NOT
        // collapse to a content-free `literal:` key — that would replay stale
        // output across an in-place upgrade. It should fold in len+mtime instead.
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempDir::new().unwrap();
        let tool = dir.path().join("tool");
        std::fs::write(&tool, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o111)).unwrap();

        let id = resolve_tool_id(dir.path(), tool.to_str().unwrap()).await;
        assert!(
            id.starts_with("meta:"),
            "exec-only tool must use a metadata identity, got {id:?}"
        );
        assert!(
            !id.starts_with("literal:"),
            "must not be a content-free key"
        );
    }

    // -----------------------------------------------------------------------
    // In-memory fake transport
    // -----------------------------------------------------------------------

    #[derive(Clone, Default)]
    struct FakeStore {
        cas: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        ac: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }

    #[async_trait::async_trait]
    impl CasTransport for FakeStore {
        async fn exists(&self, digest: &Digest) -> Result<bool> {
            Ok(self.cas.lock().unwrap().contains_key(&digest.to_hex()))
        }

        async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
            self.cas
                .lock()
                .unwrap()
                .get(&digest.to_hex())
                .cloned()
                .ok_or(ClwError::NotFound(*digest))
        }

        async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
            self.cas.lock().unwrap().insert(digest.to_hex(), bytes);
            Ok(())
        }

        async fn delete(&self, digest: &Digest) -> Result<()> {
            self.cas.lock().unwrap().remove(&digest.to_hex());
            Ok(())
        }

        async fn list_blobs(&self) -> Result<Vec<Digest>> {
            self.cas
                .lock()
                .unwrap()
                .keys()
                .map(|k| Digest::from_hex(k))
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl AcTransport for FakeStore {
        async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
            Ok(self.ac.lock().unwrap().get(&key.to_hex()).cloned())
        }

        async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
            self.ac.lock().unwrap().insert(key.to_hex(), value);
            Ok(())
        }

        async fn delete(&self, key: &Digest) -> Result<()> {
            self.ac.lock().unwrap().remove(&key.to_hex());
            Ok(())
        }

        async fn list_refs(&self) -> Result<Vec<Digest>> {
            self.ac
                .lock()
                .unwrap()
                .keys()
                .map(|k| Digest::from_hex(k))
                .collect()
        }
    }

    #[tokio::test]
    async fn fake_list_refs_and_blobs_return_map_keys() {
        let store = FakeStore::default();
        let r = Digest::of_bytes(b"r");
        let b = Digest::of_bytes(b"b");
        AcTransport::put(&store, &r, b"v".to_vec()).await.unwrap();
        CasTransport::put(&store, &b, b"data".to_vec())
            .await
            .unwrap();
        assert_eq!(AcTransport::list_refs(&store).await.unwrap(), vec![r]);
        assert_eq!(CasTransport::list_blobs(&store).await.unwrap(), vec![b]);
        CasTransport::delete(&store, &b).await.unwrap();
        assert!(CasTransport::list_blobs(&store).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fake_delete_removes_key_and_is_idempotent() {
        let store = FakeStore::default();
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
    // Test: miss executes, stores; identical second call is a hit (no re-exec)
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses /bin/sh + unix shell command (Windows port: M2)
    #[tokio::test]
    async fn miss_executes_and_stores_then_hit_replays() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();

        // Create a side-effect file path; command writes to it on first run.
        let side_effect = cwd.join("side_effect.txt");
        let side_effect_str = side_effect.to_str().unwrap().to_string();

        // Input file
        std::fs::write(cwd.join("input.txt"), b"hello").unwrap();

        let store = FakeStore::default();

        // First call: miss — should execute and write side-effect.
        let cmd = format!("echo hello > /dev/null && touch {}", side_effect_str);
        let opts = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec!["/bin/sh".to_string(), "-c".to_string(), cmd.clone()],
            env_keys: vec![],
        };
        let outcome1 = run_memoized(cwd, &store, &opts).await.unwrap();
        assert!(!outcome1.hit, "first call must be a miss");
        assert_eq!(outcome1.exit_code, 0);
        assert!(
            side_effect.exists(),
            "side-effect file must be created on miss"
        );

        // Remove side-effect to detect if command runs again.
        std::fs::remove_file(&side_effect).unwrap();

        // Second call: identical opts → must be a hit, command must NOT re-execute.
        let opts2 = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec!["/bin/sh".to_string(), "-c".to_string(), cmd],
            env_keys: vec![],
        };
        let outcome2 = run_memoized(cwd, &store, &opts2).await.unwrap();
        assert!(outcome2.hit, "second call must be a hit");
        assert_eq!(outcome2.exit_code, 0);
        assert!(
            !side_effect.exists(),
            "side-effect file must NOT be created on hit (command was not re-executed)"
        );

        // stdout replayed byte-exact.
        assert_eq!(outcome1.stdout, outcome2.stdout);
    }

    // -----------------------------------------------------------------------
    // Test: changing input content → different key → miss
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses /bin/sh (Windows port: M2)
    #[tokio::test]
    async fn changed_input_yields_different_key() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("input.txt"), b"version1").unwrap();

        let store = FakeStore::default();

        let opts1 = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo v1".to_string(),
            ],
            env_keys: vec![],
        };
        let outcome1 = run_memoized(cwd, &store, &opts1).await.unwrap();
        assert!(!outcome1.hit);

        // Mutate the input.
        std::fs::write(cwd.join("input.txt"), b"version2").unwrap();

        let opts2 = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo v1".to_string(),
            ],
            env_keys: vec![],
        };
        let outcome2 = run_memoized(cwd, &store, &opts2).await.unwrap();
        assert!(!outcome2.hit, "changed input must produce a miss");
        assert_ne!(outcome1.key, outcome2.key, "keys must differ");
    }

    // -----------------------------------------------------------------------
    // Audit Medium (cwd in key): identical inputs/command/env in DIFFERENT
    // working directories MUST yield DIFFERENT keys — otherwise a stale result
    // replays across workspaces. We create two cwds, each with a byte-identical
    // `input.txt`, and run the SAME command/env in both; the keys must differ.
    // A same-cwd repeat is also asserted stable (so the cwd fold is not noisy).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn different_cwd_yields_different_key() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        let cwd_a = dir_a.path();
        let cwd_b = dir_b.path();

        // Byte-identical inputs in both directories.
        std::fs::write(cwd_a.join("input.txt"), b"same-bytes").unwrap();
        std::fs::write(cwd_b.join("input.txt"), b"same-bytes").unwrap();

        // Identical inputs / command / env — only the cwd differs.
        let opts = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };

        let key_a = compute_key(cwd_a, &opts).await.unwrap();
        let key_b = compute_key(cwd_b, &opts).await.unwrap();
        assert_ne!(
            key_a, key_b,
            "identical inputs/command/env in different cwds must yield different keys"
        );

        // The cwd fold is deterministic: same cwd → same key (no false miss).
        let key_a2 = compute_key(cwd_a, &opts).await.unwrap();
        assert_eq!(key_a, key_a2, "same cwd must yield a stable key");
    }

    // A command ARGUMENT change (command[1..], NOT the tool at command[0]) must bust
    // the key. Every other command-key test varies command[0] (tool identity); this
    // pins that the rest of the argument vector is in the key too — otherwise
    // `mytool --flag=A` and `mytool --flag=B` would share a cache entry (a stale
    // replay of the wrong result).
    #[tokio::test]
    async fn command_arg_change_yields_different_key() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("input.txt"), b"same-bytes").unwrap();

        // Same tool (`true`) + identical inputs/cwd/env — ONLY the argument differs.
        let with_arg = |arg: &str| RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec!["true".to_string(), arg.to_string()],
            env_keys: vec![],
        };

        let key_a = compute_key(cwd, &with_arg("alpha")).await.unwrap();
        let key_b = compute_key(cwd, &with_arg("beta")).await.unwrap();
        assert_ne!(
            key_a, key_b,
            "changing a command ARGUMENT (not command[0]) must bust the memo key"
        );

        // Deterministic: the same argument vector yields the same key (no false miss).
        let key_a2 = compute_key(cwd, &with_arg("alpha")).await.unwrap();
        assert_eq!(
            key_a, key_a2,
            "same argument vector must yield a stable key"
        );
    }

    // -----------------------------------------------------------------------
    // Test: env_keys — listed var change → different key; unlisted → same key
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn env_keys_listed_var_affects_key() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("f.txt"), b"data").unwrap();

        let store = FakeStore::default();

        // Set a known env var.
        std::env::set_var("CLW_TEST_RUN_LISTED_VAR_A", "val1");
        let opts1 = RunOptions {
            inputs: vec!["f.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec!["CLW_TEST_RUN_LISTED_VAR_A".to_string()],
        };
        let o1 = run_memoized(cwd, &store, &opts1).await.unwrap();

        std::env::set_var("CLW_TEST_RUN_LISTED_VAR_A", "val2");
        let opts2 = RunOptions {
            inputs: vec!["f.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec!["CLW_TEST_RUN_LISTED_VAR_A".to_string()],
        };
        let o2 = run_memoized(cwd, &store, &opts2).await.unwrap();

        assert_ne!(o1.key, o2.key, "listed env var change must change key");
        assert!(!o2.hit, "changed listed env var must yield miss");
    }

    #[tokio::test]
    async fn env_keys_unlisted_var_does_not_affect_key() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("g.txt"), b"data").unwrap();

        let store = FakeStore::default();

        std::env::set_var("CLW_TEST_RUN_UNLISTED_VAR_B", "ignored1");
        let opts1 = RunOptions {
            inputs: vec!["g.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let o1 = run_memoized(cwd, &store, &opts1).await.unwrap();
        assert!(!o1.hit);

        std::env::set_var("CLW_TEST_RUN_UNLISTED_VAR_B", "ignored2");
        let opts2 = RunOptions {
            inputs: vec!["g.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let o2 = run_memoized(cwd, &store, &opts2).await.unwrap();

        assert_eq!(
            o1.key, o2.key,
            "unlisted env var change must not change key"
        );
        assert!(o2.hit, "unlisted env var change must yield hit");
    }

    // -----------------------------------------------------------------------
    // Test: env presence — an UNSET var vs a var SET TO "" must yield DIFFERENT
    // keys (M1.5: unset ≠ empty); unchanged presence yields the same key.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn env_unset_differs_from_empty() {
        // Unique var name to avoid races with other env-mutating tests
        // (this crate's env tests are not serialized; uniqueness is the guard).
        const VAR: &str = "CLW_TEST_PRESENCE_9f3a_M15";

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("p.txt"), b"data").unwrap();

        let opts = RunOptions {
            inputs: vec!["p.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![VAR.to_string()],
        };

        // Ensure a known starting state.
        std::env::remove_var(VAR);
        let key_unset = compute_key(cwd, &opts).await.unwrap();
        // Same UNSET state again → same key.
        let key_unset2 = compute_key(cwd, &opts).await.unwrap();

        // Now SET the var to the empty string.
        std::env::set_var(VAR, "");
        let key_empty = compute_key(cwd, &opts).await.unwrap();

        // Clean up the env var.
        std::env::remove_var(VAR);

        assert_eq!(
            key_unset, key_unset2,
            "unchanged (unset) presence must yield the same key"
        );
        assert_ne!(
            key_unset, key_empty,
            "UNSET var and a var SET TO \"\" must yield DIFFERENT keys (unset ≠ empty)"
        );
    }

    // -----------------------------------------------------------------------
    // Test: non-zero exit → outcome returned, nothing stored, second call still miss
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses /bin/sh (Windows port: M2)
    #[tokio::test]
    async fn nonzero_exit_not_cached() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("h.txt"), b"x").unwrap();

        let store = FakeStore::default();

        let opts = RunOptions {
            inputs: vec!["h.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "exit 42".to_string(),
            ],
            env_keys: vec![],
        };

        let o1 = run_memoized(cwd, &store, &opts).await.unwrap();
        assert!(!o1.hit);
        assert_eq!(o1.exit_code, 42);

        // AC must be empty for this key.
        let stored = AcTransport::get(&store, &o1.key).await.unwrap();
        assert!(stored.is_none(), "non-zero exit must not be cached");

        // Second call also a miss.
        let opts2 = RunOptions {
            inputs: vec!["h.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "exit 42".to_string(),
            ],
            env_keys: vec![],
        };
        let o2 = run_memoized(cwd, &store, &opts2).await.unwrap();
        assert!(!o2.hit, "non-zero exit: second call must also be a miss");
    }

    // -----------------------------------------------------------------------
    // Test: input as DIRECTORY picks up nested file changes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn directory_input_picks_up_nested_changes() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();

        // Create a subdirectory with files.
        std::fs::create_dir(cwd.join("subdir")).unwrap();
        std::fs::write(cwd.join("subdir/nested.txt"), b"original").unwrap();

        let store = FakeStore::default();

        let opts1 = RunOptions {
            inputs: vec!["subdir".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let o1 = run_memoized(cwd, &store, &opts1).await.unwrap();
        assert!(!o1.hit);

        // Mutate nested file.
        std::fs::write(cwd.join("subdir/nested.txt"), b"changed").unwrap();

        let opts2 = RunOptions {
            inputs: vec!["subdir".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let o2 = run_memoized(cwd, &store, &opts2).await.unwrap();
        assert!(!o2.hit, "nested file change must produce a miss");
        assert_ne!(o1.key, o2.key, "keys must differ after nested change");
    }

    // -----------------------------------------------------------------------
    // Test: empty command returns InvalidManifest error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn empty_command_returns_error() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let store = FakeStore::default();

        let opts = RunOptions {
            inputs: vec![],
            command: vec![],
            env_keys: vec![],
        };

        let err = run_memoized(cwd, &store, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(_)),
            "empty command must return InvalidManifest, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: missing input path returns Io(NotFound)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn missing_input_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let store = FakeStore::default();

        let opts = RunOptions {
            inputs: vec!["does_not_exist.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };

        let err = run_memoized(cwd, &store, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
            "missing input must return Io(NotFound), got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // FIX 3: a cache-miss command emitting MORE than the size cap is aborted with
    // TooLarge WHILE capturing — not buffered whole and then checked. We emit well
    // over CAS_BLOB_CAP_BYTES (5 MiB) of stdout; the read_capped ceiling must trip,
    // the child is killed, and nothing is memoized (the AC stays empty for the key).
    // -----------------------------------------------------------------------

    #[cfg(unix)] // uses /bin/sh + `yes`/`head` (Windows port: M2)
    #[tokio::test]
    async fn oversized_child_output_aborts_with_too_large() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();

        let store = FakeStore::default();

        // Emit ~6 MiB of stdout (> 5 MiB cap): 6 * 1024 * 1024 bytes of 'A' + newline.
        // `head -c` bounds the producer so the test terminates promptly even though
        // our reader aborts early.
        let bytes = 6 * 1024 * 1024;
        let cmd = format!("yes A | head -c {bytes}");
        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec!["/bin/sh".to_string(), "-c".to_string(), cmd],
            env_keys: vec![],
        };

        let err = run_memoized(cwd, &store, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::TooLarge { cap, .. } if cap == CAS_BLOB_CAP_BYTES as u64),
            "oversized child output must abort as TooLarge, got: {err:?}"
        );

        // The key was computed but nothing must have been memoized.
        let key = compute_key(cwd, &opts).await.unwrap();
        let stored = AcTransport::get(&store, &key).await.unwrap();
        assert!(stored.is_none(), "aborted run must not be memoized");
    }

    // -----------------------------------------------------------------------
    // FIX 3 regression: output exactly AT the cap is still captured and memoized.
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn child_output_within_cap_is_captured() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = FakeStore::default();

        // 1 KiB of output — well under the cap.
        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "yes A | head -c 1024".to_string(),
            ],
            env_keys: vec![],
        };
        let outcome = run_memoized(cwd, &store, &opts).await.unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.stdout.len(), 1024, "stdout captured in full");
        // Memoized on success.
        let stored = AcTransport::get(&store, &outcome.key).await.unwrap();
        assert!(stored.is_some(), "within-cap success must be memoized");
    }

    // -----------------------------------------------------------------------
    // Pass-2 audit: the cap BOUNDARY. The capture predicate is `projected > cap`
    // (see `read_capped`), so output of EXACTLY the cap must still be captured +
    // memoized, while cap+1 must abort as TooLarge. The prior coverage only had a
    // well-under-cap case (1 KiB) and a far-over case (~6 MiB), never touching the
    // boundary byte where an off-by-one (`>=` vs `>`) would flip behavior. These two
    // siblings pin both sides of the boundary by driving the REAL `run_memoized`.
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn child_output_exactly_at_cap_is_captured() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = FakeStore::default();

        // Emit EXACTLY CAS_BLOB_CAP_BYTES of stdout — the largest output that is
        // still within the cap (`projected > cap` is false at projected == cap).
        let cap = CAS_BLOB_CAP_BYTES;
        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                format!("yes A | head -c {cap}"),
            ],
            env_keys: vec![],
        };
        let outcome = run_memoized(cwd, &store, &opts).await.unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(
            outcome.stdout.len(),
            cap,
            "output of exactly the cap must be captured in full"
        );
        let stored = AcTransport::get(&store, &outcome.key).await.unwrap();
        assert!(stored.is_some(), "exactly-at-cap success must be memoized");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_output_one_over_cap_aborts_as_too_large() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = FakeStore::default();

        // Emit EXACTLY cap + 1 byte — the smallest output that exceeds the cap and
        // must abort (`projected > cap` becomes true at the boundary + 1).
        let over = CAS_BLOB_CAP_BYTES + 1;
        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                format!("yes A | head -c {over}"),
            ],
            env_keys: vec![],
        };
        let err = run_memoized(cwd, &store, &opts).await.unwrap_err();
        assert!(
            matches!(err, ClwError::TooLarge { cap, .. } if cap == CAS_BLOB_CAP_BYTES as u64),
            "cap+1 output must abort as TooLarge, got: {err:?}"
        );
        // A cap-blow aborts before memoization: nothing is stored.
        let key = compute_key(cwd, &opts).await.unwrap();
        let stored = AcTransport::get(&store, &key).await.unwrap();
        assert!(stored.is_none(), "an over-cap run must NOT be memoized");
    }

    // -----------------------------------------------------------------------
    // CRIT regression (pass-2 audit 2026-06-16): the dual-stream DEADLOCK. A
    // command that floods BOTH stdout and stderr UNBOUNDED is the exact case a
    // `join!`-based capture hangs on forever (one reader hits the cap and stops
    // draining → the child blocks writing to that pipe → the other reader starves,
    // never EOFs). The fix kills + returns on the first cap-blow without awaiting
    // the other stream. We wrap in a real timeout so a regression FAILS LOUDLY
    // (TooLarge returned) instead of hanging the whole test binary.
    // -----------------------------------------------------------------------

    #[cfg(unix)] // /bin/sh + `yes`
    #[tokio::test]
    async fn dual_stream_flood_aborts_without_deadlock() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = FakeStore::default();

        // Both streams blast forever; neither terminates on its own. With the old
        // join! capture this hangs; the deadlock-safe path must return TooLarge.
        // `yes` on stdout (foreground) + a backgrounded `yes` on stderr — the
        // backgrounded one becomes an orphan on kill and keeps the stderr pipe
        // open, exercising the "don't await the surviving stream" requirement.
        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "yes B 1>&2 & yes A".to_string(),
            ],
            env_keys: vec![],
        };

        let res = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            run_memoized(cwd, &store, &opts),
        )
        .await;

        let inner = res.expect("run_memoized must RETURN (deadlock regression), not time out");
        let err = inner.unwrap_err();
        assert!(
            matches!(err, ClwError::TooLarge { cap, .. } if cap == CAS_BLOB_CAP_BYTES as u64),
            "dual-stream flood must abort as TooLarge, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: tool identity (command[0] content) is folded into the key.
    // Two different executable scripts (same args) → different keys;
    // identical tool → same key. Prevents stale replay after a toolchain
    // upgrade.
    // -----------------------------------------------------------------------

    #[cfg(unix)] // shell-script tooling + mode bits are unix-only (Windows port: M2)
    #[tokio::test]
    async fn tool_identity_change_yields_different_key() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("input.txt"), b"data").unwrap();

        // Create two tiny shell scripts with DIFFERENT contents.
        let tool_a = cwd.join("tool_a.sh");
        let tool_b = cwd.join("tool_b.sh");
        std::fs::write(&tool_a, b"#!/bin/sh\necho A\n").unwrap();
        std::fs::write(&tool_b, b"#!/bin/sh\necho B\n").unwrap();
        std::fs::set_permissions(&tool_a, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&tool_b, std::fs::Permissions::from_mode(0o755)).unwrap();

        let tool_a_str = tool_a.to_str().unwrap().to_string();
        let tool_b_str = tool_b.to_str().unwrap().to_string();

        // Same args, different tool[0] (absolute paths).
        let opts_a = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec![tool_a_str.clone(), "arg".to_string()],
            env_keys: vec![],
        };
        let opts_b = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec![tool_b_str, "arg".to_string()],
            env_keys: vec![],
        };

        let key_a = compute_key(cwd, &opts_a).await.unwrap();
        let key_b = compute_key(cwd, &opts_b).await.unwrap();
        assert_ne!(
            key_a, key_b,
            "different tool[0] content must yield a different key"
        );

        // Identical tool (and everything else) → same key.
        let opts_a2 = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec![tool_a_str, "arg".to_string()],
            env_keys: vec![],
        };
        let key_a2 = compute_key(cwd, &opts_a2).await.unwrap();
        assert_eq!(key_a, key_a2, "identical tool must yield the same key");
    }

    // -----------------------------------------------------------------------
    // Test: mutating the tool's content in place (same path) changes the key.
    // This is the toolchain-upgrade scenario M3 guards against.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // FIX 1: an empty input subdir must contribute its path to the key, so a
    // build that depends on the dir's presence/absence does not get a false hit
    // when it appears or disappears. Same inputs, empty subdir present vs absent
    // → different keys.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn empty_input_dir_presence_changes_key() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        // A declared directory input with one file plus an empty subdir.
        std::fs::create_dir(cwd.join("d")).unwrap();
        std::fs::write(cwd.join("d/file.txt"), b"data").unwrap();

        let opts = RunOptions {
            inputs: vec!["d".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };

        // Without the empty subdir.
        let key_absent = compute_key(cwd, &opts).await.unwrap();

        // Now add an empty subdir (no content change to any file).
        std::fs::create_dir(cwd.join("d/empty")).unwrap();
        let key_present = compute_key(cwd, &opts).await.unwrap();

        assert_ne!(
            key_absent, key_present,
            "an empty input subdir appearing must change the key (no false hit)"
        );
    }

    // -----------------------------------------------------------------------
    // FIX 1 (directly-declared variant): an input directory that is ITSELF empty
    // must still be path-bearing — adding/removing it changes the key. `walk`
    // skips its own root, so this exercises the `walked.is_empty()` anchor.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn directly_declared_empty_dir_input_is_keyed() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("a.txt"), b"x").unwrap();
        std::fs::create_dir(cwd.join("e")).unwrap();

        let with_dir = RunOptions {
            inputs: vec!["a.txt".to_string(), "e".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let without_dir = RunOptions {
            inputs: vec!["a.txt".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };

        let key_with = compute_key(cwd, &with_dir).await.unwrap();
        let key_without = compute_key(cwd, &without_dir).await.unwrap();
        assert_ne!(
            key_with, key_without,
            "an empty directory declared as an input must contribute to the key"
        );
    }

    // -----------------------------------------------------------------------
    // FIX 2: toggling an input file's exec bit (same content) must change the
    // key — the command's behavior depends on the mode even though bytes are
    // identical.
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn input_file_mode_change_yields_different_key() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let f = cwd.join("script");
        std::fs::write(&f, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();

        let opts = RunOptions {
            inputs: vec!["script".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let key_nonexec = compute_key(cwd, &opts).await.unwrap();

        // chmod +x — content unchanged, only the exec bit flips.
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        let key_exec = compute_key(cwd, &opts).await.unwrap();

        assert_ne!(
            key_nonexec, key_exec,
            "toggling the exec bit on an input file must change the key"
        );
    }

    // Same as above but for a file discovered INSIDE a directory input, so the
    // dir-walk path also folds in mode.
    #[cfg(unix)]
    #[tokio::test]
    async fn input_dir_file_mode_change_yields_different_key() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::create_dir(cwd.join("bin")).unwrap();
        let f = cwd.join("bin/run");
        std::fs::write(&f, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();

        let opts = RunOptions {
            inputs: vec!["bin".to_string()],
            command: vec!["true".to_string()],
            env_keys: vec![],
        };
        let before = compute_key(cwd, &opts).await.unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        let after = compute_key(cwd, &opts).await.unwrap();
        assert_ne!(
            before, after,
            "exec-bit change on a file inside a dir input must change the key"
        );
    }

    // -----------------------------------------------------------------------
    // FIX 3: a path with `\` separators must normalize to the same key bytes as
    // its `/` form, so the key is separator-stable (cross-platform). On unix
    // `MAIN_SEPARATOR` is `/`, so we test `norm_sep` directly to keep the
    // behavior assertion platform-independent.
    // -----------------------------------------------------------------------

    #[test]
    fn norm_sep_normalizes_to_forward_slash() {
        // On Windows the OS separator is collapsed to `/`.
        #[cfg(windows)]
        {
            assert_eq!(norm_sep(r"a\b\c"), "a/b/c");
            assert_eq!(norm_sep("a/b/c"), "a/b/c");
            // Mixed separators land on the same canonical form.
            assert_eq!(norm_sep(r"a\b/c"), "a/b/c");
        }
        // On unix `/` is already canonical and a literal backslash is a valid
        // filename character, so it is preserved untouched.
        #[cfg(not(windows))]
        {
            // Pass-2 audit: the prior assertions only exercised the unix no-op
            // branch with `/`-already / `\`-preserved inputs. We KEEP those (they
            // pin the unix contract) AND add assertions that prove the function's
            // FULL contract as it actually runs on this platform — that `norm_sep`
            // is, on unix, the identity over the path string (it neither rewrites
            // `/` nor strips `\`, and is byte-for-byte stable / idempotent).

            // A `/`-form path is returned unchanged (the dir-walk's nested form).
            assert_eq!(norm_sep("a/b/c"), "a/b/c");
            // A literal backslash is a valid unix filename byte and is PRESERVED —
            // it must NOT be collapsed to `/` (that would corrupt the run key on
            // unix). This is the load-bearing platform-specific branch.
            assert_eq!(norm_sep(r"a\b"), r"a\b");
            // Mixed separators on unix keep the `\` verbatim and the `/` verbatim:
            // exactly the input, proving no rewrite happens on this platform.
            assert_eq!(norm_sep(r"a\b/c"), r"a\b/c");
            // Identity over an arbitrary string with no separator of either kind.
            assert_eq!(norm_sep("plain"), "plain");
            assert_eq!(norm_sep(""), "");
            // The key-stability contract: feeding the output back in is a fixpoint
            // (idempotent), so the value that feeds the run key is stable.
            let once = norm_sep(r"x\y/z");
            assert_eq!(norm_sep(&once), once, "norm_sep must be idempotent on unix");
            // And the result is byte-for-byte the input on unix (the no-op branch),
            // which is the property `MAIN_SEPARATOR == '/'` guarantees here.
            let s = r"left\mid/right.txt";
            assert_eq!(norm_sep(s).as_bytes(), s.as_bytes());
        }
    }

    #[cfg(unix)] // shell-script tooling + mode bits are unix-only (Windows port: M2)
    #[tokio::test]
    async fn tool_content_mutation_yields_different_key() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("input.txt"), b"data").unwrap();

        let tool = cwd.join("tool.sh");
        std::fs::write(&tool, b"#!/bin/sh\necho v1\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tool_str = tool.to_str().unwrap().to_string();

        let opts = RunOptions {
            inputs: vec!["input.txt".to_string()],
            command: vec![tool_str, "arg".to_string()],
            env_keys: vec![],
        };
        let key_before = compute_key(cwd, &opts).await.unwrap();

        // Upgrade the tool in place (same path, new content).
        std::fs::write(&tool, b"#!/bin/sh\necho v2\n").unwrap();
        let key_after = compute_key(cwd, &opts).await.unwrap();

        assert_ne!(
            key_before, key_after,
            "in-place tool upgrade must invalidate the key (no stale replay)"
        );
    }

    // -----------------------------------------------------------------------
    // H2 (backgrounded-grandchild hang): the direct child `sh` backgrounds a LONG
    // `sleep` that INHERITS the stdout/stderr pipe write ends, then exits 0. The old
    // drain loop waited for BOTH pipes to EOF — which cannot happen until the orphan
    // `sleep` (300s) dies — so run_memoized wedged forever with no timeout. The fix
    // observes the DIRECT child's exit in the select and applies a bounded post-exit
    // drain grace, so run_memoized must RETURN promptly. We wrap it in a wall-clock
    // timeout well under the sleep so a regression FAILS LOUDLY (times out) instead
    // of hanging the whole test binary.
    // -----------------------------------------------------------------------

    #[cfg(unix)] // /bin/sh + backgrounded `sleep`
    #[tokio::test]
    async fn backgrounded_grandchild_does_not_wedge_run() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = FakeStore::default();

        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                // Background a long sleep (inherits the pipes) then exit 0.
                "sleep 300 & exit 0".to_string(),
            ],
            env_keys: vec![],
        };

        let res = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            run_memoized(cwd, &store, &opts),
        )
        .await;

        let outcome = res
            .expect(
                "run_memoized must RETURN despite a backgrounded grandchild holding the \
                 pipe (H2 hang regression), not time out",
            )
            .expect("run must succeed");
        assert_eq!(outcome.exit_code, 0, "the direct child exited 0");
        assert!(
            outcome.stdout.is_empty(),
            "the child wrote no stdout of its own"
        );
    }

    // -----------------------------------------------------------------------
    // FIX #2: the post-success best-effort store phase must NOT materially delay an
    // already-successful run. A transport that black-holes AFTER the child exits 0
    // used to hang `clw run` for up to 3×op-deadline (~15 min) because the three
    // stores (stdout CAS, stderr CAS, AC record) ran SERIALLY, each awaited. The fix
    // runs them CONCURRENTLY and caps the whole phase with STORE_PHASE_DEADLINE (30 s).
    //
    // Observable coverage & its limit: we drive the REAL `run_memoized` against a
    // transport whose every `put` sleeps, and assert (in real wall-clock) that the
    // successful run returns in ~ONE store's time, not the sum of three — the direct
    // signature of concurrency. The 30 s deadline itself is NOT exercised as a timer
    // here: doing so in real time means a ~30 s test, and tokio's paused virtual
    // clock (`start_paused`) needs the `test-util` feature, whose enabling lives in
    // Cargo.toml — outside this WP's owner-file set. The timeout wrapper is therefore
    // verified by construction (see `STORE_PHASE_DEADLINE` in the source); this test
    // pins the concurrency, which is what makes the phase bounded by one op.
    // -----------------------------------------------------------------------

    /// A transport whose every `put` (CAS and AC) blocks for `put_delay` before
    /// delegating to an in-memory `FakeStore` — a stand-in for a slow/black-hole
    /// backend on writes AFTER the child already succeeded. `get`/`exists` stay fast
    /// + empty so `run_memoized` takes the cache-MISS path and executes the child.
    #[derive(Clone)]
    struct SlowPutStore {
        inner: FakeStore,
        put_delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl CasTransport for SlowPutStore {
        async fn exists(&self, digest: &Digest) -> Result<bool> {
            CasTransport::exists(&self.inner, digest).await
        }
        async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
            CasTransport::get(&self.inner, digest).await
        }
        async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
            tokio::time::sleep(self.put_delay).await;
            CasTransport::put(&self.inner, digest, bytes).await
        }
        async fn delete(&self, digest: &Digest) -> Result<()> {
            CasTransport::delete(&self.inner, digest).await
        }
        async fn list_blobs(&self) -> Result<Vec<Digest>> {
            CasTransport::list_blobs(&self.inner).await
        }
    }

    #[async_trait::async_trait]
    impl AcTransport for SlowPutStore {
        async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
            AcTransport::get(&self.inner, key).await
        }
        async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
            tokio::time::sleep(self.put_delay).await;
            AcTransport::put(&self.inner, key, value).await
        }
        async fn delete(&self, key: &Digest) -> Result<()> {
            AcTransport::delete(&self.inner, key).await
        }
        async fn list_refs(&self) -> Result<Vec<Digest>> {
            AcTransport::list_refs(&self.inner).await
        }
    }

    // FIX #2 — CONCURRENT + BOUNDED-BY-ONE-OP. Each of the three post-success stores
    // (stdout CAS, stderr CAS, AC record) blocks for 3 s. Run CONCURRENTLY the phase
    // is bounded by the slowest SINGLE store (~3 s); run SERIALLY (the pre-fix code)
    // it takes ~9 s (3×3 s). We drive the REAL `run_memoized` and assert the
    // successful run returns in well under 3× the per-store sleep, and still yields
    // exit 0 with its output intact + memoized. RED-on-revert: restoring the three
    // serial awaited stores makes the phase ~9 s → the `< 6 s` wall-clock bound fails.
    #[cfg(unix)]
    #[tokio::test]
    async fn store_phase_is_concurrent_not_serial_after_success() {
        const PUT_DELAY: std::time::Duration = std::time::Duration::from_secs(3);

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = SlowPutStore {
            inner: FakeStore::default(),
            put_delay: PUT_DELAY,
        };

        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "printf hi".to_string(),
            ],
            env_keys: vec![],
        };

        let start = std::time::Instant::now();
        let outcome = run_memoized(cwd, &store, &opts).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(outcome.exit_code, 0, "the child succeeded");
        assert_eq!(
            outcome.stdout, b"hi",
            "the child's output is returned intact"
        );
        // Concurrent ⇒ ~1×PUT_DELAY (+ trivial child); serial ⇒ ~3×PUT_DELAY. A bound
        // at 2×PUT_DELAY cleanly separates the two while leaving generous CI slack.
        assert!(
            elapsed < 2 * PUT_DELAY,
            "the best-effort store phase must run concurrently (~{PUT_DELAY:?}), \
             not ~3× serial; took {elapsed:?}"
        );
        // The three stores completed (well within the 30 s deadline) → memoized.
        let stored = AcTransport::get(&store, &outcome.key).await.unwrap();
        assert!(
            stored.is_some(),
            "a within-deadline success must be memoized (all three stores completed)"
        );
    }

    // -----------------------------------------------------------------------
    // H3 (credential leak): the CAS bearer token / redemption ticket must be
    // STRIPPED from the untrusted child's environment before spawn, so arbitrary CI
    // code cannot exfiltrate it. We set the secret vars (a recognizable value) plus a
    // NON-secret CLW_* marker in the parent env, run `sh -c env`, and assert the
    // secret is ABSENT from the child's env dump while the non-secret marker survives
    // (proving we stripped the secrets specifically, not env_clear()d wholesale).
    // RED-on-revert: dropping the env_remove loop lets `CLW_TOKEN` reach the child →
    // the secret appears in the dump → the absence assertion fails.
    // -----------------------------------------------------------------------

    #[cfg(unix)] // /bin/sh + `env`
    #[tokio::test]
    async fn secret_env_is_stripped_from_child() {
        const SECRET: &str = "clw_h3_secret_value_9f3a_DEADBEEF";
        const NONSECRET: &str = "clw_h3_endpoint_marker_9f3a";

        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("in.txt"), b"x").unwrap();
        let store = FakeStore::default();

        // Seed every credential-bearing var with the same recognizable secret, plus a
        // non-secret CLW_* marker that must be preserved.
        for var in SECRET_ENV_VARS {
            std::env::set_var(var, SECRET);
        }
        std::env::set_var("CLW_ENDPOINT", NONSECRET);

        let opts = RunOptions {
            inputs: vec!["in.txt".to_string()],
            command: vec!["/bin/sh".to_string(), "-c".to_string(), "env".to_string()],
            env_keys: vec![],
        };
        let outcome = run_memoized(cwd, &store, &opts).await.unwrap();

        // Restore the process-global env before asserting (a failed assert must not
        // leave the secret set for other tests).
        for var in SECRET_ENV_VARS {
            std::env::remove_var(var);
        }
        std::env::remove_var("CLW_ENDPOINT");

        let dump = String::from_utf8_lossy(&outcome.stdout);
        assert!(
            !dump.contains(SECRET),
            "credential-bearing env vars must be STRIPPED from the untrusted child (H3); \
             the child's env dump unexpectedly contained the secret"
        );
        assert!(
            dump.contains(NONSECRET),
            "a non-secret CLW_* var must still reach the child (no wholesale env_clear)"
        );
    }
}
