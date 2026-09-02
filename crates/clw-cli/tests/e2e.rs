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
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child as StdChild, Command as StdCommand, ExitStatus as StdExitStatus};
use std::sync::{Arc, Mutex};

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path, path_regex};
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

/// Base builder for the real `clw` binary with a CLEAN credential environment.
///
/// Every child `clw` must start from a known-empty identity: the test process can
/// inherit an ambient credential (a developer's exported `CLW_*`, or the C2c
/// credential-broker trio `CLW_CRED_TICKET`/`CLW_LEASE_ID`/`CLW_FABRIC_ENDPOINT`
/// that a `runs-on: corelink` fabric box injects into every job's env-0). If that
/// leaks into the child, `resolve` takes the broker path (token → redeem) instead
/// of the static-token path the tests set up, and both the happy-path and the
/// fail-closed broker tests read the wrong state. Strip the FULL namespace here so
/// each caller then sets ONLY the identity it intends (the broker tests set the
/// trio themselves, on top of this clean base).
fn clw_bin() -> Command {
    let mut cmd = Command::cargo_bin("clw").unwrap();
    for &v in clw_e2e_evidence::E2E_ENV_HYGIENE_VARS {
        cmd.env_remove(v);
    }
    cmd
}

fn clw_cmd(endpoint: &str, cache_dir: &std::path::Path) -> Command {
    let mut cmd = clw_bin();
    cmd.env("CLW_ENDPOINT", endpoint)
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache_dir.to_str().unwrap());
    cmd
}

/// Owns a process-group test's direct child so every panic path terminates the
/// exact group and reaps clw. It stays armed until cleanup has confirmed that
/// the group no longer exists; `Drop` is deliberately best-effort and never
/// panics while unwinding a test failure.
struct ProcessGroupCleanup {
    pgid: u32,
    child: StdChild,
    child_reaped: bool,
    armed: bool,
}

impl ProcessGroupCleanup {
    fn new(child: StdChild) -> Self {
        Self {
            pgid: child.id(),
            child,
            child_reaped: false,
            armed: true,
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<StdExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.child_reaped = true;
        }
        Ok(status)
    }

    fn signal_group(&self, signal: &str) -> std::io::Result<StdExitStatus> {
        StdCommand::new("/bin/kill")
            .args([
                format!("-{signal}"),
                "--".to_string(),
                format!("-{}", self.pgid),
            ])
            .status()
    }

    fn group_is_gone(&self) -> bool {
        self.signal_group("0")
            .map(|status| !status.success())
            .unwrap_or(false)
    }

