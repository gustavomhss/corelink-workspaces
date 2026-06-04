//! clw-client — HTTP transport for CAS and AC endpoints.
//!
//! Implements [`CasTransport`] and [`AcTransport`] from `clw-types` via
//! `reqwest`. Token is intentionally never logged or Debug-printed.

pub use clw_types as types;

use clw_types::{AcTransport, CasTransport, ClwError, Config, Digest, Result, CAS_BLOB_CAP_BYTES};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

const MAX_ATTEMPTS: u32 = 3;
/// Sleep durations between attempt 1→2 and 2→3.
const RETRY_SLEEP_MS: [u64; 2] = [200, 800];

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
    /// Create a new [`HttpClient`].
    ///
    /// Configures:
    /// - connect timeout: 10 s
    /// - request timeout: 120 s
    pub fn new(config: Config) -> Result<Self> {
        let inner = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| ClwError::Transport(e.to_string()))?;
        Ok(Self { inner, config })
    }

    /// Authorization header value.
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.config.token)
    }

    /// Execute `op` with up to `MAX_ATTEMPTS` attempts, sleeping between them.
    /// Retries on connect/transport errors and 5xx responses.
    /// Returns `Ok(T)` on first success; propagates the last error on exhaustion.
    async fn with_retry<F, Fut, T>(&self, op: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err: Option<ClwError> = None;
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                let sleep_ms = RETRY_SLEEP_MS[(attempt - 1) as usize];
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            }
            match op().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if is_retryable(&e) {
                        last_err = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }
}

/// Returns `true` if the error warrants a retry.
fn is_retryable(e: &ClwError) -> bool {
    match e {
        ClwError::Transport(_) => true,
        ClwError::Http { status, .. } => *status >= 500,
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

        self.with_retry(|| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                // Try HEAD first.
                let resp = client
                    .head(&url)
                    .header("Authorization", &auth)
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 => return Ok(true),
                    404 => return Ok(false),
                    401 => return Err(ClwError::Auth),
                    // 405 Method Not Allowed or 501 Not Implemented → fall back to GET.
                    405 | 501 => {}
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
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 => Ok(true),
                    404 => Ok(false),
                    401 => Err(ClwError::Auth),
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

        self.with_retry(|| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Authorization", &auth)
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

        self.with_retry(|| {
            let url = url.clone();
            let auth = auth.clone();
            let bytes = bytes.clone();
            async move {
                let resp = client
                    .put(&url)
                    .header("Authorization", &auth)
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

        self.with_retry(|| {
            let url = url.clone();
            let auth = auth.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Authorization", &auth)
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

        self.with_retry(|| {
            let url = url.clone();
            let auth = auth.clone();
            let value = value.clone();
            async move {
                let resp = client
                    .put(&url)
                    .header("Authorization", &auth)
                    .body(value)
                    .send()
                    .await
                    .map_err(|e| ClwError::Transport(e.to_string()))?;

                match resp.status().as_u16() {
                    200 | 201 | 204 => Ok(()),
                    401 => Err(ClwError::Auth),
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
    async fn retry_three_500s_returns_http_error() {
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

        // Verify we actually retried (3 attempts total).
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 3, "expected 3 attempts");
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
}
