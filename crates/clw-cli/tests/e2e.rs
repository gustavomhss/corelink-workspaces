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

/// A create-only AC PUT responder: `PUT` of a key that is ALREADY present returns
/// HTTP 409 (the live CoreLink AC store rejects in-place overwrites — see
/// `docs/REQUEST-server-TL-ac-put-upsert-2026-06-20.md`); a first PUT inserts and
/// returns 201. GET/DELETE behave exactly like [`AcResponder`]. This reproduces
/// the server semantics that make a re-snapshot of an existing name fail, which the
/// default [`AcResponder`] (unconditional upsert) cannot exercise.
#[derive(Clone)]
struct AcCreateOnlyResponder {
    store: Store,
    auth_ok: bool,
}

impl Respond for AcCreateOnlyResponder {
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
            let mut guard = self.store.lock().unwrap();
            if guard.contains_key(&key) {
                // Create-only: the key already exists → reject the overwrite.
                return ResponseTemplate::new(409);
            }
            guard.insert(key, request.body.clone());
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
/// The AC PUT route is an unconditional upsert (the common case).
async fn mount_corelink(server: &MockServer, auth_ok: bool) -> (Store, Store) {
    mount_corelink_opts(server, auth_ok, false).await
}

/// Like [`mount_corelink`], but when `ac_create_only` is set the AC PUT route
/// rejects overwrites with HTTP 409 ([`AcCreateOnlyResponder`]) — modelling the
/// live store's create-only AC semantics so a re-snapshot of an existing name
/// reproduces the real 409 path.
async fn mount_corelink_opts(
    server: &MockServer,
    auth_ok: bool,
    ac_create_only: bool,
) -> (Store, Store) {
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
    // AC PUT: create-only (409-on-overwrite) when requested, else upsert.
    if ac_create_only {
        Mock::given(method("PUT"))
            .and(path_regex(&ac_pattern))
            .respond_with(AcCreateOnlyResponder {
                store: Arc::clone(&ac_store),
                auth_ok,
            })
            .mount(server)
            .await;
    } else {
        Mock::given(method("PUT"))
            .and(path_regex(&ac_pattern))
            .respond_with(AcResponder {
                store: Arc::clone(&ac_store),
                auth_ok,
            })
            .mount(server)
            .await;
    }
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
// Test 1b: runner_model_env_only_lifecycle_and_keyspace_isolation
//
// Proves `clw` works exactly as a disposable corelink-runner drives it: env-only,
// NO config file (empty $HOME ⇒ ~/.clw/config.toml cannot exist), NO `--endpoint
// /--tenant/--token` CLI flags, and a runner ref-keyspace selected purely via
// `CLW_REF_DOMAIN=runner`. The lifecycle (snapshot → hydrate with a fresh cache,
// so bytes come off the mock server) is asserted byte-identical, and the CORE
// assertion is keyspace isolation: the same workspace name hydrated in the USER
// domain (CLW_REF_DOMAIN omitted) is NOT FOUND, because the runner ref lives only
// in `clw/ref/runner/v1/` — disjoint from the user keyspace `clw/ref/v1/`. This
// black-boxes the disjoint keyspaces through the real binary.
// ---------------------------------------------------------------------------

/// Spawn `clw` like a disposable runner does: ONLY env vars, NO CLI identity
/// flags, NO config file. `home` is an empty TempDir so `~/.clw/config.toml`
/// cannot exist; any inherited `CLW_*`/`CORELINK_*` identity is stripped and then
/// the runner's own vars are set explicitly. `ref_domain == None` ⇒ the var is NOT
/// set (the user-keyspace default); `Some("runner")` selects the runner keyspace.
fn clw_runner_cmd(
    endpoint: &str,
    home: &std::path::Path,
    cache_dir: &std::path::Path,
    ref_domain: Option<&str>,
) -> Command {
    let mut cmd = Command::cargo_bin("clw").unwrap();
    cmd.env("HOME", home.to_str().unwrap())
        // Strip any identity that could leak in from the parent env so the run is
        // genuinely env-only — no inherited CLW_*/CORELINK_* config-file fallback.
        .env_remove("CLW_ENDPOINT")
        .env_remove("CLW_TENANT")
        .env_remove("CLW_TOKEN")
        .env_remove("CLW_REF_DOMAIN")
        .env_remove("CORELINK_ENDPOINT")
        .env_remove("CORELINK_TENANT")
        .env_remove("CORELINK_TOKEN")
        // The runner's full identity, injected purely via env (no flags, no file).
        .env("CLW_ENDPOINT", endpoint)
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache_dir.to_str().unwrap());
    if let Some(domain) = ref_domain {
        cmd.env("CLW_REF_DOMAIN", domain);
    }
    cmd
}

#[tokio::test]
async fn runner_model_env_only_lifecycle_and_keyspace_isolation() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    // Empty HOME so ~/.clw/config.toml cannot exist for any of these invocations.
    let home = TempDir::new().unwrap();