    /// Send SIGKILL, wait for the group to disappear, and reap direct clw.
    /// A non-zero `kill -KILL` is acceptable when the group already exited;
    /// `kill -0` distinguishes that cleaned state from a live group.
    fn cleanup(&mut self) -> Result<(), String> {
        let kill_status = self
            .signal_group("KILL")
            .map_err(|error| format!("spawn /bin/kill -KILL: {error}"))?;

        if !self.child_reaped {
            match self.child.try_wait() {
                Ok(Some(_)) => self.child_reaped = true,
                Ok(None) => {
                    let kill_error = self.child.kill().err();
                    let wait_result = self.child.wait();
                    if wait_result.is_ok() {
                        self.child_reaped = true;
                    } else if let Err(wait_error) = wait_result {
                        return Err(format!("child kill={kill_error:?}, wait={wait_error}"));
                    }
                }
                Err(error) => {
                    let _ = self.child.kill();
                    let wait_result = self.child.wait();
                    if wait_result.is_ok() {
                        self.child_reaped = true;
                    } else {
                        return Err(format!("child try_wait={error}, wait={wait_result:?}"));
                    }
                }
            }
        }

        // Reap clw before probing the group: a zombie direct child still makes
        // `kill -0 -PGID` report a live group even though SIGKILL was delivered.
        let mut group_gone = !kill_status.success() && self.group_is_gone();
        if kill_status.success() {
            for _ in 0..100 {
                if self.group_is_gone() {
                    group_gone = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        if !group_gone {
            return Err(format!(
                "process group {} still exists after SIGKILL (kill status {kill_status:?})",
                self.pgid
            ));
        }
        if !self.child_reaped {
            return Err(format!("direct clw child {} was not reaped", self.pgid));
        }

        self.armed = false;
        Ok(())
    }
}

impl Drop for ProcessGroupCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = self.signal_group("KILL");
        if !self.child_reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
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
    let mut cmd = clw_bin();
    // `clw_bin()` already stripped the full CLW_*/CORELINK_* + broker namespace, so
    // this run is genuinely env-only (no inherited config-file fallback). Inject the
    // runner's full identity purely via env (no flags, no file).
    cmd.env("HOME", home.to_str().unwrap())
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

    // Validate the field VALUES against the KNOWN fixture (one file, `data.txt`,
    // holding the 16-byte string "json report test") — not merely that the fields
    // exist. A regression reporting the wrong file/byte/chunk counts must fail here.
    assert_eq!(
        parsed.get("files").and_then(|v| v.as_u64()),
        Some(1),
        "JSON report must count exactly the 1 fixture file, got: {parsed}"
    );
    assert_eq!(
        parsed.get("bytes_total").and_then(|v| v.as_u64()),
        Some(16), // "json report test".len()
        "JSON report bytes_total must equal the fixture's 16 bytes, got: {parsed}"
    );
    let chunks_total = parsed
        .get("chunks_total")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let chunks_uploaded = parsed
        .get("chunks_uploaded")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    assert!(
        chunks_total >= 1,
        "a non-empty file must produce at least one chunk, got: {parsed}"
    );
    assert_eq!(
        chunks_uploaded, chunks_total,
        "a fresh snapshot must upload EVERY chunk (none deduped), got: {parsed}"
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
    let out = clw_bin()
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
// Test 5b/5c (M12): a SET-BUT-EMPTY `CLW_*` var is ABSENT, not a value
//
// A missing secret in CI materialises as an env var that is set but EMPTY. clap
// hands that to the program as `Some("")`, which used to short-circuit config
// resolution — so `CLW_TOKEN=""` produced a REAL request carrying a bare
// `Bearer ` header ("authentication failed") instead of the actionable "missing
// token" with NO request at all. These drive the real binary and assert the
// message, the exit code, AND that the mock server saw nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_clw_token_is_missing_token_exit_2_and_makes_no_request() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = clw_bin()
        .env("HOME", home.path()) // empty HOME => no ~/.clw/config.toml fallback
        .env("CLW_ENDPOINT", server.uri())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", "") // set BUT EMPTY — the CI missing-secret shape
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args(["ls", "--all"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "an empty CLW_TOKEN must exit 2 (config error), got: {:?}",
        out.status.code()
    );
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("missing token: set --token, CLW_TOKEN/CORELINK_TOKEN"),
        "stderr must carry the actionable missing-token message, got: {stderr_str}"
    );
    assert!(
        !stderr_str.to_lowercase().contains("authentication failed"),
        "an empty token must never reach the network as a bare Bearer, got: {stderr_str}"
    );

    // The decisive assertion: NO request was ever made.
    let seen = server.received_requests().await.unwrap();
    assert!(
        seen.is_empty(),
        "an empty CLW_TOKEN must fail BEFORE any network request, got {} request(s)",
        seen.len()
    );
}

#[tokio::test]
async fn empty_clw_tenant_is_missing_tenant_exit_2_and_makes_no_request() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = clw_bin()
        .env("HOME", home.path())
        .env("CLW_ENDPOINT", server.uri())
        .env("CLW_TENANT", "") // set BUT EMPTY
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args(["ls", "--all"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "an empty CLW_TENANT must exit 2 (config error), got: {:?}",
        out.status.code()
    );
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_str.contains("missing tenant: set --tenant, CLW_TENANT/CORELINK_TENANT"),
        "stderr must carry the actionable missing-tenant message, got: {stderr_str}"
    );
    let seen = server.received_requests().await.unwrap();
    assert!(
        seen.is_empty(),
        "an empty CLW_TENANT must fail BEFORE any network request, got {} request(s)",
        seen.len()
    );
}

#[test]
fn empty_clw_endpoint_falls_back_to_the_default_endpoint() {
    // `doctor` is offline and echoes the RESOLVED endpoint, so it black-boxes the
    // fallback: pre-fix an empty CLW_ENDPOINT resolved to "" (reported as an
    // "endpoint is empty" problem / rejected downstream as plaintext).
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    let out = clw_bin()
        .env("HOME", home.path())
        .env("CLW_ENDPOINT", "") // set BUT EMPTY
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        .args(["doctor"])
        .output()
        .unwrap();

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout_str.contains("https://corelink-api.humangr.com"),
        "an empty CLW_ENDPOINT must resolve to the default endpoint, got:\n{stdout_str}"
    );
    assert!(
        !stdout_str.contains("endpoint is empty"),
        "an empty CLW_ENDPOINT must not survive resolution, got:\n{stdout_str}"
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

    let out = clw_bin()
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

    let out = clw_bin()
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
        clw_bin()
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

    let out = clw_bin()
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
// M8: endpoint userinfo must NEVER leak in doctor / auth whoami (text + --json).
//
// A misconfigured `CLW_ENDPOINT=https://user:s3cret@host` smuggles a secret in
// the URL userinfo. doctor/whoami ECHO the resolved endpoint, and doctor output
// is meant to be pasted into support tickets — so the userinfo must be stripped
// to `<redacted>` on every output surface. These drive the REAL binary.
// ---------------------------------------------------------------------------

/// An endpoint carrying an embedded `user:pass@` credential whose secret must
/// never survive to any user-facing output surface.
const ENDPOINT_WITH_USERINFO: &str =
    "https://alice:s3cret-endpoint-DO-NOT-LEAK@host.example.com/v1";
/// The embedded secret + userinfo fragments that must never appear verbatim.
const ENDPOINT_SECRET: &str = "s3cret-endpoint-DO-NOT-LEAK";

#[test]
fn doctor_redacts_endpoint_userinfo_text_and_json() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    for args in [vec!["doctor"], vec!["--json", "doctor"]] {
        let out = clw_bin()
            .env("HOME", home.path())
            .env("CLW_ENDPOINT", ENDPOINT_WITH_USERINFO)
            .env("CLW_TENANT", TEST_TENANT)
            .env("CLW_TOKEN", SECRET_TOKEN)
            .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
            .args(&args)
            .output()
            .unwrap();

        assert_eq!(out.status.code(), Some(0), "doctor {args:?} must exit 0");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        assert!(
            !stdout.contains(ENDPOINT_SECRET) && !stderr.contains(ENDPOINT_SECRET),
            "doctor {args:?} must NOT leak the endpoint secret, got:\n{stdout}{stderr}"
        );
        // The `user:pass@` userinfo (host-local user part included) must be gone.
        assert!(
            !stdout.contains("alice:") && !stdout.contains("alice@"),
            "doctor {args:?} must strip the endpoint userinfo, got:\n{stdout}"
        );
        assert!(
            stdout.contains("<redacted>@host.example.com"),
            "doctor {args:?} must show the redacted endpoint, got:\n{stdout}"
        );
    }
}

#[test]
fn whoami_redacts_endpoint_userinfo_text_and_json() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();

    for args in [vec!["auth", "whoami"], vec!["--json", "auth", "whoami"]] {
        let out = clw_bin()
            .env("HOME", home.path())
            .env("CLW_ENDPOINT", ENDPOINT_WITH_USERINFO)
            .env("CLW_TENANT", TEST_TENANT)
            .env("CLW_TOKEN", SECRET_TOKEN)
            .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
            .args(&args)
            .output()
            .unwrap();

        assert_eq!(out.status.code(), Some(0), "whoami {args:?} must exit 0");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        assert!(
            !stdout.contains(ENDPOINT_SECRET) && !stderr.contains(ENDPOINT_SECRET),
            "whoami {args:?} must NOT leak the endpoint secret, got:\n{stdout}{stderr}"
        );
        assert!(
            !stdout.contains("alice:") && !stdout.contains("alice@"),
            "whoami {args:?} must strip the endpoint userinfo, got:\n{stdout}"
        );
        assert!(
            stdout.contains("<redacted>@host.example.com"),
            "whoami {args:?} must show the redacted endpoint, got:\n{stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// LOW (docs/CLI-SURFACE-EXIT-FREEZE.md): a PANIC during `clw run` setup/dispatch
// is a clw-INTERNAL "child never ran" failure — it must exit the RESERVED 125,
// NOT the default panic exit 101 (which the runner mis-classifies). The binary
// carries a debug-only, env-gated panic injection on the run dispatch path so
// this contract has a RED-on-revert regression (revert the main.rs panic hook
// and the exit becomes 101).
// ---------------------------------------------------------------------------

#[test]
fn run_dispatch_panic_exits_reserved_125_not_101() {
    let home = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();
    let input = TempDir::new().unwrap();

    let out = clw_bin()
        .env("HOME", home.path())
        // A resolvable identity so config resolution SUCCEEDS and we reach the
        // dispatch path (no network call is made — the panic fires first).
        .env("CLW_ENDPOINT", "https://host.example.com")
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache_dir.path().to_str().unwrap())
        // Debug-only seam: force a clw-internal panic on the run dispatch path.
        .env("CLW_TEST_INTERNAL_PANIC", "1")
        .args([
            "run",
            "--input",
            input.path().to_str().unwrap(),
            "--",
            "true",
        ])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(125),
        "a clw-internal panic on the run dispatch path must exit the reserved 125, not 101, got: {:?}",
        out.status.code()
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

    let out = clw_bin()
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

/// A bare `clw ls` must fail with the usage guidance that names both supported
/// modes. Keep the identity configured so this reaches the subcommand's own
/// argument validation instead of stopping at config resolution.
#[test]
fn ls_without_selector_exits_2_and_directs_to_name_or_all() {
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let out = clw_bin()
        .env("HOME", home.path())
        .env("CLW_ENDPOINT", "https://example.com")
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["ls"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "bare ls must exit 2, got {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--name") && stderr.contains("--all"),
        "bare ls must direct the user to both --name and --all, got:\n{stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "bare ls guidance belongs on stderr, not stdout"
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

    let out = clw_bin()
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
fn uninstall_human_dry_run_labels_paths_preserves_footprint_and_redacts_token() {
    let (home, cache) = seed_clw_footprint();
    let clw_dir = home.path().join(".clw");
    let config_path = clw_dir.join("config.toml");

    let out = clw_bin()
        .env("HOME", home.path())
        .env_remove("CLW_TENANT")
        .env_remove("CLW_TOKEN")
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["uninstall", "--dry-run"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "human uninstall --dry-run must exit 0; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.trim().is_empty(),
        "human uninstall --dry-run should report its plan on stderr, got stdout:\n{stdout}"
    );
    assert!(
        stderr.contains("would remove"),
        "dry-run stderr must identify planned removal, got:\n{stderr}"
    );
    assert!(
        stderr.contains("config dir") && stderr.contains("cache dir"),
        "dry-run stderr must label both config and cache targets, got:\n{stderr}"
    );
    assert!(
        !stdout.contains(SECRET_TOKEN) && !stderr.contains(SECRET_TOKEN),
        "human uninstall --dry-run must NEVER print the token"
    );

    // A dry-run is observational: retain both sensitive targets and their
    // contents, including the credential itself.
    assert!(clw_dir.exists(), "dry-run must NOT remove ~/.clw");
    assert!(
        cache.path().exists(),
        "dry-run must NOT remove the cache dir"
    );
    assert!(
        std::fs::read_to_string(config_path)
            .unwrap()
            .contains(SECRET_TOKEN),
        "dry-run must leave the credential footprint unchanged"
    );
}

#[test]
fn uninstall_without_yes_noninteractive_errors_and_deletes_nothing() {
    let (home, cache) = seed_clw_footprint();
    let clw_dir = home.path().join(".clw");

    // No TTY in the test harness (stdin is not a terminal) and no --yes: the
    // binary must REFUSE and delete nothing, in both plain and --json mode.
    for json in [false, true] {
        let mut cmd = clw_bin();
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

    let out = clw_bin()
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
        let out = clw_bin()
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

    // FIX 3: rm of a name that NEVER existed must still exit 0 (idempotent) but be
    // HONEST — `deleted:false` + a "nothing to delete" human line, not a false
    // "deleted". The first rm above deleted the only ref, so this name is absent.
    let out_absent = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "rm", "--name", "e2e-rm-never-existed", "--yes"])
        .output()
        .unwrap();
    assert_eq!(
        out_absent.status.code(),
        Some(0),
        "rm of an absent ref must still exit 0 (idempotent)"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out_absent.stdout).trim()).unwrap();
    assert_eq!(
        parsed["deleted"],
        false,
        "rm of a never-existed ref must report deleted:false, got:\n{}",
        String::from_utf8_lossy(&out_absent.stdout)
    );
    // Human mode says "nothing to delete" rather than a false "deleted".
    let out_absent_human = clw_cmd(&server.uri(), cache.path())
        .args(["rm", "--name", "e2e-rm-never-existed", "--yes"])
        .output()
        .unwrap();
    assert_eq!(out_absent_human.status.code(), Some(0));
    let stdout_human = String::from_utf8_lossy(&out_absent_human.stdout);
    assert!(
        stdout_human.contains("nothing to delete"),
        "human rm of an absent ref must say 'nothing to delete', got:\n{stdout_human}"
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

    // The tenant AC keyspace is SHARED with native/Bazel ActionCache, so enumeration
    // surfaces THREE non-ref cases. Inject one of each and assert the classifier keeps
    // them apart — the good ref must STILL list through all of it:
    //   (a) a foreign NON-JSON AC value (a native ActionCache blob) → benign, counted
    //       as `foreign_ac_entries`, NOT reported as a corrupt ref (the live-observed
    //       `ac-roundtrip-value-*` case that used to flood the listing with warnings);
    //   (b) a genuinely corrupt ref: valid JSON but NOT a RefRecord/RunRecord shape →
    //       reported as `skipped` and named by its ref key.
    let foreign_key = "ee".repeat(32); // 64 hex, a non-JSON native AC value
    ac.lock().unwrap().insert(
        foreign_key.clone(),
        b"ac-roundtrip-value-000012e618bfbfe8288c5064".to_vec(),
    );
    let corrupt_key = "ff".repeat(32); // 64 hex, valid JSON but wrong shape = corrupt ref
    ac.lock()
        .unwrap()
        .insert(corrupt_key.clone(), br#"{"unexpected":"shape"}"#.to_vec());

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
        parsed["foreign_ac_entries"], 1,
        "the non-JSON native AC value must be counted foreign, NOT skipped, got:\n{stdout}"
    );
    assert_eq!(
        parsed["skipped"], 1,
        "only the valid-JSON-wrong-shape record is a corrupt ref, got:\n{stdout}"
    );
    assert_eq!(
        parsed["errors"][0]["ref_key"], corrupt_key,
        "the skipped record must be named by its (corrupt) ref key, not the foreign one"
    );

    // Remove both non-ref records, then rm the good ref: ls --all no longer lists it.
    ac.lock().unwrap().remove(&foreign_key);
    ac.lock().unwrap().remove(&corrupt_key);
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

// `clw erase` WITHOUT --yes is rail #1 of the most destructive verb: it MUST be a
// dry run — report the blast radius and delete NOTHING. A regression that made
// erase delete-on-default would be catastrophic and, until this test, invisible.
#[tokio::test]
async fn erase_dry_run_default_reports_blast_radius_and_deletes_nothing() {
    let server = MockServer::start().await;
    let (cas, ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let _s1 = snapshot_tiny(&server.uri(), cache.path(), "e2e-erase-dryrun-1");
    let _s2 = snapshot_tiny(&server.uri(), cache.path(), "e2e-erase-dryrun-2");
    let refs_before = ac.lock().unwrap().len();
    let blobs_before = cas.lock().unwrap().len();
    assert!(refs_before > 0 && blobs_before > 0);

    // Human mode: announces a DRY RUN, exits 0, deletes nothing.
    let out = clw_cmd(&server.uri(), cache.path())
        .arg("erase")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "dry-run erase exits 0, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("DRY RUN"),
        "erase without --yes must announce a dry run, got:\n{stdout}"
    );
    assert_eq!(
        ac.lock().unwrap().len(),
        refs_before,
        "dry-run erase must delete NO ref"
    );
    assert_eq!(
        cas.lock().unwrap().len(),
        blobs_before,
        "dry-run erase must delete NO blob"
    );

    // --json mode: the documented dry-run shape, and STILL deletes nothing.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "erase"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("erase --json must emit valid JSON");
    assert_eq!(v["dry_run"], serde_json::json!(true));
    assert_eq!(v["refs"], serde_json::json!(refs_before));
    assert_eq!(v["blobs"], serde_json::json!(blobs_before));
    assert_eq!(
        ac.lock().unwrap().len(),
        refs_before,
        "json dry-run still deletes nothing"
    );
    assert_eq!(cas.lock().unwrap().len(), blobs_before);
}

// The snapshot secret-scan is user-facing SAFETY output: a secret-looking file is
// UPLOADED (clw never silently drops data) but the CLI must WARN about it on stderr.
// The library collects `secret_warnings`, but nothing asserted the binary actually
// PRINTS them — a refactor could drop the whole warning block and every other test
// stays green. This locks the observable warning.
#[tokio::test]
async fn snapshot_warns_about_secret_looking_files_on_stderr() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("app.rs"), b"fn main() {}").unwrap();
    // A secret-looking file (captured, not ignored — no .gitignore here).
    std::fs::write(src.path().join(".env"), b"API_KEY=super-secret\n").unwrap();

    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "e2e-secret-warn",
        ])
        .output()
        .unwrap();

    // Advisory, NOT blocking: the snapshot still succeeds (the file IS uploaded).
    assert!(
        out.status.success(),
        "the secret-scan is advisory — snapshot must still succeed, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("WARNING") && stderr.contains("secret-looking"),
        "the CLI must warn about the secret-looking file on stderr, got:\n{stderr}"
    );
    assert!(
        stderr.contains(".env"),
        "the warning must name the offending file, got:\n{stderr}"
    );
    assert!(
        stderr.contains(".clwignore"),
        "the warning must advise adding it to .clwignore, got:\n{stderr}"
    );
}

// prune's mid-sweep failure path (`failed_at` + resumable, non-zero exit) had zero
// coverage — a destructive partial-delete that must NOT be reported as success. A
// live snapshot gives a non-empty live set (so the empty-live-set fail-safe doesn't
// trip); one planted orphan's DELETE is forced to 500 via a higher-priority mock.
#[tokio::test]
async fn prune_reports_failed_at_and_does_not_sweep_on_a_mid_sweep_delete_failure() {
    let server = MockServer::start().await;
    let (cas, _ac) = mount_corelink(&server, true).await;

    let cache = TempDir::new().unwrap();
    let _s = snapshot_tiny(&server.uri(), cache.path(), "e2e-prune-midsweep");

    // Plant a poison orphan (not referenced by any manifest) whose DELETE will 500.
    let poison = "cd".repeat(32); // 64 hex
    cas.lock()
        .unwrap()
        .insert(poison.clone(), b"poison orphan".to_vec());

    // Higher-priority (lower number) override: DELETE of the poison digest → 500.
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/cas/{TEST_TENANT}/{poison}")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&server)
        .await;

    let out = clw_cmd(&server.uri(), cache.path())
        // Cap the op deadline so the client's 5xx-retry ladder on the failing DELETE
        // doesn't run the full ~18s; the outcome (fail + failed_at) is unchanged.
        .env("CLW_OP_DEADLINE_SECS", "3")
        .args(["--json", "prune", "--yes"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "a mid-sweep DELETE failure must exit non-zero, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("prune --json must still emit JSON on a partial failure");
    assert_eq!(v["dry_run"], serde_json::json!(false));
    assert_eq!(
        v["failed_at"],
        serde_json::json!(poison),
        "must report the exact digest it failed at (so the run is resumable)"
    );
    assert!(
        cas.lock().unwrap().contains_key(&poison),
        "the orphan whose delete failed must NOT be swept from CAS"
    );
}

// erase's GDPR Art.17 fail-loud REPORTING path (exit non-zero, `complete:false`,
// `remaining_refs`) end-to-end: an AC listing that never converges to empty (a
// server enumeration fault) must make the binary FAIL, never record a completed
// erasure while data remains. (#201 tested the detection at the unit level; this
// pins the run()-level exit code + JSON shape through the real binary.)
#[tokio::test]
async fn erase_fails_loud_and_exits_nonzero_when_data_still_remains() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    // A ghost ref that the AC listing ALWAYS returns — even after erase deletes it
    // — modelling a server-side enumeration fault / concurrent writer.
    let ghost = "ef".repeat(32); // 64 hex
    Mock::given(method("GET"))
        .and(path(format!("/v1/ac/{TEST_TENANT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "refs": [{ "ref_key": ghost }],
            "next_cursor": serde_json::Value::Null,
        })))
        .with_priority(1)
        .mount(&server)
        .await;

    let cache = TempDir::new().unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "erase", "--yes", "--confirm-tenant", TEST_TENANT])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "erase MUST exit non-zero when the tenant is not verified empty (GDPR Art.17), stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("erase --json must emit a structured incomplete-erasure record");
    assert_eq!(
        v["complete"],
        serde_json::json!(false),
        "erase must report complete:false when data remains"
    );
    assert!(
        v["remaining_refs"].as_u64().unwrap_or(0) >= 1,
        "erase must report the refs that still remain, got: {v}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("INCOMPLETE"),
        "the loud error must name the erasure INCOMPLETE"
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
    // It must take the clw-internal "child not run" branch (the same sentinel as the
    // dead-endpoint case) — proving this is the spawn-failure 125, not some other path.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("child not run") || stderr.contains("internal"),
        "an un-spawnable command must report the clw-internal child-not-run failure, got:\n{stderr}"
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

    // The child touches a marker BEFORE signalling itself, so we can PROVE the child
    // actually executed (distinguishing the 143-via-signal path from a 125 spawn failure).
    let marker = work.path().join("child_started.marker");
    let shell_cmd = format!("touch {}; kill -TERM $$", marker.to_str().unwrap());
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
                &shell_cmd, // touch a marker, then the shell signals itself → SIGTERM (15)
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
        marker.exists(),
        "the child must have actually RUN (marker present) — proving this is the signal \
         path, not a spawn failure"
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

/// Sending SIGTERM to the real `clw` process group must reach both the wrapper
/// and its running command. The direct wrapper is terminated by the signal
/// itself (so its status reports SIGTERM, rather than the 128+signal mapper used
/// for a child result), while the shell's trap proves the group signal reached
/// the descendant too.
#[tokio::test]
async fn run_process_group_sigterm_reaches_clw_and_child() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;

    let work = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let input = work.path().join("in.txt");
    std::fs::write(&input, b"declared input").unwrap();
    let ready = work.path().join("child-ready.marker");
    let term_seen = work.path().join("child-term-seen.marker");

    // `assert_cmd::Command` deliberately hides spawning, so retain its resolved
    // cargo binary path and configure a std command for process-group control.
    let binary = clw_bin().get_program().to_owned();
    let mut clw = StdCommand::new(binary);
    for variable in [
        "CLW_ENDPOINT",
        "CLW_TENANT",
        "CLW_TOKEN",
        "CLW_CACHE_DIR",
        "CLW_REF_DOMAIN",
        "CORELINK_ENDPOINT",
        "CORELINK_TENANT",
        "CORELINK_TOKEN",
        "CLW_CRED_TICKET",
        "CLW_LEASE_ID",
        "CLW_FABRIC_ENDPOINT",
    ] {
        clw.env_remove(variable);
    }
    clw.env("CLW_ENDPOINT", server.uri())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache.path())
        .current_dir(work.path())
        .args([
            "run",
            "--input",
            input.to_str().unwrap(),
            "--",
            "/bin/sh",
            "-c",
            &format!(
                "trap 'touch {}' TERM; touch {}; while :; do sleep 1; done",
                term_seen.display(),
                ready.display()
            ),
        ]);

    // Safe std API: make clw the leader of a fresh process group before exec.
    clw.process_group(0);
    let child = clw.spawn().expect("real clw must spawn");
    let mut process_group = ProcessGroupCleanup::new(child);

    let ready_wait = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !ready.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if ready_wait.is_err() {
        let cleanup = process_group.cleanup();
        panic!("child shell did not write its ready marker within 10 seconds; cleanup={cleanup:?}");
    }

    // The negative PID targets the fresh group, not just clw itself.
    let signal = process_group
        .signal_group("TERM")
        .expect("/bin/kill must be available on Unix");
    assert!(
        signal.success(),
        "SIGTERM to clw's process group must succeed: {signal:?}"
    );

    let status = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(status) = process_group.try_wait().expect("wait on clw process") {
                break status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    let status = match status {
        Ok(status) => status,
        Err(_) => {
            let cleanup = process_group.cleanup();
            panic!("clw did not terminate within 10 seconds after SIGTERM; cleanup={cleanup:?}");
        }
    };

    // The direct clw process was itself in the signalled group: assert its raw
    // signal status, not the child-result 143 convention tested above.
    assert_eq!(
        status.signal(),
        Some(15),
        "direct clw must be terminated by SIGTERM: {status:?}"
    );

    let term_wait = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !term_seen.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if term_wait.is_err() {
        let cleanup = process_group.cleanup();
        panic!("child shell did not observe SIGTERM within 10 seconds; cleanup={cleanup:?}");
    }

    // The trap normally exits the child shell; this also confirms cleanup if a
    // platform shell delayed trap delivery and left a shell/sleep descendant.
    process_group
        .cleanup()
        .unwrap_or_else(|error| panic!("process-group cleanup failed: {error}"));
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
    // CAS PUT returns 403, and we PIN it to EXACTLY ONE attempt: 403 is non-retryable,
    // so the client must NOT retry-storm. `.expect(1)` is verified when `server` drops —
    // a request-level assertion (not just an observed exit code) that the client issued
    // one PUT and gave up. HEAD is unmounted (wiremock 404 ⇒ "blob missing" ⇒ PUT attempted).
    Mock::given(method("PUT"))
        .and(path_regex(format!("/v1/cas/{TEST_TENANT}/[a-f0-9]{{64}}$")))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    // Back the AC PUT with a real store so we can PROVE the failed upload wrote NO ref —
    // a 403 mid-snapshot must leave the namespace clean, not a dangling ref.
    let ac_store: Store = Arc::new(Mutex::new(HashMap::new()));
    Mock::given(method("PUT"))
        .and(path_regex(format!("/v1/ac/{TEST_TENANT}/[a-f0-9]{{64}}$")))
        .respond_with(AcResponder {
            store: Arc::clone(&ac_store),
            auth_ok: true,
        })
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
    // No partial state: the failed upload wrote NO AC ref.
    assert!(
        ac_store.lock().unwrap().is_empty(),
        "a snapshot that 403s on upload must NOT write a workspace ref"
    );
    // `server` drops here → wiremock verifies the `.expect(1)` (exactly one PUT, no retry storm).
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
    let mut cmd = clw_bin();
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

/// `auth logout --all` is a destructive local operation: without `--yes` it
/// must refuse without changing the saved identity, while `--yes` clears every
/// config key (including the PAT) from the real config file.
#[test]
fn auth_logout_all_requires_yes_and_clears_config_when_confirmed() {
    let (home, cache) = seed_clw_footprint();
    let config_path = home.path().join(".clw/config.toml");
    let cache_file = cache.path().join("blob.bin");
    let before = std::fs::read(&config_path).unwrap();
    let cache_before = std::fs::read(&cache_file).unwrap();

    let refused = clw_bin()
        .env("HOME", home.path())
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["auth", "logout", "--all"])
        .output()
        .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(2),
        "logout --all without --yes must refuse in a non-interactive test"
    );
    let refused_stdout = String::from_utf8_lossy(&refused.stdout);
    let refused_stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        refused_stderr.contains("--yes"),
        "refusal must tell the user to pass --yes, got:\n{refused_stderr}"
    );
    assert_eq!(
        std::fs::read(&config_path).unwrap(),
        before,
        "refused logout --all must not mutate the config/credential footprint"
    );
    assert_eq!(
        std::fs::read(&cache_file).unwrap(),
        cache_before,
        "refused logout --all must not mutate the cache footprint"
    );
    assert!(
        !refused_stdout.contains(SECRET_TOKEN) && !refused_stderr.contains(SECRET_TOKEN),
        "refused logout --all must NEVER print the token"
    );

    let confirmed = clw_bin()
        .env("HOME", home.path())
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["auth", "logout", "--all", "--yes"])
        .output()
        .unwrap();
    assert_eq!(
        confirmed.status.code(),
        Some(0),
        "logout --all --yes must exit 0; stderr:\n{}",
        String::from_utf8_lossy(&confirmed.stderr)
    );
    assert!(
        config_path.exists(),
        "logout --all should retain the config file with its secure mode"
    );
    let after = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(
        after, "# clw configuration\n",
        "logout --all must clear every stored config key"
    );
    let mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "cleared config.toml must remain mode 0600");
    assert!(
        !String::from_utf8_lossy(&confirmed.stdout).contains(SECRET_TOKEN)
            && !String::from_utf8_lossy(&confirmed.stderr).contains(SECRET_TOKEN),
        "confirmed logout --all must NEVER print the token"
    );
    assert!(
        cache.path().exists(),
        "auth logout --all must not remove the separate cache footprint"
    );
    assert_eq!(
        std::fs::read(cache_file).unwrap(),
        cache_before,
        "auth logout --all must not alter the separate cache footprint"
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
        clw_bin()
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

// ---------------------------------------------------------------------------
// Remaining non-retryable tiers · process-level concurrency · scale (large tree).
//
// Closes the last directions a real user hits that lacked binary coverage: the
// 402/410 tier surfacing (403 was already covered), TWO+ concurrent `clw` PROCESSES
// sharing one cache (no corruption / no cache-temp collision), and a genuinely large
// real tree (manifest built for real, not a tiny hostile fixture).
// ---------------------------------------------------------------------------

/// Recursively count regular files under `dir` (symlinks not followed).
fn count_files(dir: &std::path::Path) -> usize {
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            n += count_files(&entry.path());
        } else if ft.is_file() {
            n += 1;
        }
    }
    n
}

