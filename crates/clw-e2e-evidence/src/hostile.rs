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
}
