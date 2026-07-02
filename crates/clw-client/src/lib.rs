//! clw-client — HTTP transport for CAS and AC endpoints.
//!
//! Implements [`CasTransport`] and [`AcTransport`] from `clw-types` via
//! `reqwest`. Token is intentionally never logged or Debug-printed.

pub use clw_types as types;

use clw_types::{AcTransport, CasTransport, ClwError, Config, Digest, Result, CAS_BLOB_CAP_BYTES};
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

const MAX_ATTEMPTS: u32 = 6;
/// Sleep durations between attempt 1→2 and 2→3.
const RETRY_SLEEP_MS: [u64; 5] = [200, 800, 2_000, 5_000, 10_000];

/// Default concurrency ceiling when constructed via [`HttpClient::new`].
const DEFAULT_MAX_CONCURRENCY: usize = 8;
/// Consecutive successes that trigger one additive-increase step toward the
/// ceiling. Keeps recovery gentle so we don't immediately re-trip the limiter.
const AIMD_INCREASE_AFTER_SUCCESSES: u32 = 8;

// ---------------------------------------------------------------------------
// AIMD adaptive-concurrency governor
// ---------------------------------------------------------------------------

/// Mutable governor state, mutated only under [`Governor::state`]'s lock so that
/// `target`/`debt`/`successes` move together and concurrent 429s/successes can't
/// lose updates via read-modify-write races.
struct GovernorState {
    /// Current target permit count (the live window size).
    target: usize,
    /// Permits still owed back (to be reclaimed via `forget` on release).
    debt: usize,
    /// Consecutive successes since the last increase/decrease.
    successes: usize,
}

/// Additive-Increase / Multiplicative-Decrease concurrency governor.
///
/// Wraps a [`Semaphore`] whose live permit budget tracks `target` permits.
/// A 429 multiplicatively halves the target (floor 1); a run of successes
/// additively grows it back toward the configured `ceiling`.
///
/// Shrinking is implemented with a **debt** counter rather than mutating the
/// in-flight permits: a 429 records how many permits must be reclaimed, and the
/// next that-many requests to finish [`forget`](tokio::sync::OwnedSemaphorePermit::forget)
/// their permit (drop it *without* returning it) instead of releasing it. This
/// is race-free under concurrent in-flight requests, where eagerly forgetting
/// the available pool would be undone as held permits drop and re-return.
///
/// All of `target`/`debt`/`successes` live behind a single [`std::sync::Mutex`]
/// so each adjustment is a consistent read-modify-write. The semaphore ops are
/// non-async and tiny, so holding the lock across them (never across an
/// `.await`) is fine.
struct Governor {
    /// Live permit pool. Capacity floats between 1 and `ceiling`.
    sem: Arc<Semaphore>,
    /// Hard upper bound on `target` (additive increase never exceeds this).
    ceiling: usize,
    /// All mutable counters, moved together under the lock.
    state: Mutex<GovernorState>,
}

impl Governor {
    fn new(ceiling: usize) -> Self {
        let ceiling = ceiling.max(1);
        Self {
            sem: Arc::new(Semaphore::new(ceiling)),
            ceiling,
            state: Mutex::new(GovernorState {
                target: ceiling,
                debt: 0,
                successes: 0,
            }),
        }
    }

    /// Acquire one permit for the duration of a request. The returned guard
    /// reconciles the live pool against any outstanding shrink-debt on drop.
    async fn acquire(self: &Arc<Self>) -> GovernorPermit {
        // The semaphore is never closed, so acquire cannot fail.
        let permit = Arc::clone(&self.sem)
            .acquire_owned()
            .await
            .expect("governor semaphore is never closed");
        GovernorPermit {
            gov: Arc::clone(self),
            permit: Some(permit),
        }
    }

    /// Multiplicative decrease: halve the target (floor 1) on a 429.
    ///
    /// Forgets whatever is immediately available, and books the remainder as
    /// debt so in-flight permits are reclaimed as they finish. Done entirely
    /// under the lock so the available-permit read and the `target`/`debt`
    /// writes form a single consistent transaction.
    fn on_throttle(&self) {
        let mut state = self.state.lock().expect("governor lock poisoned");
        let new = (state.target / 2).max(1);
        if new < state.target {
            let mut to_reclaim = state.target - new;
            // Forget what's sitting idle right now; queue the rest as debt.
            let avail = self.sem.available_permits().min(to_reclaim);
            if avail > 0 {
                self.sem.forget_permits(avail);
                to_reclaim -= avail;
            }
            if to_reclaim > 0 {
                state.debt += to_reclaim;
            }
            state.target = new;
            state.successes = 0;
        }
    }

    /// Record a success; after a run of [`AIMD_INCREASE_AFTER_SUCCESSES`],
    /// additively increase the target by one toward the ceiling — never beyond.
    fn on_success(&self) {
        let mut state = self.state.lock().expect("governor lock poisoned");
        state.successes += 1;
        if state.successes >= AIMD_INCREASE_AFTER_SUCCESSES as usize && state.target < self.ceiling
        {
            state.target += 1;
            self.sem.add_permits(1);
            state.successes = 0;
        }
    }

    /// Current target window size — for tests/observability.
    #[cfg(test)]
    fn target(&self) -> usize {
        self.state.lock().expect("governor lock poisoned").target
    }
}

/// RAII guard for a governed permit. On drop it either returns the permit to
/// the pool, or — if a shrink is outstanding — forgets it to pay down the debt.
struct GovernorPermit {
    gov: Arc<Governor>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Drop for GovernorPermit {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        // Pay down shrink-debt first: forget this permit instead of returning it.
        // The decision and the decrement happen together under the lock.
        let mut state = self.gov.state.lock().expect("governor lock poisoned");
        if state.debt > 0 {
            state.debt -= 1;
            drop(state);
            permit.forget();
        }
        // Otherwise the permit drops normally, returning to the pool.
    }
}

// ---------------------------------------------------------------------------
// HttpClient
// ---------------------------------------------------------------------------

/// HTTP transport client for CoreLink CAS and AC endpoints.
///
/// Holds a `reqwest::Client` pre-configured with timeouts and the resolved
/// [`Config`]. The `token` inside `Config` is **never** logged or printed —
/// `Debug` is manually implemented to redact it.
pub struct HttpClient {
    inner: reqwest::Client,
    config: Config,
    /// AIMD adaptive-concurrency governor shared across all in-flight requests.
    governor: Arc<Governor>,
}

