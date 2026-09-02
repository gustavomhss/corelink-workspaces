#![no_main]
//! Fuzz the live symlink-target containment check with a clean link path.
//!
//! `Manifest::from_bytes` and both producer/consumer filesystem paths call
//! `validate_symlink_target_contained`, not the retired strict validator. The
//! input is two UTF-8 lines: a workspace-relative link path and its target.
//! We first enforce the real call site's `validate_rel_path` precondition, then
//! exercise lexical `..` resolution without touching the filesystem.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let (link_rel, target) = input.split_once('\n').unwrap_or((input, ""));
        if clw_types::validate_rel_path(link_rel).is_ok() {
            let _ = clw_types::validate_symlink_target_contained(link_rel, target);
        }
    }
});