    // Workspace fixture: a regular file with known bytes + an executable (0o755)
    // + a symlink (mirrors the lifecycle fixture builder's file types).
    let src_tmp = TempDir::new().unwrap();
    std::fs::write(src_tmp.path().join("small.txt"), b"hello runner model").unwrap();
    std::fs::write(src_tmp.path().join("exec.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    let mut perms = std::fs::metadata(src_tmp.path().join("exec.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(src_tmp.path().join("exec.sh"), perms).unwrap();
    std::os::unix::fs::symlink("small.txt", src_tmp.path().join("link.txt")).unwrap();

    // --- snapshot, env-only, runner keyspace, NO identity flags ---
    let cache_snap = TempDir::new().unwrap();
    clw_runner_cmd(
        &server.uri(),
        home.path(),
        cache_snap.path(),
        Some("runner"),
    )
    .args([
        "snapshot",
        src_tmp.path().to_str().unwrap(),
        "--name",
        "ws-runner",
    ])
    .assert()
    .success();

    // --- hydrate with a FRESH cache (bytes come from the mock server, not cache),
    // env-only, runner keyspace ---
    let cache_hyd = TempDir::new().unwrap();
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    clw_runner_cmd(&server.uri(), home.path(), cache_hyd.path(), Some("runner"))
        .args(["hydrate", dest.to_str().unwrap(), "--name", "ws-runner"])
        .assert()
        .success();

    // Byte-identical file, executable mode, and preserved symlink.
    assert_eq!(
        std::fs::read(src_tmp.path().join("small.txt")).unwrap(),
        std::fs::read(dest.join("small.txt")).unwrap(),
        "small.txt must be byte-identical after a fresh-cache runner hydrate"
    );
    let exec_mode = std::fs::metadata(dest.join("exec.sh"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(exec_mode, 0o755, "exec.sh must hydrate with mode 0o755");
    assert_eq!(
        std::fs::read_link(dest.join("link.txt"))
            .unwrap()
            .to_str()
            .unwrap(),
        "small.txt",
        "link.txt symlink target must be preserved"
    );

    // --- KEYSPACE ISOLATION: the same name in the USER domain is NOT FOUND ---
    // CLW_REF_DOMAIN omitted ⇒ user keyspace `clw/ref/v1/`, disjoint from the
    // runner keyspace `clw/ref/runner/v1/` the ref was written under.
    let cache_user = TempDir::new().unwrap();
    let dest_user_tmp = TempDir::new().unwrap();
    let dest_user = dest_user_tmp.path().join("out");
    let out_user = clw_runner_cmd(&server.uri(), home.path(), cache_user.path(), None)
        .args([
            "hydrate",
            dest_user.to_str().unwrap(),
            "--name",
            "ws-runner",
        ])
        .output()
        .unwrap();
    assert!(
        !out_user.status.success(),
        "user-keyspace hydrate of a runner-keyspace ref must FAIL (disjoint keyspaces), got: {:?}",
        out_user.status.code()
    );
    let stderr_user = String::from_utf8_lossy(&out_user.stderr);
    assert!(
        stderr_user.contains("not found") || stderr_user.contains("ws-runner"),
        "user-keyspace miss must be reported as not-found for 'ws-runner', got:\n{stderr_user}"
    );
    assert!(
        !dest_user.exists(),
        "a not-found hydrate must not materialize a destination tree"
    );

    // Re-hydrate WITH the runner keyspace → succeeds again (the ref is only there).
    let cache_hyd2 = TempDir::new().unwrap();
    let dest2_tmp = TempDir::new().unwrap();
    let dest2 = dest2_tmp.path().join("out");
    clw_runner_cmd(
        &server.uri(),
        home.path(),
        cache_hyd2.path(),
        Some("runner"),
    )
    .args(["hydrate", dest2.to_str().unwrap(), "--name", "ws-runner"])
    .assert()
    .success();
    assert_eq!(
        std::fs::read(src_tmp.path().join("small.txt")).unwrap(),
        std::fs::read(dest2.join("small.txt")).unwrap(),
        "runner-keyspace re-hydrate must again be byte-identical"
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

// ---------------------------------------------------------------------------
// Power-user acceptance regressions (the v0.1.2 fixes).
//
// Each of the four power-user findings fixed in v0.1.2 is locked in here as a
// binary-level regression so it cannot silently come back. See the findings note
// `memory/clw-v011-poweruser-test-findings.md` and CHANGELOG [0.1.2]. Without
// these, the very bugs we shipped fixes for had ZERO automated coverage.
// ---------------------------------------------------------------------------

// Finding #1 — re-snapshot of an EXISTING name hits the server's create-only AC
// PUT (HTTP 409). The raw 409 used to surface verbatim with no guidance; v0.1.2
// maps it to an actionable message pointing at `clw rm --name <X> --yes`. This
// test uses the create-only AC mock so the real 409 path is exercised, asserts
// the actionable wording, and proves the documented workaround (rm then re-snapshot)
// actually works.
#[tokio::test]
async fn resnapshot_existing_name_409_is_actionable_and_rm_then_retry_works() {
    let server = MockServer::start().await;
    // Create-only AC: a second PUT of the same ref key returns 409, like the live store.
    let (_cas, ac) = mount_corelink_opts(&server, true, true).await;

    let cache = TempDir::new().unwrap();
    let ws = "e2e-resnapshot-409";

    // First snapshot of the name succeeds (AC PUT is a create → 201).
    let src1 = TempDir::new().unwrap();
    std::fs::write(src1.path().join("f.txt"), b"version one").unwrap();
    clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src1.path().to_str().unwrap(), "--name", ws])
        .assert()
        .success();
    assert!(ac.lock().unwrap().contains_key(&ref_key_hex(ws)));

    // Re-snapshot the SAME name with DIFFERENT content → the AC overwrite is 409.
    // It must fail with the actionable message, NOT a raw "409"/"transport error".
    let src2 = TempDir::new().unwrap();
    std::fs::write(src2.path().join("f.txt"), b"version two - changed").unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src2.path().to_str().unwrap(), "--name", ws])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "re-snapshot of an existing name must fail on the create-only store, got: {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("workspace '{ws}' already exists")),
        "409 must be reported as an actionable 'already exists', got:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("clw rm --name {ws} --yes")),
        "the message must point at the rm-then-retry workaround, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("transport error"),
        "the actionable 409 must NOT be mislabeled 'transport error', got:\n{stderr}"
    );

    // The documented workaround: rm the ref, then snapshot again → succeeds.
    clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", ws, "--yes"])
        .assert()
        .success();
    clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src2.path().to_str().unwrap(), "--name", ws])
        .assert()
        .success();
    assert!(
        ac.lock().unwrap().contains_key(&ref_key_hex(ws)),
        "after rm, a fresh snapshot of the name must re-create the ref"
    );
}