/// The other non-retryable tiers (402 quota, 410 tombstone) must surface like 403:
/// clean exit 2 naming the status, exactly one PUT (no retry storm), and NO ref written.
#[tokio::test]
async fn snapshot_surfaces_quota_402_and_tombstone_410_cleanly() {
    for status in [402u16, 410u16] {
        let server = MockServer::start().await;
        let ac_store: Store = Arc::new(Mutex::new(HashMap::new()));
        Mock::given(method("PUT"))
            .and(path_regex(format!("/v1/cas/{TEST_TENANT}/[a-f0-9]{{64}}$")))
            .respond_with(ResponseTemplate::new(status))
            .expect(1) // non-retryable ⇒ exactly one attempt
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(format!("/v1/ac/{TEST_TENANT}/[a-f0-9]{{64}}$")))
            .respond_with(AcResponder {
                store: Arc::clone(&ac_store),
                auth_ok: true,
            })
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
                "e2e-tier",
            ])
            .output()
            .unwrap();

        assert_eq!(
            out.status.code(),
            Some(2),
            "tier {status} must fail the snapshot with exit 2, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(&status.to_string()),
            "tier {status} must be surfaced to the user, got:\n{stderr}"
        );
        assert!(!stderr.contains("panicked"), "tier {status} must not panic");
        assert!(
            ac_store.lock().unwrap().is_empty(),
            "a failed {status} upload must NOT write a workspace ref"
        );
        // `server` drops at loop-end → wiremock verifies `.expect(1)` (no retry storm).
    }
}

