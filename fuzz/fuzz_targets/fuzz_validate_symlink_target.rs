#![no_main]
//! Fuzz `clw_types::validate_symlink_target` with arbitrary &str input.
//!
//! Symlink targets in an untrusted manifest are the classic path-escape vector:
//! this validator must reject absolute targets, backslashes, `..` traversal, and
//! Windows drive letters so a hostile manifest can't make hydration write/read
//! outside the workspace. It must reach an Ok/Err verdict on ANY string without
//! panicking. Arbitrary UTF-8 exercises the component/char-boundary logic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = clw_types::validate_symlink_target(s);
    }
});
