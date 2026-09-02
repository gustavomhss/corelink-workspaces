//! PER-TIER server-RESPONSE matrix tests for the `clw` client.
//!
//! Sibling to `adversarial.rs`: same hostile-mock harness (the `HostileResponder`
//! / `RouteResponder` pattern, `wiremock`, the `cfg(...)` client builder,
//! `tokio::time::timeout`-wrapping), but focused on a different axis — the HTTP
//! status codes a real CoreLink server emits when a TIER LIMIT is hit, and clw's
//! retry CLASSIFICATION of each.
//!
//! clw's retry classifier is `is_retryable` (clw-client lib.rs:642):
//!   `Transport => true; Http{status} => status>=500 || status==429; _ => false`.
//! So **429 and 5xx are RETRYABLE** (clw backs off and retries); everything else
//! is a clean, NON-retryable error surfaced on the FIRST response.
//!
//! The 6 server tiers (free / solo / starter / pro / max / enterprise) enforce
//! limits that surface to clw as these HTTP codes. This file maps each code to
//! the tier-axis it guards and asserts clw's CORRECT behavior for each:
//!
//! | code | tier-axis enforced                          | clw classification |
//! |------|---------------------------------------------|--------------------|
//! | 429  | per-tier RPS ceiling (free 10 … ent 10000)  | RETRYABLE (backoff)|
//! | 402  | per-tier storage/quota ceiling (free 10GB…) | NON-retryable      |
//! | 403  | insufficient PAT scope                       | NON-retryable      |
//! | 409  | residency / cross-region                     | NON-retryable      |
//! | 410  | erased / tombstoned (GDPR Art.17)            | NON-retryable      |
//! | 413  | blob cap (GLOBAL 160GiB, not per-tier)       | NON-retryable      |
//!
//! 429 / 413 / 401 are already covered by `adversarial.rs`; this file adds the
//! NOT-yet-covered per-tier-enforcement codes — **402, 403, 409, 410** — plus a
//! **429 positive control** proving the classification is real (429 IS retried
//! and ultimately succeeds, while the others are not).
//!
//! NON-VACUITY: each non-retryable case asserts the mock saw **exactly ONE**
//! request for the op — a regression that made the code retryable would drive the
//! count above 1 and FAIL the test. Every call is `timeout`-wrapped so a
//! retry-storm or hang also FAILS rather than passing trivially.
//!
//! UNIX-ONLY: matches `adversarial.rs` / `http_integration.rs`.
#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;
use wiremock::MockServer;

use clw_client::HttpClient;
use clw_types::{CasTransport, ClwError, Config, Digest};

mod common;

use common::{mount_hostile, Behavior, TEST_PAT};

// ---------------------------------------------------------------------------
// Shared constants / helpers.
// ---------------------------------------------------------------------------

/// Per-test tenant keeps wiremock routes + the in-memory store isolated so the
/// suite is parallel-safe. Each test passes a unique tenant string.
fn cfg(endpoint: String, tenant: &str, cache_dir: PathBuf) -> Config {
    Config {
        endpoint,
        tenant: tenant.to_string(),
        token: TEST_PAT.to_string(),
        cache_dir,
    }
}

/// A digest whose bytes hash to it (so the client's integrity check passes).
fn digest_of(bytes: &[u8]) -> Digest {
    Digest::of_bytes(bytes)
}

/// Drive ONE clw op against a route the server has been told to answer with
/// `status`, asserting the result is a NON-retryable, clean `ClwError::Http`
/// carrying that same status, AND — the NON-VACUITY guard — that the mock saw
/// **exactly ONE** request on `route` (no retry storm). `timeout`-wrapped so a
/// hang/loop FAILS rather than passing.
///
/// We exercise the op via a CAS GET (the read path most flows hit first): an
/// integration caller asking for a blob whose tier limit (quota/scope/residency/
/// erasure) is being enforced. The classification is shared across every CAS/AC
/// op (all route the catch-all status through `ClwError::Http`), so one op proves
/// the class.
async fn assert_non_retryable_cas_get(tenant: &str, status: u16) {
    let server = MockServer::start().await;
    let h = mount_hostile(&server, tenant).await;

    // The very next CAS request answers `status`; the queue is length 1, so a
    // SECOND request (a retry) would drain to `Normal` (404) — but a correct,
    // non-retrying client never makes that second request, and we assert it.
    h.program("cas", vec![Behavior::Status(status)]);

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    // Ask for a blob the server will reject with the tier-limit status.
    let dg = digest_of(format!("tier-probe-{status}").as_bytes());

    let res = tokio::time::timeout(Duration::from_secs(10), CasTransport::get(&client, &dg))
        .await
        .unwrap_or_else(|_| panic!("CAS GET on {status} must RETURN, not retry-loop/hang"));

    let err = res
        .err()
        .unwrap_or_else(|| panic!("status {status} must be an Err, got Ok(..)"));

    // Non-retryable codes surface verbatim as ClwError::Http{ status }.
    match &err {
        ClwError::Http {
            status: got_status, ..
        } => assert_eq!(
            *got_status, status,
            "status {status} must surface as ClwError::Http{{{status}}}, got {err:?}"
        ),
        other => panic!("status {status} must be ClwError::Http{{{status}}}, got {other:?}"),
    }

    // The classifier must NOT have retried it.
    assert!(
        !is_retryable_reference(&err),
        "status {status} must be classified NON-retryable, got {err:?}"
    );

    // NON-VACUITY: exactly one request — proof clw did NOT retry-storm it.
    assert_eq!(
        h.count_of("cas"),
        1,
        "status {status} is non-retryable → exactly ONE CAS request expected, saw {}",
        h.count_of("cas")
    );
}