// Finding #2 — a relative symlink whose target uses `..` but resolves INSIDE the
// workspace (e.g. `bin/current -> ../releases/v1.txt`) used to be rejected as an
// escape. v0.1.2 allows intra-workspace `..` on both the producer and consumer.
// This asserts such a tree snapshots AND hydrates with the symlink target preserved
// verbatim and the linked file byte-identical.
#[tokio::test]
async fn intra_workspace_dotdot_symlink_roundtrips() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    // Tree: releases/v1.txt + bin/current -> ../releases/v1.txt (resolves in-tree).
    let src = TempDir::new().unwrap();
    std::fs::create_dir(src.path().join("releases")).unwrap();
    std::fs::create_dir(src.path().join("bin")).unwrap();
    std::fs::write(src.path().join("releases/v1.txt"), b"release one payload").unwrap();
    std::os::unix::fs::symlink("../releases/v1.txt", src.path().join("bin/current")).unwrap();

    let cache_snap = TempDir::new().unwrap();
    clw_cmd(&server.uri(), cache_snap.path())
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "e2e-dotdot-symlink",
        ])
        .assert()
        .success();

    let cache_hyd = TempDir::new().unwrap();
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    clw_cmd(&server.uri(), cache_hyd.path())
        .args([
            "hydrate",
            dest.to_str().unwrap(),
            "--name",
            "e2e-dotdot-symlink",
        ])
        .assert()
        .success();

    // Symlink target preserved VERBATIM (the `..` is kept, not normalized away).
    assert_eq!(
        std::fs::read_link(dest.join("bin/current"))
            .unwrap()
            .to_str()
            .unwrap(),
        "../releases/v1.txt",
        "intra-workspace `..` symlink target must hydrate verbatim"
    );
    // And it resolves to the byte-identical linked file.
    assert_eq!(
        std::fs::read(dest.join("bin/current")).unwrap(),
        b"release one payload",
        "the `..` symlink must resolve to the byte-identical in-tree file"
    );
}

// Finding #3 — operational/usage errors used to be mislabeled `transport error:`.
// `rm` with NEITHER --name nor --ref-key is a usage error (ClwError::Message); it
// must render the actionable text VERBATIM, never under a bogus transport prefix.
#[tokio::test]
async fn usage_error_is_not_mislabeled_transport_error() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--yes"]) // no --name, no --ref-key
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "rm with no selector must fail, got: {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--name") && stderr.contains("--ref-key"),
        "the usage error must name both selectors, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("transport error"),
        "a usage error must NOT be mislabeled 'transport error', got:\n{stderr}"
    );
}

