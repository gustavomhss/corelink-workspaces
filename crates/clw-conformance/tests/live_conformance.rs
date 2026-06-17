//! Live conformance tests against the real CoreLink API.
//!
//! Gated on `#[cfg(feature = "live")]` — never runs in the default CI lane.
//!
//! # Skip mechanism (intentional)
//!
//! Each test reads `CLW_TOKEN` and `CLW_TENANT` at runtime. When either
//! variable is absent the test prints a human-readable skip notice to stderr
//! and returns early, counting as a pass. This means the binary compiles and
//! runs cleanly in environments without credentials (e.g. standard CI) while
//! still exercising real network behaviour when credentials are supplied.
//!
//! # The require gate (closes the skip-equals-pass false-green — M4.5 / exit-gate #6)
//!
//! A silent skip-as-pass is safe for everyday CI but DANGEROUS as a release
//! pre-tag gate: a release run that forgot the credentials — or hit an endpoint
//! that regressed to a 405/501-unavailable state — would report GREEN having
//! asserted nothing. Set `CLW_CONFORMANCE_REQUIRE=1` (only in the pre-tag
//! release gate) to make EVERY skip a hard failure: missing creds and
//! 405/501-unavailable endpoints then PANIC instead of passing. With the
//! variable unset, behaviour is identical to
//! before (skips pass) — so this change is invisible to the normal lane.
//!
//! Run with:
//!   CLW_TENANT=<tenant> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live
//! Pre-tag gate (skips become failures):
//!   CLW_CONFORMANCE_REQUIRE=1 CLW_TENANT=<tenant> CLW_TOKEN=<pat> \
//!     cargo test -p clw-conformance --features live

#![cfg(feature = "live")]

use clw_client::HttpClient;
use clw_types::{AcTransport, CasTransport, ClwError, Config, Digest};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Test helpers
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
/// when `CLW_TOKEN` or `CLW_TENANT` is absent. Used by tests that need the raw
/// endpoint / token in addition to (or instead of) a built `HttpClient`.
fn try_config() -> Option<Config> {
    // An ABSENT secret in CI surfaces as an env var set to the EMPTY STRING (not
    // unset), so `var().ok()` returns `Some("")` — treat empty as absent. Otherwise
    // the dry-run (no PAT provisioned) would take the credentials-present branch with
    // an empty endpoint and panic in `HttpClient::new` ("refusing to send PAT over
    // plaintext") instead of skipping the live cases. Empty == not provided.
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
/// (`CLW_CONFORMANCE_REQUIRE=1`). This is what closes the skip-equals-pass
/// false-green: in the release gate a missing credential or a 405/501-unavailable
/// endpoint fails loudly instead of reporting a green that asserted nothing.
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
/// (`as_nanos()`, 16 hex) so it changes on every call — even for two tests
/// running within the same wall-clock second in the same process.
///
/// Note: using `subsec_nanos()` here would be a bug — it wraps every second,
/// so two tests crossing a second boundary at the same sub-second offset could
/// produce identical suffixes and collide AC/CAS keys on the live server.
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
// cas_roundtrip
// ---------------------------------------------------------------------------

/// PUT a small blob → `exists` must return true → GET → bytes must be identical
/// (the client verifies BLAKE3 integrity on `get` per contract).
#[tokio::test]
async fn cas_roundtrip() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let payload = format!("cas-roundtrip-{suffix}");
    let bytes = payload.into_bytes();
    let digest = Digest::of_bytes(&bytes);

    // PUT
    CasTransport::put(&client, &digest, bytes.clone())
        .await
        .expect("cas put should succeed");

    // EXISTS → true
    let present = CasTransport::exists(&client, &digest)
        .await
        .expect("cas exists should succeed");
    assert!(present, "blob should exist after put");

    // GET → exact bytes (client verifies integrity internally)
    let fetched = CasTransport::get(&client, &digest)
        .await
        .expect("cas get should succeed");
    assert_eq!(fetched, bytes, "fetched bytes must equal what was put");
}

// ---------------------------------------------------------------------------
// cas_missing
// ---------------------------------------------------------------------------