// Manual Debug impl: redact the token.
impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("endpoint", &self.config.endpoint)
            .field("tenant", &self.config.tenant)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl HttpClient {
    /// Create a new [`HttpClient`] with the default concurrency ceiling
    /// ([`DEFAULT_MAX_CONCURRENCY`]).
    ///
    /// Configures:
    /// - connect timeout: 10 s
    /// - request timeout: 120 s
    pub fn new(config: Config) -> Result<Self> {
        Self::with_concurrency(config, DEFAULT_MAX_CONCURRENCY)
    }

    /// Create a new [`HttpClient`] whose AIMD governor starts at — and never
    /// exceeds — `max` concurrent CAS/AC requests. `max` is floored at 1.
    pub fn with_concurrency(config: Config, max: usize) -> Result<Self> {
        // Refuse to send the PAT over plaintext http://, except loopback for
        // dev/tests (the wiremock suite uses http://127.0.0.1:<port>). BOTH the
        // https and the loopback judgements parse with the `url` crate reqwest
        // dials with, so neither can be fooled by embedded `@`-userinfo (the
        // PAT-smuggling vector — see `is_safe_https` / `is_loopback_http`).
        let ep = config.endpoint.trim();
        let is_https = is_safe_https(ep);
        let is_local_http = is_loopback_http(ep);
        if !is_https && !is_local_http {
            return Err(ClwError::Transport(format!(
                "refusing to send PAT over plaintext: endpoint must be https:// with no embedded credentials (got '{}'); http:// is only allowed for loopback",
                redact_endpoint_for_display(ep)
            )));
        }

        let inner = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            // SECURITY (FIX 5): never follow redirects. We talk to ONE fixed,
            // pre-validated API origin (is_safe_https / is_loopback_http above);
            // reqwest's default follows up to 10 redirects, which a hostile or buggy
            // server could use to redirect us at internal hosts (SSRF amplifier) or
            // to a different host carrying the bearer PAT. For a single-origin client
            // a 30x has no legitimate meaning, so surfacing it as a non-success
            // transport error is strictly safer than chasing the Location.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ClwError::Transport(e.to_string()))?;
        Ok(Self {
            inner,
            config,
            governor: Arc::new(Governor::new(max)),
        })
    }

    /// Authorization header value.
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.config.token)
    }

    /// Execute `op` with up to `MAX_ATTEMPTS` attempts, sleeping between them.
    ///
    /// Retries on connect/transport errors, 5xx responses, and 429 rate limits
    /// (found by dogfood day-1: prod throttled the first real snapshot). Each
    /// attempt holds one permit from the AIMD [`Governor`]: a 429 multiplicatively
    /// shrinks the window and a success grows it back. When a 429 carried a
    /// `Retry-After` header, the recorded delay overrides the fixed backoff ladder
    /// for the *next* sleep.
    ///
    /// Returns `Ok(T)` on first success; propagates the last error on exhaustion.
    ///
    /// The `op` closure receives a fresh per-call `Retry-After` slot (in raw
    /// delta-seconds, sentinel [`u64::MAX`] = none). On a 429 the op writes the
    /// parsed value into *its own* call's slot before returning the error; this
    /// loop reads and resets the slot to pick the next sleep and to classify the
    /// exhaustion message. The slot is local to this call, so concurrent requests
    /// can never steal each other's hint.
    async fn with_retry<F, Fut, T>(&self, op: F) -> Result<T>
    where
        F: Fn(Arc<AtomicU64>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        const NONE: u64 = u64::MAX;
        let mut last_err: Option<ClwError> = None;
        // Per-request Retry-After hint (raw delta-seconds; NONE sentinel = absent).
        // Created fresh for this call so no two requests share a slot.
        let slot = Arc::new(AtomicU64::new(NONE));
        // Raw seconds observed on the most recent 429 of this call, retained for
        // the quota-vs-backpressure exhaustion message (M5.6).
        let mut last_retry_after_secs: u64 = 0;
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                let base = RETRY_SLEEP_MS[(attempt - 1) as usize];
                // Honor a server-supplied Retry-After if the previous 429 wrote one
                // into our slot; otherwise fall back to the jittered ladder. Reading
                // resets the slot to the NONE sentinel.
                let raw = slot.swap(NONE, Ordering::Relaxed);
                let sleep_ms = if raw != NONE {
                    // Cap at the longest ladder rung so a hostile header can't park
                    // us for an unbounded time; floor at 1ms so a literal 0 still
                    // delays. (Matches the previous clamp semantics.)
                    raw.saturating_mul(1_000).clamp(1, RETRY_SLEEP_MS[4])
                } else {
                    // Cheap jitter in [0, base/2] derived from the clock — avoids a
                    // synchronized thundering herd without pulling in a rand dep.
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos() as u64)
                        .unwrap_or(0);
                    let jitter = if base > 0 { nanos % (base / 2 + 1) } else { 0 };
                    base + jitter
                };
                // Diagnostic channel (M5.1): one event per backoff so support can
                // see attempt count + sleep without it touching user-facing stdout.
                tracing::debug!(attempt, sleep_ms, "retry backoff before next attempt");
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            }
            // Hold one governor permit for the lifetime of this attempt.
            let _permit = self.governor.acquire().await;
            match op(Arc::clone(&slot)).await {
                Ok(v) => {
                    self.governor.on_success();
                    return Ok(v);
                }
                Err(e) => {
                    if is_retryable(&e) {
                        // A 429 shrinks the window; other retryables (5xx,
                        // transport) leave it alone.
                        if matches!(&e, ClwError::Http { status: 429, .. }) {
                            self.governor.on_throttle();
                            // Snapshot the raw seconds the op just wrote (if any)
                            // for the exhaustion message; the swap above will reset
                            // the slot before the next sleep.
                            let raw = slot.load(Ordering::Relaxed);
                            if raw != NONE {
                                last_retry_after_secs = raw.max(1);
                            }
                            // Diagnostic channel (M5.1): a 429 throttle is worth a
                            // WARN — include the attempt and any server Retry-After.
                            let retry_after_secs = (raw != NONE).then_some(raw);
                            tracing::warn!(
                                attempt,
                                ?retry_after_secs,
                                "rate-limited (429); backing off"
                            );
                        }
                        last_err = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        // On exhaustion, rewrite a final 429's context so the message
        // distinguishes a transient backpressure throttle from a plan/quota
        // ceiling, using the last raw Retry-After observed (M5.6).
        let last_err = last_err
            .expect("exhausted retry loop with at least one attempt; last_err is always Some here");
        if let ClwError::Http { status: 429, .. } = &last_err {
            let retry_after = (last_retry_after_secs > 0).then_some(last_retry_after_secs);
            return Err(ClwError::Http {
                status: 429,
                context: exhausted_429_context(retry_after, MAX_ATTEMPTS),
            });
        }
        Err(last_err)
    }

    /// GET a paginated digest-listing endpoint at `base_url` and flatten every
    /// page into a single `Vec<Digest>`. Generic over the page type `P` so the AC
    /// (`refs`/`ref_key`) and CAS (`blobs`/`hash`) envelopes share one paginator.
    ///
    /// Sends `?limit=1000` (the ratified max, 1..1000) to minimize round-trips and
    /// follows a non-null `next_cursor` internally via `?cursor=<opaque>` (URL-
    /// encoded) until the server returns `null`. Each page GET rides the SAME
    /// [`with_retry`](Self::with_retry) / [`Governor`] / `User-Agent` path as every
    /// other request. The GET listing capability is LIVE (D-7/D-8, conformance-
    /// green), so a 405|501 is a server-side regression and is surfaced as
    /// `ClwError::Transport(unavailable_msg)`, distinct per caller (D-7/D-8).
    async fn list_digests<P: ListPage + Send>(
        &self,
        base_url: &str,
        unavailable_msg: &str,
    ) -> Result<Vec<Digest>> {
        self.list_digests_capped::<P>(base_url, unavailable_msg, MAX_TOTAL_DIGESTS, MAX_PAGES)
            .await
    }

    /// Inner paginator with explicit ceilings (total entries AND page count) so the
    /// liveness aborts can be exercised with small caps in tests — driving a real
    /// 10M-entry or 1M-page server is infeasible. Production callers go through
    /// [`list_digests`](Self::list_digests), which passes [`MAX_TOTAL_DIGESTS`] and
    /// [`MAX_PAGES`].
    ///
    /// The `max_pages` cap is the HARD bound that makes the loop terminate REGARDLESS
    /// of cursor uniqueness: a server returning empty pages with always-advancing
    /// (never-repeating) cursors never trips the entry cap (no entries accumulate) nor
    /// the repeated-cursor guard (cursors never repeat) and would otherwise grow
    /// `seen_cursors` without bound (OOM). The page-count cap aborts it first.
    async fn list_digests_capped<P: ListPage + Send>(
        &self,
        base_url: &str,
        unavailable_msg: &str,
        max_total: u64,
        max_pages: u64,
    ) -> Result<Vec<Digest>> {
        let auth = self.auth_header();
        let client = &self.inner;
        let mut out: Vec<Digest> = Vec::new();
        let mut cursor: Option<String> = None;
        // Liveness guards: the cursor is opaque, so a buggy/regressed server that
        // returns a non-null cursor that never advances (or cycles) would loop
        // forever. Require STRICT advancement (a cursor never repeats) and cap the
        // total pages. `limit=1000` ⇒ the cap still allows ~10^9 entries.
        let mut seen_cursors: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut pages: u64 = 0;

        loop {
            let page: P = self
                .with_retry(|retry_slot| {
                    let base_url = base_url.to_string();
                    let auth = auth.clone();
                    let cursor = cursor.clone();
                    let unavailable_msg = unavailable_msg.to_string();
                    async move {
                        // `?limit=1000` always; the opaque cursor is appended
                        // (reqwest URL-encodes it) only when continuing.
                        let mut req = client
                            .get(&base_url)
                            .header("Authorization", &auth)
                            .header(reqwest::header::USER_AGENT, user_agent())
                            .query(&[("limit", "1000")]);
                        if let Some(c) = &cursor {
                            req = req.query(&[("cursor", c.as_str())]);
                        }
                        let resp = req
                            .send()
                            .await
                            .map_err(|e| ClwError::Transport(e.to_string()))?;

                        match resp.status().as_u16() {
                            200 => {
                                let bytes = read_body_capped(resp, LIST_PAGE_CAP_BYTES).await?;
                                serde_json::from_slice::<P>(&bytes).map_err(|e| {
                                    ClwError::Transport(format!(
                                        "malformed list page from {base_url}: {e}"
                                    ))
                                })
                            }
                            401 => Err(ClwError::Auth),
                            // Defensive: capability is LIVE (D-7/D-8) — a 405/501 here means a server-side regression.
                            405 | 501 => Err(ClwError::Transport(unavailable_msg)),
                            429 => {
                                record_retry_after(&retry_slot, &resp);
                                Err(ClwError::Http {
                                    status: 429,
                                    context: format!("GET {base_url}"),
                                })
                            }
                            s => Err(ClwError::Http {
                                status: s,
                                context: format!("GET {base_url}"),
                            }),
                        }
                    }
                })
                .await?;

            let (keys, next) = page.into_parts();
            out.extend(keys);

            // Total-entry ceiling (FIX 2): abort before the accumulated digests can
            // grow without bound, independent of the page cap. A server that keeps
            // returning full pages cannot OOM us.
            if out.len() as u64 > max_total {
                return Err(ClwError::TooLarge {
                    size: out.len() as u64,
                    cap: max_total,
                });
            }

            // HARD page-count cap — the bound that guarantees termination regardless
            // of cursor uniqueness. An adversarial server returning empty pages with
            // forever-advancing cursors trips neither the entry cap (no entries) nor
            // the repeated-cursor guard (cursors never repeat); without this cap
            // `seen_cursors` would grow without bound (OOM). Counting pages — not
            // entries — is what makes the loop terminable here.
            pages += 1;
            if pages > max_pages {
                return Err(ClwError::Transport(format!(
                    "pagination exceeded {max_pages} pages at {base_url}; aborting a likely \
                     unbounded/misbehaving server cursor"
                )));
            }
            match next {
                Some(c) => {
                    // A cursor that repeats means the server is not advancing —
                    // abort rather than loop forever.
                    if !seen_cursors.insert(c.clone()) {
                        return Err(ClwError::Transport(format!(
                            "pagination cursor did not advance at {base_url} (repeated cursor); \
                             aborting to avoid an unbounded loop"
                        )));
                    }
                    cursor = Some(c);
                }
                None => break,
            }
        }

        Ok(out)
    }
}

/// The `User-Agent` value sent on every outgoing request, e.g.
/// `clw/0.1.0 (macos; x86_64)`. Built once at first use and cached.
fn user_agent() -> &'static str {
    use std::sync::OnceLock;
    static UA: OnceLock<String> = OnceLock::new();
    UA.get_or_init(|| {
        format!(
            "clw/{} ({}; {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })
}

/// App-quota 429s carry a `Retry-After` measured in the seconds until the next
/// monthly window opens (huge); backpressure 429s carry a short delay. Above
/// this threshold we read the 429 as a plan/quota ceiling rather than transient
/// throttling, and surface an actionable message on exhaustion.
const QUOTA_RETRY_AFTER_SECS: u64 = 3_600;

/// Build the `context` string for a 429 that exhausted its retry budget. The
/// final `Retry-After` (delta-seconds) disambiguates the two real cases:
/// a small value is transient DO-backpressure ("rate-limited after N retries");
/// a large one is an app/plan quota ceiling that retrying cannot clear.
fn exhausted_429_context(retry_after_secs: Option<u64>, attempts: u32) -> String {
    match retry_after_secs {
        Some(secs) if secs > QUOTA_RETRY_AFTER_SECS => {
            format!("plan/quota limit reached — retry-after {secs}s; check your plan/quota")
        }
        _ => format!("rate-limited after {attempts} retries"),
    }
}

/// Read a `Retry-After` header (delta-seconds form) off a 429 response and stash
/// the raw delta-seconds into this request's per-call `slot` for
/// [`HttpClient::with_retry`] to consume. Must be called *before* the response is
/// mapped to a [`ClwError`] (which drops headers). Threaded out-of-band because
/// [`ClwError`] is frozen. The slot is per-request, so concurrent requests never
/// cross-talk; the loop converts the raw seconds to a clamped sleep and uses the
/// raw value to classify the exhaustion message.
fn record_retry_after(slot: &AtomicU64, resp: &reqwest::Response) {
    if let Some(secs) = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        // Floor at 1 so a literal `retry-after: 0` still registers as "present"
        // (the absent sentinel is u64::MAX, set by `with_retry`).
        slot.store(secs.max(1), Ordering::Relaxed);
    }
}

/// Returns `true` iff `ep` is a plaintext `http://` URL whose host is *exactly* a
/// loopback (`127.0.0.0/8`, `localhost`, or `::1`) and which carries **no**
/// userinfo.
///
/// SECURITY (CRIT): parsed with the same `url` crate reqwest uses to dial, so the
/// loopback judgement is made on the EXACT host that will receive the bearer PAT
/// — never a hand-split authority. A hand parser cannot see that
/// `http://127.0.0.1:80@evil.com` actually dials `evil.com` (the `127.0.0.1:80`
/// is *userinfo*), which would leak the token in cleartext to the attacker host.
/// Embedded credentials are therefore rejected outright. Only `http://` is gated
/// here; `https://` endpoints go through [`is_safe_https`] (same `@`-userinfo
/// defense on the TLS path).
fn is_loopback_http(ep: &str) -> bool {
    let Ok(url) = url::Url::parse(ep.trim()) else {
        return false;
    };
    // Plaintext only, and NEVER with embedded credentials (the `@`-smuggling
    // vector that makes a non-loopback host masquerade as loopback).
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.host() {
        // `is_loopback()` covers all of 127.0.0.0/8 (v4) and ::1 (v6).
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    }
}

/// Returns `true` iff `ep` is an `https://` URL with a real host and **no**
/// embedded userinfo.
///
/// SECURITY (HIGH): a naive `ep.starts_with("https://")` accepts
/// `https://legit-looking-host@evil.com`, but the part before the `@` is
/// *userinfo* — reqwest dials `evil.com` and the bearer PAT is sent (over TLS) to
/// the attacker host. Parsing with the same `url` crate reqwest uses to dial
/// makes the host judgement on the EXACT authority that receives the token, and
/// rejects any embedded credentials outright.
fn is_safe_https(ep: &str) -> bool {
    let Ok(url) = url::Url::parse(ep.trim()) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    url.host().is_some()
}

/// Redact any embedded `user:pass@` userinfo from an endpoint string before it is
/// shown in an error message.
///
/// SECURITY (info-hygiene): the plaintext guard REJECTS endpoints carrying embedded
/// credentials (the `@`-smuggling vector), but its rejection message must not then
/// echo the offending value verbatim — a misconfigured `https://user:secret@host`
/// would otherwise print the user's embedded secret to stderr / captured logs. This
/// is best-effort and never panics: it parses with the same `url` crate used to dial
/// when it can, else falls back to a conservative string strip. (The CLW_TOKEN PAT is
/// unaffected — it travels in the Authorization header, never the URL.)
fn redact_endpoint_for_display(ep: &str) -> String {
    if let Ok(u) = url::Url::parse(ep.trim()) {
        if !u.username().is_empty() || u.password().is_some() {
            let scheme = u.scheme();
            let host = u.host_str().unwrap_or("");
            let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
            return format!("{scheme}://<redacted>@{host}{port}{}", u.path());
        }
        return ep.to_string();
    }
    // Unparseable as a URL: strip a leading `scheme://userinfo@` if the `@` precedes
    // any path separator (so it is genuinely authority userinfo, not a later `@`).
    if let Some(scheme_end) = ep.find("://") {
        let after = &ep[scheme_end + 3..];
        let slash = after.find('/').unwrap_or(after.len());
        if let Some(at) = after.find('@') {
            if at < slash {
                return format!("{}://<redacted>@{}", &ep[..scheme_end], &after[at + 1..]);
            }
        }
    }
    ep.to_string()
}

/// Redeem a single-use, lease-bound credential ticket for a per-job CAS PAT
/// (C2c env-0). `clw` calls this ONCE at the trusted boot (runner ref-domain,
/// before any untrusted `clw run`): the fabric injected `CLW_CRED_TICKET` instead
/// of `CLW_TOKEN`, and this exchanges it at
/// `POST {fabric_endpoint}/v1/leases/{lease_id}/cas-cred` (the ticket IS the auth,
/// carried in the JSON body — not a bearer) for the scope-narrowed PAT in the 200
/// `{"cas_pat": ...}`.
///
/// **Fail-closed:** any non-2xx (`410` reused, `401` bad ticket, `404`
/// unknown/not-Held, or a transport error) is an error — the caller MUST NOT fall
/// back to an env token. The returned PAT is held in memory only. The ticket is
/// never logged; on a plaintext-endpoint refusal only the redacted endpoint is shown.
pub async fn redeem_cred_ticket(
    fabric_endpoint: &str,
    lease_id: &str,
    ticket: &str,
) -> Result<String> {
    // Same plaintext refusal as HttpClient: the ticket (and the returned PAT) are
    // PAT-equivalent secrets, so require https:// (loopback http:// only for tests).
    let ep = fabric_endpoint.trim();
    if !is_safe_https(ep) && !is_loopback_http(ep) {
        return Err(ClwError::Transport(format!(
            "refusing to redeem a cred ticket over plaintext: CLW_FABRIC_ENDPOINT must be https:// with no embedded credentials (got '{}'); http:// is only allowed for loopback",
            redact_endpoint_for_display(ep)
        )));
    }
    // `lease_id` is interpolated RAW into the URL path — reject a value that could
    // mangle it (it is fabric-injected and trusted, but this mirrors the tenant
    // path-injection guard and fails loud rather than silently misrouting).
    if lease_id.is_empty()
        || lease_id.contains('/')
        || lease_id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ClwError::Config(
            "invalid CLW_LEASE_ID for cred redemption: must be non-empty and contain no '/', whitespace, or control characters".to_string(),
        ));
    }
    let url = format!(
        "{}/v1/leases/{}/cas-cred",
        ep.trim_end_matches('/'),
        lease_id
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        // Same single-origin no-redirect posture as HttpClient (SSRF/secret-leak safe).
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ClwError::Transport(e.to_string()))?;

    // Build the JSON body manually (reqwest's `.json()`/`.json().await` need the
    // `json` feature, which the workspace reqwest doesn't enable — avoid coupling
    // this to a shared dependency's feature set).
    let payload = serde_json::json!({ "ticket": ticket }).to_string();
    let resp = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        .await
        .map_err(|e| ClwError::Transport(format!("cred-ticket redemption transport error: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        // Fail-closed. Deliberately terse (no body echo) — the response could carry
        // sensitive detail, and the caller never falls back to CLW_TOKEN regardless.
        return Err(ClwError::Transport(format!(
            "cred-ticket redemption failed: HTTP {} (fail-closed; no CLW_TOKEN fallback in the runner ref-domain)",
            status.as_u16()
        )));
    }

    // Parse via Value to avoid coupling to a serde-derive feature; extract `cas_pat`.
    let text = resp
        .text()
        .await
        .map_err(|e| ClwError::Transport(format!("cred-ticket response read error: {e}")))?;
    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ClwError::Transport(format!("cred-ticket response parse error: {e}")))?;
    let pat = body
        .get("cas_pat")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ClwError::Transport("cred-ticket 200 missing a non-empty `cas_pat`".to_string())
        })?;
    Ok(pat.to_string())
}