// Finding #4 — a dangling/corrupt AC record (one `ls --all`/`prune` surface but
// whose name cannot be decoded) can't be targeted by `clw rm --name`. v0.1.2 adds
// `clw rm --ref-key <64hex>` to delete it by raw key. This plants a corrupt record
// and proves the raw-key delete removes exactly it.
#[tokio::test]
async fn rm_ref_key_deletes_a_dangling_corrupt_ref() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    // Keep one healthy ref around to prove the raw-key delete is surgical.
    let _src = snapshot_tiny(&server.uri(), cache.path(), "e2e-refkey-healthy");
    let healthy_key = ref_key_hex("e2e-refkey-healthy");

    // Plant a corrupt AC record under a real-shaped (64-hex) but undecodable key.
    let dangling = "ab".repeat(32);
    ac.lock()
        .unwrap()
        .insert(dangling.clone(), b"{ not a valid ref record".to_vec());

    // rm --ref-key deletes exactly the dangling key; --json reports it by ref_key.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "rm", "--ref-key", &dangling, "--yes"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "rm --ref-key must succeed, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        parsed["ref_key"], dangling,
        "by-key rm --json must report the deleted ref_key, got:\n{parsed}"
    );

    let guard = ac.lock().unwrap();
    assert!(
        !guard.contains_key(&dangling),
        "rm --ref-key must delete the dangling record"
    );
    assert!(
        guard.contains_key(&healthy_key),
        "rm --ref-key must NOT touch the unrelated healthy ref"
    );
}

// ---------------------------------------------------------------------------
// `clw run` EXIT-CODE CONTRACT (frozen cross-team seam — docs/CLI-SURFACE-EXIT-FREEZE.md).
//
// The runner DRIVES `clw run` and depends on this contract verbatim:
//   - child ran      → `clw run` exits the CHILD's code (passthrough; non-zero not cached),
//   - child KILLED   → POSIX 128+signal (e.g. SIGTERM → 143),
//   - child NEVER RAN (clw-internal failure: substrate unreachable, bad config, spawn
//                      failure) → the RESERVED 125, distinct from any child code, never cached.
// `run_memoization_and_exit_codes` already covers the 0/7 passthrough + non-zero-not-cached;
// these lock the 125 and 128+signal legs, which only the real spawned binary can prove.
// ---------------------------------------------------------------------------

/// A clw-internal failure where the child NEVER ran must exit the reserved 125 — NOT
/// a child code, NOT the generic exit-2. Driven by pointing at a dead endpoint so the
/// memoization substrate is unreachable; a side-effect marker proves the child never ran.
#[tokio::test]
async fn run_dead_endpoint_exits_reserved_125_child_never_ran() {
    let work = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let input = work.path().join("in.txt");
    std::fs::write(&input, b"declared input").unwrap();
    // Absolute marker the child would `touch` IF it ever ran.
    let marker = work.path().join("child_ran.marker");

    // http://127.0.0.1:1 — port 1 is not listening: connection refused (no mock).
    // A connection-refused IS retried (Transport errors walk the AIMD ladder), so this
    // is bounded by MAX_ATTEMPTS (~20s), not instant. Wrap it in a hard timeout so a
    // FUTURE retry-policy change that turned this into an unbounded hang fails loudly
    // instead of silently stalling CI.
    let mut cmd = clw_cmd("http://127.0.0.1:1", cache.path());
    cmd.current_dir(work.path()).args([
        "run",
        "--input",
        input.to_str().unwrap(),
        "--",
        "/bin/sh",
        "-c",
        &format!("touch {}", marker.to_str().unwrap()),
    ]);
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        tokio::task::spawn_blocking(move || cmd.output().unwrap()),
    )
    .await
    .expect("clw run against a dead endpoint must fail within the retry budget, not hang")
    .unwrap();

    assert_eq!(
        out.status.code(),
        Some(125),
        "a clw-internal failure (substrate unreachable) must exit the RESERVED 125, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !marker.exists(),
        "the child must NEVER have run on a 125 (child-never-ran) failure"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("child not run") || stderr.contains("internal"),
        "the 125 path must announce a clw-internal child-not-run failure, got:\n{stderr}"
    );
}

/// A command that cannot be spawned (ENOENT) is also "child never ran" → reserved 125,
/// even though the substrate (mock) is healthy. Distinguishes spawn-failure from a child
/// that ran and exited non-zero.
#[tokio::test]
async fn run_missing_command_exits_reserved_125_child_never_ran() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let work = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let input = work.path().join("in.txt");
    std::fs::write(&input, b"declared input").unwrap();

    let out = clw_cmd(&server.uri(), cache.path())
        .current_dir(work.path())
        .args([
            "run",
            "--input",
            input.to_str().unwrap(),
            "--",
            "/no/such/command/clw-e2e-does-not-exist",
        ])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(125),
        "an un-spawnable command (child never ran) must exit the RESERVED 125, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A child killed by a signal must propagate the POSIX 128+signal code (SIGTERM → 143),