/// A local mirror of clw-client's private `is_retryable` (lib.rs:642), so the
/// test asserts against the documented classification directly rather than only
/// inferring it from the request count.
fn is_retryable_reference(e: &ClwError) -> bool {
    match e {
        ClwError::Transport(_) => true,
        ClwError::Http { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

// ===========================================================================
// 402 — per-tier storage / quota ceiling (free 10GB … enterprise unlimited).
// ===========================================================================

/// AXIS: per-tier STORAGE/QUOTA ceiling. When a tenant exceeds its tier's
/// storage budget the server rejects the write/read attempt with 402. clw must
/// surface this as a clean NON-retryable `ClwError::Http{402}` — retrying a quota
/// rejection would storm the server pointlessly and never succeed. We assert the
/// error AND exactly one request (no retry).
#[tokio::test]
async fn quota_402_is_non_retryable_clean_error() {
    assert_non_retryable_cas_get("tier-402-quota", 402).await;
}

// ===========================================================================
// 403 — insufficient PAT scope.
// ===========================================================================

/// AXIS: PAT SCOPE. A token lacking the required scope for an op gets 403 (a
/// distinct, deliberate axis from 401 unauthenticated, which adversarial.rs
/// covers). clw must NOT retry a scope failure — re-presenting the same
/// insufficient PAT can never succeed. NON-retryable `ClwError::Http{403}`,
/// exactly one request.
#[tokio::test]
async fn scope_403_is_non_retryable_clean_error() {
    assert_non_retryable_cas_get("tier-403-scope", 403).await;
}

// ===========================================================================
// 409 — residency / cross-region conflict.
// ===========================================================================

/// AXIS: DATA RESIDENCY / cross-region. An op targeting a region the tenant's
/// residency policy forbids returns 409. The conflict is structural (wrong
/// region), so a retry to the SAME endpoint can never resolve it — clw must
/// surface NON-retryable `ClwError::Http{409}` on the first response, exactly one
/// request.
#[tokio::test]
async fn residency_409_is_non_retryable_clean_error() {
    assert_non_retryable_cas_get("tier-409-residency", 409).await;
}

// ===========================================================================
// 410 — erased / tombstoned (GDPR Art.17 right-to-erasure).
// ===========================================================================

/// AXIS: ERASURE / tombstone (GDPR Art.17). A blob that has been erased is
/// permanently gone; the server answers 410 Gone. Retrying cannot resurrect it —
/// clw must surface NON-retryable `ClwError::Http{410}` immediately, exactly one
/// request (a retry storm against a tombstone is both useless and noisy).
#[tokio::test]
async fn tombstone_410_is_non_retryable_clean_error() {
    assert_non_retryable_cas_get("tier-410-tombstone", 410).await;
}

// ===========================================================================
// 429 — POSITIVE CONTROL: per-tier RPS ceiling IS retried, then succeeds.
// ===========================================================================

/// AXIS: per-tier RPS ceiling (free 10rps … enterprise 10000rps). This is the
/// POSITIVE CONTROL proving the classification is REAL: unlike 402/403/409/410,
/// a 429 IS retryable. The CAS route answers 429 twice, then serves the stored
/// blob. clw must BACK OFF and RETRY — so the call ultimately SUCCEEDS and the
/// mock sees MORE THAN ONE request (>1). If clw treated 429 like the others
/// (count==1), the call would error on the first 429 and this assertion would
/// FAIL — proving the four non-retryable tests above are non-vacuous.
#[tokio::test]
async fn rps_429_is_retried_then_succeeds() {
    let server = MockServer::start().await;
    let tenant = "tier-429-rps";
    let h = mount_hostile(&server, tenant).await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    // Seed a real blob via a Normal PUT (script empty → stored), then read it
    // back through a 429 storm. We measure ONLY the GET attempts.
    let data = b"served-after-the-429-storm-clears".to_vec();
    let dg = digest_of(&data);
    CasTransport::put(&client, &dg, data.clone())
        .await
        .expect("seed PUT must succeed");
    let after_put = h.count_of("cas");

    // First 2 GETs → 429, then drain to Normal → the stored blob (200).
    h.program("cas", vec![Behavior::Status(429), Behavior::Status(429)]);

    let res = tokio::time::timeout(Duration::from_secs(30), CasTransport::get(&client, &dg))
        .await
        .expect("GET must RETURN within budget (bounded retries)");
    let got = res.expect("429 IS retryable → GET must ultimately SUCCEED after backoff");
    assert_eq!(got, data, "the retried GET must return the real blob bytes");

    // POSITIVE CONTROL: clw retried — strictly MORE than one GET attempt.
    let get_attempts = h.count_of("cas") - after_put;
    assert!(
        get_attempts > 1,
        "429 must be RETRIED (>1 GET attempt); saw {get_attempts} — \
         if this is 1, clw misclassified 429 as non-retryable"
    );
}
