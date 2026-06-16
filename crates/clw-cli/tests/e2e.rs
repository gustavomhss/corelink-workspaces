//! Binary-level end-to-end tests for the `clw` CLI.
//!
//! Every test spawns the REAL `clw` binary via `assert_cmd::Command::cargo_bin`
//! and drives it against a stateful wiremock CoreLink mock server (in-memory
//! CAS + AC maps, Bearer auth enforced).  No request ever reaches the real
//! `corelink-api.humangr.com`.
//!
//! Test isolation: each test creates its own `TempDir` trees and unique
//! workspace names, so tests can run in parallel safely.
//!
//! UNIX-ONLY: this binary-level suite builds its fixtures with unix symlinks +
//! `chmod` and asserts unix file modes (0o600/0o755), so it is gated to unix.
//! Windows binary-level e2e coverage lands with the M2.4 cross-platform CI lane
//! (it needs Windows symlink/ACL-aware fixtures, not just a compile fix). The
//! `#![cfg(unix)]` makes the crate compile to nothing on Windows so a future
//! `cargo test --target *-windows-*` build does not break on the unix imports.
#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

// ---------------------------------------------------------------------------
// Shared mock-server helpers (identical pattern to clw-integration-tests)
// ---------------------------------------------------------------------------

const TEST_TENANT: &str = "test";
const TEST_PAT: &str = "test-pat";

type Store = Arc<Mutex<HashMap<String, Vec<u8>>>>;

fn last_segment(url: &wiremock::http::Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("")
        .to_string()
}

#[derive(Clone)]
struct CasResponder {
    store: Store,
    auth_ok: bool,
}

impl Respond for CasResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let auth = request
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !self.auth_ok || auth != format!("Bearer {TEST_PAT}") {
            return ResponseTemplate::new(401);
        }

        let key = last_segment(&request.url);
        let m = request.method.as_str();

        if m == "PUT" {
            self.store.lock().unwrap().insert(key, request.body.clone());
            return ResponseTemplate::new(201);
        }
        if m == "HEAD" || m == "GET" {
            let guard = self.store.lock().unwrap();
            if let Some(data) = guard.get(&key) {
                if m == "HEAD" {
                    return ResponseTemplate::new(200);
                }
                return ResponseTemplate::new(200).set_body_bytes(data.clone());
            } else {
                return ResponseTemplate::new(404);
            }
        }
        if m == "DELETE" {
            // Idempotent: 204 whether or not the key was present.
            self.store.lock().unwrap().remove(&key);
            return ResponseTemplate::new(204);
        }
        ResponseTemplate::new(405)
    }
}

#[derive(Clone)]
struct AcResponder {
    store: Store,
    auth_ok: bool,
}

impl Respond for AcResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let auth = request
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !self.auth_ok || auth != format!("Bearer {TEST_PAT}") {
            return ResponseTemplate::new(401);
        }

        let key = last_segment(&request.url);
        let m = request.method.as_str();

        if m == "PUT" {
            self.store.lock().unwrap().insert(key, request.body.clone());
            return ResponseTemplate::new(201);
        }
        if m == "GET" {
            let guard = self.store.lock().unwrap();
            if let Some(data) = guard.get(&key) {
                return ResponseTemplate::new(200).set_body_bytes(data.clone());
            } else {
                return ResponseTemplate::new(404);
            }
        }
        if m == "DELETE" {
            // Idempotent: 204 whether or not the key was present.
            self.store.lock().unwrap().remove(&key);
            return ResponseTemplate::new(204);
        }
        ResponseTemplate::new(405)
    }
}

/// `GET /v1/cas/{tenant}` (no trailing digest) → CAS blob listing page in the
/// ratified D-8 envelope `{"blobs":[{"hash":"<64hex>"}, …], "next_cursor":null}`.
/// Enumerates the live CAS store keys as a single full page. `?limit`/`?cursor`
/// are accepted and ignored (a single page is sufficient for the suite). Auth is
/// enforced exactly like the per-digest CAS routes.
#[derive(Clone)]
struct CasListResponder {
    store: Store,
    auth_ok: bool,
}

impl Respond for CasListResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let auth = request
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !self.auth_ok || auth != format!("Bearer {TEST_PAT}") {
            return ResponseTemplate::new(401);
        }
        let blobs: Vec<_> = self
            .store
            .lock()
            .unwrap()
            .keys()
            .map(|h| json!({ "hash": h }))
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "blobs": blobs,
            "next_cursor": serde_json::Value::Null,
        }))
    }
}

/// `GET /v1/ac/{tenant}` (no trailing digest) → AC ref listing page in the
/// ratified D-7 envelope `{"refs":[{"ref_key":"<64hex>"}, …], "next_cursor":null}`.
/// Enumerates the live AC store keys as a single full page. `?limit`/`?cursor`
/// are accepted and ignored. Auth is enforced exactly like the per-digest routes.
#[derive(Clone)]
struct AcListResponder {
    store: Store,
    auth_ok: bool,
}

impl Respond for AcListResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let auth = request
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !self.auth_ok || auth != format!("Bearer {TEST_PAT}") {
            return ResponseTemplate::new(401);
        }
        let refs: Vec<_> = self
            .store
            .lock()
            .unwrap()
            .keys()
            .map(|k| json!({ "ref_key": k }))
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "refs": refs,
            "next_cursor": serde_json::Value::Null,
        }))
    }
}

