//! clw-conformance — live-API conformance suite for CoreLink CAS + AC.
//!
//! # Running the live suite
//!
//! The tests are gated behind the `live` feature so they are **never** executed
//! in CI by default (no token is available there). Run them manually with:
//!
//! ```text
//! CLW_TENANT=<your-tenant> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live
//! ```
//!
//! Optionally override the endpoint (defaults to the production URL):
//!
//! ```text
//! CLW_ENDPOINT=https://corelink-api.humangr.com \
//!   CLW_TENANT=<your-tenant> \
//!   CLW_TOKEN=<pat> \
//!   cargo test -p clw-conformance --features live
//! ```
//!
//! If `CLW_TOKEN` or `CLW_TENANT` is absent every test prints a skip message
//! and passes vacuously — this is intentional so the test binary can still be
//! compiled and run without credentials (e.g. in dry-run CI scenarios).

pub use clw_types as types;
