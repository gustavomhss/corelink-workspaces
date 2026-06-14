#![no_main]
//! Fuzz `clw_types::validate_rel_path` with arbitrary &str input.
//!
//! This is the relative-path guard applied to every manifest entry path coming
//! from an untrusted manifest (rejects empty, absolute, `..`/`.` components,
//! backslashes, doubled separators, Windows drive letters). It must classify
//! any string as Ok/Err without panicking. We feed arbitrary UTF-8 so the
//! char-boundary / component-splitting logic is exercised on weird (but valid)
//! Unicode as well as plain ASCII.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Only valid UTF-8 reaches the validator (the real input is a JSON string,
    // always valid UTF-8). Non-UTF-8 byte sequences are simply skipped.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = clw_types::validate_rel_path(s);
    }
});
