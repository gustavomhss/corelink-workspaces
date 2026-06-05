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
//! Run with:
//!   CLW_TENANT=<tenant> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live

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
    let token = std::env::var("CLW_TOKEN").ok();
    let tenant = std::env::var("CLW_TENANT").ok();

    match (token, tenant) {
        (Some(t), Some(tn)) => {
            let endpoint = std::env::var("CLW_ENDPOINT")
                .unwrap_or_else(|_| "https://corelink-api.humangr.com".to_string());
            let cfg = Config {
                endpoint,
                tenant: tn.clone(),
                token: t,
                cache_dir: PathBuf::from("/tmp/clw-conformance-cache"),
            };
            let client = HttpClient::new(cfg).expect("failed to build HttpClient");
            Some((client, tn))
        }
        _ => {
            eprintln!("SKIPPED: set CLW_TOKEN + CLW_TENANT to run live conformance");
            None
        }
    }
}

/// A unique, per-run suffix that avoids key collisions between test invocations.
/// Derived from `std::process::id()` XOR-folded with `SystemTime` nanos so it
/// changes each run even within the same process.
fn run_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}{:08x}", pid, nanos)
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
            eprintln!("SKIPPED: set CLW_TOKEN + CLW_TENANT to run live conformance");
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
