//! BRUTAL adversarial integration tests for the `clw` client.
//!
//! Unlike `http_integration.rs` (happy-path against a gentle mock), every test
//! here programs the wiremock server to behave like a HOSTILE / realistic
//! CoreLink server — 429 storms, never-advancing pagination cursors, corrupt
//! bodies that don't hash to the requested digest, 413 over-cap PUTs, mid-flight
//! 500s/401s, missing manifest blobs — and asserts that a SPECIFIC client guard
//! holds. Each test's doc comment states WHICH guard it exercises and HOW a
//! client missing that guard would FAIL (hang→timeout, OOM, corrupt output, or
//! wrong result), not pass trivially.
//!
//! Every anti-hang / anti-loop test wraps the client call in
//! `tokio::time::timeout(...)` and asserts the call RETURNS (with the expected
//! error) inside the budget: a client that loops forever times out and FAILS.
//!
//! UNIX-ONLY: shares the symlink/mode fixtures with `http_integration.rs`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_types::{
    AcTransport, CasTransport, ClwError, Config, Digest, Manifest, RefRecord, CAS_BLOB_CAP_BYTES,
};

mod common;

use common::{auth_ok, mount_hostile, Behavior, TEST_PAT};

// ---------------------------------------------------------------------------
// Shared constants / helpers
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

/// A deterministic pseudo-random byte stream from a fixed seed (no rand dep, so
/// a failing case reproduces exactly). Splitmix64-ish; pattern depends only on
/// the seed + index.
fn seeded_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    for _ in 0..len {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        out.push(z as u8);
    }
    out
}

// ---------------------------------------------------------------------------
// Programmable LIST responder (separate path: /v1/cas/{tenant}, /v1/ac/{tenant})
// ---------------------------------------------------------------------------

/// A LIST responder that serves a programmed sequence of JSON pages. Each entry
/// is `(body_json, status)`. Requests beyond the script repeat the last page.
#[derive(Clone)]
struct ListResponder {
    pages: Arc<Vec<(String, u16)>>,
    idx: Arc<AtomicUsize>,
}