/// Mount stateful CAS + AC mocks on `server`. Returns `(cas_store, ac_store)`.
async fn mount_corelink(server: &MockServer, auth_ok: bool) -> (Store, Store) {
    let cas_store: Store = Arc::new(Mutex::new(HashMap::new()));
    let ac_store: Store = Arc::new(Mutex::new(HashMap::new()));

    // Per-digest routes: tenant segment FOLLOWED BY a 64-hex digest (anchored to
    // end so the trailing `$` keeps these from being matched by anything broader).
    let cas_pattern = format!("/v1/cas/{TEST_TENANT}/[a-f0-9]{{64}}$");
    let ac_pattern = format!("/v1/ac/{TEST_TENANT}/[a-f0-9]{{64}}$");
    // LIST routes: tenant segment with NO `/{hex}` after it (anchored with `$` so
    // it does NOT also match the per-digest routes above). wiremock matches the
    // path only, so `?limit`/`?cursor` query strings don't affect the match.
    let cas_list_pattern = format!("/v1/cas/{TEST_TENANT}$");
    let ac_list_pattern = format!("/v1/ac/{TEST_TENANT}$");

    Mock::given(method("PUT"))
        .and(path_regex(&cas_pattern))
        .respond_with(CasResponder {
            store: Arc::clone(&cas_store),
            auth_ok,
        })
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(&cas_pattern))
        .respond_with(CasResponder {
            store: Arc::clone(&cas_store),
            auth_ok,
        })
        .mount(server)
        .await;
    Mock::given(method("HEAD"))
        .and(path_regex(&cas_pattern))
        .respond_with(CasResponder {
            store: Arc::clone(&cas_store),
            auth_ok,
        })
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(&ac_pattern))
        .respond_with(AcResponder {
            store: Arc::clone(&ac_store),
            auth_ok,
        })
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(&ac_pattern))
        .respond_with(AcResponder {
            store: Arc::clone(&ac_store),
            auth_ok,
        })
        .mount(server)
        .await;

    // DELETE per-digest routes (prune/erase/rm depend on these). Idempotent 204.
    Mock::given(method("DELETE"))
        .and(path_regex(&cas_pattern))
        .respond_with(CasResponder {
            store: Arc::clone(&cas_store),
            auth_ok,
        })
        .mount(server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(&ac_pattern))
        .respond_with(AcResponder {
            store: Arc::clone(&ac_store),
            auth_ok,
        })
        .mount(server)
        .await;

    // LIST routes (ls --all / prune / erase depend on these). Anchored so they do
    // NOT shadow — and are not shadowed by — the per-digest routes above.
    Mock::given(method("GET"))
        .and(path_regex(&cas_list_pattern))
        .respond_with(CasListResponder {
            store: Arc::clone(&cas_store),
            auth_ok,
        })
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(&ac_list_pattern))
        .respond_with(AcListResponder {
            store: Arc::clone(&ac_store),
            auth_ok,
        })
        .mount(server)
        .await;

    (cas_store, ac_store)
}

// ---------------------------------------------------------------------------
// Source tree builder
// ---------------------------------------------------------------------------

/// Build a multi-chunk, multi-type workspace tree under `root`:
/// - `small.txt` — tiny file
/// - `nested/data.bin` — >1 MiB pseudo-random (forces multi-chunk)
/// - `exec.sh` — executable (0o755)
/// - `link.txt` → symlink to `small.txt`
/// - `empty_subdir/` — empty directory
fn build_source_tree(root: &std::path::Path) {
    use std::fs;

    fs::write(root.join("small.txt"), b"hello e2e test").unwrap();

    // Nested directory with a large file
    fs::create_dir(root.join("nested")).unwrap();
    let big: Vec<u8> = (0u32..=(1024 * 1024 + 512 * 1024))
        .map(|i| (i.wrapping_mul(0x9e37_79b9u32)) as u8)
        .collect();
    fs::write(root.join("nested/data.bin"), &big).unwrap();

    // Executable
    fs::write(root.join("exec.sh"), b"#!/bin/sh\necho hello\n").unwrap();
    let mut perms = fs::metadata(root.join("exec.sh")).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(root.join("exec.sh"), perms).unwrap();

    // Symlink
    std::os::unix::fs::symlink("small.txt", root.join("link.txt")).unwrap();

    // Empty directory
    fs::create_dir(root.join("empty_subdir")).unwrap();
}

// ---------------------------------------------------------------------------
// Helper: spawn `clw` with common env vars
// ---------------------------------------------------------------------------

fn clw_cmd(endpoint: &str, cache_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("clw").unwrap();
    cmd.env("CLW_ENDPOINT", endpoint)
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache_dir.to_str().unwrap());
    cmd
}

/// The 64-hex AC store key for a workspace `name` — the exact `ref_key` the binary
/// derives (`BLAKE3(domain‖name)`), so a test can assert ref presence/absence in
/// `ac_store` by the same key the real `rm`/`snapshot` path writes/deletes under.
fn ref_key_hex(name: &str) -> String {
    clw_types::ref_key(name).to_hex()
}