/// not a truncated 255, and a signal-killed (non-zero) result must NOT be cached.
#[tokio::test]
async fn run_signal_killed_child_propagates_128_plus_signal_and_is_not_cached() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let work = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let input = work.path().join("in.txt");
    std::fs::write(&input, b"declared input").unwrap();

    let run_self_sigterm = || {
        clw_cmd(&server.uri(), cache.path())
            .current_dir(work.path())
            .args([
                "run",
                "--input",
                input.to_str().unwrap(),
                "--",
                "/bin/sh",
                "-c",
                "kill -TERM $$", // the shell signals itself → SIGTERM (15)
            ])
            .output()
            .unwrap()
    };

    // First run: cache miss, child runs, kills itself with SIGTERM → 128+15 = 143.
    let out1 = run_self_sigterm();
    assert_eq!(
        out1.status.code(),
        Some(143),
        "a SIGTERM-killed child must propagate 128+15=143, got: {:?}",
        out1.status.code()
    );
    assert!(
        String::from_utf8_lossy(&out1.stderr).contains("cache miss"),
        "the first run must be a cache miss"
    );

    // Second identical run: a signal-killed (non-zero) result is NOT cached, so it is
    // STILL a miss and STILL 143 — never replayed from cache as if it were a real exit.
    let out2 = run_self_sigterm();
    assert_eq!(
        out2.status.code(),
        Some(143),
        "a signal-killed result must not be cached → still 143 on re-run, got: {:?}",
        out2.status.code()
    );
    assert!(
        String::from_utf8_lossy(&out2.stderr).contains("cache miss"),
        "a signal-killed result must NOT be cached (second run is still a miss)"
    );
}

// ---------------------------------------------------------------------------
// Resource-abuse caps + non-retryable error tiers, surfaced through the BINARY.
//
// The cap LOGIC (enforce_resource_bounds) and the tier classification
// (retryable vs not) are unit/integration-tested; these prove the CLI surfaces
// them as a real user sees them — a clean exit-2, an explanatory message, NO
// panic, and NO partial destination — rather than crashing or hanging.
// ---------------------------------------------------------------------------

/// A hostile-but-well-formed manifest whose single chunk DECLARES a size above the
/// 1 TiB hydrate disk-fill cap must be refused up-front (exit 2), before any chunk is
/// fetched or any destination is written. Planted directly in the mock CAS + an AC ref
/// (the chunk is never fetched, so it need not exist). This is the anti disk-fill guard
/// a real user is protected by when hydrating an untrusted ref.
#[tokio::test]
async fn hydrate_refuses_manifest_declaring_over_total_bytes_cap() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();

    let huge: u64 = (1u64 << 40) + 1; // 1 TiB + 1 byte → over MAX_HYDRATE_TOTAL_BYTES
    let manifest = clw_types::Manifest {
        version: clw_types::MANIFEST_VERSION,
        total_size: huge,
        entries: vec![clw_types::Entry::File {
            path: "big.bin".to_string(),
            mode: 0o644,
            size: huge,
            chunks: vec![clw_types::ChunkRef {
                digest: clw_types::Digest::of_bytes(b"never fetched"),
                size: huge,
            }],
        }],
    };
    let manifest_bytes = manifest.canonical_bytes().unwrap();
    let root = clw_types::Digest::of_bytes(&manifest_bytes);
    cas.lock()
        .unwrap()
        .insert(root.to_hex(), manifest_bytes.clone());

    let name = "e2e-cap-totalbytes";
    let rec = clw_types::RefRecord {
        name: name.to_string(),
        root,
        manifest_size: manifest_bytes.len() as u64,
        created_at_unix: 0,
        parent: None,
        tool_version: "test".to_string(),
    };
    ac.lock()
        .unwrap()
        .insert(ref_key_hex(name), rec.to_bytes().unwrap());

    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", name])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "an over-cap manifest must be refused with exit 2, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("exceeding the hydrate cap") || stderr.contains("disk-fill"),
        "the refusal must explain the resource cap, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "a cap violation must be a clean error, not a panic, got:\n{stderr}"
    );
    assert!(
        !dest.exists(),
        "a refused hydrate must NOT materialize a partial destination tree"
    );
}

