#![no_main]
//! Fuzz the tenant URL-path injection guard with arbitrary UTF-8.
//!
//! `Config::resolve` calls this before interpolating a tenant into the raw CAS
//! and AC request paths. An arbitrary string must always get an Ok/Err verdict
//! without panicking; it must never become an unchecked URL component.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(tenant) = std::str::from_utf8(data) {
        let _ = clw_types::validate_tenant(tenant);
    }
});
