//! HTTP-level integration tests for the clw stack.
//!
//! All tests run against a local wiremock `MockServer`; no requests ever reach
//! `corelink-api.humangr.com`.
//!
//! The mock server maintains two in-memory `Arc<Mutex<HashMap>>` stores — one
//! for CAS (`/v1/cas/:tenant/:hash`) and one for AC (`/v1/ac/:tenant/:key`).
//! Every route requires `Authorization: Bearer test-pat`; anything else → 401.
//!
//! UNIX-ONLY: the fixtures use unix symlinks + `chmod` and assert unix file modes
//! (`exec.sh` 0o755), so this suite is gated to unix. Windows integration coverage
//! lands with the M2.4 cross-platform CI lane. `#![cfg(unix)]` makes the crate
//! compile to nothing on Windows so a future `cargo test --target *-windows-*`
//! build does not break on the unix imports.
#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use wiremock::matchers::{header, method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_types::Config;

// ---------------------------------------------------------------------------
// Stateful mock CoreLink server helpers
// ---------------------------------------------------------------------------

const TEST_TENANT: &str = "test-tenant";
const TEST_PAT: &str = "test-pat";

type Store = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// Build a `Config` pointing at the wiremock server URI.
fn test_config(endpoint: String, token: &str, cache_dir: PathBuf) -> Config {
    Config {
        endpoint,
        tenant: TEST_TENANT.to_string(),
        token: token.to_string(),
        cache_dir,
    }
}

/// Extract the last path segment (`/v1/cas/tenant/<hash>` → `<hash>`).
fn last_segment(url: &wiremock::http::Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("")
        .to_string()
}

// --------------------------------
// Custom Respond implementations
// --------------------------------

/// A wiremock responder that routes CAS requests based on HTTP method.
#[derive(Clone)]
struct CasResponder {
    store: Store,
    /// If false, return 401 immediately on every request.
    auth_ok: bool,
}

impl Respond for CasResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        // Auth check first.
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

/// AC responder (PUT/GET only, no HEAD).
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

/// Mount stateful CAS + AC mocks on a `MockServer`.
/// Returns `(cas_store, ac_store)`.
async fn mount_corelink(server: &MockServer, auth_ok: bool) -> (Store, Store) {
    let cas_store: Store = Arc::new(Mutex::new(HashMap::new()));
    let ac_store: Store = Arc::new(Mutex::new(HashMap::new()));

    let cas_pattern = format!("/v1/cas/{TEST_TENANT}/[a-f0-9]{{64}}");
    let ac_pattern = format!("/v1/ac/{TEST_TENANT}/[a-f0-9]{{64}}");

    // CAS: PUT, GET, HEAD
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

    // AC: PUT, GET
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

/// Count the number of PUT requests to the CAS path that the server received.
async fn count_cas_puts(server: &MockServer) -> usize {
    let cas_pattern = format!("/v1/cas/{TEST_TENANT}/");
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path().starts_with(&cas_pattern))
        .count()
}

// ---------------------------------------------------------------------------
// Build the test workspace tree
// ---------------------------------------------------------------------------