/// A non-retryable error tier (403 scope/forbidden here, representative of the
/// 402/403/410 family the client maps as non-retryable) must surface as a clean CLI
/// exit-2 that names the HTTP status — no panic, no retry storm/hang. The retryable-
/// vs-not classification itself is covered exhaustively in clw-integration-tests.
#[tokio::test]
async fn snapshot_surfaces_nonretryable_tier_403_cleanly() {
    let server = MockServer::start().await;
    // Every CAS PUT returns 403; HEAD is unmounted (wiremock 404 ⇒ "blob missing" ⇒
    // the client attempts the PUT, which 403s). 403 is non-retryable, so it fails fast.
    Mock::given(method("PUT"))
        .and(path_regex(format!("/v1/cas/{TEST_TENANT}/[a-f0-9]{{64}}$")))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"tier payload").unwrap();
    let cache = TempDir::new().unwrap();

    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "e2e-tier-403",
        ])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "a non-retryable 403 tier must fail the snapshot with exit 2, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("403"),
        "the CLI must surface the HTTP 403 status to the user, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "a tier error must be a clean error, not a panic, got:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Config RESOLUTION precedence + identity persistence, through the BINARY.
//
// Precedence (config.rs): CLI flag > CLW_* env > CORELINK_* legacy env > ~/.clw/
// config.toml > default. `config.rs` unit-tests the resolver in-process; these prove
// it holds for a SPAWNED `clw` (what a runner / a real user actually gets), observed
// via `auth whoami`, which prints the RESOLVED tenant offline (no network). Identity
// persistence (`auth login`/`logout`) is asserted against an isolated $HOME.
// ---------------------------------------------------------------------------

/// Build a `clw` invocation with a pristine identity environment: an isolated $HOME
/// (so no real ~/.clw leaks in) and every CLW_*/CORELINK_* var stripped. Tests then
/// set EXACTLY the layers they are exercising.
fn clw_clean_env(home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("clw").unwrap();
    cmd.env("HOME", home.to_str().unwrap())
        .env_remove("CLW_ENDPOINT")
        .env_remove("CLW_TENANT")
        .env_remove("CLW_TOKEN")
        .env_remove("CLW_REF_DOMAIN")
        .env_remove("CLW_CACHE_DIR")
        .env_remove("CORELINK_ENDPOINT")
        .env_remove("CORELINK_TENANT")
        .env_remove("CORELINK_TOKEN");
    cmd
}

