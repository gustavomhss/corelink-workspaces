#![no_main]
//! Fuzz `clw_types::RefRecord::from_bytes` with arbitrary remote AC bytes.
//!
//! A ref record is the mutable pointer fetched from the untrusted action-cache
//! namespace before hydrate, status, ls, and prune use its manifest digest. The
//! parser must accept a valid record or return an error, never panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = clw_types::RefRecord::from_bytes(data);
});