/// Several `clw` PROCESSES snapshotting the same tree concurrently into different names,
/// SHARING one cache dir, must all succeed with no corruption — exercising concurrent
/// cache writes of the same chunks (the temp-collision path) and concurrent uploads.
#[tokio::test]
async fn concurrent_snapshot_processes_shared_cache_no_corruption() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();

    let src = TempDir::new().unwrap();
    build_source_tree(src.path());

    let uri = server.uri();
    let cache_path = cache.path().to_path_buf();
    let src_path = src.path().to_path_buf();
    let names = ["cc-1", "cc-2", "cc-3", "cc-4"];

    // Each name snapshotted by its OWN clw process, all racing on the shared cache.
    let handles: Vec<_> = names
        .iter()
        .map(|name| {
            let uri = uri.clone();
            let cache_path = cache_path.clone();
            let src_path = src_path.clone();
            let name = name.to_string();
            std::thread::spawn(move || {
                clw_cmd(&uri, &cache_path)
                    .args(["snapshot", src_path.to_str().unwrap(), "--name", &name])
                    .output()
                    .unwrap()
            })
        })
        .collect();
    for (h, name) in handles.into_iter().zip(names.iter()) {
        let out = h.join().unwrap();
        assert!(
            out.status.success(),
            "concurrent snapshot '{name}' must succeed, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Every ref landed.
    for name in names {
        assert!(
            ac.lock().unwrap().contains_key(&ref_key_hex(name)),
            "concurrent snapshot must have written ref '{name}'"
        );
    }

    // And a fresh-cache hydrate of one is byte-identical — proving the 4 concurrent
    // processes did not crash on the shared cache and the server-side content is intact.
    // (Upload sends in-memory bytes, not a cache read-back, so this validates server
    // content + the no-crash concurrent path, not cache-write integrity per se.)
    let cache_hyd = TempDir::new().unwrap();
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    clw_cmd(&server.uri(), cache_hyd.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", "cc-1"])
        .assert()
        .success();
    assert_eq!(
        std::fs::read(dest.join("small.txt")).unwrap(),
        b"hello e2e test",
        "a hydrate after concurrent snapshots must be byte-identical (no corruption)"
    );
    assert_eq!(
        std::fs::read(dest.join("nested/data.bin")).unwrap(),
        std::fs::read(src.path().join("nested/data.bin")).unwrap(),
        "the multi-chunk file must also survive concurrent snapshots intact"
    );
}

/// A genuinely large real tree (built and chunked for real, not a tiny hostile fixture):
/// snapshot reports the exact file count, and a fresh-cache hydrate restores EVERY file
/// byte-identically — the happy path at scale.
#[tokio::test]
async fn snapshot_large_tree_reports_count_and_hydrates_every_file() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();

    const N: usize = 1500;
    let src = TempDir::new().unwrap();
    for i in 0..N {
        let sub = src.path().join(format!("d{:02}", i % 16)); // 16 subdirs
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f{i}.txt")), format!("content-{i}")).unwrap();
    }

    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "--json",
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "e2e-large",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "large-tree snapshot must succeed, stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        parsed["files"].as_u64(),
        Some(N as u64),
        "the snapshot must count exactly {N} files, got: {parsed}"
    );

    // Fresh-cache hydrate restores every file, byte-identical.
    let cache_hyd = TempDir::new().unwrap();
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    clw_cmd(&server.uri(), cache_hyd.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", "e2e-large"])
        .assert()
        .success();
    assert_eq!(count_files(&dest), N, "hydrate must restore all {N} files");
    for i in [0usize, 749, 1499] {
        let sub = dest.join(format!("d{:02}", i % 16));
        assert_eq!(
            std::fs::read(sub.join(format!("f{i}.txt"))).unwrap(),
            format!("content-{i}").into_bytes(),
            "file {i} must hydrate byte-identical at scale"
        );
    }
}

// `clw run` memo records share the tenant's AC keyspace with workspace refs, so
// `ls --all` enumerates them too. They must be recognized as run-cache entries (counted
// separately, NOT warned as "unparseable"), while a genuinely-corrupt record is still
// reported — and neither floods stderr with a per-record warning line (the user-sim
// campaign found `ls --all` double-printing + drowning in run-memo noise).
#[tokio::test]
async fn ls_all_distinguishes_run_cache_from_corrupt_refs_without_flooding() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-lsall-runcache";
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws); // one real workspace ref

    // A valid run-cache record (clw-run RunRecord shape) under a 64-hex key.
    let digest = "ab".repeat(32);
    ac.lock().unwrap().insert(
        "11".repeat(32),
        format!("{{\"exit_code\":0,\"stdout\":\"{digest}\",\"stderr\":\"{digest}\"}}").into_bytes(),
    );
    // A foreign native ActionCache value (non-JSON bytes) sharing the keyspace.
    let foreign = "33".repeat(32);
    ac.lock()
        .unwrap()
        .insert(foreign.clone(), b"ac-roundtrip-value-abc123".to_vec());
    // A genuinely-corrupt ref: valid JSON but NOT a RefRecord/RunRecord shape.
    let corrupt = "22".repeat(32);
    ac.lock()
        .unwrap()
        .insert(corrupt.clone(), br#"{"unexpected":"shape"}"#.to_vec());

    // --json: the real ref counts, the run-cache entry is recognized, the foreign AC
    // value is counted (not skipped), and only the JSON-wrong-shape ref is skipped.
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "ls", "--all"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "ls --all must exit 0");
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        parsed["count"], 1,
        "the real workspace ref must be counted, got:\n{parsed}"
    );
    assert_eq!(
        parsed["run_cache_entries"], 1,
        "the run-cache entry must be recognized, NOT counted as corrupt, got:\n{parsed}"
    );
    assert_eq!(
        parsed["foreign_ac_entries"], 1,
        "the non-JSON native AC value must be counted foreign, NOT skipped, got:\n{parsed}"
    );
    assert_eq!(
        parsed["skipped"], 1,
        "only the JSON-wrong-shape record is skipped, got:\n{parsed}"
    );
    assert_eq!(
        parsed["errors"][0]["ref_key"], corrupt,
        "the skipped record must be the corrupt one (not the run-cache or foreign entry)"
    );

    // human mode: summary names all three, and there is NO per-record warning flood on stderr.
    let human = clw_cmd(&server.uri(), cache.path())
        .args(["ls", "--all"])
        .output()
        .unwrap();
    let hstdout = String::from_utf8_lossy(&human.stdout);
    assert!(
        hstdout.contains("run-cache entries")
            && hstdout.contains("non-ref AC entries")
            && hstdout.contains("corrupt record(s) skipped"),
        "the summary must name run-cache + non-ref AC entries + the corrupt skip, got:\n{hstdout}"
    );
    assert!(
        !String::from_utf8_lossy(&human.stderr).contains("warning: skipping"),
        "there must be NO per-record 'warning: skipping' flood on stderr"
    );
}