/// Snapshot a tiny tree under a fresh temp dir into workspace `name`, asserting
/// success. Returns the `TempDir` (kept alive by the caller) so the source stays
/// on disk for the duration of the test.
fn snapshot_tiny(server_uri: &str, cache: &std::path::Path, name: &str) -> TempDir {
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), format!("payload for {name}")).unwrap();
    clw_cmd(server_uri, cache)
        .args(["snapshot", src.path().to_str().unwrap(), "--name", name])
        .assert()
        .success();
    src
}

// ---------------------------------------------------------------------------
// Test 1: roundtrip_bit_identical
// ---------------------------------------------------------------------------

#[tokio::test]
async fn roundtrip_bit_identical() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let src_tmp = TempDir::new().unwrap();
    build_source_tree(src_tmp.path());

    let cache_snap = TempDir::new().unwrap();
    let cache_hyd = TempDir::new().unwrap();

    // --- snapshot ---
    clw_cmd(&server.uri(), cache_snap.path())
        .args([
            "snapshot",
            src_tmp.path().to_str().unwrap(),
            "--name",
            "e2e-roundtrip",
        ])
        .assert()
        .success();

    // --- hydrate with FRESH empty cache ---
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");

    clw_cmd(&server.uri(), cache_hyd.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", "e2e-roundtrip"])
        .assert()
        .success();

    // --- recursive comparison ---

    // small.txt — byte-identical
    let small_src = std::fs::read(src_tmp.path().join("small.txt")).unwrap();
    let small_dst = std::fs::read(dest.join("small.txt")).unwrap();
    assert_eq!(small_src, small_dst, "small.txt must be byte-identical");

    // nested/data.bin — byte-identical (multi-chunk)
    let big_src = std::fs::read(src_tmp.path().join("nested/data.bin")).unwrap();
    let big_dst = std::fs::read(dest.join("nested/data.bin")).unwrap();
    assert_eq!(big_src, big_dst, "nested/data.bin must be byte-identical");

    // exec.sh — byte-identical + mode 0o755
    let exec_src = std::fs::read(src_tmp.path().join("exec.sh")).unwrap();
    let exec_dst = std::fs::read(dest.join("exec.sh")).unwrap();
    assert_eq!(exec_src, exec_dst, "exec.sh must be byte-identical");
    let exec_mode = std::fs::metadata(dest.join("exec.sh"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(exec_mode, 0o755, "exec.sh must have mode 0o755");

    // symlink target preserved
    let link_target = std::fs::read_link(dest.join("link.txt")).unwrap();
    assert_eq!(
        link_target.to_str().unwrap(),
        "small.txt",
        "link.txt symlink target must be small.txt"
    );

    // empty directory present
    assert!(
        dest.join("empty_subdir").is_dir(),
        "empty_subdir must be present after hydrate"
    );
}

// ---------------------------------------------------------------------------
// Test 2: status_clean_dirty
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_clean_dirty() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let src_tmp = TempDir::new().unwrap();
    build_source_tree(src_tmp.path());

    let cache_dir = TempDir::new().unwrap();

    // Snapshot
    clw_cmd(&server.uri(), cache_dir.path())
        .args([
            "snapshot",
            src_tmp.path().to_str().unwrap(),
            "--name",
            "e2e-status",
        ])
        .assert()
        .success();

    // status clean → exit 0
    clw_cmd(&server.uri(), cache_dir.path())
        .args([
            "status",
            src_tmp.path().to_str().unwrap(),
            "--name",
            "e2e-status",
        ])
        .assert()
        .success();

    // Modify one file
    std::fs::write(src_tmp.path().join("small.txt"), b"modified content").unwrap();

    // status dirty → exit 1, changed path in stdout
    let out = clw_cmd(&server.uri(), cache_dir.path())
        .args([
            "status",
            src_tmp.path().to_str().unwrap(),
            "--name",
            "e2e-status",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "dirty status must exit 1");
    let stdout_str = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout_str.contains("small.txt"),
        "dirty output must mention changed path 'small.txt', got: {stdout_str}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: run_memoization_and_exit_codes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_memoization_and_exit_codes() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let work_dir = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();
    let input_file = work_dir.path().join("run_input.txt");
    std::fs::write(&input_file, b"memoized input data").unwrap();

    let input_str = input_file.to_str().unwrap();

    // First run: cache miss — exits 0
    let out1 = clw_cmd(&server.uri(), cache_dir.path())
        .current_dir(work_dir.path())
        .args([
            "run",
            "--input",
            input_str,
            "--",
            "/bin/sh",
            "-c",
            "cat run_input.txt; exit 0",
        ])
        .output()
        .unwrap();
    assert_eq!(out1.status.code(), Some(0), "first run must exit 0");
    let stderr1 = String::from_utf8_lossy(&out1.stderr);
    assert!(
        stderr1.contains("cache miss"),
        "first run must print cache miss, got: {stderr1}"
    );

    // Second run: cache hit — exits 0, stdout byte-equal, stderr mentions "cache hit"
    let out2 = clw_cmd(&server.uri(), cache_dir.path())
        .current_dir(work_dir.path())
        .args([
            "run",
            "--input",
            input_str,
            "--",
            "/bin/sh",
            "-c",
            "cat run_input.txt; exit 0",
        ])
        .output()
        .unwrap();
    assert_eq!(out2.status.code(), Some(0), "second run must exit 0");
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr2.contains("cache hit"),
        "second run must print cache hit, got: {stderr2}"
    );
    assert_eq!(
        out1.stdout, out2.stdout,
        "replayed stdout must be byte-identical"
    );

    // Non-zero exit → clw propagates exit code 7
    let out_fail = clw_cmd(&server.uri(), cache_dir.path())
        .current_dir(work_dir.path())
        .args([
            "run",
            "--input",
            input_str,
            "--",
            "/bin/sh",
            "-c",
            "cat run_input.txt; exit 7",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out_fail.status.code(),
        Some(7),
        "non-zero command must propagate exit code 7"
    );
    let stderr_fail = String::from_utf8_lossy(&out_fail.stderr);
    assert!(
        stderr_fail.contains("cache miss"),
        "non-zero run must be a miss, got: {stderr_fail}"
    );

    // Repeat of the non-zero command → still a miss (non-zero not cached)
    let out_fail2 = clw_cmd(&server.uri(), cache_dir.path())
        .current_dir(work_dir.path())
        .args([
            "run",
            "--input",
            input_str,
            "--",
            "/bin/sh",
            "-c",
            "cat run_input.txt; exit 7",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out_fail2.status.code(),
        Some(7),
        "repeated non-zero command must still exit 7"
    );
    let stderr_fail2 = String::from_utf8_lossy(&out_fail2.stderr);
    assert!(
        stderr_fail2.contains("cache miss"),
        "repeated non-zero run must still be a miss, got: {stderr_fail2}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: json_report
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_report() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let src_tmp = TempDir::new().unwrap();
    std::fs::write(src_tmp.path().join("data.txt"), b"json report test").unwrap();

    let cache_dir = TempDir::new().unwrap();

    let out = clw_cmd(&server.uri(), cache_dir.path())
        .args([
            "--json",
            "snapshot",
            src_tmp.path().to_str().unwrap(),
            "--name",
            "e2e-json",
        ])
        .output()
        .unwrap();

    assert!(out.status.success(), "snapshot --json must succeed");

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout_str.trim()).expect("stdout must be valid JSON");

    // Check required report fields
    assert!(
        parsed.get("root").and_then(|v| v.as_str()).is_some(),
        "JSON report must have 'root' field, got: {parsed}"
    );
    assert!(
        parsed.get("files").and_then(|v| v.as_u64()).is_some(),
        "JSON report must have 'files' field, got: {parsed}"
    );
    assert!(
        parsed.get("bytes_total").and_then(|v| v.as_u64()).is_some(),
        "JSON report must have 'bytes_total' field, got: {parsed}"
    );
    assert!(
        parsed
            .get("chunks_total")
            .and_then(|v| v.as_u64())
            .is_some(),
        "JSON report must have 'chunks_total' field, got: {parsed}"
    );
    assert!(
        parsed
            .get("chunks_uploaded")
            .and_then(|v| v.as_u64())
            .is_some(),
        "JSON report must have 'chunks_uploaded' field, got: {parsed}"
    );

    // root must be a 64-char hex string
    let root = parsed["root"].as_str().unwrap();
    assert_eq!(root.len(), 64, "root digest must be 64 hex chars");
    assert!(
        root.chars().all(|c| c.is_ascii_hexdigit()),
        "root digest must be lowercase hex"
    );
}

// ---------------------------------------------------------------------------
// Test 5: auth_failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_failure() {
    let server = MockServer::start().await;
    // Mount with auth_ok=true; our client will send the wrong token.
    mount_corelink(&server, true).await;

    let src_tmp = TempDir::new().unwrap();
    std::fs::write(src_tmp.path().join("file.txt"), b"auth test data").unwrap();

    let cache_dir = TempDir::new().unwrap();

    // Use wrong token by overriding CLW_TOKEN
    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("CLW_ENDPOINT", server.uri())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", "wrong-token")
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args([
            "snapshot",
            src_tmp.path().to_str().unwrap(),
            "--name",
            "e2e-auth-fail",
        ])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "wrong token must cause non-zero exit, got: {:?}",
        out.status.code()
    );
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.to_lowercase().contains("auth")
            || stderr_str.contains("401")
            || stderr_str.contains("authentication"),
        "stderr must mention auth failure, got: {stderr_str}"
    );
}

