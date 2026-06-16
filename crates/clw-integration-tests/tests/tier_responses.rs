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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use clw_client::HttpClient;
use clw_types::{CasTransport, ClwError, Config, Digest};

// ---------------------------------------------------------------------------
// Shared constants / helpers (mirrors adversarial.rs — each tests/*.rs is its
// own crate, so the harness is replicated here rather than imported).
// ---------------------------------------------------------------------------

const TEST_PAT: &str = "test-pat";

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

fn last_segment(url: &wiremock::http::Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("")
        .to_string()
}

fn auth_ok(request: &Request) -> bool {
    request
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|a| a == format!("Bearer {TEST_PAT}"))
        .unwrap_or(false)
}

/// A digest whose bytes hash to it (so the client's integrity check passes).
fn digest_of(bytes: &[u8]) -> Digest {
    Digest::of_bytes(bytes)
}

// ---------------------------------------------------------------------------
// HostileResponder — a configurable, stateful, programmable misbehaving server
// (same shape as adversarial.rs).
// ---------------------------------------------------------------------------

/// One thing a route can be told to do on a given request.
#[derive(Clone)]
enum Behavior {
    /// Normal in-memory CAS/AC semantics (PUT stores, GET/HEAD reads).
    Normal,
    /// Return this raw status with no body.
    Status(u16),
}

/// A programmable responder shared (via `Arc`) across the routes of one tenant.
///
/// `script[route]` is a per-route queue of behaviors consumed one per matching
/// request; once drained the route falls back to `Normal`. `counts[route]`
/// counts requests seen on that route.
#[derive(Clone)]
struct HostileResponder {
    cas: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ac: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// route name → queue of behaviors (front = next request).
    script: Arc<Mutex<HashMap<&'static str, Vec<Behavior>>>>,
    counts: Arc<Mutex<HashMap<&'static str, Arc<AtomicUsize>>>>,
}

impl HostileResponder {
    fn new() -> Self {
        Self {
            cas: Arc::new(Mutex::new(HashMap::new())),
            ac: Arc::new(Mutex::new(HashMap::new())),
            script: Arc::new(Mutex::new(HashMap::new())),
            counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn counter(&self, route: &'static str) -> Arc<AtomicUsize> {
        Arc::clone(
            self.counts
                .lock()
                .unwrap()
                .entry(route)
                .or_insert_with(|| Arc::new(AtomicUsize::new(0))),
        )
    }

    fn count_of(&self, route: &'static str) -> usize {
        self.counter(route).load(Ordering::SeqCst)
    }

    /// Program a queue of behaviors for a route (consumed one per request).
    fn program(&self, route: &'static str, behaviors: Vec<Behavior>) {
        self.script.lock().unwrap().insert(route, behaviors);
    }

    /// Pop the next scripted behavior for `route`, or `Normal` once drained.
    fn next_behavior(&self, route: &'static str) -> Behavior {
        let mut s = self.script.lock().unwrap();
        match s.get_mut(route) {
            Some(q) if !q.is_empty() => q.remove(0),
            _ => Behavior::Normal,
        }
    }

    fn route_responder(&self, route: &'static str, kind: RouteKind) -> RouteResponder {
        RouteResponder {
            inner: self.clone(),
            route,
            kind,
            counter: self.counter(route),
        }
    }
}

#[derive(Clone, Copy)]
enum RouteKind {
    Cas,
    Ac,
}

/// A `Respond` impl bound to a single route name + kind, sharing the parent's
/// stores/script/counters.
#[derive(Clone)]
struct RouteResponder {
    inner: HostileResponder,
    route: &'static str,
    kind: RouteKind,
    counter: Arc<AtomicUsize>,
}

impl Respond for RouteResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.counter.fetch_add(1, Ordering::SeqCst);

        if !auth_ok(request) {
            return ResponseTemplate::new(401);
        }

        match self.inner.next_behavior(self.route) {
            Behavior::Status(s) => return ResponseTemplate::new(s),
            Behavior::Normal => {}
        }

        // Normal content-addressed semantics.
        let store = match self.kind {
            RouteKind::Cas => &self.inner.cas,
            RouteKind::Ac => &self.inner.ac,
        };
        let key = last_segment(&request.url);
        match request.method.as_str() {
            "PUT" => {
                store.lock().unwrap().insert(key, request.body.clone());
                ResponseTemplate::new(201)
            }
            "HEAD" => {
                if store.lock().unwrap().contains_key(&key) {
                    ResponseTemplate::new(200)
                } else {
                    ResponseTemplate::new(404)
                }
            }
            "GET" => match store.lock().unwrap().get(&key) {
                Some(d) => ResponseTemplate::new(200).set_body_bytes(d.clone()),
                None => ResponseTemplate::new(404),
            },
            "DELETE" => {
                store.lock().unwrap().remove(&key);
                ResponseTemplate::new(204)
            }
            _ => ResponseTemplate::new(405),
        }
    }
}

/// Mount the four object routes (CAS GET/PUT/HEAD/DELETE, AC GET/PUT/DELETE) of a
/// `HostileResponder` for `tenant`. Returns the responder so the test can program
/// behaviors + read counters.
async fn mount_hostile(server: &MockServer, tenant: &str) -> HostileResponder {
    let h = HostileResponder::new();
    let cas_obj = format!("/v1/cas/{tenant}/[a-f0-9]{{64}}");
    let ac_obj = format!("/v1/ac/{tenant}/[a-f0-9]{{64}}");

    for m in ["PUT", "GET", "HEAD", "DELETE"] {
        Mock::given(method(m))
            .and(path_regex(&cas_obj))
            .respond_with(h.route_responder("cas", RouteKind::Cas))
            .mount(server)
            .await;
    }
    for m in ["PUT", "GET", "DELETE"] {
        Mock::given(method(m))
            .and(path_regex(&ac_obj))
            .respond_with(h.route_responder("ac", RouteKind::Ac))
            .mount(server)
            .await;
    }
    h
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