// Re-snapshot ergonomics (user-sim headline finding): against a create-only AC store,
// (a) re-snapshotting IDENTICAL content under the same name is a clean no-op success
// (not a 409); (b) DIFFERENT content without --force still 409s with guidance that now
// names --force; (c) --force replaces it (rm-then-write) and succeeds with the new root.
#[tokio::test]
async fn resnapshot_identical_is_noop_and_force_replaces() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink_opts(&server, true, true).await; // create-only AC
    let cache = TempDir::new().unwrap();
    let ws = "e2e-resnap-noop";

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"content v1").unwrap();
    clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", ws])
        .assert()
        .success();

    // (a) IDENTICAL re-snapshot → no-op success, unchanged=true, NOT a 409.
    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "--json",
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            ws,
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "re-snapshot of identical content must be a no-op success, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        parsed["unchanged"], true,
        "identical re-snapshot must report unchanged=true, got:\n{parsed}"
    );

    // (b) DIFFERENT content WITHOUT --force → 409 actionable message that names --force.
    std::fs::write(src.path().join("f.txt"), b"content v2 - changed").unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", ws])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "differing re-snapshot without --force must exit 2"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already exists") && stderr.contains("--force"),
        "the 409 guidance must now point at --force, got:\n{stderr}"
    );

    // (c) WITH --force → replace succeeds (rm-then-write), unchanged=false, new root.
    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "--json",
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            ws,
            "--force",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "snapshot --force must replace a differing ref, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        parsed["unchanged"], false,
        "a --force replace is a real write, not a no-op"
    );
    assert!(
        ac.lock().unwrap().contains_key(&ref_key_hex(ws)),
        "after --force the ref must be present (re-created)"
    );
}

// ===========================================================================
// Validation-gap closure batch — flags/journeys/errors previously covered only at
// parse/unit/lib level, now driven through the REAL binary end-to-end.
// ===========================================================================

/// `hydrate --manifest-digest <root>` round-trips content-addressed by the manifest
/// digest (the v0.1.5 headline feature #153) — was only parse/unit/lib-tested.
#[tokio::test]
async fn hydrate_manifest_digest_roundtrips_byte_identical() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"digest-hydrate payload").unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "md-ws",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "snapshot must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("snapshot --json stdout");
    let root = v["root"]
        .as_str()
        .expect("root digest in --json")
        .to_string();

    let dest = TempDir::new().unwrap();
    let outp = dest.path().join("out");
    clw_cmd(&server.uri(), cache.path())
        .args([
            "hydrate",
            outp.to_str().unwrap(),
            "--manifest-digest",
            &root,
        ])
        .assert()
        .success();
    assert_eq!(
        std::fs::read(outp.join("f.txt")).unwrap(),
        b"digest-hydrate payload"
    );
}

/// `hydrate --manifest-digest <valid-but-absent>` → clean, actionable failure (not a crash).
#[tokio::test]
async fn hydrate_manifest_digest_absent_fails_cleanly() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let absent = "0".repeat(64);
    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "hydrate",
            dest.path().join("out").to_str().unwrap(),
            "--manifest-digest",
            &absent,
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "an absent manifest digest must fail");
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("not found") || err.contains("manifest") || err.contains("404"),
        "must be an actionable not-found, got: {err}"
    );
}

/// `hydrate --manifest-digest zzz` (malformed) → clean InvalidDigest before any request.
#[test]
fn hydrate_manifest_digest_malformed_rejected() {
    let cache = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let out = clw_cmd("http://127.0.0.1:1", cache.path())
        .args([
            "hydrate",
            dest.path().join("out").to_str().unwrap(),
            "--manifest-digest",
            "zzz",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "a malformed digest must fail");
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("digest") || err.contains("hex"),
        "must name the invalid digest, got: {err}"
    );
}

/// `run --env FOO` folds FOO's VALUE into the memoization key: a changed value MISSES
/// (re-runs with the new value) rather than replaying the cached one. Was parse-only.
#[tokio::test]
async fn run_env_var_value_busts_the_cache_key() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"x").unwrap();

    let run = |val: &str| {
        clw_cmd(&server.uri(), cache.path())
            .env("FOO", val)
            .args([
                "run",
                "--input",
                src.path().to_str().unwrap(),
                "--env",
                "FOO",
                "--",
                "sh",
                "-c",
                "printf %s \"$FOO\"",
            ])
            .output()
            .unwrap()
    };

    let a1 = run("alpha");
    assert!(a1.status.success());
    assert_eq!(a1.stdout, b"alpha", "first run reflects FOO=alpha");
    // Same value -> cache HIT (stored stdout replayed; the child does NOT re-run).
    let a2 = run("alpha");
    assert!(
        String::from_utf8_lossy(&a2.stderr).contains("cache hit"),
        "an unchanged --env value must hit the cache, stderr: {}",
        String::from_utf8_lossy(&a2.stderr)
    );
    // A CHANGED --env value must bust the key: if FOO were NOT in the key this would be a
    // cache HIT replaying "alpha". Getting "beta" proves the value is folded into the key.
    let b1 = run("beta");
    assert_eq!(
        b1.stdout, b"beta",
        "a changed --env value must miss the cache and re-run (not replay the cached output)"
    );
}

