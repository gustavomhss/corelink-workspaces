//! H4 — a controllable HOSTILE backend the REAL `clw` binary points at.
//!
//! Prod won't produce a redirect, a corrupt body, a throttle, a 5xx, or a 413 on
//! demand — so the security/robustness DEFENSES against them can't be evidenced
//! against prod. This is a local HTTP server (wiremock, bound to loopback, which clw
//! accepts as `http://127.0.0.1:…`) that the real binary drives via `CLW_ENDPOINT`.
//! It is STILL user-simulation — the binary makes real requests over a real socket;
//! only the backend is ours to make hostile. The binary's observed reaction (exit,
//! error, timing, and which hosts it did/didn't contact) is the evidence.

use wiremock::matchers::{any, method, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// A running hostile backend. Its `endpoint()` goes into `CLW_ENDPOINT`.
pub struct Hostile {
    server: MockServer,
}

impl Hostile {
    pub fn endpoint(&self) -> String {
        self.server.uri()
    }

    /// The request paths the server actually received — evidence of what the binary
    /// did (e.g. proof it did NOT follow a redirect to an internal host).
    pub async fn received_paths(&self) -> Vec<String> {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|r: &Request| r.url.path().to_string())
            .collect()
    }

    /// The HTTP methods the server actually received — evidence that the binary
    /// reached a given stage (e.g. that it actually issued a `PUT` before the 413).
    pub async fn received_methods(&self) -> Vec<String> {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|r: &Request| r.method.to_string())
            .collect()
    }

    /// Every request → 503. Evidence target: clw retries within its op-deadline and
    /// then FAILS with a clear error — it does not hang forever, does not cache the
    /// failure, and surfaces the status.
    pub async fn always_5xx() -> Hostile {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Hostile { server }
    }

    /// Every request → 302 redirecting to an "internal metadata" path on the same
    /// server. That internal path is mounted separately so we can PROVE, from the
    /// server's own request log, that clw NEVER followed the redirect (the SSRF /
    /// credential-exfiltration defense). clw's redirect policy is `none`, so it must
    /// surface the 3xx as an error instead of chasing the Location.
    pub async fn ssrf_redirect() -> Hostile {
        let server = MockServer::start().await;
        // The forbidden internal target. High priority so it (not the catch-all)
        // would record any follow. If clw is correct, it is never hit.
        Mock::given(path_regex(r"^/internal-metadata"))
            .respond_with(ResponseTemplate::new(200).set_body_string("SECRET-INTERNAL-DATA"))
            .with_priority(1)
            .named("ssrf-target (must never be followed)")
            .mount(&server)
            .await;
        let location = format!("{}/internal-metadata", server.uri());
        Mock::given(any())
            .respond_with(ResponseTemplate::new(302).insert_header("location", location.as_str()))
            .with_priority(5)
            .mount(&server)
            .await;
        Hostile { server }
    }

    /// HEAD/GET → 404 (blob absent → proceed to upload), PUT → 413. Evidence target:
    /// clw surfaces the refused/oversized blob as a clean, actionable error — never a
    /// silent drop or a hang.
    pub async fn put_413() -> Hostile {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(413))
            .mount(&server)
            .await;
        Hostile { server }
    }

    /// Every request → 429 with no `Retry-After`. Evidence target: clw backs off
    /// (AIMD) and then exhausts to a clear error — it does not hammer the server in a
    /// tight loop and does not hang.
    pub async fn always_429() -> Hostile {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        Hostile { server }
    }

    /// The AC ref key is ALREADY OCCUPIED (by different content), but the CAS accepts
    /// every blob. Concretely: HEAD/GET on a CAS blob → 404 (absent → clw uploads it),
    /// PUT on a CAS blob → 201 (accepted), GET on the AC ref → 200 with a non-empty body
    /// (the key is occupied), PUT on the AC ref → 409 (create-only reject).
    ///
    /// This reproduces the "a teammate re-snapshots a name that already exists with
    /// DIFFERENT content, without --force" story. Evidence target (a BEHAVIOUR, not a
    /// status): how many blobs does clw PUT to the CAS *before* it discovers the ref is
    /// occupied and 409s? Those uploads are wasted work + orphaned blobs. `cas_puts()`
    /// counts them from the server's own request log.
    ///
    /// (The ref body is deliberately opaque: clw treats an unparseable-but-present ref
    /// as "occupied, lineage unknown" — raw_present=true, no no-op short-circuit — so a
    /// create-only PUT still 409s. No clw-internal serialization is needed here.)
    pub async fn occupied_ref_conflict() -> Hostile {
        let server = MockServer::start().await;
        // AC ref GET → occupied (opaque body). High priority so it wins over the CAS
        // catch-alls for the /v1/ac/ path.
        Mock::given(method("GET"))
            .and(path_regex(r"/v1/ac/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("occupied-by-different-content"),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        // AC ref PUT → 409 create-only conflict.
        Mock::given(method("PUT"))
            .and(path_regex(r"/v1/ac/"))
            .respond_with(
                ResponseTemplate::new(409).set_body_string("already exists with different content"),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        // AC ref DELETE (only reached under --force) → accept.
        Mock::given(method("DELETE"))
            .and(path_regex(r"/v1/ac/"))
            .respond_with(ResponseTemplate::new(200))
            .with_priority(1)
            .mount(&server)
            .await;
        // CAS blob HEAD/GET → 404 (absent → clw will upload).
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        // CAS blob PUT → 201 accepted. These are the (wasted) uploads we count.
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
        Hostile { server }
    }

    /// How many blobs clw PUT to the CAS (`PUT /v1/cas/…`) — the count of chunk +
    /// manifest uploads. Under [`occupied_ref_conflict`], every one of these happened
    /// BEFORE the ref 409, so it is wasted work + an orphaned blob.
    pub async fn cas_puts(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r: &&Request| {
                r.method.to_string().eq_ignore_ascii_case("PUT")
                    && r.url.path().contains("/v1/cas/")
            })
            .count()
    }
}