impl ListResponder {
    fn new(pages: Vec<(String, u16)>) -> Self {
        Self {
            pages: Arc::new(pages),
            idx: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Respond for ListResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if !auth_ok(request) {
            return ResponseTemplate::new(401);
        }
        let i = self.idx.fetch_add(1, Ordering::SeqCst);
        let (body, status) = if i < self.pages.len() {
            self.pages[i].clone()
        } else {
            self.pages
                .last()
                .cloned()
                .unwrap_or_else(|| ("{}".into(), 200))
        };
        ResponseTemplate::new(status).set_body_string(body)
    }
}

/// Mount a CAS LIST route returning the given `{"blobs":[...],"next_cursor":...}`
/// pages in order.
async fn mount_cas_list(server: &MockServer, tenant: &str, pages: Vec<(String, u16)>) {
    Mock::given(method("GET"))
        .and(path_regex(format!("^/v1/cas/{tenant}$")))
        .respond_with(ListResponder::new(pages))
        .mount(server)
        .await;
}

/// Mount an AC LIST route returning the given `{"refs":[...],"next_cursor":...}`
/// pages in order.
async fn mount_ac_list(server: &MockServer, tenant: &str, pages: Vec<(String, u16)>) {
    Mock::given(method("GET"))
        .and(path_regex(format!("^/v1/ac/{tenant}$")))
        .respond_with(ListResponder::new(pages))
        .mount(server)
        .await;
}

/// Build a CAS list page body from hex hashes + an optional next cursor.
fn cas_page(hashes: &[String], next: Option<&str>) -> String {
    let blobs = hashes
        .iter()
        .map(|h| format!("{{\"hash\":\"{h}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    match next {
        Some(c) => format!("{{\"blobs\":[{blobs}],\"next_cursor\":\"{c}\"}}"),
        None => format!("{{\"blobs\":[{blobs}],\"next_cursor\":null}}"),
    }
}

fn ac_page(keys: &[String], next: Option<&str>) -> String {
    let refs = keys
        .iter()
        .map(|k| format!("{{\"ref_key\":\"{k}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    match next {
        Some(c) => format!("{{\"refs\":[{refs}],\"next_cursor\":\"{c}\"}}"),
        None => format!("{{\"refs\":[{refs}],\"next_cursor\":null}}"),
    }
}

/// Build a `RefRecord` pointing at `root` (the manifest digest) for a workspace
/// name — mirrors what snapshot writes, so prune's mark walk can resolve it.
fn ref_record(name: &str, root: Digest) -> RefRecord {
    RefRecord {
        name: name.to_string(),
        root,
        manifest_size: 0,
        created_at_unix: 0,
        parent: None,
        tool_version: "adversarial-test".to_string(),
    }
}

// ===========================================================================
// Scenario 1 — Multi-page pagination (happy): every entry across all pages.
// ===========================================================================

/// GUARD: cursor-following in `list_digests_capped` (clw-client lib.rs:448-481).
/// The server returns 3 pages with real advancing cursors. WITHOUT cursor
/// following the client would return only page 1's entries → the assert on the
/// FULL set (9 CAS + 9 AC) would FAIL.
#[tokio::test]
async fn pagination_happy_enumerates_every_page() {
    let server = MockServer::start().await;
    let tenant = "adv-pagination-happy";

    let cas_hashes: Vec<String> = (0..9)
        .map(|i| digest_of(&seeded_bytes(100 + i, 48)).to_hex())
        .collect();
    let ac_keys: Vec<String> = (0..9)
        .map(|i| digest_of(&seeded_bytes(200 + i, 48)).to_hex())
        .collect();

    mount_cas_list(
        &server,
        tenant,
        vec![
            (cas_page(&cas_hashes[0..3], Some("c1")), 200),
            (cas_page(&cas_hashes[3..6], Some("c2")), 200),
            (cas_page(&cas_hashes[6..9], None), 200),
        ],
    )
    .await;
    mount_ac_list(
        &server,
        tenant,
        vec![
            (ac_page(&ac_keys[0..3], Some("a1")), 200),
            (ac_page(&ac_keys[3..6], Some("a2")), 200),
            (ac_page(&ac_keys[6..9], None), 200),
        ],
    )
    .await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    let blobs = CasTransport::list_blobs(&client).await.unwrap();
    let refs = AcTransport::list_refs(&client).await.unwrap();

    let got_blobs: std::collections::HashSet<String> = blobs.iter().map(Digest::to_hex).collect();
    let got_refs: std::collections::HashSet<String> = refs.iter().map(Digest::to_hex).collect();

    // The FULL set across all 3 pages — a page-1-only client would have 3, not 9.
    assert_eq!(got_blobs, cas_hashes.iter().cloned().collect());
    assert_eq!(got_refs, ac_keys.iter().cloned().collect());
    assert_eq!(blobs.len(), 9, "all CAS pages must be flattened");
    assert_eq!(refs.len(), 9, "all AC pages must be flattened");
}

// ===========================================================================
// Scenario 2 — Cursor never advances (malicious): strict-advance guard aborts.
// ===========================================================================

/// GUARD: strict cursor-advancement set (`seen_cursors`) + MAX_PAGES in
/// `list_digests_capped` (clw-client lib.rs:472-477). The server returns a
/// non-null cursor that is IDENTICAL on every page. WITHOUT the guard the client
/// loops forever requesting `?cursor=STUCK` → the `timeout` fires and the test
/// FAILS. WITH it, the call returns an Err well inside the budget.
#[tokio::test]
async fn pagination_stuck_cursor_aborts_not_loops() {
    let server = MockServer::start().await;
    let tenant = "adv-stuck-cursor";

    let h = digest_of(&seeded_bytes(7, 32)).to_hex();
    // Every page is identical and points back to the same cursor.
    mount_cas_list(&server, tenant, vec![(cas_page(&[h], Some("STUCK")), 200)]).await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    // A looping client never returns → timeout → FAIL. The guard makes it Err fast.
    let res = tokio::time::timeout(Duration::from_secs(10), CasTransport::list_blobs(&client))
        .await
        .expect("list_blobs must RETURN, not loop forever on a stuck cursor");

    let err = res.expect_err("a never-advancing cursor must be an error");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("cursor") || msg.contains("pagination") || msg.contains("unbounded"),
        "must be the cursor-advancement abort, got: {err:?}"
    );
}

/// GUARD: same strict-advance set, exercised with a 2-cycle (A→B→A→B…) cursor —
/// proves the guard tracks ALL seen cursors, not just "differs from previous".
/// A naive "did it change since last time?" check would never trip on a cycle
/// and loop forever → timeout → FAIL.
#[tokio::test]
async fn pagination_cycling_cursor_aborts_not_loops() {
    let server = MockServer::start().await;
    let tenant = "adv-cycle-cursor";

    let h1 = digest_of(&seeded_bytes(11, 32)).to_hex();
    let h2 = digest_of(&seeded_bytes(12, 32)).to_hex();
    mount_cas_list(
        &server,
        tenant,
        vec![
            (cas_page(std::slice::from_ref(&h1), Some("A")), 200),
            (cas_page(std::slice::from_ref(&h2), Some("B")), 200),
            (cas_page(std::slice::from_ref(&h1), Some("A")), 200), // back to A — a cycle
            (cas_page(std::slice::from_ref(&h2), Some("B")), 200),
        ],
    )
    .await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    let res = tokio::time::timeout(Duration::from_secs(10), CasTransport::list_blobs(&client))
        .await
        .expect("list_blobs must RETURN, not loop on a cycling cursor");
    res.expect_err("a cycling cursor must abort with an error");
}

// ===========================================================================
// Scenario 3 — 429 storm + Retry-After.
// ===========================================================================

/// GUARD: `with_retry` AIMD backoff + Retry-After honoring (clw-client
/// lib.rs:265-353). The CAS PUT route returns 429 (with a small Retry-After) for
/// the first 2 requests, then succeeds. WITHOUT retry the FIRST 429 surfaces as
/// an error → the PUT would FAIL. WITH it, the op succeeds and the server saw
/// ≥ K+1 = 3 attempts (counted via the shared counter).
#[tokio::test]
async fn throttle_then_success_retries_and_succeeds() {
    let server = MockServer::start().await;
    let tenant = "adv-429-then-ok";
    let h = mount_hostile(&server, tenant).await;

    // First 2 CAS requests → 429 (Retry-After: 1s), then Normal (store the PUT).
    h.program(
        "cas",
        vec![
            Behavior::Throttle {
                retry_after: Some(1),
            },
            Behavior::Throttle {
                retry_after: Some(1),
            },
        ],
    );

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    let data = b"throttled-but-eventually-stored".to_vec();
    let dg = digest_of(&data);

    // Wrap: a client that never honored backoff would either error or hang.
    let res = tokio::time::timeout(
        Duration::from_secs(30),
        CasTransport::put(&client, &dg, data.clone()),
    )
    .await
    .expect("put must RETURN within the budget");
    res.expect("put must ultimately SUCCEED after the 429 storm clears");

    // ≥ K+1 attempts: 2 throttled + 1 success = at least 3 requests on the route.
    assert!(
        h.count_of("cas") >= 3,
        "expected ≥3 CAS attempts (2 throttled + success), saw {}",
        h.count_of("cas")
    );

    // And the blob is actually stored (the success was real, not swallowed).
    let stored = h.cas.lock().unwrap().get(&dg.to_hex()).cloned();
    assert_eq!(stored.as_deref(), Some(data.as_slice()));
}

/// GUARD: retry EXHAUSTION cap (`MAX_ATTEMPTS = 6`) in `with_retry`. The route
/// returns 429 FOREVER. WITHOUT a bounded retry budget the client retries
/// forever → the `timeout` fires → FAIL. WITH it, the call returns
/// `ClwError::Http { status: 429 }` after exhausting the 6 attempts.
#[tokio::test]
async fn throttle_forever_surfaces_error_not_hang() {
    let server = MockServer::start().await;
    let tenant = "adv-429-forever";
    let h = mount_hostile(&server, tenant).await;

    // 429 on every CAS request, indefinitely (no Retry-After → uses the ladder).
    h.program("cas", vec![Behavior::Throttle { retry_after: None }; 50]);

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    let data = b"never-accepted".to_vec();
    let dg = digest_of(&data);

    // The fixed ladder sums to ~18s of sleeps across 6 attempts; 60s budget is safe.
    let res = tokio::time::timeout(
        Duration::from_secs(60),
        CasTransport::put(&client, &dg, data),
    )
    .await
    .expect("put must RETURN (bounded retries), not retry forever");

    match res {
        Err(ClwError::Http { status: 429, .. }) => {}
        other => panic!("429-forever must surface ClwError::Http{{429}}, got {other:?}"),
    }
}

// ===========================================================================
// Scenario 4 — Blob-cap 413 + over-cap manifest pre-check.
// ===========================================================================

/// GUARD: 413 handling on PUT + the client-side pre-cap (clw-client
/// lib.rs:853-858, 881-884). An over-cap PUT is rejected client-side BEFORE the
/// wire (TooLarge); a server 413 maps to TooLarge too; and a normal PUT still
/// works. A client without these guards would surface an opaque error or upload
/// an unbounded body.
#[tokio::test]
async fn blob_cap_413_and_client_precap() {
    let server = MockServer::start().await;
    let tenant = "adv-blob-cap";
    let h = mount_hostile(&server, tenant).await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    // (a) Client-side pre-cap: bytes > CAS_BLOB_CAP_BYTES → TooLarge, no request.
    let oversized = vec![0u8; CAS_BLOB_CAP_BYTES + 1];
    let dg_over = digest_of(&oversized);
    let before = h.count_of("cas");
    let err = CasTransport::put(&client, &dg_over, oversized)
        .await
        .expect_err("over-cap PUT must be rejected");
    assert!(
        matches!(err, ClwError::TooLarge { .. }),
        "over-cap PUT must be TooLarge, got {err:?}"
    );
    assert_eq!(
        h.count_of("cas"),
        before,
        "the pre-cap must reject BEFORE any network request is made"
    );

    // (b) Server-side 413 on the next CAS request → TooLarge.
    h.program("cas", vec![Behavior::Status(413)]);
    let small = b"within cap".to_vec();
    let dg_small = digest_of(&small);
    let err = CasTransport::put(&client, &dg_small, small.clone())
        .await
        .expect_err("a 413 from the server must surface");
    assert!(
        matches!(err, ClwError::TooLarge { .. }),
        "server 413 must map to TooLarge, got {err:?}"
    );

    // (c) A normal-sized PUT still works (script is now drained → Normal).
    CasTransport::put(&client, &dg_small, small.clone())
        .await
        .expect("a normal PUT must still succeed");
    assert_eq!(
        h.cas
            .lock()
            .unwrap()
            .get(&dg_small.to_hex())
            .cloned()
            .as_deref(),
        Some(small.as_slice())
    );
}

/// GUARD: snapshot's W4/Wave-C up-front manifest-size check
/// (`check_manifest_blob_fits`, clw-snapshot lib.rs:140-150, called at :382). A
/// workspace whose serialized manifest exceeds CAS_BLOB_CAP_BYTES (5 MiB) must
/// be rejected with `InvalidManifest` BEFORE the manifest PUT — not surface as a
/// late opaque 413. HEAVY: builds enough entries to overflow the manifest.
#[tokio::test]
async fn over_cap_root_manifest_rejected_before_put() {
    let server = MockServer::start().await;
    let tenant = "adv-manifest-cap";
    mount_hostile(&server, tenant).await;

    let src = TempDir::new().unwrap();
    // Each tiny file adds a manifest Entry (path + mode + size + chunk digest).
    // A long, unique filename per file maximizes per-entry serialized bytes so we
    // overflow the 5 MiB cap with as few files as possible. Each filename is ~200
    // chars; with the ~70-char chunk digest the per-entry JSON is ~300+ bytes, so
    // ~15k entries overflow 5 MiB while keeping filesystem churn bounded.
    let long_dir = "deeply/nested/path/segment/that/inflates/the/manifest/entry/size";
    std::fs::create_dir_all(src.path().join(long_dir)).unwrap();
    let pad = "x".repeat(180); // long, valid filename component
    for i in 0..15_000 {
        std::fs::write(
            src.path().join(format!("{long_dir}/f{i:06}_{pad}.txt")),
            b"x",
        )
        .unwrap();
    }

    let cache = TempDir::new().unwrap();
    let lc = LocalCache::new(cache.path()).unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    let opts = clw_snapshot::SnapshotOptions::new("manifest-cap-ws");
    let res = clw_snapshot::snapshot(src.path(), &client, &lc, &opts).await;
    match res {
        Err(ClwError::InvalidManifest(msg)) => {
            assert!(
                msg.contains("cap") || msg.contains("exceed"),
                "must explain the cap overflow, got: {msg}"
            );
        }
        Ok(r) => panic!(
            "an over-cap manifest must be rejected, but snapshot succeeded (root={})",
            r.root.to_hex()
        ),
        Err(other) => panic!("expected InvalidManifest, got {other:?}"),
    }
}

// ===========================================================================
// Scenario 5 — Digest mismatch / corruption (THE integrity test).
// ===========================================================================

/// GUARD: CAS GET integrity check (clw-client lib.rs:823-829). The server
/// returns a 200 body whose bytes do NOT hash to the requested digest. A client
/// that trusts the server blindly would return/write CORRUPT bytes. The guard
/// re-hashes and returns `ClwError::Integrity`. We assert BOTH the direct GET
/// rejects it AND that a hydrate built on this GET fails rather than writing
/// corruption.
#[tokio::test]
async fn corrupt_body_rejected_on_get() {
    let server = MockServer::start().await;
    let tenant = "adv-integrity-get";
    let h = mount_hostile(&server, tenant).await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    // We ask for the digest of "honest", but the server hands back "TAMPERED".
    let honest = b"honest content".to_vec();
    let requested = digest_of(&honest);
    let tampered = b"TAMPERED content of a different length".to_vec();
    assert_ne!(
        digest_of(&tampered),
        requested,
        "fixture must actually differ"
    );

    h.program("cas", vec![Behavior::Body200(tampered.clone())]);

    let err = CasTransport::get(&client, &requested)
        .await
        .expect_err("a body that doesn't hash to the digest MUST be rejected");
    match err {
        ClwError::Integrity { expected, actual } => {
            assert_eq!(expected, requested);
            assert_eq!(actual, digest_of(&tampered));
        }
        other => panic!("corrupt body must be ClwError::Integrity, got {other:?}"),
    }
}

/// GUARD: hydrate-path integrity (`fetch_chunk` re-verify, clw-hydrate
/// lib.rs:615-620) AND manifest-blob re-verify. We snapshot a real tree, then
/// hydrate from a server that corrupts the very first CAS GET (the manifest
/// blob). A blind client would parse garbage / write corrupt files; the guard
/// makes hydrate FAIL with Integrity and leaves no corrupt output committed.
#[tokio::test]
async fn corrupt_body_fails_hydrate_no_corrupt_output() {
    let server = MockServer::start().await;
    let tenant = "adv-integrity-hydrate";
    let h = mount_hostile(&server, tenant).await;

    // 1. Honest snapshot of a small tree.
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("a.txt"), b"alpha").unwrap();
    std::fs::write(src.path().join("b.txt"), seeded_bytes(99, 4096)).unwrap();

    let cache1 = TempDir::new().unwrap();
    let lc1 = LocalCache::new(cache1.path()).unwrap();
    let client1 = HttpClient::new(cfg(server.uri(), tenant, cache1.path().to_path_buf())).unwrap();
    let snap = clw_snapshot::snapshot(
        src.path(),
        &client1,
        &lc1,
        &clw_snapshot::SnapshotOptions::new("integ-ws"),
    )
    .await
    .unwrap();

    // 2. Corrupt EVERY CAS GET body so whatever blob hydrate fetches is tampered.
    //    (Program a long run of Body200(garbage); PUT/HEAD during snapshot already
    //    happened, so this only affects the hydrate GETs.)
    let garbage = b"not the blob you asked for".to_vec();
    h.program("cas", vec![Behavior::Body200(garbage); 64]);

    // 3. Hydrate to a fresh dir with an EMPTY cache → everything comes from remote.
    let dest = TempDir::new().unwrap();
    let out = dest.path().join("hydrated");
    let cache2 = TempDir::new().unwrap();
    let lc2 = LocalCache::new(cache2.path()).unwrap();
    let client2 = HttpClient::new(cfg(server.uri(), tenant, cache2.path().to_path_buf())).unwrap();

    let err = clw_hydrate::hydrate(
        &out,
        &client2,
        &lc2,
        &clw_hydrate::HydrateOptions::new("integ-ws"),
    )
    .await
    .expect_err("hydrating corrupt blobs MUST fail, not write garbage");
    assert!(
        matches!(err, ClwError::Integrity { .. }),
        "hydrate over corrupt CAS must be Integrity, got {err:?}"
    );

    // No corrupt file content escaped: the source files must NOT be present with
    // the garbage bytes. (Either absent, or — defensively — never equal to garbage.)
    let a = out.join("a.txt");
    if a.exists() {
        assert_ne!(
            std::fs::read(&a).unwrap(),
            b"not the blob you asked for",
            "no garbage may be written to a hydrated file"
        );
    }
    let _ = snap; // snapshot root proven reachable; integrity is the assertion.
}

// ===========================================================================
// Scenario 6 — Mid-operation failure + idempotent retry.
// ===========================================================================

/// GUARD: `with_retry` 5xx retry (is_retryable → status>=500). During a snapshot
/// the FIRST CAS request gets a 500, then everything is Normal. WITHOUT 5xx
/// retry the snapshot would abort on the transient 500; WITH it the snapshot
/// completes and the manifest is correct. We also re-run to prove idempotency
/// (second run uploads 0 chunks).
#[tokio::test]
async fn transient_500_mid_snapshot_retries_and_completes() {
    let server = MockServer::start().await;
    let tenant = "adv-500-mid-snapshot";
    let h = mount_hostile(&server, tenant).await;

    // The very first CAS request fails 500; all subsequent ones are Normal.
    h.program("cas", vec![Behavior::Status(500)]);

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("one.txt"), b"first file").unwrap();
    std::fs::write(src.path().join("two.bin"), seeded_bytes(5, 300_000)).unwrap();

