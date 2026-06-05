//! clw-integration-tests — HTTP-level integration tests for the clw stack.
//!
//! Tests live in `tests/` and exercise `clw_snapshot`, `clw_hydrate`,
//! `clw_run`, and `clw_cache` through `HttpClient` against a wiremock server.
//! No requests are ever sent to the real `corelink-api.humangr.com`.