/// A `--tenant` FLAG must beat `CLW_TENANT` env. whoami prints the resolved tenant.
#[test]
fn config_precedence_flag_tenant_beats_env() {
    let home = TempDir::new().unwrap();
    let out = clw_clean_env(home.path())
        .env("CLW_TENANT", "env-tenant")
        .env("CLW_TOKEN", "tok")
        .args(["--tenant", "flag-tenant", "auth", "whoami"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "whoami must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("flag-tenant"),
        "the --tenant flag must win over CLW_TENANT, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("env-tenant"),
        "the env tenant must NOT be the resolved value when the flag is set, got:\n{stdout}"
    );
}

/// `CLW_TENANT` env must beat the `~/.clw/config.toml` file value.
#[test]
fn config_precedence_env_tenant_beats_file() {
    let home = TempDir::new().unwrap();
    // Seed a config file with a DIFFERENT tenant + a token (so resolve succeeds).
    let clw_dir = home.path().join(".clw");
    std::fs::create_dir_all(&clw_dir).unwrap();
    std::fs::write(
        clw_dir.join("config.toml"),
        "tenant = \"file-tenant\"\ntoken = \"filetok\"\n",
    )
    .unwrap();

    let out = clw_clean_env(home.path())
        .env("CLW_TENANT", "env-tenant")
        .args(["auth", "whoami"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "whoami must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("env-tenant"),
        "CLW_TENANT env must win over the config file tenant, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("file-tenant"),
        "the file tenant must NOT be resolved when CLW_TENANT is set, got:\n{stdout}"
    );
}

/// The `CORELINK_*` legacy aliases are accepted as a fallback, AND `CLW_*` wins when
/// both are present (the runner injects CORELINK_*; an explicit CLW_* must override).
#[test]
fn config_corelink_alias_is_accepted_and_clw_wins() {
    // (a) CORELINK_TENANT alone is honored.
    let home_a = TempDir::new().unwrap();
    let out_a = clw_clean_env(home_a.path())
        .env("CORELINK_TENANT", "corelink-tenant")
        .env("CORELINK_TOKEN", "ctok")
        .args(["auth", "whoami"])
        .output()
        .unwrap();
    assert_eq!(
        out_a.status.code(),
        Some(0),
        "whoami must exit 0 (CORELINK_*)"
    );
    assert!(
        String::from_utf8_lossy(&out_a.stdout).contains("corelink-tenant"),
        "CORELINK_TENANT must be accepted as a fallback, got:\n{}",
        String::from_utf8_lossy(&out_a.stdout)
    );

    // (b) When BOTH are set, CLW_TENANT wins over CORELINK_TENANT.
    let home_b = TempDir::new().unwrap();
    let out_b = clw_clean_env(home_b.path())
        .env("CLW_TENANT", "clw-tenant")
        .env("CLW_TOKEN", "tok")
        .env("CORELINK_TENANT", "corelink-tenant")
        .env("CORELINK_TOKEN", "ctok")
        .args(["auth", "whoami"])
        .output()
        .unwrap();
    assert_eq!(
        out_b.status.code(),
        Some(0),
        "whoami must exit 0 (both set)"
    );
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);
    assert!(
        stdout_b.contains("clw-tenant") && !stdout_b.contains("corelink-tenant"),
        "CLW_TENANT must win over CORELINK_TENANT when both are set, got:\n{stdout_b}"
    );
}

/// `auth login` persists the resolved identity to ~/.clw/config.toml at 0600 and NEVER
/// echoes the token value (only `<set>` presence). Mirrors a real user logging in.
#[test]
fn auth_login_persists_0600_config_without_echoing_token() {
    let home = TempDir::new().unwrap();
    let out = clw_clean_env(home.path())
        .env("CLW_ENDPOINT", "https://example.com")
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", SECRET_TOKEN)
        .args(["auth", "login"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "auth login must exit 0");

    let config_path = home.path().join(".clw").join("config.toml");
    assert!(config_path.exists(), "login must write the config file");
    let mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "login must write config.toml 0600 (holds the PAT)"
    );

    // The token VALUE must never appear on either stream — only the path + <set>.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout.contains(SECRET_TOKEN) && !stderr.contains(SECRET_TOKEN),
        "login must NEVER echo the token value"
    );
    assert!(
        stderr.contains(TEST_TENANT) || stdout.contains(TEST_TENANT),
        "login should report the tenant it persisted, got stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// `auth logout` removes the token line while KEEPING other keys, and never echoes the
/// token. A real user logging out must scrub the on-disk PAT but not lose other config.
#[test]
fn auth_logout_removes_token_keeps_other_keys() {
    let home = TempDir::new().unwrap();
    let clw_dir = home.path().join(".clw");
    std::fs::create_dir_all(&clw_dir).unwrap();
    let config_path = clw_dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!("endpoint = \"https://example.com\"\ntenant = \"{TEST_TENANT}\"\ntoken = \"{SECRET_TOKEN}\"\n"),
    )
    .unwrap();

    let out = clw_clean_env(home.path())
        .args(["auth", "logout"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "auth logout must exit 0");

    let after = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        !after.contains(SECRET_TOKEN),
        "logout must remove the token from the config file"
    );
    assert!(
        after.contains(TEST_TENANT) && after.contains("endpoint"),
        "logout must KEEP non-token keys (tenant, endpoint), got:\n{after}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout.contains(SECRET_TOKEN) && !stderr.contains(SECRET_TOKEN),
        "logout must NEVER echo the token value"
    );
}

// ---------------------------------------------------------------------------
// Coverage-fill (common single-command paths) + a full real-user JOURNEY.
//
// The audit found a handful of frequently-used paths with no binary test
// (`ls --name`, `hydrate` into a non-empty dest, `init --force`) and NO single
// scenario chaining the whole lifecycle as a real user actually drives it. These
// close that, the last of the binary-E2E gap series.
// ---------------------------------------------------------------------------

/// `clw ls --name <NAME>` (the single-ref lookup, distinct from `--all`) must report
/// the snapshotted ref by name and exit 0.
#[tokio::test]
async fn ls_name_lists_the_single_ref() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-ls-name";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["ls", "--name", ws])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "ls --name must exit 0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(ws),
        "ls --name must report the looked-up ref '{ws}', got:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `clw hydrate` into a NON-EMPTY destination must refuse (exit 2) and leave the
/// pre-existing content untouched — never clobber a user's directory.
#[tokio::test]
async fn hydrate_into_nonempty_dest_refuses_and_preserves_existing() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-hyd-nonempty";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);

    // A destination dir that already holds a file the user cares about.
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("precious.txt"), b"do not clobber me").unwrap();

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", ws])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "hydrate into a non-empty dest must refuse with exit 2, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    // The pre-existing file is byte-for-byte intact.
    assert_eq!(
        std::fs::read(dest.join("precious.txt")).unwrap(),
        b"do not clobber me",
        "a refused hydrate must NOT touch pre-existing destination content"
    );
}

