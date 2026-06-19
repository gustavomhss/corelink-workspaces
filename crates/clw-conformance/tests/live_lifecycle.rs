//! Live full-lifecycle integration test against the real CoreLink API.
//!
//! Gated on `#[cfg(feature = "live")]` — never runs in the default CI lane.
//! UNIX-ONLY: the fixture uses unix symlinks + `chmod` and asserts unix file
//! modes (`exec.sh` 0o755), so the test body is gated to unix.
//!
//! # What this exercises (the SOLO lifecycle)
//!
//! Unlike `live_conformance.rs` (which probes the raw transport endpoints one
//! at a time), this drives the REAL clw LIBRARY APIs end-to-end against the live
//! sandbox tenant — `clw_snapshot::snapshot` → `clw_hydrate::hydrate` →
//! `clw_run::run_memoized` → `list_refs` → `delete` — through one workspace
//! lifecycle with a unique, isolated, per-run workspace name (`ws-<suffix>`).
//!
//! # Skip mechanism (intentional — mirrors live_conformance.rs)
//!
//! The test reads `CLW_TOKEN` and `CLW_TENANT` at runtime. When either variable
//! is absent it prints a human-readable skip notice to stderr and returns early,
//! counting as a pass — so the binary compiles and runs cleanly without
//! credentials. With `CLW_CONFORMANCE_REQUIRE=1` (the release pre-tag gate) every
//! skip becomes a hard failure instead.
//!
//! # Destructive safety
//!
//! Only ever touches the sandbox tenant and the unique-suffixed `ws-<suffix>`
//! names this test creates. Best-effort cleanup deletes only the AC ref this
//! test wrote. It NEVER calls a tenant-wide `erase`/`prune` — those stay manual.
//!
//! Run with:
//!   CLW_TENANT=<tenant> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live
//! Pre-tag gate (skips become failures):
//!   CLW_CONFORMANCE_REQUIRE=1 CLW_TENANT=<tenant> CLW_TOKEN=<pat> \
//!     cargo test -p clw-conformance --features live

#![cfg(feature = "live")]