/// `clw run` with a child emitting > the 5 MiB output cap aborts with the reserved
/// oversized exit code 137 — the frozen exit-contract leg, previously only unit-mapped.
#[tokio::test]
async fn run_oversized_child_output_exits_137() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"x").unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "run",
            "--input",
            src.path().to_str().unwrap(),
            "--",
            "sh",
            "-c",
            "head -c 6291456 /dev/zero", // 6 MiB > 5 MiB cap
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(137),
        "oversized child output must exit the reserved 137, got {:?} (stderr: {})",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `rm --name x --ref-key y` — the clap `conflicts_with` rejection (ls/hydrate had a
/// both-together test; rm only had the neither-case).
#[test]
fn rm_name_and_ref_key_together_conflict() {
    let cache = TempDir::new().unwrap();
    let out = clw_cmd("http://127.0.0.1:1", cache.path())
        .args(["rm", "--name", "x", "--ref-key", &"0".repeat(64), "--yes"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "both --name and --ref-key must be rejected"
    );
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("cannot be used with") || err.contains("conflict"),
        "must be a clap conflict error, got: {err}"
    );
}

/// A tenant with a path separator (`a/b`) is rejected cleanly (InvalidTenant), never sent.
#[test]
fn invalid_tenant_with_slash_rejected_cleanly() {
    let cache = TempDir::new().unwrap();
    let out = clw_bin()
        .env("CLW_ENDPOINT", "http://127.0.0.1:1")
        .env("CLW_TENANT", "a/b")
        .env("CLW_TOKEN", TEST_PAT)
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .args(["ls", "--all"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "an invalid tenant must fail");
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("tenant"),
        "must name the tenant problem, got: {err}"
    );
}

/// `snapshot --concurrency N` (non-default) threads through and round-trips — the flag's
/// value path was parse-only.
#[tokio::test]
async fn snapshot_concurrency_flag_nondefault_roundtrips() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    build_source_tree(src.path());
    clw_cmd(&server.uri(), cache.path())
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "conc-ws",
            "--concurrency",
            "2",
        ])
        .assert()
        .success();
    let dest = TempDir::new().unwrap();
    let outp = dest.path().join("out");
    clw_cmd(&server.uri(), cache.path())
        .args([
            "hydrate",
            outp.to_str().unwrap(),
            "--name",
            "conc-ws",
            "--concurrency",
            "3",
        ])
        .assert()
        .success();
    assert_eq!(
        std::fs::read(outp.join("small.txt")).unwrap(),
        b"hello e2e test",
        "a non-default --concurrency must still round-trip byte-identical"
    );
}

/// `snapshot --ref-domain runner` uses the runner keyspace (flag form; previously only the
/// `CLW_REF_DOMAIN=runner` env form was tested).
#[tokio::test]
async fn snapshot_ref_domain_flag_uses_runner_keyspace() {
    let server = MockServer::start().await;
    let (_cas, ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"runner-domain").unwrap();
    clw_cmd(&server.uri(), cache.path())
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "rd-ws",
            "--ref-domain",
            "runner",
        ])
        .assert()
        .success();
    let runner_key = clw_types::ref_key_in(clw_types::RefDomain::Runner, "rd-ws").to_hex();
    let user_key = ref_key_hex("rd-ws");
    let store = ac.lock().unwrap();
    assert!(
        store.contains_key(&runner_key),
        "ref must land under the RUNNER keyspace"
    );
    assert!(
        !store.contains_key(&user_key),
        "ref must NOT land under the default user keyspace"
    );
}

/// `clw auth rotate` (human output) directs the user to re-login — no silent no-op.
/// (v1 has no one-step rotate; it prints guidance. Needs a resolved config to dispatch,
/// so provide the env; rotate itself makes no network call.)
#[test]
fn auth_rotate_directs_to_login() {
    let cache = TempDir::new().unwrap();
    let out = clw_cmd("http://127.0.0.1:1", cache.path())
        .args(["auth", "rotate"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    assert!(
        combined.contains("login"),
        "rotate must direct to `clw auth login`, got: {combined}"
    );
}

/// A corrupt `~/.clw/config.toml` must not block an env-configured op (warn + fall back).
#[tokio::test]
async fn corrupt_config_file_does_not_block_env_configured_op() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".clw")).unwrap();
    std::fs::write(home.path().join(".clw/config.toml"), b"not valid toml {{{").unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"y").unwrap();
    let out = clw_cmd(&server.uri(), cache.path())
        .env("HOME", home.path().to_str().unwrap())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", "cc-ws"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "a corrupt config file must not block an env-configured op: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ===========================================================================
// C2c cred-ticket redemption (CLW_CRED_TICKET) — the runner's single-use credential
// path. Previously only the classifier was unit-tested; now driven end-to-end through
// the real binary against a mock fabric endpoint + CAS/AC.
// ===========================================================================

/// Build one C2c runner invocation with no static PAT and an isolated HOME.
///
/// This deliberately configures every value on the child process only; the test
/// process's environment is never mutated, so these tests remain parallel-safe.
fn clw_cred_ticket_cmd(
    server: &MockServer,
    home: &std::path::Path,
    cache_dir: &std::path::Path,
) -> Command {
    let mut cmd = clw_bin();
    cmd.env("HOME", home)
        .env("CLW_ENDPOINT", server.uri())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_CACHE_DIR", cache_dir)
        .env("CLW_REF_DOMAIN", "runner")
        .env("CLW_FABRIC_ENDPOINT", server.uri())
        .env("CLW_LEASE_ID", "lease-1")
        .env("CLW_CRED_TICKET", "the-ticket")
        .env_remove("CLW_TOKEN");
    cmd
}

async fn mount_cred_ticket_success(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/leases/.*/cas-cred$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(format!(r#"{{"cas_pat":"{TEST_PAT}"}}"#)),
        )
        .mount(server)
        .await;
}

async fn assert_cred_ticket_redeems(server: &MockServer, expected: usize, command: &str) {
    let requests = server.received_requests().await.unwrap();
    let actual = requests
        .iter()
        .filter(|request| {
            let path = request.url.path();
            path.starts_with("/v1/leases/") && path.ends_with("/cas-cred")
        })
        .count();
    assert_eq!(
        actual, expected,
        "{command} must make exactly {expected} C2c redeem request(s), got {actual}"
    );
}

fn assert_broker_pat_never_persisted(home: &std::path::Path, command: &str) {
    let config = home.join(".clw/config.toml");
    if config.exists() {
        let content = std::fs::read_to_string(&config).unwrap();
        assert!(
            !content.contains(TEST_PAT),
            "{command} must not persist a redeemed broker PAT to {}",
            config.display()
        );
    }
}

/// `init` is local-only. A regression to the old negative predicate burns the
/// ticket before writing config, so this binary test is red-on-revert.
#[tokio::test]
async fn cred_ticket_init_does_not_redeem_or_persist_broker_pat() {
    let server = MockServer::start().await;
    mount_cred_ticket_success(&server).await;
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .arg("init")
        .assert()
        .success();

    assert_cred_ticket_redeems(&server, 0, "init").await;
    assert_broker_pat_never_persisted(home.path(), "init");
}

/// `auth login` is local-only. It must never exchange the ticket or write the
/// short-lived broker PAT into the persistent config file.
#[tokio::test]
async fn cred_ticket_auth_login_does_not_redeem_or_persist_broker_pat() {
    let server = MockServer::start().await;
    mount_cred_ticket_success(&server).await;
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args(["auth", "login"])
        .assert()
        .success();

    assert_cred_ticket_redeems(&server, 0, "auth login").await;
    assert_broker_pat_never_persisted(home.path(), "auth login");
}

/// `auth rotate` is local-only. A C2c ticket must remain unused for the later
/// workload even though this command requires normal strict config resolution.
#[tokio::test]
async fn cred_ticket_auth_rotate_does_not_redeem_or_persist_broker_pat() {
    let server = MockServer::start().await;
    mount_cred_ticket_success(&server).await;
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args(["auth", "rotate"])
        .assert()
        .success();

    assert_cred_ticket_redeems(&server, 0, "auth rotate").await;
    assert_broker_pat_never_persisted(home.path(), "auth rotate");
}

/// These paths return before credential resolution. Keep them black-boxed so a
/// future startup reordering cannot turn help/completion rendering into a redeem.
#[tokio::test]
async fn cred_ticket_early_exit_commands_do_not_redeem() {
    for (command, args) in [
        ("completions", vec!["completions", "bash"]),
        ("--help", vec!["--help"]),
        ("--version", vec!["--version"]),
    ] {
        let server = MockServer::start().await;
        mount_cred_ticket_success(&server).await;
        let home = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let out = clw_cred_ticket_cmd(&server, home.path(), cache.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{command} must succeed");
        assert_cred_ticket_redeems(&server, 0, command).await;
    }
}

/// An incompatible `--json completions` invocation must reject before both
/// script rendering and C2c ticket redemption. This is intentionally distinct
/// from the successful-completions case above: a future reordering must not
/// burn a single-use ticket merely to report an output-mode error.
#[tokio::test]
async fn cred_ticket_json_completions_rejects_without_redeeming() {
    let server = MockServer::start().await;
    mount_cred_ticket_success(&server).await;
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    let out = clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args(["--json", "completions", "bash"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "--json completions must reject with the standard CLI error exit"
    );
    assert!(
        out.stdout.is_empty(),
        "--json completions must not emit a generated script"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--json") && stderr.contains("incompatible"),
        "the error must name the --json incompatibility, got: {stderr}"
    );
    assert_cred_ticket_redeems(&server, 0, "--json completions").await;
}

/// Happy path: a network command with CLW_CRED_TICKET (no CLW_TOKEN) redeems the ticket
/// at the fabric endpoint, receives a cas_pat, and uses it for the operation.
#[tokio::test]
async fn cred_ticket_redeem_happy_path_snapshots() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await; // CAS/AC accept `Bearer test-pat`
                                                           // Fabric redeem route returns cas_pat = TEST_PAT so the CAS mock then accepts it.
    mount_cred_ticket_success(&server).await;

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"cred-ticket payload").unwrap();
    let cache = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let out = clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", "ct-ws"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "redeem + snapshot with a cred ticket (no PAT) must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_cred_ticket_redeems(&server, 1, "snapshot").await;
}

/// Hydration is a CAS/AC command, so it gets exactly one redeem. Seed the
/// runner ref using the static-PAT path, then run hydrate in a separate C2c child.
#[tokio::test]
async fn cred_ticket_redeem_happy_path_hydrates_once() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    mount_cred_ticket_success(&server).await;
    let seed_home = TempDir::new().unwrap();
    let seed_cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"hydrate credential ticket").unwrap();
    clw_runner_cmd(
        &server.uri(),
        seed_home.path(),
        seed_cache.path(),
        Some("runner"),
    )
    .args([
        "snapshot",
        src.path().to_str().unwrap(),
        "--name",
        "ct-hydrate",
    ])
    .assert()
    .success();
    assert_cred_ticket_redeems(&server, 0, "static seed snapshot").await;

    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let destination_root = TempDir::new().unwrap();
    let destination = destination_root.path().join("hydrate-output");
    clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args([
            "hydrate",
            destination.to_str().unwrap(),
            "--name",
            "ct-hydrate",
        ])
        .assert()
        .success();
    assert_cred_ticket_redeems(&server, 1, "hydrate").await;
}