/// GET of a digest that was never stored must return `ClwError::NotFound`.
/// The payload is unique per run so it cannot accidentally exist.
#[tokio::test]
async fn cas_missing() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    // Construct a digest from a payload we will NEVER store.
    let suffix = run_suffix();
    let never_stored = format!("cas-never-stored-{suffix}");
    let digest = Digest::of_bytes(never_stored.as_bytes());

    let err = CasTransport::get(&client, &digest)
        .await
        .expect_err("get of a never-stored digest must return an error");

    assert!(
        matches!(err, ClwError::NotFound(_)),
        "expected NotFound, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// ac_roundtrip
// ---------------------------------------------------------------------------

/// PUT a value under a ref-key-style digest → GET must return the same bytes.
#[tokio::test]
async fn ac_roundtrip() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    // Use a domain-separated key similar to how `ref_key` works.
    let key_input = format!("clw/ref/v1/conformance-test-{suffix}");
    let key = Digest::of_bytes(key_input.as_bytes());

    let value = format!("ac-roundtrip-value-{suffix}").into_bytes();

    // PUT
    AcTransport::put(&client, &key, value.clone())
        .await
        .expect("ac put should succeed");

    // GET → same bytes
    let fetched = AcTransport::get(&client, &key)
        .await
        .expect("ac get should succeed");

    assert_eq!(
        fetched,
        Some(value),
        "ac get must return exactly the bytes that were put"
    );
}

// ---------------------------------------------------------------------------
// ac_missing
// ---------------------------------------------------------------------------

/// GET of an AC key that was never stored must return `Ok(None)` — directly,
/// not merely as a post-delete side-effect. A server that returned `Some([])`
/// (or any non-`None`) for an absent key would otherwise go undetected: the
/// only other place `get → None` is asserted is after an explicit delete, which
/// does not catch an "empty-but-present" absent-key response. The key is unique
/// per run (`run_suffix`) so it cannot accidentally exist.
#[tokio::test]
async fn ac_missing() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let key_input = format!("clw/ref/v1/conformance-never-stored-{suffix}");
    let key = Digest::of_bytes(key_input.as_bytes());

    let fetched = AcTransport::get(&client, &key)
        .await
        .expect("ac get of a never-stored key should succeed");

    assert_eq!(
        fetched, None,
        "ac get of a never-stored key must return None, got: {fetched:?}"
    );
}

// ---------------------------------------------------------------------------
// auth_rejected
// ---------------------------------------------------------------------------

/// A client constructed with a deliberately invalid token must receive
/// `ClwError::Auth` (HTTP 401) from the server on any authenticated call.
#[tokio::test]
async fn auth_rejected() {
    // We need CLW_TENANT to build a valid URL, but we deliberately use an
    // invalid token. If CLW_TENANT is absent we skip normally.
    let tenant = match std::env::var("CLW_TENANT") {
        Ok(t) => t,
        Err(_) => {
            skip_or_fail("set CLW_TOKEN + CLW_TENANT to run live conformance");
            return;
        }
    };

    let endpoint = std::env::var("CLW_ENDPOINT")
        .unwrap_or_else(|_| "https://corelink-api.humangr.com".to_string());

    let bad_cfg = Config {
        endpoint,
        tenant,
        // Intentionally invalid PAT; server must reject it with 401.
        token: "corelink_pat_invalid.invalid.invalid".to_string(),
        cache_dir: PathBuf::from("/tmp/clw-conformance-cache"),
    };
    let bad_client = HttpClient::new(bad_cfg).expect("failed to build bad HttpClient");

    // Use a deterministic digest — the auth check fires before the blob lookup.
    let probe_digest = Digest::of_bytes(b"auth-probe");

    let err = CasTransport::exists(&bad_client, &probe_digest)
        .await
        .expect_err("invalid token must cause an error");

    assert!(
        matches!(err, ClwError::Auth),
        "expected ClwError::Auth (401), got: {err}"
    );
}

// ---------------------------------------------------------------------------
// ref_delete_roundtrip
// ---------------------------------------------------------------------------

