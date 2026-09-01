//! Owner-only Windows DACL hardening — the workspace's ONLY `unsafe` code.
//!
//! # Why this crate exists at all
//!
//! Replacing a file's Windows DACL has no safe Rust API; it must call the Win32
//! security APIs through `windows-sys` FFI, which is `unsafe` by definition.
//! Without it the plaintext-PAT `~/.clw/config.toml` inherits the parent's
//! (often group/Users-readable) ACL, because the unix `0600`/`0700` hardening is
//! a no-op on Windows.
//!
//! # Why it is a crate and not a function in `clw-cli`
//!
//! `[workspace.lints.rust] unsafe_code = "forbid"` cannot be locally re-allowed —
//! that is what `forbid` means — so the crate holding this block must sit at
//! `deny` instead, where an `#[allow]` still works. Whatever crate that is
//! becomes a place where `unsafe` can be reintroduced with the compiler silent.
//! `scripts/check-unsafe-posture.sh` therefore pins this crate BYTE FOR BYTE
//! against `.github/win-acl-sources.expected`.
//!
//! This block used to live in `clw-cli`, which put 8,554 lines in that position, and
//! cold review twice found an attribute spelling the guard's regex did not model.
//! (The spellings are recorded in `SECURITY.md` and replayed by
//! `scripts/selftest-unsafe-posture.sh`, not here: this file's contents are
//! hash-pinned and scanned, so naming an attribute in a comment would be
//! indistinguishable from using one.)
//!
//! So the guard no longer models Rust's grammar. Every OTHER member is at `forbid`,
//! where the compiler rejects any permission attribute however it is spelled — all
//! `error[E0453]`. And this crate, the one place an attribute legitimately works, is
//! pinned against that fixture: changing anything here means updating it in the same
//! commit, where a human reads it. The pin answers "did this crate change", not "how
//! many attributes are in it" — there is no count anywhere for a spelling to slip past.
//!
//! Keep this crate small. Every line added here is a line the compiler is not
//! checking for you, and a line a reviewer must read instead.
//!
//! # Non-Windows
//!
//! The whole module is `cfg(windows)`. On other targets [`restrict_to_owner`]
//! does not exist; callers gate their use the same way.

#[cfg(windows)]
mod imp;

#[cfg(windows)]
pub use imp::restrict_to_owner;