/// `run` must redeem once before it executes the wrapped child.
#[tokio::test]
async fn cred_ticket_redeem_happy_path_runs_once() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    mount_cred_ticket_success(&server).await;
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let input = TempDir::new().unwrap();
    let input_file = input.path().join("input.txt");
    std::fs::write(&input_file, b"run credential ticket").unwrap();

    clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args(["run", "--input", input_file.to_str().unwrap(), "--", "true"])
        .assert()
        .success();
    assert_cred_ticket_redeems(&server, 1, "run").await;
}

/// A failed `run` redemption is still the frozen reserved-125 "child never ran"
/// outcome, and it must make only its one attempted broker request.
#[tokio::test]
async fn cred_ticket_run_redeem_failure_exits_125() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/leases/.*/cas-cred$"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let input = TempDir::new().unwrap();
    let input_file = input.path().join("input.txt");
    std::fs::write(&input_file, b"run failed credential ticket").unwrap();

    let out = clw_cred_ticket_cmd(&server, home.path(), cache.path())
        .args(["run", "--input", input_file.to_str().unwrap(), "--", "true"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(125),
        "a failed C2c redemption must prevent the child from running"
    );
    assert_cred_ticket_redeems(&server, 1, "run redemption failure").await;
}

/// Fail-closed: CLW_CRED_TICKET set but CLW_LEASE_ID missing → clean Config error, no redeem.
#[test]
fn cred_ticket_missing_lease_id_fails_closed() {
    let cache = TempDir::new().unwrap();
    let out = clw_bin()
        .env("CLW_ENDPOINT", "http://127.0.0.1:1")
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .env("CLW_REF_DOMAIN", "runner")
        .env("CLW_FABRIC_ENDPOINT", "http://127.0.0.1:1")
        .env("CLW_CRED_TICKET", "the-ticket")
        .env_remove("CLW_LEASE_ID")
        .env_remove("CLW_TOKEN")
        .args(["ls", "--all"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a cred ticket without a lease id must fail closed"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("CLW_LEASE_ID"),
        "must name the missing lease id, got: {err}"
    );
}

/// Fail-closed: a burned/invalid ticket (fabric 410) surfaces a clean error and NEVER
/// falls back to a PAT (there is none in the runner ref-domain).
#[tokio::test]
async fn cred_ticket_fabric_410_fails_closed() {
    let server = MockServer::start().await;
    let (_cas, _ac) = mount_corelink(&server, true).await;
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/leases/.*/cas-cred$"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"x").unwrap();
    let cache = TempDir::new().unwrap();
    let out = clw_bin()
        .env("CLW_ENDPOINT", server.uri())
        .env("CLW_TENANT", TEST_TENANT)
        .env("CLW_CACHE_DIR", cache.path().to_str().unwrap())
        .env("CLW_REF_DOMAIN", "runner")
        .env("CLW_FABRIC_ENDPOINT", server.uri())
        .env("CLW_LEASE_ID", "lease-1")
        .env("CLW_CRED_TICKET", "burned")
        .env_remove("CLW_TOKEN")
        .args([
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "ct-burned",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a burned ticket (fabric 410) must fail closed, never succeed"
    );
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("410")
            || err.contains("cred")
            || err.contains("redemption")
            || err.contains("ticket"),
        "must surface a clean cred-ticket redemption failure, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Tier-2 journey-map gap fills (docs/TEST-JOURNEY-MAP.md, Tier-3 items 11+12):
// the `--json` shapes never driven e2e (`ls --name`, `status`, `logout` no-op),
// `status` error→exit-2, plus the NO_COLOR / RUST_LOG env knobs and the
// external-symlink SKIP at the CLI boundary (the library path is unit-tested).
// ---------------------------------------------------------------------------

/// `clw --json ls --name <NAME>` (the SINGLE-ref lookup) must emit the ratified
/// object shape `{name, root, created_at_unix, parent, files, bytes}` with values
/// that match the known one-file fixture — not merely that the keys exist. Only
/// the human-mode `ls --name` was driven e2e before this.
#[tokio::test]
async fn ls_json_name_single_ref_shape() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-lsjson-name";
    // snapshot_tiny writes exactly one file holding `payload for {ws}`.
    let _src = snapshot_tiny(&server.uri(), cache.path(), ws);
    let expected_bytes = format!("payload for {ws}").len() as u64;

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["--json", "ls", "--name", ws])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "ls --json --name must exit 0");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("ls --json --name stdout must be valid JSON");

    assert_eq!(
        parsed.get("name").and_then(|v| v.as_str()),
        Some(ws),
        "ls --json --name must echo the looked-up name, got: {parsed}"
    );
    // root: a 64-hex digest.
    let root = parsed
        .get("root")
        .and_then(|v| v.as_str())
        .expect("ls --json --name must carry a root string");
    assert_eq!(
        root.len(),
        64,
        "root must be a 64-hex digest, got: {parsed}"
    );
    assert!(
        root.chars().all(|c| c.is_ascii_hexdigit()),
        "root must be lowercase hex, got: {parsed}"
    );
    // created_at_unix: an integer epoch.
    assert!(
        parsed
            .get("created_at_unix")
            .and_then(|v| v.as_u64())
            .is_some(),
        "created_at_unix must be an integer epoch, got: {parsed}"
    );
    // parent: null for a FIRST snapshot (no lineage).
    assert!(
        parsed.get("parent").is_some_and(serde_json::Value::is_null),
        "a first snapshot's parent must be JSON null, got: {parsed}"
    );
    // files/bytes must match the one-file fixture EXACTLY.
    assert_eq!(
        parsed.get("files").and_then(|v| v.as_u64()),
        Some(1),
        "ls --json --name must count the 1 fixture file, got: {parsed}"
    );
    assert_eq!(
        parsed.get("bytes").and_then(|v| v.as_u64()),
        Some(expected_bytes),
        "ls --json --name bytes must equal the fixture payload length, got: {parsed}"
    );
}

