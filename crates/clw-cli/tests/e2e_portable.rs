//! Portable-source, real-binary CLI tests for offline command dispatch.
//!
//! This target has no whole-file Unix cfg gate. Actual execution still depends
//! on the selected host and test command: source portability is not native
//! Windows acceptance. `windows.yml` builds on Ubuntu and runs explicit Wine
//! smoke plus hydrate `lexical_` tests; it does not execute this test target.
//!
//! These cases cover version, completions, doctor and selected configuration
//! behavior. They do not cover hydrate filesystem semantics or `run` subprocess
//! contracts. Doctor's redaction assertion here examines stdout; broader output
//! and native-platform evidence must name their own tests and observed results.
//! See docs/TEST-JOURNEY-MAP.md and docs/TESTING.md for the selection boundaries.

use assert_cmd::Command;
use tempfile::TempDir;

/// A clean, offline invocation: a throwaway cache dir, a tenant + token in the env
/// (so `doctor` reports a fully-configured identity), and no endpoint reachability
/// is required (doctor never phones home). Distinctive token so redaction is provable.
const SECRET_TOKEN: &str = "portable-secret-token-DO-NOT-LEAK";

/// Base builder for the real `clw` binary with a CLEAN credential environment —
/// see the same helper in `e2e.rs`. The test process can inherit an ambient
/// identity (an exported `CLW_*`, or the C2c broker trio a `runs-on: corelink`
/// fabric box injects into env-0); stripping it here keeps every offline journey
/// deterministic regardless of what the host/CI environment carries.
fn clw_bin() -> Command {
    let mut cmd = Command::cargo_bin("clw").unwrap();
    for &v in clw_e2e_evidence::E2E_ENV_HYGIENE_VARS {
        cmd.env_remove(v);
    }
    cmd
}

fn base(cache: &std::path::Path) -> Command {
    let mut cmd = clw_bin();
    cmd.env("CLW_TENANT", "portable-tenant")
        .env("CLW_TOKEN", SECRET_TOKEN)
        .env("CLW_CACHE_DIR", cache.to_str().unwrap());
    cmd
}

#[test]
fn version_flag_prints_a_version() {
    let out = clw_bin().arg("--version").output().unwrap();
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

/// Completion scripts are raw shell source, so `--json` is an incompatible
/// output mode. The rejection must happen before config resolution, which is
/// why this deliberately supplies no identity at all.
#[test]
fn json_completions_is_rejected_early_without_stdout() {
    let out = clw_bin()
        .args(["--json", "completions", "bash"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "--json completions must use the standard CLI error exit"
    );
    assert!(
        out.stdout.is_empty(),
        "--json completions must not generate a partial completion script"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--json") && stderr.contains("incompatible"),
        "the early error must name the --json incompatibility, got: {stderr}"
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
