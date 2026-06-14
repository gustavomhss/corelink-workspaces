#![no_main]
//! Fuzz the manifest deserialize + validate path of clw-types.
//!
//! `Manifest::from_bytes` is the single point where a manifest fetched from an
//! UNTRUSTED remote CAS becomes an in-memory `Manifest`: it runs the serde_json
//! parse AND the per-entry path / symlink-target / Windows-drive-letter
//! validation. A malicious server controls these bytes entirely, so this path
//! must NEVER panic, overflow, or hit UB on arbitrary input — it may only ever
//! return `Ok` (a fully validated manifest) or `Err`. We discard both; the only
//! property under test is "does not crash".
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Result is intentionally ignored: any Err is the expected, safe outcome.
    let _ = clw_types::Manifest::from_bytes(data);
});
