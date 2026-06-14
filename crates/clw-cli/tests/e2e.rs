//! Binary-level end-to-end tests for the `clw` CLI.
//!
//! Every test spawns the REAL `clw` binary via `assert_cmd::Command::cargo_bin`
//! and drives it against a stateful wiremock CoreLink mock server (in-memory
//! CAS + AC maps, Bearer auth enforced).  No request ever reaches the real
//! `corelink-api.humangr.com`.
//!
//! Test isolation: each test creates its own `TempDir` trees and unique
//! workspace names, so tests can run in parallel safely.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;
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
        ResponseTemplate::new(405)
    }
}

/// Mount stateful CAS + AC mocks on `server`. Returns `(cas_store, ac_store)`.
async fn mount_corelink(server: &MockServer, auth_ok: bool) -> (Store, Store) {
    let cas_store: Store = Arc::new(Mutex::new(HashMap::new()));
    let ac_store: Store = Arc::new(Mutex::new(HashMap::new()));

    let cas_pattern = format!("/v1/cas/{TEST_TENANT}/[a-f0-9]{{64}}");
    let ac_pattern = format!("/v1/ac/{TEST_TENANT}/[a-f0-9]{{64}}");

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