/// `clw init --force` overwrites an existing config (the default `init` refuses), and
/// the rewritten file stays 0600.
#[test]
fn init_force_overwrites_existing_config_keeping_0600() {
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let run_init = |args: &[&str], token: &str| {
        Command::cargo_bin("clw")
            .unwrap()
            .env("HOME", home.path())
            .env("CLW_ENDPOINT", "https://example.com")
            .env("CLW_TENANT", TEST_TENANT)
            .env("CLW_TOKEN", token)
            .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
            .args(["init"].iter().chain(args.iter()))
            .output()
            .unwrap()
    };

    // First init writes the file; a second WITHOUT --force refuses (regression guard).
    assert_eq!(run_init(&[], "tok-one").status.code(), Some(0));
    assert!(
        !run_init(&[], "tok-two").status.success(),
        "re-init without --force must refuse"
    );

    // init --force succeeds and rewrites the file, still 0600.
    let forced = run_init(&["--force"], "tok-three");
    assert_eq!(
        forced.status.code(),
        Some(0),
        "init --force must overwrite an existing config, got: {:?}\nstderr={}",
        forced.status.code(),
        String::from_utf8_lossy(&forced.stderr)
    );
    let config_path = home.path().join(".clw").join("config.toml");
    let mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "init --force must keep config.toml 0600");
    // No token value is echoed on the forced overwrite either.
    assert!(
        !String::from_utf8_lossy(&forced.stdout).contains("tok-three")
            && !String::from_utf8_lossy(&forced.stderr).contains("tok-three"),
        "init --force must not echo the token"
    );
}

/// A full real-user JOURNEY in ONE scenario: save work, verify it, detect a local
/// change, reuse via a memoized run, restore it elsewhere, then clean up — every stage
/// chained through the real binary against one backend, asserting the state transitions
/// a power user actually depends on end-to-end.
#[tokio::test]
async fn full_lifecycle_journey_save_verify_run_hydrate_remove() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-journey";

    // 1. A real workspace tree (multi-type) and SNAPSHOT it.
    let src = TempDir::new().unwrap();
    build_source_tree(src.path());
    clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", ws])
        .assert()
        .success();
    assert!(
        ac.lock().unwrap().contains_key(&ref_key_hex(ws)),
        "snapshot must have written the ref"
    );

    // 2. STATUS is clean right after a snapshot.
    clw_cmd(&server.uri(), cache.path())
        .args(["status", src.path().to_str().unwrap(), "--name", ws])
        .assert()
        .success();

    // 3. LS --name surfaces the ref by name.
    let ls = clw_cmd(&server.uri(), cache.path())
        .args(["ls", "--name", ws])
        .output()
        .unwrap();
    assert!(
        ls.status.success() && String::from_utf8_lossy(&ls.stdout).contains(ws),
        "ls --name must list the just-snapshotted ref"
    );

    // 4. A local edit makes STATUS dirty (exit 1) and names the changed file.
    std::fs::write(src.path().join("small.txt"), b"locally edited").unwrap();
    let dirty = clw_cmd(&server.uri(), cache.path())
        .args(["status", src.path().to_str().unwrap(), "--name", ws])
        .output()
        .unwrap();
    assert_eq!(
        dirty.status.code(),
        Some(1),
        "a local edit must make status dirty (exit 1)"
    );
    assert!(String::from_utf8_lossy(&dirty.stdout).contains("small.txt"));

    // 5. A memoized RUN: first miss, second hit, identical replayed stdout.
    let run = |label: &str| {
        clw_cmd(&server.uri(), cache.path())
            .current_dir(src.path())
            .args([
                "run",
                "--input",
                "small.txt",
                "--",
                "/bin/sh",
                "-c",
                "echo journey-output",
            ])
            .output()
            .unwrap_or_else(|_| panic!("run {label} must spawn"))
    };
    let r1 = run("miss");
    assert!(
        r1.status.success() && String::from_utf8_lossy(&r1.stderr).contains("cache miss"),
        "first run must be a cache miss"
    );
    let r2 = run("hit");
    assert!(
        r2.status.success() && String::from_utf8_lossy(&r2.stderr).contains("cache hit"),
        "second identical run must be a cache hit"
    );
    assert_eq!(
        r1.stdout, r2.stdout,
        "replayed stdout must be byte-identical"
    );

    // 6. HYDRATE the saved ref to a fresh destination — byte-identical restore of the
    //    ORIGINAL snapshot (not the local edit), with a fresh cache (bytes off server).
    let cache_hyd = TempDir::new().unwrap();
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("restored");
    clw_cmd(&server.uri(), cache_hyd.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", ws])
        .assert()
        .success();
    assert_eq!(
        std::fs::read(dest.join("small.txt")).unwrap(),
        b"hello e2e test",
        "hydrate must restore the SNAPSHOTTED content, not the later local edit"
    );

    // 7. RM the ref, then LS --all no longer lists it: the workspace is cleaned up.
    clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", ws, "--yes"])
        .assert()
        .success();
    assert!(!ac.lock().unwrap().contains_key(&ref_key_hex(ws)));
    let ls_after = clw_cmd(&server.uri(), cache.path())
        .args(["ls", "--all"])
        .output()
        .unwrap();
    assert!(
        ls_after.status.success() && !String::from_utf8_lossy(&ls_after.stdout).contains(ws),
        "after rm, ls --all must no longer list '{ws}'"
    );
}