/// Populate a directory tree used by snapshot/hydrate tests:
/// - `small.txt` (< CHUNK_MIN)
/// - `big.bin` (> 1 MiB → forces multi-chunk)
/// - `link.txt` → symlink to `small.txt`
/// - `empty_subdir/` (empty directory)
/// - `exec.sh` (executable, mode 0o755)
fn build_test_tree(root: &std::path::Path) {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Small file (< CHUNK_MIN_BYTES = 256 KiB)
    fs::write(root.join("small.txt"), b"hello integration test").unwrap();

    // Large file: 1.5 MiB of pseudo-random bytes → guaranteed multi-chunk.
    let big: Vec<u8> = (0u32..=(1024 * 1024 + 512 * 1024))
        .map(|i| (i.wrapping_mul(0x9e37_79b9u32)) as u8)
        .collect();
    fs::write(root.join("big.bin"), &big).unwrap();

    // Symlink
    std::os::unix::fs::symlink("small.txt", root.join("link.txt")).unwrap();

    // Empty directory (must be recorded in the manifest)
    fs::create_dir(root.join("empty_subdir")).unwrap();

    // Executable file
    fs::write(root.join("exec.sh"), b"#!/bin/sh\necho hello\n").unwrap();
    let mut perms = fs::metadata(root.join("exec.sh")).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(root.join("exec.sh"), perms).unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: full roundtrip snapshot → hydrate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_roundtrip_snapshot_hydrate() {
    let server = MockServer::start().await;
    let (_, _ac) = mount_corelink(&server, true).await;

    // Source tree
    let src_tmp = TempDir::new().unwrap();
    let src_root = src_tmp.path();
    build_test_tree(src_root);

    // Cache 1 (used during snapshot)
    let cache1_tmp = TempDir::new().unwrap();
    let cache1 = LocalCache::new(cache1_tmp.path()).unwrap();

    let client1 = HttpClient::new(test_config(
        server.uri(),
        TEST_PAT,
        cache1_tmp.path().to_path_buf(),
    ))
    .unwrap();

    // Snapshot
    let snap_opts = clw_snapshot::SnapshotOptions::new("roundtrip-ws");
    let report = clw_snapshot::snapshot(src_root, &client1, &cache1, &snap_opts)
        .await
        .unwrap();

    // Fresh dest + EMPTY second cache
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("hydrated");

    let cache2_tmp = TempDir::new().unwrap();
    let cache2 = LocalCache::new(cache2_tmp.path()).unwrap();

    let client2 = HttpClient::new(test_config(
        server.uri(),
        TEST_PAT,
        cache2_tmp.path().to_path_buf(),
    ))
    .unwrap();

    let hydrate_opts = clw_hydrate::HydrateOptions::new("roundtrip-ws");
    let hydrate_report = clw_hydrate::hydrate(&dest, &client2, &cache2, &hydrate_opts)
        .await
        .unwrap();

    // Assertions
    assert_eq!(hydrate_report.root, report.root);

    // small.txt byte-identical
    let small_src = std::fs::read(src_root.join("small.txt")).unwrap();
    let small_dst = std::fs::read(dest.join("small.txt")).unwrap();
    assert_eq!(small_src, small_dst, "small.txt must be byte-identical");

    // big.bin byte-identical
    let big_src = std::fs::read(src_root.join("big.bin")).unwrap();
    let big_dst = std::fs::read(dest.join("big.bin")).unwrap();
    assert_eq!(big_src, big_dst, "big.bin must be byte-identical");

    // exec.sh byte-identical
    let exec_src = std::fs::read(src_root.join("exec.sh")).unwrap();
    let exec_dst = std::fs::read(dest.join("exec.sh")).unwrap();
    assert_eq!(exec_src, exec_dst, "exec.sh must be byte-identical");

    // exec.sh mode preserved (0o755)
    let exec_mode = std::fs::metadata(dest.join("exec.sh"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(exec_mode, 0o755, "exec.sh must have mode 0o755");

    // symlink target preserved
    let link_target = std::fs::read_link(dest.join("link.txt")).unwrap();
    assert_eq!(link_target.to_str().unwrap(), "small.txt");

    // empty directory present
    assert!(
        dest.join("empty_subdir").is_dir(),
        "empty_subdir must exist"
    );

    // bytes_downloaded > 0 (cache2 was empty, so everything came from remote)
    assert!(
        hydrate_report.bytes_downloaded > 0,
        "bytes_downloaded must be > 0 on a fresh cache hydrate, got {}",
        hydrate_report.bytes_downloaded
    );
}

// ---------------------------------------------------------------------------
// Test 2: dedupe — second snapshot of unchanged tree → chunks_uploaded == 0
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_dedupe_over_http() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let src_tmp = TempDir::new().unwrap();
    let src_root = src_tmp.path();
    build_test_tree(src_root);

    let cache_tmp = TempDir::new().unwrap();
    let cache = LocalCache::new(cache_tmp.path()).unwrap();

    let client = HttpClient::new(test_config(
        server.uri(),
        TEST_PAT,
        cache_tmp.path().to_path_buf(),
    ))
    .unwrap();

    let opts = clw_snapshot::SnapshotOptions::new("dedupe-ws");

    // First snapshot — uploads everything.
    let r1 = clw_snapshot::snapshot(src_root, &client, &cache, &opts)
        .await
        .unwrap();
    assert!(r1.chunks_uploaded > 0, "first snapshot must upload chunks");

    let puts_after_first = count_cas_puts(&server).await;

    // Second snapshot of identical tree — nothing new to upload.
    let r2 = clw_snapshot::snapshot(src_root, &client, &cache, &opts)
        .await
        .unwrap();

    assert_eq!(r2.root, r1.root, "same tree must yield same root");
    assert_eq!(
        r2.chunks_uploaded, 0,
        "second snapshot of unchanged tree must upload 0 chunks"
    );

    // Server-side: no new PUTs to CAS (only the manifest might re-check existence
    // but must not PUT again since it already exists).
    let puts_after_second = count_cas_puts(&server).await;
    // The manifest PUT from the first snapshot could mean 0 additional PUTs.
    // All chunks + manifest are already present, so no new PUTs.
    assert_eq!(
        puts_after_second, puts_after_first,
        "no new CAS PUTs on second identical snapshot (server saw {} → {})",
        puts_after_first, puts_after_second
    );
}

// ---------------------------------------------------------------------------
// Test 3: memoized run over HTTP — second call is a hit with byte-exact stdout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_memoized_run_over_http() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let cwd_tmp = TempDir::new().unwrap();
    let cwd = cwd_tmp.path();

    // Write a small input file.
    std::fs::write(cwd.join("input.txt"), b"memoized input").unwrap();

    let client = HttpClient::new(test_config(server.uri(), TEST_PAT, cwd.to_path_buf())).unwrap();

    let opts1 = clw_run::RunOptions {
        inputs: vec!["input.txt".to_string()],
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "printf 'hello from memoized run'".to_string(),
        ],
        env_keys: vec![],
    };

    // First call: miss — executes the command.
    let outcome1 = clw_run::run_memoized(cwd, &client, &opts1).await.unwrap();
    assert!(!outcome1.hit, "first call must be a miss");
    assert_eq!(outcome1.exit_code, 0);

    // Second call with identical options: must be a hit.
    let opts2 = clw_run::RunOptions {
        inputs: vec!["input.txt".to_string()],
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "printf 'hello from memoized run'".to_string(),
        ],
        env_keys: vec![],
    };
    let outcome2 = clw_run::run_memoized(cwd, &client, &opts2).await.unwrap();
    assert!(outcome2.hit, "second call must be a hit");
    assert_eq!(outcome2.exit_code, 0);

    // stdout must be byte-exact on replay.
    assert_eq!(
        outcome1.stdout, outcome2.stdout,
        "replayed stdout must be byte-identical"
    );
    assert_eq!(
        outcome2.stdout, b"hello from memoized run",
        "stdout content must match command output"
    );
}