// ---------------------------------------------------------------------------
// Binary-level smoke tests for the offline subcommands: doctor / init / auth /
// ls. These never touch the network — they exercise config resolution,
// filesystem effects, and the redaction invariant against the REAL binary.
//
// Each test that needs a config home points $HOME at its own `TempDir`, so the
// per-process subprocess env is isolated and the tests stay parallel-safe
// without any shared-state locking.
// ---------------------------------------------------------------------------

/// A token value that must NEVER appear verbatim in any redacted output.
const SECRET_TOKEN: &str = "super-secret-pat-DO-NOT-LEAK-9f8e7d6c";

// ---------------------------------------------------------------------------
// Test 6: doctor_runs_and_redacts_token
// ---------------------------------------------------------------------------

#[test]
fn doctor_runs_and_redacts_token() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("HOME", home.path())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", SECRET_TOKEN)
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .arg("doctor")
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "doctor must exit 0");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Reports version and OS.
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "doctor must print the clw version, got:\n{stdout}"
    );
    assert!(
        stdout.contains(std::env::consts::OS),
        "doctor must print the OS, got:\n{stdout}"
    );

    // Redaction invariant: the token value must never appear on either stream.
    assert!(
        !stdout.contains(SECRET_TOKEN),
        "token value must NEVER appear on stdout, got:\n{stdout}"
    );
    assert!(
        !stderr.contains(SECRET_TOKEN),
        "token value must NEVER appear on stderr, got:\n{stderr}"
    );
    // Presence is reported as the sentinel instead of the value.
    assert!(
        stdout.contains("<set>"),
        "token presence must be shown as <set>, got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Test 7: doctor_json_redacts_token
// ---------------------------------------------------------------------------

#[test]
fn doctor_json_redacts_token() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("HOME", home.path())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", SECRET_TOKEN)
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args(["--json", "doctor"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "doctor --json must exit 0");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Valid JSON.
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("doctor --json stdout must be valid JSON");

    // Token field is redacted to the presence sentinel, never the value.
    assert_eq!(
        parsed["token"], "<set>",
        "doctor --json token field must be redacted to <set>, got: {parsed}"
    );
    assert!(
        !stdout.contains(SECRET_TOKEN),
        "token value must NEVER appear in doctor --json stdout, got:\n{stdout}"
    );
    assert!(
        !stderr.contains(SECRET_TOKEN),
        "token value must NEVER appear on stderr, got:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: init_writes_0600_config_and_rejects_reinit
// ---------------------------------------------------------------------------

#[test]
fn init_writes_0600_config_and_rejects_reinit() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let run_init = |args: &[&str]| {
        Command::cargo_bin("clw")
            .unwrap()
            .env("HOME", home.path())
            .env("CLW_ENDPOINT", "https://example.com")
            .env("CLW_TENANT", TEST_TENANT)
            .env("CLW_TOKEN", SECRET_TOKEN)
            .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
            .args(["init"].iter().chain(args.iter()))
            .output()
            .unwrap()
    };

    // First init writes the config file.
    let out1 = run_init(&[]);
    assert_eq!(out1.status.code(), Some(0), "first init must exit 0");

    let config_path = home.path().join(".clw").join("config.toml");
    assert!(
        config_path.exists(),
        "init must write {}",
        config_path.display()
    );

    // The file holds the plaintext PAT, so it must be owner-only (0600).
    let mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "config.toml must be written 0600");

    // The token is never echoed on stderr/stdout, only persisted to the file.
    let out1_stdout = String::from_utf8_lossy(&out1.stdout);
    let out1_stderr = String::from_utf8_lossy(&out1.stderr);
    assert!(
        !out1_stdout.contains(SECRET_TOKEN) && !out1_stderr.contains(SECRET_TOKEN),
        "init must NEVER echo the token value"
    );

    // Re-init WITHOUT --force must fail cleanly (no panic) with an actionable error.
    let out2 = run_init(&[]);
    assert!(
        !out2.status.success(),
        "re-init without --force must fail, got: {:?}",
        out2.status.code()
    );
    let out2_stderr = String::from_utf8_lossy(&out2.stderr);
    assert!(
        out2_stderr.contains("already exists") && out2_stderr.contains("--force"),
        "re-init error must mention the existing file and --force, got:\n{out2_stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 9: auth_whoami_prints_tenant_not_token
// ---------------------------------------------------------------------------

#[test]
fn auth_whoami_prints_tenant_not_token() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("HOME", home.path())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", SECRET_TOKEN)
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args(["auth", "whoami"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "auth whoami must exit 0");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // The tenant IS printed (it is an identifier, not a secret).
    assert!(
        stdout.contains(TEST_TENANT),
        "whoami must print the tenant '{TEST_TENANT}', got:\n{stdout}"
    );
    // The token value is NEVER printed — presence sentinel only.
    assert!(
        !stdout.contains(SECRET_TOKEN) && !stderr.contains(SECRET_TOKEN),
        "whoami must NEVER print the token value"
    );
    assert!(
        stdout.contains("<set>"),
        "whoami must show token presence as <set>, got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Test 10: ls_unconfigured_fails_cleanly
// ---------------------------------------------------------------------------

#[test]
fn ls_unconfigured_fails_cleanly() {
    // No tenant/token anywhere (empty HOME, no CLW_* identity env) — config
    // resolution must fail with an actionable error BEFORE any network call,
    // and the binary must never panic.
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("HOME", home.path())
        // Strip any inherited identity so the binary is genuinely unconfigured.
        .env_remove("CLW_ENDPOINT")
        .env_remove("CLW_TENANT")
        .env_remove("CLW_TOKEN")
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args(["ls", "--name", "missing-ws"])
        .output()
        .unwrap();

    // Clean failure: exit 2 (config error), not a panic / abort signal.
    assert_eq!(
        out.status.code(),
        Some(2),
        "unconfigured ls must exit 2 (config error), got: {:?}",
        out.status.code()
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    // Must not have panicked.
    assert!(
        !stderr.contains("panicked"),
        "ls must not panic when unconfigured, got:\n{stderr}"
    );
    // Actionable: tells the user what to set.
    assert!(
        stderr.contains("tenant")
            && (stderr.contains("CLW_TENANT")
                || stderr.contains("--tenant")
                || stderr.contains("config.toml")),
        "unconfigured ls must give an actionable 'missing tenant' error, got:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 11: uninstall — consent, dry-run, redaction, real removal
//
// These exercise the REAL binary against an isolated HOME + cache dir, so no
// real ~/.clw is ever touched. They cover the security-critical contract:
//   - --dry-run lists paths and removes NOTHING (and never prints the token),
//   - no --yes in a non-interactive context ERRORS without deleting,
//   - --yes actually scrubs ~/.clw + the resolved cache dir,
//   - the running binary is never auto-deleted (path + command are printed).
// ---------------------------------------------------------------------------

/// Seed an isolated `HOME/.clw/config.toml` holding the secret PAT, plus a
/// populated cache dir, and return (home, cache) temp dirs.
fn seed_clw_footprint() -> (TempDir, TempDir) {
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    let clw_dir = home.path().join(".clw");
    std::fs::create_dir_all(&clw_dir).unwrap();
    std::fs::write(
        clw_dir.join("config.toml"),
        format!("# clw configuration\ntenant = \"{TEST_TENANT}\"\ntoken = \"{SECRET_TOKEN}\"\n"),
    )
    .unwrap();
    // A stand-in for cached customer file content.
    std::fs::write(cache.path().join("blob.bin"), b"customer file content").unwrap();

    (home, cache)
}

#[test]
fn uninstall_dry_run_lists_paths_removes_nothing_and_no_token() {
    let (home, cache) = seed_clw_footprint();
    let clw_dir = home.path().join(".clw");

    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("CLW_TENANT")
        .env_remove("CLW_TOKEN")
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["--json", "uninstall", "--dry-run"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "dry-run must exit 0");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Removed NOTHING: both the config dir and the cache dir still exist.
    assert!(clw_dir.exists(), "dry-run must NOT remove ~/.clw");
    assert!(
        cache.path().exists(),
        "dry-run must NOT remove the cache dir"
    );

    // Never leaks the token, on either stream.
    assert!(
        !stdout.contains(SECRET_TOKEN) && !stderr.contains(SECRET_TOKEN),
        "uninstall must NEVER print the token"
    );

    // JSON reports dry_run + the resolved paths, removed=false for each target.
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(parsed["dry_run"], true);
    let targets = parsed["targets"].as_array().unwrap();
    assert!(!targets.is_empty(), "must list at least one target path");
    for t in targets {
        assert_eq!(t["removed"], false, "dry-run removes nothing");
    }
    // The running binary is reported but NEVER auto-deleted.
    assert_eq!(parsed["binary"]["removed"], false);
}

#[test]
fn uninstall_without_yes_noninteractive_errors_and_deletes_nothing() {
    let (home, cache) = seed_clw_footprint();
    let clw_dir = home.path().join(".clw");

    // No TTY in the test harness (stdin is not a terminal) and no --yes: the
    // binary must REFUSE and delete nothing, in both plain and --json mode.
    for json in [false, true] {
        let mut cmd = Command::cargo_bin("clw").unwrap();
        cmd.env("HOME", home.path())
            .env_remove("CLW_TENANT")
            .env_remove("CLW_TOKEN")
            .env("CLW_CACHE_DIR", cache.path().to_str().unwrap());
        if json {
            cmd.arg("--json");
        }
        cmd.arg("uninstall");
        let out = cmd.output().unwrap();

        assert_eq!(
            out.status.code(),
            Some(2),
            "uninstall without --yes (non-interactive) must exit 2 (json={json})"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("--yes"),
            "error must instruct to pass --yes, got:\n{stderr}"
        );
        // Nothing deleted.
        assert!(clw_dir.exists(), "refused uninstall must NOT remove ~/.clw");
        assert!(
            cache.path().exists(),
            "refused uninstall must NOT remove cache"
        );
        // Never leaks the token.
        assert!(
            !stderr.contains(SECRET_TOKEN),
            "must not leak token on error"
        );
    }
}

#[test]
fn uninstall_yes_scrubs_config_and_cache_keeps_binary() {
    let (home, cache) = seed_clw_footprint();
    let clw_dir = home.path().join(".clw");

    let out = Command::cargo_bin("clw")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("CLW_TENANT")
        .env_remove("CLW_TOKEN")
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["--json", "uninstall", "--yes"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "uninstall --yes must exit 0");

    // Actually scrubbed: both sensitive trees are gone.
    assert!(!clw_dir.exists(), "uninstall --yes must remove ~/.clw");
    assert!(
        !cache.path().exists(),
        "uninstall --yes must remove the resolved cache dir"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout.contains(SECRET_TOKEN) && !stderr.contains(SECRET_TOKEN),
        "uninstall must NEVER print the token"
    );

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(parsed["dry_run"], false);
    // The binary was reported with a remove command but NOT auto-deleted.
    assert_eq!(parsed["binary"]["removed"], false);
    assert!(parsed["binary"]["remove_command"]
        .as_str()
        .unwrap()
        .starts_with("rm "));
    // The actual clw test binary still exists on disk (we never self-deleted).
    let exe = parsed["binary"]["path"].as_str().unwrap();
    assert!(
        std::path::Path::new(exe).exists(),
        "the running binary must NOT be deleted by uninstall"
    );
}

// ---------------------------------------------------------------------------
// Test 12: completions — each shell yields non-empty output mentioning `clw`,
// and works with NO config/token (a fresh, unconfigured user).
// ---------------------------------------------------------------------------

#[test]
fn completions_each_shell_nonempty_and_offline() {
    let home = TempDir::new().unwrap();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = Command::cargo_bin("clw")
            .unwrap()
            // Fresh, unconfigured user: no identity at all.
            .env("HOME", home.path())
            .env_remove("CLW_ENDPOINT")
            .env_remove("CLW_TENANT")
            .env_remove("CLW_TOKEN")
            .args(["completions", shell])
            .output()
            .unwrap();

        assert_eq!(
            out.status.code(),
            Some(0),
            "completions {shell} must exit 0 even when unconfigured"
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.trim().is_empty(),
            "completions {shell} must produce a non-empty script"
        );
        assert!(
            stdout.contains("clw"),
            "completions {shell} script must reference the clw binary"
        );
    }
}

// ---------------------------------------------------------------------------
// Destructive-command e2e tests (rm / ls --all / prune / erase).
//
// Each snapshots a small tree first (so refs + blobs really exist in the mock
// stores), then drives the REAL `clw` binary and asserts BOTH the exit code AND
// the resulting state of the in-memory CAS/AC stores returned by mount_corelink.
// Unique workspace names + per-test TempDirs keep the parallel suite isolated.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test: rm — deletes the ref, is idempotent, refuses without --yes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rm_deletes_ref_is_idempotent_and_requires_yes() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-rm-ws";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    let rk = ref_key_hex(ws);
    assert!(
        ac.lock().unwrap().contains_key(&rk),
        "snapshot must have written the ref into the AC store"
    );

    // rm --yes deletes the ref (assert the key is gone from the AC store).
    clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", ws, "--yes"])
        .assert()
        .success();
    assert!(
        !ac.lock().unwrap().contains_key(&rk),
        "rm --yes must delete the ref key from the AC store"
    );

    // A second rm --yes is idempotent: still exit 0, ref still absent.
    clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", ws, "--yes"])
        .assert()
        .success();
    assert!(
        !ac.lock().unwrap().contains_key(&rk),
        "idempotent rm must leave the ref absent"
    );

    // Re-snapshot, then rm WITHOUT --yes in a non-interactive context must REFUSE
    // (non-zero exit) and delete nothing.
    let _src2 = snapshot_tiny(&server.uri(), cache.path(), ws);
    assert!(ac.lock().unwrap().contains_key(&rk));
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", ws])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "rm without --yes (non-interactive) must refuse with a non-zero exit, got: {:?}",
        out.status.code()
    );
    assert!(
        ac.lock().unwrap().contains_key(&rk),
        "refused rm must NOT delete the ref"
    );
}

// ---------------------------------------------------------------------------
// Test: ls --all — lists refs, drops them after rm, tolerates a corrupt record
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ls_all_lists_refs_tolerates_corruption_and_reflects_rm() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-lsall-ws";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    // ls --all lists the snapshotted ref by name.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["ls", "--all"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "ls --all must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(ws),
        "ls --all must list the snapshotted ref '{ws}', got:\n{stdout}"
    );

    // Inject a corrupt AC record under a fake ref key (a non-decodable value).
    // The good ref must STILL list, AND the skipped record must be reported.
    let fake_key = "ff".repeat(32); // 64 hex chars, not a real ref
    ac.lock().unwrap().insert(
        fake_key.clone(),
        b"{ this is not a valid ref record".to_vec(),
    );

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "ls", "--all"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "ls --all must tolerate a corrupt record and still exit 0"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(
        parsed["count"], 1,
        "the one good ref must still be listed, got:\n{stdout}"
    );
    assert_eq!(
        parsed["skipped"], 1,
        "the corrupt record must be reported as skipped, got:\n{stdout}"
    );
    assert_eq!(
        parsed["errors"][0]["ref_key"], fake_key,
        "the skipped record must be named by its ref key"
    );

    // Remove the corrupt record, then rm the good ref: ls --all no longer lists it.
    ac.lock().unwrap().remove(&fake_key);
    clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", ws, "--yes"])
        .assert()
        .success();
    assert!(!ac.lock().unwrap().contains_key(&ref_key_hex(ws)));

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["ls", "--all"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "ls --all must exit 0 when empty"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(ws),
        "after rm, ls --all must NOT list '{ws}', got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Test: prune — dry-run keeps reachable / sweeps orphan; --yes deletes orphan;
// fail-safe abort when a manifest blob is missing; empty-live-set floor refuses.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prune_dry_run_then_yes_removes_only_orphans() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-prune-ws";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    // Snapshot the reachable (live) blob set so we can assert it survives.
    let live_blobs: Vec<String> = cas.lock().unwrap().keys().cloned().collect();
    assert!(
        !live_blobs.is_empty(),
        "snapshot must have populated the CAS store"
    );

    // Plant an orphan blob directly in CAS (not referenced by any manifest).
    let orphan = "ab".repeat(32); // 64 hex chars
    cas.lock()
        .unwrap()
        .insert(orphan.clone(), b"orphaned content".to_vec());

    // Dry-run (default): reports the orphan as would-delete, 0 for the live set,
    // and deletes NOTHING — the orphan and every live blob are still present.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "prune"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "prune dry-run must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(parsed["dry_run"], true);
    let would: Vec<String> = parsed["would_delete"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        would,
        vec![orphan.clone()],
        "dry-run must report exactly the planted orphan as would-delete"
    );
    {
        let guard = cas.lock().unwrap();
        assert!(
            guard.contains_key(&orphan),
            "dry-run must NOT delete the orphan"
        );
        for b in &live_blobs {
            assert!(guard.contains_key(b), "dry-run must NOT delete a live blob");
        }
    }

    // prune --yes: removes the orphan, keeps every reachable blob.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["prune", "--yes"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "prune --yes must exit 0");
    {
        let guard = cas.lock().unwrap();
        assert!(
            !guard.contains_key(&orphan),
            "prune --yes must delete the orphan blob"
        );
        for b in &live_blobs {
            assert!(
                guard.contains_key(b),
                "prune --yes must KEEP reachable blob {b}"
            );
        }
    }

    // The ref is untouched by prune.
    assert!(ac.lock().unwrap().contains_key(&ref_key_hex(ws)));
}

#[tokio::test]
async fn prune_aborts_fail_safe_when_manifest_blob_missing() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-prune-failsafe-ws";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    // Recover the ref's manifest-root digest from the AC record, then delete that
    // manifest blob from CAS — an incomplete live set. prune MUST abort fail-safe.
    let rk = ref_key_hex(ws);
    let root_hex = {
        let guard = ac.lock().unwrap();
        let bytes = guard.get(&rk).expect("ref record must exist");
        clw_types::RefRecord::from_bytes(bytes)
            .unwrap()
            .root
            .to_hex()
    };

    // Plant an orphan so we can prove the abort deleted NOTHING (not even orphans).
    let orphan = "cd".repeat(32);
    cas.lock()
        .unwrap()
        .insert(orphan.clone(), b"orphan during failsafe".to_vec());

    // Drop the manifest root blob → live set cannot be computed in full.
    assert!(
        cas.lock().unwrap().remove(&root_hex).is_some(),
        "manifest root must have been present in CAS before we remove it"
    );

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["prune", "--yes"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "prune must abort fail-safe (exit 2) on an incomplete live set, got: {:?}",
        out.status.code()
    );
    // Nothing deleted: the planted orphan is STILL present after the abort.
    assert!(
        cas.lock().unwrap().contains_key(&orphan),
        "fail-safe abort must delete NOTHING — the orphan must still be present"
    );
}

#[tokio::test]
async fn prune_empty_live_set_over_nonempty_cas_refuses() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-prune-floor-ws";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    // No refs but blobs present: clear every ref directly, leaving the CAS blobs.
    // This is the "delete everything" mis-enumeration signature the floor refuses.
    ac.lock().unwrap().clear();
    let blobs_before: Vec<String> = cas.lock().unwrap().keys().cloned().collect();
    assert!(
        !blobs_before.is_empty(),
        "CAS must be non-empty for this test"
    );

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["prune", "--yes"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "empty-live-set over non-empty CAS must be refused (exit 2), got: {:?}",
        out.status.code()
    );
    // Nothing deleted: every blob still present.
    let guard = cas.lock().unwrap();
    for b in &blobs_before {
        assert!(
            guard.contains_key(b),
            "empty-live-set refusal must delete NOTHING — blob {b} must remain"
        );
    }
}

// ---------------------------------------------------------------------------
// Test: erase — full delete + verified-empty; wrong --confirm-tenant aborts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn erase_wrong_confirm_tenant_aborts_deletes_nothing() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-erase-abort-ws";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    let refs_before = ac.lock().unwrap().len();
    let blobs_before = cas.lock().unwrap().len();
    assert!(refs_before > 0 && blobs_before > 0);

    // Wrong typed-tenant confirmation must abort (non-zero) and delete nothing.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["erase", "--yes", "--confirm-tenant", "not-the-tenant"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "erase with a wrong --confirm-tenant must abort, got: {:?}",
        out.status.code()
    );
    assert_eq!(
        ac.lock().unwrap().len(),
        refs_before,
        "aborted erase must NOT delete any ref"
    );
    assert_eq!(
        cas.lock().unwrap().len(),
        blobs_before,
        "aborted erase must NOT delete any blob"
    );
}

#[tokio::test]
async fn erase_yes_confirmed_empties_both_stores() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    // Two workspaces so erase has more than one ref to clear.
    let _src1 = snapshot_tiny(&server.uri(), cache.path(), "e2e-erase-ws-1");
    let _src2 = snapshot_tiny(&server.uri(), cache.path(), "e2e-erase-ws-2");
    assert!(!ac.lock().unwrap().is_empty());
    assert!(!cas.lock().unwrap().is_empty());

    // Correct typed-tenant confirmation: erase deletes all refs + blobs and the
    // verify re-enumeration sees the tenant empty (success exit 0).
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["erase", "--yes", "--confirm-tenant", TEST_TENANT])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "erase with correct confirmation must succeed, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        ac.lock().unwrap().is_empty(),
        "erase must leave the AC store empty"
    );
    assert!(
        cas.lock().unwrap().is_empty(),
        "erase must leave the CAS store empty"
    );
}
