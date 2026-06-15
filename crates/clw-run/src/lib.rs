//! clw-run — AC-backed command memoization.
//!
//! The core primitive: "never run the same deterministic command twice."
//! Keys are computed from input file/directory digests, command vector, and
//! env vars. Results (stdout + stderr blobs) are stored in the CAS; the AC
//! maps the run key → RunRecord.

pub use clw_types as types;

use clw_manifest::walk;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result, CAS_BLOB_CAP_BYTES};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Public constants + types (frozen anchor)
// ---------------------------------------------------------------------------

pub const RUN_KEY_DOMAIN: &str = "clw/run/v3/";

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
/// Returns `Err(ClwError::Io(...))` if any input path is missing, or
/// `Err(ClwError::InvalidManifest(...))` for walk errors.
async fn compute_key(cwd: &Path, opts: &RunOptions) -> Result<Digest> {
    // --- inputs ---
    let mut input_pairs: Vec<[String; 2]> = Vec::new();

    for rel in &opts.inputs {
        let abs = cwd.join(rel);
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
            let target_str = target.to_string_lossy().into_owned();
            input_pairs.push([rel.clone(), format!("symlink:{}", target_str)]);
        } else if meta.file_type().is_dir() {
            // Directory: walk it, digest each file, collect entries sorted by rel path.
            let walked = walk(&abs)?;
            for entry in walked {
                match entry {
                    clw_manifest::WalkedEntry::File(f) => {
                        let content = tokio::fs::read(&f.abs).await?;
                        let digest = Digest::of_bytes(&content);
                        let entry_rel = format!("{}/{}", rel, f.rel);
                        input_pairs.push([entry_rel, digest.to_hex()]);
                    }
                    clw_manifest::WalkedEntry::Symlink {
                        rel: sym_rel,
                        target,
                    } => {
                        let entry_rel = format!("{}/{}", rel, sym_rel);
                        input_pairs.push([entry_rel, format!("symlink:{}", target)]);
                    }
                    clw_manifest::WalkedEntry::EmptyDir { .. } => {
                        // Empty dirs contribute no content; skip.
                    }
                }
            }
        } else {
            // Regular file: digest bytes.
            let content = tokio::fs::read(&abs).await?;
            let digest = Digest::of_bytes(&content);
            input_pairs.push([rel.clone(), digest.to_hex()]);
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

    // --- canonical JSON ---
    let payload = KeyPayload {
        inputs: input_pairs,
        command: opts.command.clone(),
        env: env_pairs,
        tool,
    };
    let json_bytes = serde_json::to_vec(&payload)?;

    // --- BLAKE3(domain || json) ---
    let mut hasher = blake3::Hasher::new();
    hasher.update(RUN_KEY_DOMAIN.as_bytes());
    hasher.update(&json_bytes);
    Ok(Digest(*hasher.finalize().as_bytes()))
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

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
    let child = tokio::process::Command::new(&opts.command[0])
        .args(&opts.command[1..])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let output = child.wait_with_output().await?;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = output.stdout;
    let stderr = output.stderr;

    // --- Memoize only on success + within size cap ---
    let within_cap = stdout.len() <= CAS_BLOB_CAP_BYTES && stderr.len() <= CAS_BLOB_CAP_BYTES;

    if exit_code == 0 && within_cap {
        let stdout_digest = Digest::of_bytes(&stdout);
        let stderr_digest = Digest::of_bytes(&stderr);

        // Best-effort: ignore errors storing (the outcome is still valid).
        let _ = CasTransport::put(client, &stdout_digest, stdout.clone()).await;
        let _ = CasTransport::put(client, &stderr_digest, stderr.clone()).await;

        let record = RunRecord {
            exit_code,
            stdout: stdout_digest,
            stderr: stderr_digest,
        };
        if let Ok(record_bytes) = serde_json::to_vec(&record) {
            let _ = AcTransport::put(client, &key, record_bytes).await;
        }
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
}
