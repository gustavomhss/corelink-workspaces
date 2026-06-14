#![no_main]
//! Fuzz `clw_types::Digest::from_hex` with arbitrary &str input.
//!
//! Digests arrive as hex strings from untrusted manifests, AC RefRecords, and
//! URLs. `from_hex` must parse exactly a 64-char lowercase hex string into a
//! [u8; 32] and reject everything else (bad length, non-hex, odd nibble count)
//! without panicking or mis-decoding. We feed arbitrary UTF-8 to stress the
//! hex-decode + `try_into::<[u8;32]>()` length check.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = clw_types::Digest::from_hex(s);
    }
});