/// Generous ceiling for a single list page (D-7/D-8): ≤1000 small JSON items per
/// page, so 16 MiB is orders of magnitude above any legitimate page while still
/// bounding a hostile/buggy server's response.
const LIST_PAGE_CAP_BYTES: usize = 16 * 1024 * 1024;

/// Total-ENTRY ceiling for a paginated listing (FIX 2). `MAX_PAGES` bounds the
/// number of pages, but at `limit=1000`/page a hostile server paginating forever
/// with FULL pages would still grow the flattened `Vec` toward ~10^9 digests
/// (~32 GiB at 32 bytes each) and OOM the client. 10_000_000 ref/blob keys is far
/// above any legitimate tenant (~320 MiB in the flat Vec — already generous
/// headroom) yet finite, so an unbounded server is aborted with `TooLarge` instead
/// of exhausting memory.
const MAX_TOTAL_DIGESTS: u64 = 10_000_000;

/// HARD page-count ceiling for a paginated listing (FIX #4). The cursor is opaque,
/// so cycle-detection (`seen_cursors`) catches a REPEATING cursor — but a server
/// returning empty pages with always-ADVANCING (never-repeating) cursors trips
/// neither the repeated-cursor guard nor the total-entry cap (no entries accumulate),
/// growing `seen_cursors` without bound (OOM). This page cap is the bound that
/// terminates the loop regardless of cursor uniqueness. At `limit=1000`/page it still
/// permits ~10^9 entries, so it never aborts a legitimate listing.
const MAX_PAGES: u64 = 1_000_000;

