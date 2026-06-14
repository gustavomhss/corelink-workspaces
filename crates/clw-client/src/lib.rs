//! clw-client — HTTP transport for CAS and AC endpoints.
//!
//! Implements [`CasTransport`] and [`AcTransport`] from `clw-types` via
//! `reqwest`. Token is intentionally never logged or Debug-printed.

pub use clw_types as types;

use clw_types::{AcTransport, CasTransport, ClwError, Config, Digest, Result, CAS_BLOB_CAP_BYTES};
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
        // dev/tests (the wiremock suite uses http://127.0.0.1:<port>).
        let ep = config.endpoint.trim();
        let is_https = ep.starts_with("https://");
        let is_local_http = is_loopback_http(ep);
        if !is_https && !is_local_http {
            return Err(ClwError::Transport(format!(
                "refusing to send PAT over plaintext: endpoint must be https:// (got '{ep}'); http:// is only allowed for loopback"
            )));
        }

        let inner = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
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
        let last_err = last_err.unwrap();
        if let ClwError::Http { status: 429, .. } = &last_err {
            let retry_after = (last_retry_after_secs > 0).then_some(last_retry_after_secs);
            return Err(ClwError::Http {
                status: 429,
                context: exhausted_429_context(retry_after, MAX_ATTEMPTS),
            });
        }
        Err(last_err)
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
/// loopback (`127.0.0.1`, `localhost`, or `[::1]`), with an optional `:port` and
/// path. This is an exact-host check, **not** a prefix check: a prefix match would
/// accept attacker-controlled hosts like `http://127.0.0.1.evil.com` or
/// `http://localhostx` and leak the PAT in cleartext to them.
fn is_loopback_http(ep: &str) -> bool {
    let Some(rest) = ep.trim().strip_prefix("http://") else {
        return false;
    };
    // Authority = everything before the first '/' (drop any path).
    let authority = rest.split('/').next().unwrap_or(rest);
    // IPv6 literal: `[::1]` optionally followed by `:port`.
    if let Some(after) = authority.strip_prefix('[') {
        return match after.split_once(']') {
            Some((host, port)) => host == "::1" && (port.is_empty() || port.starts_with(':')),
            None => false,
        };
    }
    // `host[:port]` — strip an optional trailing `:port`.
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
    host == "127.0.0.1" || host == "localhost"
}

/// Returns `true` if the error warrants a retry.
fn is_retryable(e: &ClwError) -> bool {
    match e {
        ClwError::Transport(_) => true,
        ClwError::Http { status, .. } => *status >= 500 || *status == 429,
        _ => false,
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
                        let bytes = resp
                            .bytes()
                            .await
                            .map_err(|e| ClwError::Transport(e.to_string()))?;
                        let actual = Digest::of_bytes(&bytes);
                        if actual != digest {
                            return Err(ClwError::Integrity {
                                expected: digest,
                                actual,
                            });
                        }
                        Ok(bytes.to_vec())
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
                        let bytes = resp
                            .bytes()
                            .await
                            .map_err(|e| ClwError::Transport(e.to_string()))?;
                        Ok(Some(bytes.to_vec()))
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
                    // Optimistic-pending case: the server has no DELETE handler yet
                    // (this branch is HELD until corelink-server ships D-1).
                    405 | 501 => Err(ClwError::Transport(format!(
                        "ref deletion requires a corelink-server capability not yet deployed \
                         (DELETE /v1/ac); tracked — see docs/REQUESTS-to-other-TLs.md D-1 ({url})"
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(endpoint: String) -> Config {
        Config {
            endpoint,
            tenant: "test-tenant".into(),
            token: "test-token".into(),
            cache_dir: PathBuf::from("/tmp/clw-test-cache"),
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
    // AC delete — status mappings (optimistic; server DELETE pending)
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
    async fn ac_delete_405_surfaces_not_deployed_error() {
        let server = MockServer::start().await;
        let (key, _) = digest_and_bytes();
        let path_str = format!("/v1/ac/test-tenant/{}", key);

        // The optimistic-pending case: corelink-server has no DELETE handler yet.
        Mock::given(method("DELETE"))
            .and(path(&path_str))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;

        let client = make_client(server.uri());
        let err = AcTransport::delete(&client, &key).await.unwrap_err();
        match err {
            ClwError::Transport(msg) => assert!(
                msg.contains("not yet deployed"),
                "405 must surface the not-deployed message, got: {msg}"
            ),
            other => panic!("expected Transport not-deployed error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac_delete_501_surfaces_not_deployed_error() {
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
            matches!(err, ClwError::Transport(msg) if msg.contains("not yet deployed")),
            "501 must surface the not-deployed message"
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