// ---------------------------------------------------------------------------
// Test 4: auth — wrong token → ClwError::Auth on snapshot
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_auth_wrong_token_returns_auth_error() {
    let server = MockServer::start().await;
    // Mount with valid auth required; our client will send the WRONG token.
    mount_corelink(&server, true).await;

    let src_tmp = TempDir::new().unwrap();
    let src_root = src_tmp.path();

    // Just a tiny file — enough to trigger at least one CAS PUT.
    std::fs::write(src_root.join("file.txt"), b"auth test").unwrap();

    let cache_tmp = TempDir::new().unwrap();
    let cache = LocalCache::new(cache_tmp.path()).unwrap();

    // Wrong token.
    let bad_client = HttpClient::new(test_config(
        server.uri(),
        "wrong-token",
        cache_tmp.path().to_path_buf(),
    ))
    .unwrap();

    let opts = clw_snapshot::SnapshotOptions::new("auth-ws");
    let err = clw_snapshot::snapshot(src_root, &bad_client, &cache, &opts)
        .await
        .err()
        .expect("wrong token must produce an error");

    assert!(
        matches!(err, clw_types::ClwError::Auth),
        "wrong token must yield ClwError::Auth, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Baseline: wiremock auth-enforcement sanity check (not counted in suite)
// ---------------------------------------------------------------------------

/// Confirm that the mock server correctly rejects a missing auth header with 401.
#[tokio::test]
async fn test_mock_server_rejects_missing_auth() {
    let server = MockServer::start().await;

    // Register a single route that requires the correct auth header.
    Mock::given(method("GET"))
        .and(path_regex("/v1/cas/.*"))
        .and(header(
            "Authorization",
            format!("Bearer {TEST_PAT}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".as_slice()))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();

    // Request without Authorization → not matched → 404 (wiremock unmatched default).
    let resp = client
        .get(format!("{}/v1/cas/test-tenant/abc123", server.uri()))
        .send()
        .await
        .unwrap();
    assert_ne!(
        resp.status().as_u16(),
        200,
        "request without auth must not get 200"
    );
}