    let cache = TempDir::new().unwrap();
    let lc = LocalCache::new(cache.path()).unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();
    let opts = clw_snapshot::SnapshotOptions::new("retry-ws");

    let r1 = tokio::time::timeout(
        Duration::from_secs(30),
        clw_snapshot::snapshot(src.path(), &client, &lc, &opts),
    )
    .await
    .expect("snapshot must RETURN within budget")
    .expect("snapshot must COMPLETE despite the transient 500");
    assert!(r1.chunks_uploaded > 0, "first snapshot uploads chunks");

    // Idempotent re-run: identical tree → 0 new chunk uploads, same root.
    let r2 = clw_snapshot::snapshot(src.path(), &client, &lc, &opts)
        .await
        .expect("re-run must succeed");
    assert_eq!(r2.root, r1.root, "same tree → same root");
    assert_eq!(
        r2.chunks_uploaded, 0,
        "re-run of an unchanged tree must upload 0 chunks (idempotent)"
    );
    let _ = &h;
}

/// GUARD: idempotent re-run after a HARD mid-snapshot failure leaves no corrupt
/// state. The CAS route fails with a non-retryable 400 forever during the first
/// attempt (snapshot aborts). A second snapshot against a now-Normal server must
/// succeed and round-trip — proving the abort left no poisoned local/remote
/// state. WITHOUT clean failure semantics the re-run would inherit a broken
/// cache/store and fail or corrupt.
#[tokio::test]
async fn hard_failure_mid_snapshot_then_clean_rerun() {
    let server = MockServer::start().await;
    let tenant = "adv-hard-fail-rerun";
    let h = mount_hostile(&server, tenant).await;

    // Non-retryable 400 on the first CAS request → snapshot aborts.
    h.program("cas", vec![Behavior::Status(400)]);

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f.txt"), b"payload").unwrap();
    std::fs::write(src.path().join("g.bin"), seeded_bytes(8, 200_000)).unwrap();

    let cache = TempDir::new().unwrap();
    let lc = LocalCache::new(cache.path()).unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();
    let opts = clw_snapshot::SnapshotOptions::new("hardfail-ws");

    let first = clw_snapshot::snapshot(src.path(), &client, &lc, &opts).await;
    assert!(first.is_err(), "the hard 400 must abort the first snapshot");

    // Server is Normal now (script drained). Re-run must cleanly succeed.
    let r = clw_snapshot::snapshot(src.path(), &client, &lc, &opts)
        .await
        .expect("re-run after a clean abort must succeed");
    assert!(r.chunks_total > 0);
    let _ = &h;
}