/// Read a response body into memory with a HARD size ceiling, so a hostile or
/// buggy server cannot OOM the client by answering a small request with a
/// multi-gigabyte body. The CAS contract caps blobs at `CAS_BLOB_CAP_BYTES`, but
/// that is enforced only on `put`; the download side (`get`/list) must defend
/// itself too — and for CAS the BLAKE3 integrity check runs *after* the body is
/// buffered, so this cap is the only thing between an oversized body and the
/// allocation. We reject early on an honest `Content-Length`, then stream with a
/// running ceiling so a server that omits or lies about the length still cannot
/// exceed `cap`.
async fn read_body_capped(mut resp: reqwest::Response, cap: usize) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length() {
        if len > cap as u64 {
            return Err(ClwError::TooLarge {
                size: len,
                cap: cap as u64,
            });
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ClwError::Transport(e.to_string()))?
    {
        let projected = buf.len() as u64 + chunk.len() as u64;
        if projected > cap as u64 {
            return Err(ClwError::TooLarge {
                size: projected,
                cap: cap as u64,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Returns `true` if the error warrants a retry.
fn is_retryable(e: &ClwError) -> bool {
    match e {
        ClwError::Transport(_) => true,
        ClwError::Http { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

/// One page of a digest-listing endpoint, per the RATIFIED server contract
/// (corelink-server PR #269; D-7/D-8 — see `docs/RELAY-server-D1-D7-D8.md`).
///
/// Both endpoints share the cursor envelope
/// `{ "<items>": [ { "<key>": "<hex>", … } ], "next_cursor": "<opaque|null>" }`
/// but differ in the array field and the per-item key field, so each has its own
/// page type implementing [`ListPage`]:
///
/// * AC  `GET /v1/ac/{tenant}`  → [`AcListPage`]  — `{"refs":  [{"ref_key":"<hex>", …}], "next_cursor"}`
/// * CAS `GET /v1/cas/{tenant}` → [`CasListPage`] — `{"blobs": [{"hash":   "<hex>", …}], "next_cursor"}`
///
/// `Digest`'s own `Deserialize` validates each hex string; extra item fields
/// (`updated_at`/`size`/`created_at`) are ignored. A non-null `next_cursor` means
/// more pages follow; the caller re-requests with `?cursor=…`.
trait ListPage: serde::de::DeserializeOwned {
    /// Split a page into its digests and the next cursor (`None` ⇒ last page).
    fn into_parts(self) -> (Vec<Digest>, Option<String>);
}

/// AC ref listing (D-7): `{"refs": [{"ref_key": "<hex>", "updated_at", "size"}], "next_cursor"}`.
///
/// `refs` is REQUIRED (no `#[serde(default)]`): a 200 whose body omits the array
/// (server bug, schema drift, a proxy returning `{}` or an unrelated JSON 200)
/// must be a hard parse error, NOT a silently-empty list — an empty live set
/// would otherwise drive `prune` to delete the whole tenant. Only `next_cursor`
/// defaults (absent/null = terminal page, which IS contract-correct).
#[derive(Debug, Deserialize)]
struct AcListPage {
    refs: Vec<AcRefItem>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AcRefItem {
    ref_key: Digest,
}

/// CAS blob listing (D-8): `{"blobs": [{"hash": "<hex>", "size", "created_at"}], "next_cursor"}`.
///
/// `blobs` is REQUIRED (see [`AcListPage`] for why a missing array must be a hard
/// error, not a silent empty list). Only `next_cursor` defaults.
#[derive(Debug, Deserialize)]
struct CasListPage {
    blobs: Vec<CasBlobItem>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CasBlobItem {
    hash: Digest,
}

/// Treat an empty-string cursor as "no more pages". The contract uses JSON
/// `null` for the terminal page, but an empty string would otherwise loop.
fn norm_cursor(c: Option<String>) -> Option<String> {
    c.filter(|c| !c.is_empty())
}

impl ListPage for AcListPage {
    fn into_parts(self) -> (Vec<Digest>, Option<String>) {
        let keys = self.refs.into_iter().map(|r| r.ref_key).collect();
        (keys, norm_cursor(self.next_cursor))
    }
}

impl ListPage for CasListPage {
    fn into_parts(self) -> (Vec<Digest>, Option<String>) {
        let keys = self.blobs.into_iter().map(|b| b.hash).collect();
        (keys, norm_cursor(self.next_cursor))
    }
}

// ---------------------------------------------------------------------------
// CasTransport
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl CasTransport for HttpClient {
    async fn exists(&self, digest: &Digest) -> Result<bool> {
        let url = self.config.cas_url(digest);
        let auth = self.auth_header();
        let client = &self.inner;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                // Try HEAD first.
                let resp = client
                    .head(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 => return Ok(true),
                    404 => return Ok(false),
                    401 => return Err(ClwError::Auth),
                    // 405 Method Not Allowed or 501 Not Implemented → fall back to GET.
                    405 | 501 => {}
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        return Err(ClwError::Http {
                            status: 429,
                            context: format!("HEAD {url}"),
                        });
                    }
                    s => {
                        return Err(ClwError::Http {
                            status: s,
                            context: format!("HEAD {url}"),
                        })
                    }
                }

                // Fallback: GET, discard body.
                let resp = client
                    .get(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 => Ok(true),
                    404 => Ok(false),
                    401 => Err(ClwError::Auth),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("GET {url} (exists-fallback)"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("GET {url} (exists-fallback)"),
                    }),
                }
            }
        })
        .await
    }

    async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
        let url = self.config.cas_url(digest);
        let auth = self.auth_header();
        let client = &self.inner;
        let digest = *digest;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 => {
                        // Cap the download: a blob can never legitimately exceed
                        // CAS_BLOB_CAP_BYTES (put enforces it), and the integrity
                        // check below only runs AFTER the body is buffered.
                        let bytes = read_body_capped(resp, CAS_BLOB_CAP_BYTES).await?;
                        let actual = Digest::of_bytes(&bytes);
                        if actual != digest {
                            return Err(ClwError::Integrity {
                                expected: digest,
                                actual,
                            });
                        }
                        Ok(bytes)
                    }
                    404 => Err(ClwError::NotFound(digest)),
                    401 => Err(ClwError::Auth),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("GET {url}"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("GET {url}"),
                    }),
                }
            }
        })
        .await
    }

    async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
        // Pre-network guard: reject before any network activity.
        if bytes.len() > CAS_BLOB_CAP_BYTES {
            return Err(ClwError::TooLarge {
                size: bytes.len() as u64,
                cap: CAS_BLOB_CAP_BYTES as u64,
            });
        }

        let url = self.config.cas_url(digest);
        let auth = self.auth_header();
        let client = &self.inner;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            let bytes = bytes.clone();
            async move {
                let resp = client
                    .put(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .body(bytes)
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 | 201 | 204 => Ok(()),
                    401 => Err(ClwError::Auth),
                    413 => Err(ClwError::TooLarge {
                        size: 0, // server-side; client size unknown here
                        cap: CAS_BLOB_CAP_BYTES as u64,
                    }),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("PUT {url}"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("PUT {url}"),
                    }),
                }
            }
        })
        .await
    }

    /// DELETE a blob by its hash (`DELETE /v1/cas/{tenant}/{hash}`). Idempotent:
    /// 200|202|204|404 all return Ok(()). The DELETE /v1/cas capability is LIVE
    /// (D-8, conformance-green), so a 405|501 is a server-side regression and is
    /// surfaced as a clear server-regression error — see docs/RELAY-server-D1-D7-D8.md.
    async fn delete(&self, digest: &Digest) -> Result<()> {
        let url = self.config.cas_url(digest);
        let auth = self.auth_header();
        let client = &self.inner;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                let resp = client
                    .delete(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    // Idempotent: an absent blob (404) is a successful delete.
                    200 | 202 | 204 | 404 => Ok(()),
                    401 => Err(ClwError::Auth),
                    405 | 501 => Err(ClwError::Transport(format!(
                        "CAS delete failed: server returned 405/501 for {url} — the DELETE /v1/cas capability is LIVE (D-8, conformance-green), so this indicates a server-side regression, not an expected state; see docs/RELAY-server-D1-D7-D8.md"
                    ))),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("DELETE {url}"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("DELETE {url}"),
                    }),
                }
            }
        })
        .await
    }

    /// List the tenant's CAS blob hashes (`GET /v1/cas/{tenant}`), paginating
    /// internally into a flat `Vec` per the ratified D-8 envelope
    /// `{"blobs":[{"hash":…}],"next_cursor"}`. The GET /v1/cas capability is LIVE
    /// (D-8, conformance-green), so 405|501 surface a clear server-regression error.
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        let base_url = format!(
            "{}/v1/cas/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.tenant
        );
        self.list_digests::<CasListPage>(
            &base_url,
            "CAS listing failed: server returned 405/501 — the GET /v1/cas capability is LIVE (D-8, conformance-green), so this indicates a server-side regression, not an expected state; see docs/RELAY-server-D1-D7-D8.md",
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// AcTransport
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl AcTransport for HttpClient {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
        let url = self.config.ac_url(key);
        let auth = self.auth_header();
        let client = &self.inner;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 => {
                        // AC values are small ref records; the CAS cap is a
                        // generous ceiling that still bounds a hostile response.
                        let bytes = read_body_capped(resp, CAS_BLOB_CAP_BYTES).await?;
                        Ok(Some(bytes))
                    }
                    404 => Ok(None),
                    401 => Err(ClwError::Auth),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("GET {url}"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("GET {url}"),
                    }),
                }
            }
        })
        .await
    }

    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
        let url = self.config.ac_url(key);
        let auth = self.auth_header();
        let client = &self.inner;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            let value = value.clone();
            async move {
                let resp = client
                    .put(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .body(value)
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 | 201 | 204 => Ok(()),
                    401 => Err(ClwError::Auth),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("PUT {url}"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("PUT {url}"),
                    }),
                }
            }
        })
        .await
    }

    async fn delete(&self, key: &Digest) -> Result<()> {
        let url = self.config.ac_url(key);
        let auth = self.auth_header();
        let client = &self.inner;

        self.with_retry(|retry_slot| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                let resp = client
                    .delete(&url)
                    .header("Authorization", &auth)
                    .header(reqwest::header::USER_AGENT, user_agent())
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    // Idempotent: an absent key (404) is a successful delete.
                    200 | 202 | 204 | 404 => Ok(()),
                    401 => Err(ClwError::Auth),
                    // Defensive: DELETE /v1/ac is LIVE (D-1) — a 405/501 here means a server-side regression.
                    405 | 501 => Err(ClwError::Transport(format!(
                        "ref deletion failed: server returned 405/501 for DELETE {url} — the DELETE /v1/ac capability is LIVE (D-1, conformance-green), so this indicates a server-side regression or misroute, not an expected state; see docs/RELAY-server-D1-D7-D8.md"
                    ))),
                    429 => {
                        record_retry_after(&retry_slot, &resp);
                        Err(ClwError::Http {
                            status: 429,
                            context: format!("DELETE {url}"),
                        })
                    }
                    s => Err(ClwError::Http {
                        status: s,
                        context: format!("DELETE {url}"),
                    }),
                }
            }
        })
        .await
    }

    /// List the tenant's ref keys (`GET /v1/ac/{tenant}`), paginating internally
    /// into a flat `Vec` per the ratified D-7 envelope
    /// `{"refs":[{"ref_key":…}],"next_cursor"}`. Needed because
    /// `ref_key = BLAKE3(domain‖name)` is one-way and the client keeps no
    /// name→key index. The GET /v1/ac capability is LIVE (D-7, conformance-green),
    /// so 405|501 surface a clear server-regression error.
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        let base_url = format!(
            "{}/v1/ac/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.tenant
        );
        self.list_digests::<AcListPage>(
            &base_url,
            "ref listing failed: server returned 405/501 — the GET /v1/ac capability is LIVE (D-7, conformance-green), so this indicates a server-side regression, not an expected state; see docs/RELAY-server-D1-D7-D8.md",
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(endpoint: String) -> Config {
        Config {
            endpoint,
            tenant: "test-tenant".into(),
            token: "test-token".into(),
            cache_dir: PathBuf::from("/tmp/clw-test-cache"),
        }
    }

    // Security regression (sweep finding, LOW): the plaintext-guard rejection message
    // must NOT echo an embedded userinfo credential. The guard correctly REJECTS such
    // an endpoint; it must not reflect the secret back to stderr/logs.
    #[test]
    fn plaintext_guard_error_redacts_embedded_userinfo() {
        // https-with-userinfo is rejected by is_safe_https (embedded creds) → the guard
        // fires; the error must not contain the secret.
        let secret = "SENTINEL_USERINFO_SECRET_do_not_leak";
        let cfg = test_config(format!("https://user:{secret}@evil.example.com"));
        let err = HttpClient::with_concurrency(cfg, 4)
            .expect_err("an endpoint with embedded credentials must be refused");
        let msg = err.to_string();
        assert!(
            !msg.contains(secret),
            "the plaintext-guard error must NOT echo the embedded userinfo secret, got: {msg}"
        );
        assert!(
            msg.contains("refusing to send PAT over plaintext") && msg.contains("<redacted>"),
            "the error must still be the actionable guard message with userinfo redacted, got: {msg}"
        );
    }

    // The redaction helper handles both URL-parseable and unparseable inputs without
    // leaking userinfo, and leaves clean endpoints untouched.
    #[test]
    fn redact_endpoint_for_display_strips_userinfo_only() {
        assert_eq!(
            redact_endpoint_for_display("https://u:p@host.example.com/v1"),
            "https://<redacted>@host.example.com/v1"
        );
        // Clean endpoint (no userinfo) is returned unchanged (modulo url normalization
        // is avoided — no userinfo branch ⇒ verbatim).
        assert_eq!(
            redact_endpoint_for_display("https://host.example.com"),
            "https://host.example.com"
        );
        // Unparseable-but-has-authority-userinfo still gets stripped.
        assert_eq!(
            redact_endpoint_for_display("ht!tp://user:secret@host/x"),
            "ht!tp://<redacted>@host/x"
        );
    }

    // -----------------------------------------------------------------------
    // redeem_cred_ticket (C2c env-0)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn redeem_refuses_plaintext_fabric_endpoint() {
        // A non-loopback http:// fabric base is refused BEFORE any request (the
        // ticket + returned PAT are PAT-equivalent secrets).
        let err = redeem_cred_ticket("http://fabric.example.com", "lease1", "tkt")
            .await
            .unwrap_err();
        match err {
            ClwError::Transport(m) => {
                assert!(
                    m.contains("plaintext"),
                    "expected a plaintext refusal, got: {m}"
                )
            }
            other => panic!("expected Transport plaintext refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redeem_happy_path_returns_cas_pat() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/leases/lease-xyz/cas-cred"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"cas_pat":"per-job-pat-abc"}"#),
            )
            .mount(&server)
            .await;
        let pat = redeem_cred_ticket(&server.uri(), "lease-xyz", "the-ticket")
            .await
            .expect("redeem should succeed");
        assert_eq!(pat, "per-job-pat-abc");
    }

    #[tokio::test]
    async fn redeem_410_reuse_is_fail_closed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/leases/lease-1/cas-cred"))
            .respond_with(ResponseTemplate::new(410))
            .mount(&server)
            .await;
        let err = redeem_cred_ticket(&server.uri(), "lease-1", "reused")
            .await
            .unwrap_err();
        match err {
            ClwError::Transport(m) => assert!(m.contains("410"), "expected HTTP 410, got: {m}"),
            other => panic!("expected Transport fail-closed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redeem_200_missing_cas_pat_is_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/leases/l/cas-cred"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"nope":"x"}"#))
            .mount(&server)
            .await;
        let err = redeem_cred_ticket(&server.uri(), "l", "t")
            .await
            .unwrap_err();
        match err {
            ClwError::Transport(m) => assert!(m.contains("cas_pat"), "got: {m}"),
            other => panic!("expected missing-cas_pat error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redeem_rejects_path_injection_lease_id() {
        // A lease id with a '/' would mangle the URL path — rejected before any
        // request (loopback endpoint passes the plaintext gate, so this exercises
        // the lease-id guard specifically).
        let err = redeem_cred_ticket("http://127.0.0.1:1", "bad/lease", "t")
            .await
            .unwrap_err();
        match err {
            ClwError::Config(m) => assert!(m.contains("CLW_LEASE_ID"), "got: {m}"),
            other => panic!("expected a lease-id config error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redeem_does_not_follow_redirects_and_fails_closed() {
        // With redirects disabled, a 3xx is surfaced as a non-success → fail-closed
        // (never chased to another host carrying the ticket/PAT).
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/leases/l/cas-cred"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "https://evil.example/"),
            )
            .mount(&server)
            .await;
        let err = redeem_cred_ticket(&server.uri(), "l", "t")
            .await
            .unwrap_err();
        match err {
            ClwError::Transport(m) => assert!(m.contains("302"), "expected HTTP 302, got: {m}"),
            other => panic!("expected fail-closed on 3xx, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // PART A — proptest: the LIST-envelope JSON parsers (AcListPage /
    // CasListPage, both pub(crate)) must never panic on arbitrary bytes — only
    // Ok/Err. These are the prune/ls live-set parsers fed straight from a
    // possibly-hostile server body. A panic = test failure (non-vacuous).
    // -----------------------------------------------------------------------
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        // Case count for the in-crate fuzzers. The deep run (50k) is OPT-IN via
        // the repo's existing fuzz lane signal (CLW_FUZZ_GATE=1, see
        // scripts/gates.sh); the default keeps the per-PR gate fast while still
        // exercising a meaningful 1k cases.
        fn fuzz_cases() -> u32 {
            if std::env::var("CLW_FUZZ_GATE").is_ok() {
                50_000
            } else {
                1_000
            }
        }

        // A list-page-shaped JSON value (right envelope keys, hostile item values)
        // so the fuzzer reaches serde's field decode + Digest::from_hex on each
        // item, not just the JSON-syntax reject.
        fn listpageish_json(
            item_key: &'static str,
            arr_key: &'static str,
        ) -> impl Strategy<Value = Vec<u8>> {
            let hexish = prop_oneof![
                "[0-9a-f]{64}",      // valid digest
                "[0-9a-fA-F]{0,80}", // wrong length / case
                ".{0,16}",           // garbage
            ];
            let item = hexish.prop_map(move |h| serde_json::json!({ item_key: h }));
            let cursor = prop_oneof![
                Just(serde_json::Value::Null),
                "[a-z0-9]{0,8}".prop_map(serde_json::Value::String),
            ];
            (proptest::collection::vec(item, 0..6), cursor).prop_map(move |(items, cur)| {
                serde_json::to_vec(&serde_json::json!({
                    arr_key: items, "next_cursor": cur,
                }))
                .unwrap()
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]

            /// `serde_json::from_slice::<AcListPage>` on ARBITRARY bytes never
            /// panics — only Ok/Err.
            #[test]
            fn ac_list_page_from_slice_never_panics(
                bytes in proptest::collection::vec(any::<u8>(), 0..4096)
            ) {
                let _: std::result::Result<AcListPage, _> = serde_json::from_slice(&bytes);
            }

            /// `serde_json::from_slice::<CasListPage>` on ARBITRARY bytes never
            /// panics — only Ok/Err.
            #[test]
            fn cas_list_page_from_slice_never_panics(
                bytes in proptest::collection::vec(any::<u8>(), 0..4096)
            ) {
                let _: std::result::Result<CasListPage, _> = serde_json::from_slice(&bytes);
            }

            /// STRUCTURED AcListPage envelope (valid keys, hostile ref_key hex)
            /// never panics. INVARIANT: a page that parses Ok yields a finite,
            /// non-looping cursor (empty-string cursor normalized to None).
            #[test]
            fn ac_list_page_structured_never_panics(
                bytes in listpageish_json("ref_key", "refs")
            ) {
                if let Ok(p) = serde_json::from_slice::<AcListPage>(&bytes) {
                    let (_keys, cursor) = p.into_parts();
                    prop_assert!(cursor.as_deref() != Some(""));
                }
            }

            /// STRUCTURED CasListPage envelope (valid keys, hostile hash hex)
            /// never panics; same cursor-normalization invariant.
            #[test]
            fn cas_list_page_structured_never_panics(
                bytes in listpageish_json("hash", "blobs")
            ) {
                if let Ok(p) = serde_json::from_slice::<CasListPage>(&bytes) {
                    let (_keys, cursor) = p.into_parts();
                    prop_assert!(cursor.as_deref() != Some(""));
                }
            }

            /// A missing array key (server bug / proxy `{}`) must be a HARD parse
            /// ERROR, never a silently-empty live set (which would drive prune to
            /// wipe the tenant). This pins that contract under fuzzing.
            #[test]
            fn ac_list_page_missing_array_is_error(cursor in "[a-z0-9]{0,8}") {
                let bytes = serde_json::to_vec(&serde_json::json!({ "next_cursor": cursor })).unwrap();
                prop_assert!(serde_json::from_slice::<AcListPage>(&bytes).is_err());
            }

            /// `redact_endpoint_for_display` (the plaintext-guard error path) on an
            /// ARBITRARY endpoint string never panics — only returns a String.
            #[test]
            fn redact_endpoint_never_panics(ep in ".*") {
                let _ = redact_endpoint_for_display(&ep);
            }

            /// SECURITY INVARIANT: a `user:secret@host` userinfo must NEVER survive
            /// into the displayed string. We assemble a `scheme://user:PASSWORD@host`
            /// endpoint from fuzzed parts and assert the unique password sentinel is
            /// absent from the redaction. Both code paths (url-crate parse AND the
            /// string-scan fallback) are exercised because the host/scheme draws mix
            /// parseable and unparseable authorities.
            #[test]
            fn redact_endpoint_strips_credentials(
                scheme in prop_oneof!["https", "http", "[a-z]{1,6}"],
                user in "[a-zA-Z0-9._-]{1,12}",
                host in prop_oneof!["[a-z0-9.-]{1,20}", "[a-z0-9.:\\[\\]-]{1,24}"],
                tail in "[a-zA-Z0-9/_.-]{0,16}",
            ) {
                // A high-entropy password sentinel that cannot occur by chance in
                // the fuzzed scheme/user/host/tail alphabets above.
                let password = "Zq7Pw3xK9vN2secretSENTINEL";
                let ep = format!("{scheme}://{user}:{password}@{host}/{tail}");
                let shown = redact_endpoint_for_display(&ep);
                prop_assert!(
                    !shown.contains(password),
                    "password leaked through redaction: input={:?} shown={:?}", ep, shown
                );
            }
        }
    }

    fn make_client(endpoint: String) -> HttpClient {
        HttpClient::new(test_config(endpoint)).unwrap()
    }

    fn digest_and_bytes() -> (Digest, Vec<u8>) {
        let bytes = b"hello corelink".to_vec();
        let digest = Digest::of_bytes(&bytes);
        (digest, bytes)
    }

    // -------------------------------------------------------------------------
    // CAS exists — HEAD mappings
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cas_exists_head_200_returns_true() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("HEAD"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        assert!(CasTransport::exists(&client, &digest).await.unwrap());
    }

    #[tokio::test]
    async fn cas_exists_head_404_returns_false() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("HEAD"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        assert!(!CasTransport::exists(&client, &digest).await.unwrap());
    }

    #[tokio::test]
    async fn cas_exists_head_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("HEAD"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::exists(&client, &digest).await.unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    // -------------------------------------------------------------------------
    // CAS exists — HEAD 405 → GET fallback
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cas_exists_head_405_falls_back_to_get_found() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("HEAD"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        assert!(CasTransport::exists(&client, &digest).await.unwrap());
    }

    #[tokio::test]
    async fn cas_exists_head_501_falls_back_to_get_not_found() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("HEAD"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(501))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        assert!(!CasTransport::exists(&client, &digest).await.unwrap());
    }

    // -------------------------------------------------------------------------
    // CAS get — status mappings
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cas_get_200_returns_bytes() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let result = CasTransport::get(&client, &digest).await.unwrap();
        assert_eq!(result, bytes);
    }

    #[tokio::test]
    async fn cas_get_404_returns_not_found() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        assert!(matches!(err, ClwError::NotFound(_)));
    }

    #[tokio::test]
    async fn cas_get_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    #[tokio::test]
    async fn cas_get_integrity_violation() {
        let server = MockServer::start().await;
        let (digest, _real_bytes) = digest_and_bytes();
        let wrong_bytes = b"wrong data entirely".to_vec();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(wrong_bytes))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Integrity { expected, actual } if expected == digest && actual != digest)
        );
    }

    #[tokio::test]
    async fn cas_get_oversized_body_rejected_before_oom() {
        // Brutal-audit (2026-06-15) MED: a hostile/buggy server answering a CAS
        // GET with a body larger than CAS_BLOB_CAP_BYTES must be rejected (TooLarge)
        // rather than buffered into memory. The integrity check never gets the
        // chance to run — the cap is what stands between the body and the alloc.
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let oversized = vec![0u8; CAS_BLOB_CAP_BYTES + 1];
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(oversized))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        assert!(
            matches!(err, ClwError::TooLarge { cap, .. } if cap == CAS_BLOB_CAP_BYTES as u64),
            "oversized CAS body must be refused as TooLarge, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cas_get_other_status_returns_http_error() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        assert!(matches!(err, ClwError::Http { status: 403, .. }));
    }

    // -------------------------------------------------------------------------
    // CAS put — status mappings
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cas_put_201_returns_ok() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        CasTransport::put(&client, &digest, bytes).await.unwrap();
    }

    #[tokio::test]
    async fn cas_put_204_returns_ok() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        CasTransport::put(&client, &digest, bytes).await.unwrap();
    }

    #[tokio::test]
    async fn cas_put_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::put(&client, &digest, bytes)
            .await
            .unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    #[tokio::test]
    async fn cas_put_413_returns_too_large() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(413))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::put(&client, &digest, bytes)
            .await
            .unwrap_err();
        assert!(matches!(err, ClwError::TooLarge { .. }));
    }

    #[tokio::test]
    async fn cas_put_too_large_pre_network_guard() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();

        // No mocks registered — zero hits expected.
        let client = make_client(server.uri());
        let big_bytes = vec![0u8; CAS_BLOB_CAP_BYTES + 1];
        let err = CasTransport::put(&client, &digest, big_bytes)
            .await
            .unwrap_err();
        assert!(matches!(err, ClwError::TooLarge { .. }));

        // Verify no requests reached the mock server.
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 0, "no request should have been sent");
    }

    // -------------------------------------------------------------------------
    // Bearer header is sent on all request types
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn bearer_header_sent_on_cas_get() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let result = CasTransport::get(&client, &digest).await.unwrap();
        assert_eq!(result, bytes);
    }

    #[tokio::test]
    async fn bearer_header_sent_on_ac_put() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap();
    }

    // -------------------------------------------------------------------------
    // User-Agent header is sent on outgoing requests (M5.2)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn user_agent_header_sent_on_cas_get() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        // Match only requests whose User-Agent is exactly our built value. If the
        // header is missing or wrong, the mock won't match and the call errors.
        Mock::given(method("GET"))
            .and(path(&path_str))
            .and(header("User-Agent", super::user_agent()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let result = CasTransport::get(&client, &digest).await.unwrap();
        assert_eq!(result, bytes);

        // And the value carries the clw/<version> (<os>; <arch>) shape.
        let ua = super::user_agent();
        assert!(ua.starts_with("clw/"), "unexpected User-Agent: {ua}");
        assert!(ua.contains(std::env::consts::OS), "UA missing os: {ua}");
        assert!(ua.contains(std::env::consts::ARCH), "UA missing arch: {ua}");
    }

    #[tokio::test]
    async fn user_agent_header_sent_on_ac_put() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .and(header("User-Agent", super::user_agent()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap();
    }

    // -------------------------------------------------------------------------
    // AC get — status mappings
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac_get_200_returns_some() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);
        let value = b"ref-record-bytes".to_vec();

        Mock::given(method("GET"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(value.clone()))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        assert_eq!(AcTransport::get(&client, &key).await.unwrap(), Some(value));
    }

    #[tokio::test]
    async fn ac_get_404_returns_none() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        assert_eq!(AcTransport::get(&client, &key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn ac_get_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::get(&client, &key).await.unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    // -------------------------------------------------------------------------
    // AC put — status mappings
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac_put_200_returns_ok() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ac_put_201_returns_ok() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ac_put_204_returns_ok() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ac_put_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::put(&client, &key, b"value".to_vec())
            .await
            .unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    // -------------------------------------------------------------------------
    // AC delete — status mappings (LIVE D-1; 405/501 → server-regression)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac_delete_204_returns_ok() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        AcTransport::delete(&client, &key).await.unwrap();
    }

    #[tokio::test]
    async fn ac_delete_404_is_idempotent_ok() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        // Deleting an absent key returns Ok(()) — idempotent.
        AcTransport::delete(&client, &key).await.unwrap();
    }

    #[tokio::test]
    async fn ac_delete_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::delete(&client, &key).await.unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    #[tokio::test]
    async fn ac_delete_405_surfaces_unavailable_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        // The server-regression case: a LIVE capability returns 405/501.
        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::delete(&client, &key).await.unwrap_err();
        match err {
            ClwError::Transport(msg) => assert!(
                msg.contains("405/501"),
                "405 must surface the server-regression message, got: {msg}"
            ),
            other => panic!("expected Transport server-regression error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac_delete_501_surfaces_unavailable_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(501))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::delete(&client, &key).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(msg) if msg.contains("405/501")),
            "501 must surface the server-regression message"
        );
    }

    #[tokio::test]
    async fn ac_delete_other_status_returns_http_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::delete(&client, &key).await.unwrap_err();
        assert!(matches!(err, ClwError::Http { status: 403, .. }));
    }

    // -------------------------------------------------------------------------
    // AC list_refs — array / cursor pagination / server-regression (D-7)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac_list_refs_single_page() {
        // Ratified D-7 envelope: {"refs":[{"ref_key":"<hex>",..}], "next_cursor":null}.
        let server = MockServer::start().await;
        let d1 = Digest::of_bytes(b"ref-one");
        let d2 = Digest::of_bytes(b"ref-two");
        // Include the extra contract fields to prove they're tolerated/ignored.
        let body = format!(
            r#"{{"refs":[{{"ref_key":"{d1}","updated_at":"2026-06-14T00:00:00Z","size":10}},{{"ref_key":"{d2}","updated_at":"2026-06-14T00:00:01Z","size":20}}],"next_cursor":null}}"#
        );

        Mock::given(method("GET"))
            .and(path("/v1/ac/test-tenant"))
            .and(header("Authorization", "Bearer test-token"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let refs = AcTransport::list_refs(&client).await.unwrap();
        assert_eq!(refs, vec![d1, d2]);
    }

    #[tokio::test]
    async fn ac_list_refs_cursor_pagination_flattens_all_pages() {
        let server = MockServer::start().await;
        let d1 = Digest::of_bytes(b"page1-a");
        let d2 = Digest::of_bytes(b"page2-a");

        // Page 2 (continuation): cursor present → terminal page (next_cursor null).
        Mock::given(method("GET"))
            .and(path("/v1/ac/test-tenant"))
            .and(query_param("cursor", "next-1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"refs":[{{"ref_key":"{d2}"}}],"next_cursor":null}}"#
            )))
            .mount(&server)
            .await;
        // Page 1 (first, cursor-less): a non-null next_cursor points at page 2.
        Mock::given(method("GET"))
            .and(path("/v1/ac/test-tenant"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"refs":[{{"ref_key":"{d1}"}}],"next_cursor":"next-1"}}"#
            )))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let refs = AcTransport::list_refs(&client).await.unwrap();
        // Both pages flattened, in page order. (null cursor on page 2 stops.)
        assert_eq!(refs, vec![d1, d2]);
    }

    #[tokio::test]
    async fn ac_list_refs_405_surfaces_unavailable_d7() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/ac/test-tenant"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::list_refs(&client).await.unwrap_err();
        match err {
            ClwError::Transport(msg) => {
                assert!(msg.contains("405/501"), "got: {msg}");
                assert!(msg.contains("D-7"), "must cite D-7, got: {msg}");
            }
            other => panic!("expected Transport server-regression error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac_list_refs_501_surfaces_unavailable_d7() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/ac/test-tenant"))
            .respond_with(ResponseTemplate::new(501))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::list_refs(&client).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(msg) if msg.contains("405/501") && msg.contains("D-7"))
        );
    }

    // -------------------------------------------------------------------------
    // CAS delete — idempotent (D-8); 204/404 → Ok, 405|501 → server-regression
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cas_delete_204_returns_ok() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        CasTransport::delete(&client, &digest).await.unwrap();
    }

    #[tokio::test]
    async fn cas_delete_404_is_idempotent_ok() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        // Deleting an absent blob returns Ok(()) — idempotent.
        CasTransport::delete(&client, &digest).await.unwrap();
    }

    #[tokio::test]
    async fn cas_delete_401_returns_auth_error() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::delete(&client, &digest).await.unwrap_err();
        assert!(matches!(err, ClwError::Auth));
    }

    #[tokio::test]
    async fn cas_delete_405_surfaces_unavailable_d8() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::delete(&client, &digest).await.unwrap_err();
        match err {
            ClwError::Transport(msg) => {
                assert!(msg.contains("405/501"), "got: {msg}");
                assert!(msg.contains("D-8"), "must cite D-8, got: {msg}");
            }
            other => panic!("expected Transport server-regression error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cas_delete_501_surfaces_unavailable_d8() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(501))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::delete(&client, &digest).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(msg) if msg.contains("405/501") && msg.contains("D-8"))
        );
    }

    // -------------------------------------------------------------------------
    // CAS list_blobs — array / cursor pagination / server-regression (D-8)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cas_list_blobs_single_page() {
        // Ratified D-8 envelope: {"blobs":[{"hash":"<hex>",..}], "next_cursor":null}.
        let server = MockServer::start().await;
        let d1 = Digest::of_bytes(b"blob-one");
        let d2 = Digest::of_bytes(b"blob-two");
        let body = format!(
            r#"{{"blobs":[{{"hash":"{d1}","size":10,"created_at":"2026-06-14T00:00:00Z"}},{{"hash":"{d2}","size":20,"created_at":"2026-06-14T00:00:01Z"}}],"next_cursor":null}}"#
        );

        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .and(header("Authorization", "Bearer test-token"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let blobs = CasTransport::list_blobs(&client).await.unwrap();
        assert_eq!(blobs, vec![d1, d2]);
    }

    #[tokio::test]
    async fn cas_list_blobs_cursor_pagination_flattens_all_pages() {
        let server = MockServer::start().await;
        let d1 = Digest::of_bytes(b"cblob-1");
        let d2 = Digest::of_bytes(b"cblob-2");

        // Page 2 (continuation): terminal page (next_cursor null).
        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .and(query_param("cursor", "c2"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"blobs":[{{"hash":"{d2}"}}],"next_cursor":null}}"#
            )))
            .mount(&server)
            .await;
        // Page 1: a non-null next_cursor points at page 2.
        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"blobs":[{{"hash":"{d1}"}}],"next_cursor":"c2"}}"#
            )))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let blobs = CasTransport::list_blobs(&client).await.unwrap();
        assert_eq!(blobs, vec![d1, d2]);
    }

    #[tokio::test]
    async fn cas_list_blobs_405_surfaces_unavailable_d8() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::list_blobs(&client).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(msg) if msg.contains("405/501") && msg.contains("D-8"))
        );
    }

    #[tokio::test]
    async fn cas_list_blobs_501_surfaces_unavailable_d8() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .respond_with(ResponseTemplate::new(501))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::list_blobs(&client).await.unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(msg) if msg.contains("405/501") && msg.contains("D-8"))
        );
    }

    // -------------------------------------------------------------------------
    // Retry behavior
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn retry_500_then_200_succeeds() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        // First call returns 500, second returns 200.
        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let result = CasTransport::get(&client, &digest).await.unwrap();
        assert_eq!(result, bytes);
    }

    #[tokio::test]
    async fn retry_persistent_500s_returns_http_error() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        assert!(matches!(err, ClwError::Http { status: 500, .. }));

        // Verify we exhausted the full retry budget (MAX_ATTEMPTS total).
        let received = server.received_requests().await.unwrap();
        assert_eq!(
            received.len(),
            MAX_ATTEMPTS as usize,
            "expected MAX_ATTEMPTS attempts"
        );
    }

    #[tokio::test]
    async fn no_retry_on_404() {
        let server = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let _ = CasTransport::get(&client, &digest).await;

        // Only 1 attempt — no retry on 4xx.
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1, "404 must not be retried");
    }

    #[tokio::test]
    async fn no_retry_on_401() {
        let server = MockServer::start().await;
        let (digest, bytes) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);

        Mock::given(method("PUT"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let _ = CasTransport::put(&client, &digest, bytes).await;

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1, "401 must not be retried");
    }

    // -------------------------------------------------------------------------
    // Transport hardening — refuse PAT over plaintext http:// (loopback allowed)
    // -------------------------------------------------------------------------

    #[test]
    fn new_rejects_plaintext_http_endpoint() {
        let err = HttpClient::new(test_config("http://example.com".into())).unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(_)),
            "plaintext http:// non-loopback must be refused, got {err:?}"
        );
    }

    #[test]
    fn new_accepts_https_endpoint() {
        let client = HttpClient::new(test_config("https://corelink-api.humangr.com".into()));
        assert!(client.is_ok(), "https:// endpoint must be accepted");
    }

    #[test]
    fn new_accepts_loopback_http_endpoint() {
        let client = HttpClient::new(test_config("http://127.0.0.1:8080".into()));
        assert!(
            client.is_ok(),
            "http://127.0.0.1 loopback must be accepted for dev/tests"
        );
    }

    #[test]
    fn new_rejects_loopback_prefix_impersonation() {
        // Regression (M0.5): the loopback allow-list must be an EXACT-host check,
        // not a prefix match. Attacker-controlled hosts that merely START WITH a
        // loopback string must be refused, or the PAT leaks in cleartext to them.
        for ep in [
            "http://127.0.0.1.evil.com",
            "http://localhostx",
            "http://localhost.attacker.net",
            "http://127.0.0.1@evil.com/",
            "http://[::1].evil.com",
            // Regression (CRIT, brutal-audit 2026-06-15): the `:port@host` userinfo
            // smuggle. The hand-rolled parser split the single colon and read the
            // host as `127.0.0.1`, while url/reqwest dial the REAL host `evil.com`
            // — a one-line PAT-exfiltration. All must be refused.
            "http://127.0.0.1:8080@evil.com/v1",
            "http://localhost:1@evil.com",
            "http://[::1]:1@evil.com",
            "http://127.0.0.1:80@evil.com:443/v1/cas",
        ] {
            let err = HttpClient::new(test_config(ep.into())).unwrap_err();
            assert!(
                matches!(err, ClwError::Transport(_)),
                "loopback-prefix impersonation {ep:?} must be refused, got {err:?}"
            );
        }
        // And the genuine loopback hosts (with/without port/path) stay accepted.
        for ep in [
            "http://localhost",
            "http://127.0.0.1/v1",
            "http://[::1]:9000",
        ] {
            assert!(
                HttpClient::new(test_config(ep.into())).is_ok(),
                "genuine loopback {ep:?} must be accepted"
            );
        }
    }

    #[test]
    fn new_rejects_https_userinfo_smuggling() {
        // Regression (HIGH, brutal-audit 2026-06-15): the https gate was a naive
        // `starts_with("https://")`, so `https://legit@evil.com` passed while
        // reqwest dials `evil.com` (the `legit` is userinfo) — the PAT is sent
        // over TLS to the attacker host. Same `@`-smuggle class as the loopback
        // CRIT, on the TLS path. All embedded-credential forms must be refused.
        for ep in [
            "https://legit-host@evil.com/v1",
            "https://127.0.0.1@evil.com",
            "https://user:pass@evil.com/v1/cas",
            "https://api.corelink.dev:443@evil.com",
        ] {
            let err = HttpClient::new(test_config(ep.into())).unwrap_err();
            assert!(
                matches!(err, ClwError::Transport(_)),
                "https userinfo smuggle {ep:?} must be refused, got {err:?}"
            );
        }
        // Genuine https endpoints (with/without port/path) stay accepted.
        for ep in [
            "https://api.corelink.dev",
            "https://api.corelink.dev:8443/v1",
            "https://10.0.0.5/v1/cas",
        ] {
            assert!(
                HttpClient::new(test_config(ep.into())).is_ok(),
                "genuine https {ep:?} must be accepted"
            );
        }
    }

    #[test]
    fn is_loopback_http_resists_exotic_forms() {
        // Adversarial hardening of the CRIT fix: prove the `url`-parsed guard is
        // safe across exotic host/credential representations, not just the two
        // cases that motivated it. Invariant: return `true` only for a host the
        // SAME parser (and thus reqwest) resolves to genuine loopback.

        // MUST be refused — non-loopback hosts, embedded credentials, or hosts the
        // parser keeps as a domain (never dialed as loopback).
        for ep in [
            "http://127.0.0.1:8080@evil.com/v1", // userinfo smuggle (the CRIT)
            "http://localhost:1@evil.com",
            "http://[::1]:1@evil.com",
            "http://user:pass@127.0.0.1/", // explicit userinfo on a real loopback host
            "http://127.0.0.1.evil.com/",  // suffix attack
            "http://localhost.evil.com/",
            "http://notlocalhost/",
            "http://169.254.169.254/", // link-local metadata, NOT loopback
            "http://10.0.0.1/",
            "ftp://127.0.0.1/", // non-http scheme
        ] {
            assert!(
                !is_loopback_http(ep),
                "exotic non-loopback {ep:?} must NOT be treated as loopback"
            );
        }

        // MUST be accepted — genuine loopback the parser resolves to 127.0.0.0/8,
        // ::1, or `localhost`, regardless of case, port, path, query, or a `@`
        // that lands in the PATH (not userinfo).
        for ep in [
            "http://127.0.0.1",
            "http://127.0.0.1:8080/v1/cas?x=1",
            "http://127.5.6.7/", // all of 127.0.0.0/8 is loopback
            "http://LOCALHOST/", // host is case-insensitive
            "http://localhost:3000/path@stillpath", // `@` in the PATH, host stays loopback
            "http://[::1]/",
            // Alternate IPv4 encodings: url canonicalizes these to 127.0.0.1 for
            // the http scheme — the SAME value reqwest dials — so judging them
            // loopback is consistent and safe (the PAT never leaves the machine).
            "http://2130706433/", // decimal 127.0.0.1
            "http://0x7f000001/", // hex 127.0.0.1
        ] {
            assert!(
                is_loopback_http(ep),
                "genuine loopback {ep:?} must be accepted"
            );
        }
    }

    // -------------------------------------------------------------------------
    // FIX 2: the total-ENTRY ceiling aborts a server that paginates forever with
    // full pages, independent of the page cap. Driving 10M real entries is
    // infeasible, so we exercise the inner paginator with a tiny max_total: the
    // server always returns a one-entry page with a fresh, strictly-advancing
    // cursor (so neither MAX_PAGES nor the repeated-cursor guard fires first), and
    // the accumulation must abort with TooLarge once it exceeds the cap.
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn list_aborts_when_total_entries_exceed_cap() {
        let server = MockServer::start().await;

        // Each call returns a distinct blob hash + a distinct next_cursor, so the
        // server "paginates forever". A shared counter makes every page unique.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        SEQ.store(0, Ordering::SeqCst);

        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .respond_with(|_: &wiremock::Request| {
                let n = SEQ.fetch_add(1, Ordering::SeqCst);
                // A unique, valid blob hash per page and a unique advancing cursor
                // so the liveness guards never trip before the entry cap does.
                let d = Digest::of_bytes(format!("blob-{n}").as_bytes());
                ResponseTemplate::new(200).set_body_string(format!(
                    r#"{{"blobs":[{{"hash":"{d}"}}],"next_cursor":"cur-{}"}}"#,
                    n + 1
                ))
            })
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let base_url = format!("{}/v1/cas/test-tenant", server.uri());
        // Cap at 3 entries (page cap left huge so the ENTRY cap trips first): the
        // 4th accumulated entry must trip the abort.
        let err = client
            .list_digests_capped::<CasListPage>(&base_url, "unused", 3, u64::MAX)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClwError::TooLarge { cap, .. } if cap == 3),
            "exceeding the total-entry cap must abort with TooLarge, got: {err:?}"
        );
    }

    // -------------------------------------------------------------------------
    // FIX #4: a server returning EMPTY pages with ALWAYS-ADVANCING (never-repeating)
    // cursors trips neither the entry cap (zero entries accumulate) nor the repeated-
    // cursor guard (every cursor is unique) — without a hard PAGE cap, `seen_cursors`
    // would grow without bound (OOM). The page-count cap must abort the loop at the
    // cap with a clear "pagination exceeded N pages" error, not OOM/hang. We drive the
    // inner paginator with a tiny max_pages so the abort is reachable in a test.
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn list_aborts_on_always_advancing_empty_pages_at_page_cap() {
        let server = MockServer::start().await;

        // Every response: an EMPTY blob list + a UNIQUE, advancing cursor → the loop
        // would page forever, accumulating one fresh cursor per page, but never any
        // entry. Only the page cap can stop it.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        SEQ.store(0, Ordering::SeqCst);
        Mock::given(method("GET"))
            .and(path("/v1/cas/test-tenant"))
            .respond_with(|_: &wiremock::Request| {
                let n = SEQ.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .set_body_string(format!(r#"{{"blobs":[],"next_cursor":"cur-{}"}}"#, n + 1))
            })
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let base_url = format!("{}/v1/cas/test-tenant", server.uri());
        // Entry cap huge (never trips — no entries); page cap = 5 → aborts at page 6.
        let err = client
            .list_digests_capped::<CasListPage>(&base_url, "unused", u64::MAX, 5)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClwError::Transport(ref m) if m.contains("pagination exceeded 5 pages")),
            "always-advancing empty pages must abort at the page cap, got: {err:?}"
        );
    }

    // -------------------------------------------------------------------------
    // FIX 5: the client must NOT follow redirects. A 302 to ANOTHER host is left
    // unfollowed: the redirect target receives no request (so the bearer PAT is
    // never sent there), and the 302 surfaces as a non-success Http error rather
    // than the leaked 200 body.
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn redirect_is_not_followed_and_pat_not_leaked() {
        let target = MockServer::start().await;
        // The redirect TARGET: if the client wrongly followed the 302, this 200
        // would be returned (and would have carried the Authorization header).
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"leaked".to_vec()))
            .mount(&target)
            .await;

        let origin = MockServer::start().await;
        let (digest, _) = digest_and_bytes();
        let path_str = format!("/v1/cas/test-tenant/{}", digest);
        // The origin answers the CAS GET with a 302 → the OTHER host.
        Mock::given(method("GET"))
            .and(path(&path_str))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", target.uri().as_str()),
            )
            .mount(&origin)
            .await;

        let client = make_client(origin.uri());
        let err = CasTransport::get(&client, &digest).await.unwrap_err();
        // A 302 is NOT a success: with redirects disabled it surfaces as an Http
        // error (the catch-all arm), never as the leaked 200 body.
        assert!(
            matches!(err, ClwError::Http { status: 302, .. }),
            "302 must surface as a non-success Http error, got: {err:?}"
        );

        // The crux: the redirect target received ZERO requests — the PAT was never
        // sent to another host.
        let target_hits = target.received_requests().await.unwrap();
        assert_eq!(
            target_hits.len(),
            0,
            "redirect target must receive no request (PAT must not be sent there)"
        );
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;
    use clw_types::{CasTransport, Config, Digest};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn config_for(server: &MockServer) -> Config {
        Config {
            endpoint: server.uri(),
            tenant: "t1".into(),
            token: "test-pat".into(),
            cache_dir: std::env::temp_dir(),
        }
    }

    /// Dogfood day-1 regression: prod answered 429 to the first real snapshot
    /// and the client gave up. 429 MUST back off and retry, then succeed.
    #[tokio::test]
    async fn put_retries_through_429_then_succeeds() {
        let server = MockServer::start().await;
        let bytes = b"rate-limited-blob".to_vec();
        let digest = Digest::of_bytes(&bytes);
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = HttpClient::new(config_for(&server)).unwrap();
        CasTransport::put(&client, &digest, bytes)
            .await
            .expect("429s must be retried through");
    }

    /// A 429 carrying `retry-after: 1` must make the client sleep ~that long
    /// before the retry that succeeds — proving Retry-After overrides the fast
    /// (200 ms) first ladder rung.
    #[tokio::test]
    async fn retry_after_header_drives_backoff_then_succeeds() {
        let server = MockServer::start().await;
        let bytes = b"retry-after-blob".to_vec();
        let digest = Digest::of_bytes(&bytes);
        // First attempt: 429 + Retry-After: 1 (second).
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let client = HttpClient::new(config_for(&server)).unwrap();
        let start = std::time::Instant::now();
        CasTransport::put(&client, &digest, bytes)
            .await
            .expect("retry after backoff must succeed");
        let elapsed = start.elapsed();
        // Retry-After of 1 s must dominate the 200 ms first ladder rung. Allow a
        // little slack below 1 s for scheduling, but require well above the rung.
        assert!(
            elapsed >= std::time::Duration::from_millis(900),
            "expected ~1s Retry-After backoff, slept only {elapsed:?}"
        );
    }

    /// The AIMD governor multiplicatively shrinks on a 429 and additively
    /// recovers after a run of successes.
    #[tokio::test]
    async fn governor_shrinks_on_throttle_then_recovers() {
        let gov = std::sync::Arc::new(super::Governor::new(8));
        assert_eq!(gov.target(), 8, "starts at ceiling");

        // One 429 halves 8 -> 4.
        gov.on_throttle();
        assert_eq!(gov.target(), 4, "429 halves the window");
        // Another halves 4 -> 2, then 2 -> 1, then floors at 1.
        gov.on_throttle();
        gov.on_throttle();
        gov.on_throttle();
        assert_eq!(gov.target(), 1, "floor at 1");

        // A run of successes additively increases by one per run.
        for _ in 0..super::AIMD_INCREASE_AFTER_SUCCESSES {
            gov.on_success();
        }
        assert_eq!(gov.target(), 2, "additive increase after a success run");

        // And it never exceeds the ceiling no matter how many successes.
        for _ in 0..(super::AIMD_INCREASE_AFTER_SUCCESSES as usize * 20) {
            gov.on_success();
        }
        assert_eq!(
            gov.target(),
            8,
            "additive increase is capped at the ceiling"
        );
    }

    /// The governor permit pool actually bounds in-flight acquisitions: after a
    /// throttle, the live permit budget shrinks (debt is repaid on release).
    #[tokio::test]
    async fn governor_permit_budget_shrinks_after_throttle() {
        let gov = std::sync::Arc::new(super::Governor::new(4));
        // Acquire all 4, then throttle: target -> 2, two permits owed as debt
        // since none are idle.
        let p1 = gov.acquire().await;
        let p2 = gov.acquire().await;
        let p3 = gov.acquire().await;
        let p4 = gov.acquire().await;
        gov.on_throttle();
        assert_eq!(gov.target(), 2);
        // Releasing all four repays the 2-permit debt: only 2 return to the pool.
        drop(p1);
        drop(p2);
        drop(p3);
        drop(p4);
        assert_eq!(
            gov.sem.available_permits(),
            2,
            "debt repaid on release leaves the window at the shrunk target"
        );
    }

    /// Exhausted 429s still surface as Http{429} (no infinite loop).
    #[tokio::test]
    async fn put_exhausts_429_retries_with_error() {
        let server = MockServer::start().await;
        let bytes = b"always-throttled".to_vec();
        let digest = Digest::of_bytes(&bytes);
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let client = HttpClient::new(config_for(&server)).unwrap();
        let err = CasTransport::put(&client, &digest, bytes)
            .await
            .unwrap_err();
        match err {
            clw_types::ClwError::Http { status, .. } => assert_eq!(status, 429),
            other => panic!("expected Http 429, got {other:?}"),
        }
    }

    /// M5.6: a 429 whose final `Retry-After` is *huge* (≈ seconds-until-next
    /// month — an app/plan quota ceiling) must surface an actionable
    /// plan/quota message on exhaustion, not the generic rate-limited wording.
    /// Early attempts carry a small Retry-After to keep the backoff fast; the
    /// last attempt carries the huge value that drives the message.
    #[tokio::test]
    async fn exhausted_429_huge_retry_after_surfaces_quota_message() {
        let server = MockServer::start().await;
        let bytes = b"quota-ceiling".to_vec();
        let digest = Digest::of_bytes(&bytes);
        // All but the final attempt: 429 + small Retry-After (fast backoff).
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
            .up_to_n_times(5)
            .mount(&server)
            .await;
        // Final attempt: 429 + a huge Retry-After (~28 days), the quota signal.
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "2419200"))
            .mount(&server)
            .await;

        let client = HttpClient::new(config_for(&server)).unwrap();
        let err = CasTransport::put(&client, &digest, bytes)
            .await
            .unwrap_err();
        match err {
            clw_types::ClwError::Http { status, context } => {
                assert_eq!(status, 429);
                assert!(
                    context.contains("plan/quota"),
                    "expected plan/quota wording, got: {context}"
                );
                assert!(
                    context.contains("2419200"),
                    "quota message should include the retry-after seconds, got: {context}"
                );
            }
            other => panic!("expected Http 429, got {other:?}"),
        }
    }

    /// M5.6: a 429 whose final `Retry-After` is *small* (DO backpressure) must
    /// surface the generic rate-limited wording, not the quota message.
    #[tokio::test]
    async fn exhausted_429_small_retry_after_surfaces_rate_limited_message() {
        let server = MockServer::start().await;
        let bytes = b"transient-backpressure".to_vec();
        let digest = Digest::of_bytes(&bytes);
        // Every attempt: 429 + a small Retry-After (transient backpressure).
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/cas/t1/[0-9a-f]{64}$"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
            .mount(&server)
            .await;

        let client = HttpClient::new(config_for(&server)).unwrap();
        let err = CasTransport::put(&client, &digest, bytes)
            .await
            .unwrap_err();
        match err {
            clw_types::ClwError::Http { status, context } => {
                assert_eq!(status, 429);
                assert!(
                    context.contains("rate-limited"),
                    "expected rate-limited wording, got: {context}"
                );
                assert!(
                    !context.contains("plan/quota"),
                    "small retry-after must NOT trigger quota wording, got: {context}"
                );
            }
            other => panic!("expected Http 429, got {other:?}"),
        }
    }

    /// Adversarial-review regression: hammer the governor from many tasks at
    /// once — concurrent `on_throttle` / `on_success` / `acquire` — and assert
    /// the invariants hold under contention. The pre-fix lock-free counters lost
    /// decrements (over-forgetting could drive live permits to 0 → CAS/AC
    /// deadlock) and `on_success` could exceed the ceiling. With the single
    /// `Mutex<GovernorState>` each adjustment is atomic, so:
    ///   * no deadlock (every spawned acquire completes),
    ///   * `target` stays within `[1, ceiling]`,
    ///   * live permits never exceed the ceiling.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn governor_concurrent_throttle_success_acquire_is_consistent() {
        use tokio::task::JoinSet;

        const CEILING: usize = 8;
        let gov = std::sync::Arc::new(super::Governor::new(CEILING));

        let mut set: JoinSet<()> = JoinSet::new();
        // Throttlers: repeatedly shrink the window.
        for _ in 0..16 {
            let g = std::sync::Arc::clone(&gov);
            set.spawn(async move {
                for _ in 0..200 {
                    g.on_throttle();
                    tokio::task::yield_now().await;
                }
            });
        }
        // Successes: repeatedly grow the window back toward the ceiling.
        for _ in 0..16 {
            let g = std::sync::Arc::clone(&gov);
            set.spawn(async move {
                for _ in 0..200 {
                    g.on_success();
                    tokio::task::yield_now().await;
                }
            });
        }
        // Acquirers: hold a permit briefly, then release (exercising debt repay).
        for _ in 0..32 {
            let g = std::sync::Arc::clone(&gov);
            set.spawn(async move {
                for _ in 0..50 {
                    // A bounded acquire: if the window is momentarily at 0 live
                    // permits this would hang forever, surfacing a deadlock as a
                    // test timeout. Invariant under test: that never happens.
                    let permit = g.acquire().await;
                    tokio::task::yield_now().await;
                    drop(permit);
                    // After every release the live pool must still be bounded.
                    assert!(
                        g.sem.available_permits() <= CEILING,
                        "live permits exceeded ceiling: {} > {CEILING}",
                        g.sem.available_permits()
                    );
                }
            });
        }

        // Join with a generous timeout: a deadlock (permits driven to 0) would
        // hang the JoinSet and trip this.
        let joined = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while let Some(res) = set.join_next().await {
                res.expect("governor task panicked");
            }
        })
        .await;
        assert!(joined.is_ok(), "governor tasks deadlocked / timed out");

        // Final invariants: target within bounds, live pool within bounds.
        let target = gov.target();
        assert!(
            (1..=CEILING).contains(&target),
            "target out of [1, {CEILING}] range: {target}"
        );
        assert!(
            gov.sem.available_permits() <= CEILING,
            "final live permits exceeded ceiling"
        );
    }
}