use clw_client::HttpClient;
use clw_types::{AcTransport, ClwError, Config, Digest};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Test helpers (replicated from live_conformance.rs — each tests/*.rs is its
// own crate, so the small live-gating harness is duplicated here intentionally).
// ---------------------------------------------------------------------------

/// Read runtime env for credentials. Returns `None` (and prints skip notice)
/// when `CLW_TOKEN` or `CLW_TENANT` is absent.
fn try_client() -> Option<(HttpClient, String)> {
    let cfg = try_config()?;
    let tenant = cfg.tenant.clone();
    let client = HttpClient::new(cfg).expect("failed to build HttpClient");
    Some((client, tenant))
}

/// Read runtime env into a `Config`. Returns `None` (and prints skip notice)
/// when `CLW_TOKEN` or `CLW_TENANT` is absent.
fn try_config() -> Option<Config> {
    // An ABSENT secret in CI surfaces as an env var set to the EMPTY STRING (not
    // unset), so `var().ok()` returns `Some("")` — treat empty as absent, else the
    // no-PAT dry-run takes the credentials-present branch with an empty endpoint and
    // panics in `HttpClient::new` instead of skipping. (Same fix as live_conformance.rs.)
    let token = std::env::var("CLW_TOKEN").ok().filter(|s| !s.is_empty());
    let tenant = std::env::var("CLW_TENANT").ok().filter(|s| !s.is_empty());

    match (token, tenant) {
        (Some(t), Some(tn)) => {
            let endpoint = std::env::var("CLW_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "https://corelink-api.humangr.com".to_string());
            Some(Config {
                endpoint,
                tenant: tn,
                token: t,
                cache_dir: PathBuf::from("/tmp/clw-conformance-cache"),
            })
        }
        _ => {
            skip_or_fail("set CLW_TOKEN + CLW_TENANT to run live conformance");
            None
        }
    }
}

/// True only in the **release pre-tag gate**, where a skip must be a hard
/// failure rather than a silent pass. Set `CLW_CONFORMANCE_REQUIRE=1` (or
/// `=true`) to require every live test to actually execute its assertions.
fn require_live() -> bool {
    matches!(
        std::env::var("CLW_CONFORMANCE_REQUIRE").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Emit a skip notice to stderr — or PANIC when running under the require gate
/// (`CLW_CONFORMANCE_REQUIRE=1`). This closes the skip-equals-pass false-green.
fn skip_or_fail(reason: &str) {
    if require_live() {
        panic!(
            "CONFORMANCE REQUIRED but a test skipped: {reason} \
             (CLW_CONFORMANCE_REQUIRE=1 forbids skips at the pre-tag gate)"
        );
    }
    eprintln!("SKIPPED: {reason}");
}

/// A unique, per-run suffix that avoids key collisions between test invocations.
/// Concatenates `std::process::id()` (8 hex) with the FULL epoch nanos
/// (`as_nanos()`, 16 hex) so it changes on every call. NOTE: `subsec_nanos()`
/// would be a bug — it wraps every second, so two runs at the same sub-second
/// offset could collide (same fix as live_conformance.rs).
fn run_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    format!("{pid:08x}{nanos:016x}")
}

/// True if `err` is the server-regression signal: the capability is LIVE but the
/// server returned 405/501 (the client maps 405/501 to a `Transport` error
/// carrying the `405/501` token). Live tests SKIP rather than fail when a LIVE
/// capability is transiently unavailable.
fn is_capability_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("405/501"))
}

// ---------------------------------------------------------------------------
// Workspace fixture
// ---------------------------------------------------------------------------

/// Build a real workspace tree in `root`: a small known-bytes text file, an
/// executable `exec.sh` (0o755), a symlink, and ONE larger multi-chunk file
/// (5 MiB of deterministic bytes) so chunking + multi-chunk upload is exercised.
#[cfg(unix)]
fn build_workspace(root: &std::path::Path, suffix: &str) -> WorkspaceFixture {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Per-run uniqueness seed (FNV-1a over the run suffix). The CAS is
    // content-addressed + DEDUPLICATED and the test tenant PERSISTS across runs, so
    // a byte-identical fixture uploads chunks only on the very first run ever — every
    // later run is all dedup hits (chunks_uploaded == 0) and fails the "fresh upload"
    // assertion. Fold the suffix into the file CONTENT so each run uploads new chunks.
    let seed: u32 = suffix.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });

    // Small file with known, run-unique bytes.
    let small_bytes = format!("hello live lifecycle conformance {suffix}\n").into_bytes();
    fs::write(root.join("small.txt"), &small_bytes).unwrap();

    // Executable file (mode 0o755).
    let exec_bytes = b"#!/bin/sh\necho hello\n".to_vec();
    fs::write(root.join("exec.sh"), &exec_bytes).unwrap();
    let mut perms = fs::metadata(root.join("exec.sh")).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(root.join("exec.sh"), perms).unwrap();

    // Symlink → small.txt (target preserved verbatim by hydrate).
    let symlink_target = "small.txt";
    std::os::unix::fs::symlink(symlink_target, root.join("link.txt")).unwrap();

    // One larger multi-chunk file: 5 MiB of deterministic pseudo-random bytes
    // (well above the ~1 MiB chunk average → exercises chunk splitting + the
    // multi-chunk upload path).
    let big_len = 5 * 1024 * 1024usize;
    let big_bytes: Vec<u8> = (0u32..big_len as u32)
        .map(|i| (i.wrapping_mul(0x9e37_79b9u32).wrapping_add(seed) >> 13) as u8)
        .collect();
    fs::write(root.join("big.bin"), &big_bytes).unwrap();

    WorkspaceFixture {
        small_bytes,
        exec_bytes,
        symlink_target: symlink_target.to_string(),
        big_bytes,
    }
}

#[cfg(unix)]
struct WorkspaceFixture {
    small_bytes: Vec<u8>,
    exec_bytes: Vec<u8>,
    symlink_target: String,
    big_bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// full_lifecycle_roundtrip_live
// ---------------------------------------------------------------------------

/// SOLO full-lifecycle round-trip against the live sandbox tenant:
///   1. build a real workspace fixture (small + exec 0o755 + symlink + 5 MiB
///      multi-chunk file) in a TempDir,
///   2. `clw_snapshot::snapshot` under `ws-<suffix>` → non-zero root digest +
///      chunks uploaded,
///   3. `clw_hydrate::hydrate` to a FRESH dest with a FRESH cache → bytes come
///      from the live server, every file byte-identical (exec mode + symlink),
///   4. `clw_run::run_memoized` the same deterministic command twice → first a
///      cache MISS, second a cache HIT,
///   5. `list_refs` includes the snapshot's ref,
///   6. `AcTransport::delete` the ref → gone + idempotent,
///   7. best-effort cleanup of the AC ref this test created.
///
/// SKIPS cleanly (green) without creds; SKIPS if a LIVE endpoint returns a
/// 405/501-unavailable state (unless `CLW_CONFORMANCE_REQUIRE=1`).
#[cfg(unix)]
#[tokio::test]
async fn full_lifecycle_roundtrip_live() {
    use clw_cache::LocalCache;
    use std::os::unix::fs::PermissionsExt;

    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let ws_name = format!("ws-{suffix}");
    // The default RefDomain (User) is what SnapshotOptions::new / HydrateOptions::new
    // use, so the AC ref key for this workspace is `ref_key(&ws_name)`.
    let ref_key = clw_types::ref_key(&ws_name);

    // -- 1. Build the workspace fixture in a TempDir. ------------------------
    let src_tmp = tempfile::tempdir().expect("create src tempdir");
    let src_root = src_tmp.path();
    let fixture = build_workspace(src_root, &suffix);

    // -- 2. snapshot → non-zero root digest + chunks uploaded. --------------
    let snap_cache_tmp = tempfile::tempdir().expect("create snapshot cache tempdir");
    let snap_cache = LocalCache::new(snap_cache_tmp.path()).expect("build snapshot cache");
    let snap_opts = clw_snapshot::SnapshotOptions::new(&ws_name);

    let report = match clw_snapshot::snapshot(src_root, &client, &snap_cache, &snap_opts).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            skip_or_fail(
                "snapshot hit a 405/501-unavailable endpoint — lifecycle round-trip skipped",
            );
            return;
        }
        Err(e) => panic!("snapshot failed: {e}"),
    };

    assert_ne!(
        report.root,
        Digest::of_bytes(&[]),
        "snapshot root digest must be non-zero / non-trivial"
    );
    // `SnapshotReport.files` counts regular `File` manifest entries ONLY — symlinks
    // and directories are deliberately NOT counted (documented on the field + pinned
    // by clw-snapshot's own unit test). The fixture has 3 regular files (small.txt,
    // exec.sh, big.bin); link.txt is a symlink (verified to round-trip below), so the
    // expected file count is 3, not 4. (Was `>= 4` — an off-by-one that counted the
    // symlink; it had never executed in CI because the gate skipped without a PAT.)
    assert!(
        report.files >= 3,
        "expected at least 3 regular files, got {}",
        report.files
    );
    assert!(
        report.chunks_total >= 2,
        "5 MiB file must split into multiple chunks, got chunks_total={}",
        report.chunks_total
    );
    assert!(
        report.chunks_uploaded > 0,
        "a fresh workspace must upload chunks, got chunks_uploaded=0"
    );

    // -- 3. hydrate to a FRESH dest + FRESH cache (bytes from the server). ---
    let dest_tmp = tempfile::tempdir().expect("create dest tempdir");
    // `hydrate` requires a non-existent / empty dest; use a fresh subdir.
    let dest = dest_tmp.path().join("hydrated");
    let hydrate_cache_tmp = tempfile::tempdir().expect("create hydrate cache tempdir");
    let hydrate_cache = LocalCache::new(hydrate_cache_tmp.path()).expect("build hydrate cache");
    let hydrate_opts = clw_hydrate::HydrateOptions::new(&ws_name);

    let hydrate_report =
        match clw_hydrate::hydrate(&dest, &client, &hydrate_cache, &hydrate_opts).await {
            Ok(r) => r,
            Err(e) if is_capability_unavailable(&e) => {
                let _ = best_effort_cleanup(&client, &ref_key).await;
                skip_or_fail(
                    "hydrate hit a 405/501-unavailable endpoint — lifecycle round-trip skipped",
                );
                return;
            }
            Err(e) => {
                let _ = best_effort_cleanup(&client, &ref_key).await;
                panic!("hydrate failed: {e}");
            }
        };

    assert_eq!(
        hydrate_report.root, report.root,
        "hydrate must resolve the same root the snapshot wrote"
    );
    // FRESH cache ⇒ the bytes had to come from the live server.
    assert!(
        hydrate_report.bytes_downloaded > 0,
        "fresh cache must download bytes from the server, got bytes_downloaded=0"
    );

    // Byte-identical files.
    assert_eq!(
        std::fs::read(dest.join("small.txt")).expect("read hydrated small.txt"),
        fixture.small_bytes,
        "small.txt bytes must round-trip identically"
    );
    assert_eq!(
        std::fs::read(dest.join("big.bin")).expect("read hydrated big.bin"),
        fixture.big_bytes,
        "5 MiB multi-chunk file must round-trip identically"
    );
    assert_eq!(
        std::fs::read(dest.join("exec.sh")).expect("read hydrated exec.sh"),
        fixture.exec_bytes,
        "exec.sh bytes must round-trip identically"
    );

    // exec.sh mode 0o755.
    let exec_mode = std::fs::metadata(dest.join("exec.sh"))
        .expect("stat hydrated exec.sh")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(exec_mode, 0o755, "exec.sh must hydrate with mode 0o755");

    // Symlink target preserved.
    let link_meta = std::fs::symlink_metadata(dest.join("link.txt"))
        .expect("symlink_metadata of hydrated link.txt");
    assert!(
        link_meta.file_type().is_symlink(),
        "link.txt must hydrate as a symlink"
    );
    let link_target =
        std::fs::read_link(dest.join("link.txt")).expect("read_link hydrated link.txt");
    assert_eq!(
        link_target,
        std::path::Path::new(&fixture.symlink_target),
        "symlink target must be preserved"
    );

    // -- 4. run_memoized the same command twice → MISS then HIT. ------------
    // Run inside the freshly-hydrated dest so the inputs exist and are stable.
    let run_opts = clw_run::RunOptions {
        inputs: vec!["small.txt".to_string()],
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo lifecycle-deterministic".to_string(),
        ],
        env_keys: vec![],
    };

    let first = match clw_run::run_memoized(&dest, &client, &run_opts).await {
        Ok(o) => o,
        Err(e) if is_capability_unavailable(&e) => {
            let _ = best_effort_cleanup(&client, &ref_key).await;
            skip_or_fail(
                "run_memoized hit a 405/501-unavailable endpoint — lifecycle round-trip skipped",
            );
            return;
        }
        Err(e) => {
            let _ = best_effort_cleanup(&client, &ref_key).await;
            panic!("run_memoized (first) failed: {e}");
        }
    };
    assert!(!first.hit, "first run of fresh inputs must be a cache MISS");
    assert_eq!(first.exit_code, 0, "command must exit 0 to be memoized");

    let second = clw_run::run_memoized(&dest, &client, &run_opts)
        .await
        .expect("run_memoized (second) failed");
    assert!(second.hit, "second identical run must be a cache HIT");
    assert_eq!(
        first.stdout, second.stdout,
        "the memoized replay must reproduce the recorded stdout"
    );
    assert_eq!(
        first.key, second.key,
        "identical inputs must yield the same key"
    );

    // -- 5. list_refs must include the snapshot's ref. ----------------------
    let refs = match AcTransport::list_refs(&client).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            let _ = best_effort_cleanup(&client, &ref_key).await;
            skip_or_fail("list_refs (D-7) returned 405/501 — lifecycle round-trip skipped");
            return;
        }
        Err(e) => {
            let _ = best_effort_cleanup(&client, &ref_key).await;
            panic!("list_refs failed: {e}");
        }
    };
    assert!(
        refs.contains(&ref_key),
        "list_refs must include the snapshot's ref key (tolerating other records)"
    );

    // -- 6. delete the ref → gone + idempotent. -----------------------------
    match AcTransport::delete(&client, &ref_key).await {
        Ok(()) => {}
        Err(e) if is_capability_unavailable(&e) => {
            skip_or_fail("DELETE /v1/ac (D-1) returned 405/501 — lifecycle ref-delete skipped");
            return;
        }
        Err(e) => panic!("ac delete failed: {e}"),
    }

    let after = AcTransport::get(&client, &ref_key)
        .await
        .expect("ac get after delete should succeed");
    assert_eq!(after, None, "ref must be absent after delete (get → None)");

    // Idempotent second delete.
    AcTransport::delete(&client, &ref_key)
        .await
        .expect("second delete of an absent ref must be idempotent");

    // -- 7. Best-effort cleanup (ref already deleted above; this is a no-op
    //       safety net that ignores 405/501-unavailable / transient errors). -
    let _ = best_effort_cleanup(&client, &ref_key).await;
}

/// Best-effort delete of the AC ref this test created. Mirrors the cleanup
/// discipline in `live_conformance.rs`: ignore every error (idempotent,
/// 405/501-unavailable, transient). NEVER touches anything but this one key.
#[cfg(unix)]
async fn best_effort_cleanup(client: &HttpClient, ref_key: &Digest) -> Result<(), ()> {
    let _ = AcTransport::delete(client, ref_key).await;
    Ok(())
}