/// D-1: PUT a ref → confirm present → `AcTransport::delete` → confirm gone.
///
/// Exercises the real client `delete` impl (the server chose `204`-for-both,
/// idempotent). DELETE /v1/ac is LIVE (D-1); if a LIVE endpoint returns 405/501
/// the client maps it to a `405/501` server-regression error → the test SKIPS.
#[tokio::test]
async fn ref_delete_roundtrip() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let key_input = format!("clw/ref/v1/conformance-delete-{suffix}");
    let key = Digest::of_bytes(key_input.as_bytes());
    let value = format!("ref-delete-value-{suffix}").into_bytes();

    // PUT the ref.
    AcTransport::put(&client, &key, value.clone())
        .await
        .expect("ac put should succeed");

    // Confirm it exists.
    let present = AcTransport::get(&client, &key)
        .await
        .expect("ac get should succeed");
    assert_eq!(
        present,
        Some(value),
        "ref must be present after put, before delete"
    );

    // DELETE via the real client impl (D-1).
    match AcTransport::delete(&client, &key).await {
        Ok(()) => {}
        Err(e) if is_capability_unavailable(&e) => {
            skip_or_fail("DELETE /v1/ac (D-1) returned 405/501 — capability unavailable; ref-delete round-trip skipped");
            return;
        }
        Err(e) => panic!("ac delete failed: {e}"),
    }

    // Confirm the ref is gone.
    let after = AcTransport::get(&client, &key)
        .await
        .expect("ac get after delete should succeed");
    assert_eq!(after, None, "ref must be absent after delete (get → None)");

    // Idempotent: a second delete of the same (now-absent) key still succeeds.
    AcTransport::delete(&client, &key)
        .await
        .expect("second delete of an absent ref must be idempotent (204)");
}

// ---------------------------------------------------------------------------
// ref_list_roundtrip (D-7)
// ---------------------------------------------------------------------------

/// D-7: PUT a ref → `list_refs` must include its key → best-effort cleanup.
/// SKIPS if `GET /v1/ac/{tenant}` (LIVE) returns a 405/501-unavailable state.
#[tokio::test]
async fn ref_list_roundtrip() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let key = Digest::of_bytes(format!("clw/ref/v1/conformance-list-{suffix}").as_bytes());
    let value = format!("ref-list-value-{suffix}").into_bytes();

    AcTransport::put(&client, &key, value)
        .await
        .expect("ac put should succeed");

    let refs = match AcTransport::list_refs(&client).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            skip_or_fail("GET /v1/ac (D-7) returned 405/501 — capability unavailable; ref-list round-trip skipped");
            return;
        }
        Err(e) => panic!("list_refs failed: {e}"),
    };
    assert!(
        refs.contains(&key),
        "list_refs must include the just-put ref key"
    );

    // Best-effort cleanup (ignore 405/501-unavailable / transient errors).
    let _ = AcTransport::delete(&client, &key).await;
}

// ---------------------------------------------------------------------------
// cas_delete_roundtrip (D-8)
// ---------------------------------------------------------------------------

/// D-8: PUT a blob → exists true → `CasTransport::delete` → exists false.
/// SKIPS if `DELETE /v1/cas/{tenant}/{hash}` (LIVE) returns 405/501-unavailable.
#[tokio::test]
async fn cas_delete_roundtrip() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let bytes = format!("cas-delete-{suffix}").into_bytes();
    let digest = Digest::of_bytes(&bytes);

    CasTransport::put(&client, &digest, bytes)
        .await
        .expect("cas put should succeed");
    assert!(
        CasTransport::exists(&client, &digest)
            .await
            .expect("cas exists should succeed"),
        "blob must be present after put"
    );

    match CasTransport::delete(&client, &digest).await {
        Ok(()) => {}
        Err(e) if is_capability_unavailable(&e) => {
            skip_or_fail("DELETE /v1/cas (D-8) returned 405/501 — capability unavailable; cas-delete round-trip skipped");
            return;
        }
        Err(e) => panic!("cas delete failed: {e}"),
    }

    let still = CasTransport::exists(&client, &digest)
        .await
        .expect("cas exists after delete should succeed");
    assert!(!still, "blob must be absent after delete");
}

// ---------------------------------------------------------------------------
// cas_list_roundtrip (D-8)
// ---------------------------------------------------------------------------

/// D-8: PUT a blob → `list_blobs` must include its hash → best-effort cleanup.
/// SKIPS if `GET /v1/cas/{tenant}` (LIVE) returns a 405/501-unavailable state.
#[tokio::test]
async fn cas_list_roundtrip() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };

    let suffix = run_suffix();
    let bytes = format!("cas-list-{suffix}").into_bytes();
    let digest = Digest::of_bytes(&bytes);

    CasTransport::put(&client, &digest, bytes)
        .await
        .expect("cas put should succeed");

    let blobs = match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_capability_unavailable(&e) => {
            skip_or_fail("GET /v1/cas (D-8) returned 405/501 — capability unavailable; cas-list round-trip skipped");
            return;
        }
        Err(e) => panic!("list_blobs failed: {e}"),
    };
    assert!(
        blobs.contains(&digest),
        "list_blobs must include the just-put blob hash"
    );

    // Best-effort cleanup.
    let _ = CasTransport::delete(&client, &digest).await;
}