/// Two things the audit flagged as e2e-untested for `status`:
///   (a) `--json` emits the object shape `{clean, added, removed, changed}`, and
///   (b) a lookup of an ABSENT ref is a config-independent error that exits the
///       generic `2` (not the `0`=clean / `1`=dirty diff codes).
#[tokio::test]
async fn status_json_shape_and_absent_ref_exits_2() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-status-json";

    // Snapshot a tree, then a same-tree status is CLEAN.
    let src = TempDir::new().unwrap();
    build_source_tree(src.path());
    clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", ws])
        .assert()
        .success();

    // (a) --json clean shape.
    let out = clw_cmd(&server.uri(), cache.path())
        .args([
            "--json",
            "status",
            src.path().to_str().unwrap(),
            "--name",
            ws,
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "clean status --json must exit 0"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
            .expect("status --json stdout must be valid JSON");
    assert_eq!(
        parsed.get("clean").and_then(serde_json::Value::as_bool),
        Some(true),
        "a same-tree status must report clean:true, got: {parsed}"
    );
    for key in ["added", "removed", "changed"] {
        let arr = parsed
            .get(key)
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("status --json must carry '{key}' array, got: {parsed}"));
        assert!(
            arr.is_empty(),
            "a clean status must have an empty '{key}', got: {parsed}"
        );
    }

    // (b) status of an ABSENT ref → generic fatal exit 2 (not 0/1).
    let out_absent = clw_cmd(&server.uri(), cache.path())
        .args([
            "status",
            src.path().to_str().unwrap(),
            "--name",
            "no-such-workspace",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out_absent.status.code(),
        Some(2),
        "status of an absent ref must exit 2 (error), not 0/1, got: {:?}\nstderr={}",
        out_absent.status.code(),
        String::from_utf8_lossy(&out_absent.stderr)
    );
    let stderr = String::from_utf8_lossy(&out_absent.stderr);
    assert!(
        !stderr.contains("panicked"),
        "an absent-ref status must fail cleanly, not panic, got:\n{stderr}"
    );
    assert!(
        stderr.contains("no-such-workspace") || stderr.to_lowercase().contains("not found"),
        "the error must name the missing workspace, got:\n{stderr}"
    );
}

/// `clw --json auth logout` is a NO-OP when there is nothing to remove — and its
/// no-op shape is deliberately `{removed:[], note}` with NO `file` key (the `file`
/// key appears ONLY when something was actually removed, via `report_removed`).
/// Both no-op branches are exercised: no config file at all, and a config file
/// that carries no `token` line.
#[test]
fn logout_json_noop_shapes_omit_file_key() {
    // Branch 1: no config file (empty HOME) → note "no config file".
    let home_missing = TempDir::new().unwrap();
    let out = clw_clean_env(home_missing.path())
        .args(["--json", "auth", "logout"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "logout --json must exit 0 (no-op)"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
            .expect("logout --json stdout must be valid JSON");
    assert_eq!(
        parsed
            .get("removed")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(0),
        "no-config logout must report removed:[], got: {parsed}"
    );
    assert!(
        parsed.get("note").and_then(|v| v.as_str()).is_some(),
        "no-op logout must carry a 'note', got: {parsed}"
    );
    assert!(
        parsed.get("file").is_none(),
        "a no-op logout must NOT carry a 'file' key (that appears only on real removal), got: {parsed}"
    );

    // Branch 2: a config file WITHOUT a token line → note "no token set".
    let home_notoken = TempDir::new().unwrap();
    let clw_dir = home_notoken.path().join(".clw");
    std::fs::create_dir_all(&clw_dir).unwrap();
    std::fs::write(
        clw_dir.join("config.toml"),
        format!("endpoint = \"https://example.com\"\ntenant = \"{TEST_TENANT}\"\n"),
    )
    .unwrap();
    let out2 = clw_clean_env(home_notoken.path())
        .args(["--json", "auth", "logout"])
        .output()
        .unwrap();
    assert_eq!(
        out2.status.code(),
        Some(0),
        "no-token logout --json must exit 0"
    );
    let parsed2: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out2.stdout).trim())
            .expect("logout --json stdout must be valid JSON");
    assert_eq!(
        parsed2
            .get("removed")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(0),
        "no-token logout must report removed:[], got: {parsed2}"
    );
    assert!(
        parsed2.get("file").is_none(),
        "a no-token no-op logout must NOT carry a 'file' key, got: {parsed2}"
    );
    // The pre-existing config (no token) is untouched.
    let after = std::fs::read_to_string(clw_dir.join("config.toml")).unwrap();
    assert!(
        after.contains(TEST_TENANT),
        "a no-op logout must not scrub the existing (token-less) config, got:\n{after}"
    );
}

/// `NO_COLOR=1` must suppress ANSI color everywhere. Drives a stderr-emitting
/// command (`snapshot -v`, which logs a `tracing::info!` line) with `NO_COLOR=1`
/// and asserts the stderr stream carries no ESC (0x1b) byte.
///
/// Boundary note: an assert_cmd subprocess pipes stderr, so it is already
/// non-TTY (a second, independent reason clw disables color); this test locks the
/// NO_COLOR contract at the CLI boundary but cannot, without a pty, isolate the
/// env knob from the non-TTY gate. The `-v` guarantees there IS stderr content to
/// inspect, so the assertion is over a non-empty stream, not a vacuous one.
#[tokio::test]
async fn no_color_env_produces_no_ansi_on_stderr() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"no-color body").unwrap();

    let out = clw_cmd(&server.uri(), cache.path())
        .env("NO_COLOR", "1")
        .args([
            "-v",
            "snapshot",
            src.path().to_str().unwrap(),
            "--name",
            "e2e-no-color",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "snapshot -v must succeed, stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.stderr.is_empty(),
        "snapshot -v must emit at least one diagnostic line on stderr"
    );
    assert!(
        !out.stderr.contains(&0x1b),
        "NO_COLOR=1 must yield ZERO ANSI escape (0x1b) bytes on stderr, got:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `RUST_LOG` must OVERRIDE the `-v`/`-q` verbosity knobs. A `run` with `-q`
/// (ERROR-only) would normally suppress the DEBUG-level "run cache miss;
/// executing" diagnostic — but `RUST_LOG=debug` wins, so that line reappears on
/// stderr even under `-q`. A control run with `-q` alone confirms the line is
/// absent, so the assertion is genuinely discriminating (not vacuous).
#[tokio::test]
async fn rust_log_overrides_quiet_emits_debug() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let work = TempDir::new().unwrap();
    std::fs::write(work.path().join("in.txt"), b"rust-log input").unwrap();

    // The DEBUG diagnostic emitted by clw-run on a cache miss (distinct from the
    // human-facing "cache miss" line the run subcommand always prints). Each run
    // uses a UNIQUE command so its cache key differs — the run-memo record lives in
    // the shared mock CAS, so a repeated identical command would be a HIT, not a
    // miss.
    const DEBUG_LINE: &str = "run cache miss; executing";

    let run_with = |extra: &[(&str, &str)], cache: &std::path::Path, echo: &str| {
        let mut cmd = clw_cmd(&server.uri(), cache);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.current_dir(work.path())
            .args([
                "-q", "run", "--input", "in.txt", "--", "/bin/sh", "-c", echo,
            ])
            .output()
            .unwrap()
    };

    // Control: `-q` alone → the DEBUG diagnostic is suppressed (ERROR level).
    let cache_ctl = TempDir::new().unwrap();
    let ctl = run_with(&[], cache_ctl.path(), "echo ctl");
    assert!(ctl.status.success(), "control run must succeed");
    let ctl_stderr = String::from_utf8_lossy(&ctl.stderr);
    assert!(
        !ctl_stderr.contains(DEBUG_LINE),
        "with -q alone the DEBUG cache-miss diagnostic must be suppressed, got:\n{ctl_stderr}"
    );

    // RUST_LOG=debug wins over -q → the DEBUG diagnostic reappears (a UNIQUE
    // command makes this a genuine miss, not a hit of the control's record).
    let cache_dbg = TempDir::new().unwrap();
    let dbg = run_with(&[("RUST_LOG", "debug")], cache_dbg.path(), "echo dbg");
    assert!(dbg.status.success(), "RUST_LOG run must succeed");
    let dbg_stderr = String::from_utf8_lossy(&dbg.stderr);
    assert!(
        dbg_stderr.contains(DEBUG_LINE),
        "RUST_LOG=debug must override -q and emit the DEBUG cache-miss diagnostic, got:\n{dbg_stderr}"
    );
}

/// External-symlink SKIP through the CLI boundary: a tree carrying an
/// absolute-target (workspace-escaping) symlink must `snapshot` with exit 0, the
/// stderr must NAME the skipped link, and the resulting snapshot must contain NO
/// symlink entry — proven by a fresh-cache hydrate where the link never
/// materializes while the regular file round-trips. The library path is
/// unit-tested (clw-snapshot); this locks the CLI seam.
#[tokio::test]
async fn snapshot_external_symlink_skipped_through_cli() {
    let server = MockServer::start().await;
    mount_corelink(&server, true).await;
    let cache = TempDir::new().unwrap();
    let ws = "e2e-external-symlink";

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("keep.txt"), b"in-tree content").unwrap();
    // An ABSOLUTE target escapes the workspace: it cannot be represented in — nor
    // restored from — the snapshot, so it must be SKIPPED (target need not exist).
    std::os::unix::fs::symlink("/etc/hostname", src.path().join("outlink")).unwrap();

    let out = clw_cmd(&server.uri(), cache.path())
        .args(["snapshot", src.path().to_str().unwrap(), "--name", ws])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "an external symlink must NOT fail the snapshot, got: {:?}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("SKIPPED") && stderr.contains("outlink"),
        "stderr must NAME the skipped external symlink 'outlink', got:\n{stderr}"
    );

    // The manifest carries NO symlink entry: a fresh-cache hydrate restores the
    // in-tree file but the escaping link never materializes.
    let cache_hyd = TempDir::new().unwrap();
    let dest_tmp = TempDir::new().unwrap();
    let dest = dest_tmp.path().join("out");
    clw_cmd(&server.uri(), cache_hyd.path())
        .args(["hydrate", dest.to_str().unwrap(), "--name", ws])
        .assert()
        .success();
    assert_eq!(
        std::fs::read(dest.join("keep.txt")).unwrap(),
        b"in-tree content",
        "the in-tree file must round-trip through the snapshot"
    );
    assert!(
        std::fs::symlink_metadata(dest.join("outlink")).is_err(),
        "the external symlink must be absent from the hydrated tree (no manifest entry)"
    );
}
