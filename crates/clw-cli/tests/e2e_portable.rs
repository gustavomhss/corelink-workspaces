//! PORTABLE real-binary e2e — the platform-agnostic command journeys, run on
//! EVERY OS (NOT `#[cfg(unix)]`, unlike `e2e.rs` which relies on unix symlinks +
//! modes). This closes the "the shipped Windows binary's command dispatch is never
//! observed on Windows" gap (docs/TEST-JOURNEY-MAP.md): the `windows.yml` lane runs
//! `cargo test --workspace`, so these now exercise the real `clw` binary on Windows.
//!
//! Every test here spawns the real binary and uses only offline commands
//! (completions / --version / doctor / config resolution) — no network, no
//! symlinks, no chmod — so the behavior is identical across platforms.

use assert_cmd::Command;
use tempfile::TempDir;

/// A clean, offline invocation: a throwaway cache dir, a tenant + token in the env
/// (so `doctor` reports a fully-configured identity), and no endpoint reachability
/// is required (doctor never phones home). Distinctive token so redaction is provable.
const SECRET_TOKEN: &str = "portable-secret-token-DO-NOT-LEAK";

fn base(cache: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("clw").unwrap();
    cmd.env("CLW_TENANT", "portable-tenant")
        .env("CLW_TOKEN", SECRET_TOKEN)
        .env("CLW_CACHE_DIR", cache.to_str().unwrap());
    cmd
}

#[test]
fn version_flag_prints_a_version() {
    let out = Command::cargo_bin("clw")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success(), "clw --version must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("clw") && stdout.chars().any(|c| c.is_ascii_digit()),
        "--version must print a name + a version number, got: {stdout}"
    );
}

#[test]
fn completions_every_shell_is_nonempty_and_offline() {
    let cache = TempDir::new().unwrap();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = base(cache.path())
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "completions {shell} must succeed, stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.trim().is_empty() && stdout.contains("clw"),
            "completions {shell} must emit a non-empty script mentioning clw"
        );
    }
}

#[test]
fn completions_unknown_shell_is_rejected() {
    let cache = TempDir::new().unwrap();
    let out = base(cache.path())
        .args(["completions", "notashell"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "an unknown completions shell must be rejected (non-zero exit)"
    );
}

#[test]
fn doctor_json_redacts_the_token_on_this_platform() {
    let cache = TempDir::new().unwrap();
    let out = base(cache.path())
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "doctor is offline and must exit 0, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The plaintext token must NEVER appear anywhere in the output.
    assert!(
        !stdout.contains(SECRET_TOKEN),
        "doctor must NOT leak the plaintext token"
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json must emit valid JSON");
    // The token field is present but redacted to a marker, not the value.
    assert_eq!(
        v["token"], "<set>",
        "doctor must report the token as <set>, not its value, got: {}",
        v["token"]
    );
    assert!(
        v["problems"].is_array(),
        "doctor --json must carry a problems array"
    );
}

#[test]
fn config_precedence_flag_tenant_beats_env_on_this_platform() {
    let cache = TempDir::new().unwrap();
    // CLW_TENANT=portable-tenant is in the env (via `base`); the --tenant flag must win.
    let out = base(cache.path())
        .args(["--tenant", "flag-wins-tenant", "--json", "doctor"])
        .output()
        .unwrap();
    assert!(out.status.success(), "doctor must exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["tenant_value"], "flag-wins-tenant",
        "the --tenant flag must beat CLW_TENANT env (precedence), got: {}",
        v["tenant_value"]
    );
}