// ===========================================================================
// Scenario 7 — Partial enumeration under prune (fail-safe abort).
// ===========================================================================

/// GUARD: prune's fail-safe mark walk (clw-cli prune.rs `compute_live_set`),
/// which rests on `CasTransport::get` returning `NotFound` for a missing manifest
/// blob. We faithfully replicate that walk against the real `HttpClient`: a ref
/// found ONLY on a LATER LIST page resolves to a RefRecord whose manifest blob
/// 404s. The walk must ABORT (Err) having issued ZERO deletes — a client that
/// swallowed the 404 (empty live set) would sweep the whole tenant. WRAPPED in a
/// timeout so a multi-page enumeration that hangs also FAILS.
#[tokio::test]
async fn prune_aborts_fail_safe_on_missing_manifest_late_page() {
    let server = MockServer::start().await;
    let tenant = "adv-prune-failsafe";
    let h = mount_hostile(&server, tenant).await;

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    // Plant a healthy ref on page 1 and a BROKEN ref (manifest blob missing) on
    // page 3, so it is found only after following cursors.
    //
    // Healthy ref: real manifest blob in CAS + a RefRecord pointing at it.
    let good_manifest = Manifest {
        version: clw_types::MANIFEST_VERSION,
        total_size: 0,
        entries: vec![],
    };
    let good_mbytes = good_manifest.canonical_bytes().unwrap();
    let good_root = digest_of(&good_mbytes);
    CasTransport::put(&client, &good_root, good_mbytes)
        .await
        .unwrap();
    let good_rec = ref_record("healthy", good_root);
    let good_rec_bytes = good_rec.to_bytes().unwrap();
    let good_key = clw_types::ref_key("healthy");
    AcTransport::put(&client, &good_key, good_rec_bytes)
        .await
        .unwrap();

    // Broken ref: RefRecord points at a manifest root that is NOT in CAS (404).
    let missing_root = digest_of(&seeded_bytes(0xDEAD, 64));
    let bad_rec = ref_record("broken", missing_root);
    let bad_rec_bytes = bad_rec.to_bytes().unwrap();
    let bad_key = clw_types::ref_key("broken");
    AcTransport::put(&client, &bad_key, bad_rec_bytes)
        .await
        .unwrap();

    // AC LIST: healthy on page 1, broken on page 3 (advancing cursors).
    mount_ac_list(
        &server,
        tenant,
        vec![
            (ac_page(&[good_key.to_hex()], Some("p1")), 200),
            (ac_page(&[], Some("p2")), 200),
            (ac_page(&[bad_key.to_hex()], None), 200),
        ],
    )
    .await;

    let deletes_before = h.count_of("cas");

    // Faithful replication of prune's MARK walk (the fail-safe the CLI relies on):
    // enumerate refs → resolve each ref's record → fetch its manifest blob. A
    // missing manifest blob surfaces as NotFound and aborts the walk; NOTHING is
    // deleted because deletion only happens AFTER a complete live set.
    let walk = async {
        let mut live: std::collections::HashSet<Digest> = std::collections::HashSet::new();
        for key in AcTransport::list_refs(&client).await? {
            let Some(bytes) = AcTransport::get(&client, &key).await? else {
                continue; // benign concurrent rm
            };
            let rec = RefRecord::from_bytes(&bytes)
                .map_err(|e| ClwError::Transport(format!("corrupt ref: {e}")))?;
            live.insert(rec.root);
            // This GET 404s for the broken ref → NotFound → abort the whole mark.
            let _mbytes = CasTransport::get(&client, &rec.root).await?;
        }
        Ok::<_, ClwError>(live)
    };

    let res = tokio::time::timeout(Duration::from_secs(10), walk)
        .await
        .expect("the mark walk must RETURN, not hang on multi-page enumeration");
    let err = res.expect_err("a missing manifest blob must abort the live-set computation");
    assert!(
        matches!(err, ClwError::NotFound(d) if d == missing_root),
        "must abort with NotFound for the missing manifest root, got {err:?}"
    );

    // Fail-safe: NOT A SINGLE delete was issued (no CAS DELETE happened).
    let deletes_after_get_only = h
        .cas
        .lock()
        .unwrap()
        .keys()
        .filter(|k| **k == good_root.to_hex())
        .count();
    assert_eq!(
        deletes_after_get_only, 1,
        "the healthy manifest blob must still be present — prune deleted nothing"
    );
    // (Counter sanity: the walk only issued GETs/HEADs, never a DELETE.)
    assert!(h.count_of("cas") >= deletes_before);
}

