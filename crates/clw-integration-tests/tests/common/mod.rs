//! Shared hostile WireMock harness for the adversarial integration suites.
//!
//! A responder is created and mounted per test/tenant. Its in-memory CAS/AC
//! stores, scripted per-route behavior queues, and request counters are all
//! instance-local, keeping parallel tests isolated.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub const TEST_PAT: &str = "test-pat";

pub fn last_segment(url: &wiremock::http::Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("")
        .to_string()
}

pub fn auth_ok(request: &Request) -> bool {
    request
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|a| a == format!("Bearer {TEST_PAT}"))
        .unwrap_or(false)
}

/// One thing a route can be told to do on a given request.
// `tier_responses.rs` uses only `Normal` and `Status`; this shared superset is
// also compiled by `adversarial.rs`, which exercises the two hostile variants.
#[allow(dead_code)]
#[derive(Clone)]
pub enum Behavior {
    /// Normal in-memory CAS/AC semantics (PUT stores, GET/HEAD reads).
    Normal,
    /// Return this raw status with no body.
    Status(u16),
    /// Return 200 with this exact body — used to inject a wrong body for a digest.
    Body200(Vec<u8>),
    /// Return 429 with an optional `Retry-After: <secs>` header.
    Throttle { retry_after: Option<u64> },
}

/// A programmable responder shared (via `Arc`) across the routes of one tenant.
///
/// `script[route]` is a per-route queue of behaviors consumed one per matching
/// request; once drained the route falls back to `Normal`. `counts[route]`
/// counts requests seen on that route. The in-memory `cas`/`ac` stores give
/// real content-addressed semantics so snapshots/hydrates round-trip.
#[derive(Clone)]
pub struct HostileResponder {
    pub cas: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    pub ac: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// route name → queue of behaviors (front = next request).
    script: Arc<Mutex<HashMap<&'static str, Vec<Behavior>>>>,
    counts: Arc<Mutex<HashMap<&'static str, Arc<AtomicUsize>>>>,
}

impl HostileResponder {
    pub fn new() -> Self {
        Self {
            cas: Arc::new(Mutex::new(HashMap::new())),
            ac: Arc::new(Mutex::new(HashMap::new())),
            script: Arc::new(Mutex::new(HashMap::new())),
            counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn counter(&self, route: &'static str) -> Arc<AtomicUsize> {
        Arc::clone(
            self.counts
                .lock()
                .unwrap()
                .entry(route)
                .or_insert_with(|| Arc::new(AtomicUsize::new(0))),
        )
    }

    pub fn count_of(&self, route: &'static str) -> usize {
        self.counter(route).load(Ordering::SeqCst)
    }

    /// Program a queue of behaviors for a route (consumed one per request).
    pub fn program(&self, route: &'static str, behaviors: Vec<Behavior>) {
        self.script.lock().unwrap().insert(route, behaviors);
    }

    /// Pop the next scripted behavior for `route`, or `Normal` once drained.
    pub fn next_behavior(&self, route: &'static str) -> Behavior {
        let mut script = self.script.lock().unwrap();
        match script.get_mut(route) {
            Some(queue) if !queue.is_empty() => queue.remove(0),
            _ => Behavior::Normal,
        }
    }

    pub fn route_responder(&self, route: &'static str, kind: RouteKind) -> RouteResponder {
        RouteResponder {
            inner: self.clone(),
            route,
            kind,
            counter: self.counter(route),
        }
    }
}

#[derive(Clone, Copy)]
pub enum RouteKind {
    Cas,
    Ac,
}

/// A `Respond` impl bound to a single route name + kind, sharing the parent's
/// stores, script, and counters.
#[derive(Clone)]
pub struct RouteResponder {
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
            Behavior::Status(status) => return ResponseTemplate::new(status),
            Behavior::Body200(body) => return ResponseTemplate::new(200).set_body_bytes(body),
            Behavior::Throttle { retry_after } => {
                let mut template = ResponseTemplate::new(429);
                if let Some(secs) = retry_after {
                    template = template.append_header("Retry-After", secs.to_string().as_str());
                }
                return template;
            }
            Behavior::Normal => {}
        }

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
                Some(data) => ResponseTemplate::new(200).set_body_bytes(data.clone()),
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

/// Mount the four object routes (CAS GET/PUT/HEAD/DELETE, AC GET/PUT/DELETE)
/// for a per-tenant `HostileResponder`. LIST routes are mounted separately.
pub async fn mount_hostile(server: &MockServer, tenant: &str) -> HostileResponder {
    let responder = HostileResponder::new();
    let cas_obj = format!("/v1/cas/{tenant}/[a-f0-9]{{64}}");
    let ac_obj = format!("/v1/ac/{tenant}/[a-f0-9]{{64}}");

    for verb in ["PUT", "GET", "HEAD", "DELETE"] {
        Mock::given(method(verb))
            .and(path_regex(&cas_obj))
            .respond_with(responder.route_responder("cas", RouteKind::Cas))
            .mount(server)
            .await;
    }
    for verb in ["PUT", "GET", "DELETE"] {
        Mock::given(method(verb))
            .and(path_regex(&ac_obj))
            .respond_with(responder.route_responder("ac", RouteKind::Ac))
            .mount(server)
            .await;
    }
    responder
}