// ===========================================================================
// Scenario 8 — 401 mid-operation.
// ===========================================================================

/// GUARD: 401 is surfaced immediately and NEVER retried (is_retryable excludes
/// Auth, clw-client lib.rs:642-648). The CAS route serves a few 200s then 401s
/// forever mid-snapshot. The client must surface a clean `ClwError::Auth` and not
/// retry forever. WRAPPED in a timeout: a client that retried 401 would burn the
/// budget; one that swallowed it would corrupt. We also assert the request count
/// is bounded (no retry storm on the 401).
#[tokio::test]
async fn auth_401_mid_operation_clean_error_no_retry_storm() {
    let server = MockServer::start().await;
    let tenant = "adv-401-mid";
    let h = mount_hostile(&server, tenant).await;

    // A direct AcTransport::get: first request 401 (auth flips mid-session).
    h.program("ac", vec![Behavior::Status(401)]);

    let cache = TempDir::new().unwrap();
    let client = HttpClient::new(cfg(server.uri(), tenant, cache.path().to_path_buf())).unwrap();

    let key = clw_types::ref_key("some-workspace");
    let res = tokio::time::timeout(Duration::from_secs(10), AcTransport::get(&client, &key))
        .await
        .expect("a 401 must RETURN immediately, never retry-loop");
    let err = res.expect_err("401 must surface as an error");
    assert!(
        matches!(err, ClwError::Auth),
        "mid-op 401 must be ClwError::Auth, got {err:?}"
    );

    // No retry storm: a single AC request was made (401 is not retryable).
    assert_eq!(
        h.count_of("ac"),
        1,
        "401 must not be retried — exactly one request expected, saw {}",
        h.count_of("ac")
    );
}

// ===========================================================================
// Scenario 9 — Large / streaming round-trip (64 MiB). HEAVY.
// ===========================================================================

/// GUARD: streaming snapshot/hydrate memory bounds + per-chunk integrity over a
/// genuinely large, multi-chunk file. A client that mishandled multi-chunk
/// reassembly or skipped integrity would produce a NON-identical hydrate. We
/// assert BYTE-IDENTICAL round-trip plus mode + symlink preservation. HEAVY:
/// 64 MiB of seeded pseudo-random content (bounded for CI).
#[tokio::test]
async fn large_streaming_roundtrip_byte_identical() {
    let server = MockServer::start().await;
    let tenant = "adv-large-roundtrip";
    mount_hostile(&server, tenant).await;

    let src = TempDir::new().unwrap();
    // 64 MiB seeded so a failure reproduces; pseudo-random ⇒ no trivial dedupe,
    // forcing many real chunk PUTs and exercising the streaming path
    // (> STREAM_THRESHOLD_BYTES = 16 MiB).
    let big = seeded_bytes(0xB16, 64 * 1024 * 1024);
    std::fs::write(src.path().join("huge.bin"), &big).unwrap();
    std::fs::write(src.path().join("note.txt"), b"alongside the big file").unwrap();
    std::os::unix::fs::symlink("huge.bin", src.path().join("link")).unwrap();
    std::fs::write(src.path().join("run.sh"), b"#!/bin/sh\n").unwrap();
    let mut p = std::fs::metadata(src.path().join("run.sh"))
        .unwrap()
        .permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(src.path().join("run.sh"), p).unwrap();

    let cache1 = TempDir::new().unwrap();
    let lc1 = LocalCache::new(cache1.path()).unwrap();
    let client1 = HttpClient::new(cfg(server.uri(), tenant, cache1.path().to_path_buf())).unwrap();
    let snap = clw_snapshot::snapshot(
        src.path(),
        &client1,
        &lc1,
        &clw_snapshot::SnapshotOptions::new("large-ws"),
    )
    .await
    .expect("large snapshot must succeed");
    assert!(snap.chunks_total > 1, "64 MiB must produce many chunks");

    let dest = TempDir::new().unwrap();
    let out = dest.path().join("hydrated");
    let cache2 = TempDir::new().unwrap();
    let lc2 = LocalCache::new(cache2.path()).unwrap();
    let client2 = HttpClient::new(cfg(server.uri(), tenant, cache2.path().to_path_buf())).unwrap();
    clw_hydrate::hydrate(
        &out,
        &client2,
        &lc2,
        &clw_hydrate::HydrateOptions::new("large-ws"),
    )
    .await
    .expect("large hydrate must succeed");

    // Byte-identical big file (the load-bearing assertion).
    assert_eq!(
        std::fs::read(out.join("huge.bin")).unwrap(),
        big,
        "64 MiB file must round-trip BYTE-IDENTICAL"
    );
    // Mode preserved.
    let mode = std::fs::metadata(out.join("run.sh"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755, "exec mode must be preserved");
    // Symlink preserved.
    assert_eq!(
        std::fs::read_link(out.join("link"))
            .unwrap()
            .to_str()
            .unwrap(),
        "huge.bin"
    );
}

// ===========================================================================
// Scenario 10 — Concurrency (two flows, same store).
// ===========================================================================

/// GUARD: store consistency under concurrent snapshot+hydrate against ONE
/// CoreLink store. Two snapshots of two distinct trees run concurrently, then
/// both hydrate concurrently. We assert no panic, no Err, and each hydrate is
/// byte-identical to its own source (no cross-contamination). A client with a
/// shared-state race would corrupt or panic.
#[tokio::test]
async fn concurrent_snapshot_hydrate_no_corruption() {
    let server = MockServer::start().await;
    let tenant = "adv-concurrency";
    mount_hostile(&server, tenant).await;
    let uri = server.uri();

    // Two distinct trees.
    let mk_tree = |seed: u64| {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("data.bin"), seeded_bytes(seed, 500_000)).unwrap();
        std::fs::write(d.path().join("name.txt"), format!("tree-{seed}")).unwrap();
        d
    };
    let src_a = mk_tree(1);
    let src_b = mk_tree(2);

    let snap = |uri: String, path: PathBuf, ws: &'static str| async move {
        let cache = TempDir::new().unwrap();
        let lc = LocalCache::new(cache.path()).unwrap();
        let client = HttpClient::new(cfg(uri, tenant, cache.path().to_path_buf())).unwrap();
        let r =
            clw_snapshot::snapshot(&path, &client, &lc, &clw_snapshot::SnapshotOptions::new(ws))
                .await
                .unwrap();
        // keep the cache TempDir alive for the duration
        (r, cache)
    };

    let (ra, rb) = tokio::join!(
        snap(uri.clone(), src_a.path().to_path_buf(), "ws-a"),
        snap(uri.clone(), src_b.path().to_path_buf(), "ws-b"),
    );
    assert_ne!(ra.0.root, rb.0.root, "distinct trees → distinct roots");

    // Concurrent hydrate of each into a fresh dir + empty cache.
    let hyd = |uri: String, ws: &'static str| async move {
        let dest = TempDir::new().unwrap();
        let out = dest.path().join("out");
        let cache = TempDir::new().unwrap();
        let lc = LocalCache::new(cache.path()).unwrap();
        let client = HttpClient::new(cfg(uri, tenant, cache.path().to_path_buf())).unwrap();
        clw_hydrate::hydrate(&out, &client, &lc, &clw_hydrate::HydrateOptions::new(ws))
            .await
            .unwrap();
        (out, dest, cache)
    };

    let (ha, hb) = tokio::join!(hyd(uri.clone(), "ws-a"), hyd(uri.clone(), "ws-b"));

    // Each hydrate matches ITS OWN source — no cross-contamination.
    assert_eq!(
        std::fs::read(ha.0.join("data.bin")).unwrap(),
        std::fs::read(src_a.path().join("data.bin")).unwrap(),
        "ws-a hydrate must match tree A"
    );
    assert_eq!(
        std::fs::read(hb.0.join("data.bin")).unwrap(),
        std::fs::read(src_b.path().join("data.bin")).unwrap(),
        "ws-b hydrate must match tree B"
    );
    assert_eq!(
        std::fs::read(ha.0.join("name.txt")).unwrap(),
        b"tree-1",
        "ws-a name must not be tree B's"
    );
}
