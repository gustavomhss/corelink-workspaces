//! `clw doctor` — print a support-ready, REDACTED diagnostics bundle.
//!
//! Emits environment, config, and cache information useful when filing a bug
//! report. The personal access token is NEVER printed — only its presence is
//! reported as `<set>`/`<unset>`. Always exits 0.

use clap::Args;
use clw_types::{validate_tenant, ClwError};
use serde_json::json;

use crate::config::{clw_config_path, ResolvedConfig};

#[derive(Debug, Args)]
#[command(about = "Print a support-ready, redacted diagnostics bundle (never prints the token)")]
pub struct DoctorArgs {}

/// Sentinel shown in place of any secret value. Used so tests can assert the
/// token never appears verbatim in the output.
const REDACTED_SET: &str = "<set>";
const REDACTED_UNSET: &str = "<unset>";

pub async fn run(_args: DoctorArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    if json {
        print!("{}", build_report_json(&cfg));
    } else {
        let report = build_report(&cfg);
        print!("{report}");
    }
    Ok(())
}

/// Build the diagnostics report as a machine-readable JSON object.
///
/// Same redaction invariant as [`build_report`]: the token is reported as
/// `<set>`/`<unset>` presence only, NEVER its value.
fn build_report_json(cfg: &ResolvedConfig) -> String {
    let c = &cfg.clw_config;
    let (config_file, config_exists, config_mode) = match clw_config_path() {
        Some(path) => {
            let exists = path.exists();
            let mode = if exists { Some(file_mode(&path)) } else { None };
            (Some(path.display().to_string()), exists, mode)
        }
        None => (None, false, None),
    };
    let out = json!({
        "clw_version": env!("CARGO_PKG_VERSION"),
        "os": OS,
        "arch": ARCH,
        "config_file": config_file,
        "config_exists": config_exists,
        "config_mode": config_mode,
        "endpoint": c.endpoint,
        "tenant": redact_present(&c.tenant),
        "tenant_value": if c.tenant.is_empty() { None } else { Some(c.tenant.clone()) },
        // NEVER print the token value — presence only.
        "token": redact_present(&c.token),
        "cache_dir": c.cache_dir.display().to_string(),
        "cache_exists": c.cache_dir.exists(),
        // P1: missing/invalid tenant/token/endpoint reported as diagnostics,
        // never a hard error (doctor still exits 0).
        "problems": config_problems(cfg),
    });
    format!("{out}\n")
}

/// Build the full diagnostics report as a string.
///
/// Kept pure (no I/O beyond filesystem stat) so it can be unit-tested for
/// redaction.
fn build_report(cfg: &ResolvedConfig) -> String {
    let c = &cfg.clw_config;
    let mut out = String::new();

    out.push_str("clw doctor — diagnostics (redacted)\n");
    out.push_str("===================================\n");

    // --- version / platform ------------------------------------------------
    out.push_str(&format!("clw version:    {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("os/arch:        {}/{}\n", OS, ARCH));

    // --- config file -------------------------------------------------------
    match clw_config_path() {
        Some(path) => {
            let exists = path.exists();
            out.push_str(&format!("config file:    {}\n", path.display()));
            out.push_str(&format!("config exists:  {exists}\n"));
            if exists {
                out.push_str(&format!("config mode:    {}\n", file_mode(&path)));
            }
        }
        None => {
            out.push_str("config file:    <unknown> (no HOME/USERPROFILE)\n");
        }
    }

    // --- resolved config (redacted) ----------------------------------------
    out.push_str(&format!("endpoint:       {}\n", c.endpoint));
    out.push_str(&format!("tenant:         {}\n", redact_present(&c.tenant)));
    if !c.tenant.is_empty() {
        out.push_str(&format!("tenant value:   {}\n", c.tenant));
    }
    // NEVER print the token — presence only.
    out.push_str(&format!("token:          {}\n", redact_present(&c.token)));

    // --- cache dir ---------------------------------------------------------
    out.push_str(&format!("cache dir:      {}\n", c.cache_dir.display()));
    out.push_str(&format!("cache exists:   {}\n", c.cache_dir.exists()));

    // --- problems (P1) -----------------------------------------------------
    // Report a missing/invalid tenant/token/endpoint as advisories instead of
    // failing — `doctor` is the command you run to DIAGNOSE a broken config.
    let problems = config_problems(cfg);
    if problems.is_empty() {
        out.push_str("problems:       none\n");
    } else {
        out.push_str(&format!("problems:       {}\n", problems.len()));
        for p in &problems {
            out.push_str(&format!("  - {p}\n"));
        }
    }

    // --- reporting note ----------------------------------------------------
    out.push_str("-----------------------------------\n");
    out.push_str(
        "note: this bundle is redacted; your token is never shown. To report a bug, see \
         SECURITY.md for responsible-disclosure of security issues, or open an issue and \
         paste this output.\n",
    );

    out
}

/// Collect config PROBLEMS as human-readable diagnostic lines (P1).
///
/// `doctor` runs on a LENIENTLY-resolved config (missing tenant/token => empty),
/// so the exact broken state a user invokes `doctor` to diagnose is reported here
/// as advisories rather than a hard error — `doctor` still exits 0. Pure over its
/// inputs so the diagnosis is unit-testable. Returns an empty vec when nothing is
/// wrong (a fully-configured, valid config).
fn config_problems(cfg: &ResolvedConfig) -> Vec<String> {
    let c = &cfg.clw_config;
    let mut problems = Vec::new();
    if c.tenant.is_empty() {
        problems.push(
            "tenant is not set — set --tenant, CLW_TENANT, or add tenant to ~/.clw/config.toml"
                .to_string(),
        );
    } else if let Err(e) = validate_tenant(&c.tenant) {
        problems.push(format!("tenant is invalid: {e}"));
    }
    if c.token.is_empty() {
        problems.push(
            "token is not set — set --token, CLW_TOKEN, or add token to ~/.clw/config.toml"
                .to_string(),
        );
    }
    if c.endpoint.is_empty() {
        problems.push("endpoint is empty — set --endpoint or CLW_ENDPOINT".to_string());
    }
    problems
}

/// Map a possibly-empty secret/identifier to a presence sentinel.
fn redact_present(value: &str) -> &'static str {
    if value.is_empty() {
        REDACTED_UNSET
    } else {
        REDACTED_SET
    }
}

// `std::env::consts` indirection so the strings are easy to reference in one
// place.
const OS: &str = std::env::consts::OS;
const ARCH: &str = std::env::consts::ARCH;

/// Return the file's permission mode.
///
/// On unix, an octal `0oNNN` string (e.g. `0o600`). On non-unix platforms the
/// concept does not apply, so we report `n/a`.
#[cfg(unix)]
fn file_mode(path: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt as _;
    match std::fs::metadata(path) {
        Ok(meta) => format!("0o{:o}", meta.permissions().mode() & 0o777),
        Err(_) => "<unreadable>".to_string(),
    }
}

#[cfg(not(unix))]
fn file_mode(_path: &std::path::Path) -> String {
    "n/a (non-unix)".to_string()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::Config;
    use std::path::PathBuf;

    fn cfg_with_token(token: &str) -> ResolvedConfig {
        ResolvedConfig {
            clw_config: Config {
                endpoint: "https://example.com".to_string(),
                tenant: "acme-tenant".to_string(),
                token: token.to_string(),
                cache_dir: PathBuf::from("/tmp/clw-cache"),
            },
        }
    }

    #[test]
    fn report_never_leaks_token() {
        let secret = "super-secret-pat-abc123XYZ";
        let cfg = cfg_with_token(secret);
        let report = build_report(&cfg);
        assert!(
            !report.contains(secret),
            "token value must NEVER appear in doctor output, got:\n{report}"
        );
        // Presence is reported as the sentinel instead.
        assert!(
            report.contains("token:          <set>"),
            "token presence must be shown as <set>, got:\n{report}"
        );
    }

    #[test]
    fn report_shows_unset_for_empty_token() {
        let cfg = cfg_with_token("");
        let report = build_report(&cfg);
        assert!(
            report.contains("token:          <unset>"),
            "empty token must show <unset>, got:\n{report}"
        );
    }

    #[test]
    fn report_includes_version_and_platform() {
        let cfg = cfg_with_token("tok");
        let report = build_report(&cfg);
        assert!(
            report.contains(env!("CARGO_PKG_VERSION")),
            "report must include clw version"
        );
        assert!(
            report.contains(std::env::consts::OS),
            "report must include OS"
        );
        assert!(
            report.contains(std::env::consts::ARCH),
            "report must include arch"
        );
    }

    #[test]
    fn report_shows_endpoint_tenant_and_cache() {
        let cfg = cfg_with_token("tok");
        let report = build_report(&cfg);
        assert!(report.contains("https://example.com"), "must show endpoint");
        assert!(report.contains("acme-tenant"), "must show tenant value");
        assert!(report.contains("/tmp/clw-cache"), "must show cache dir");
    }

    #[test]
    fn report_points_to_security_md() {
        let cfg = cfg_with_token("tok");
        let report = build_report(&cfg);
        assert!(
            report.contains("SECURITY.md"),
            "report must point to SECURITY.md for bug/security reporting"
        );
    }

    #[test]
    fn redact_present_maps_correctly() {
        assert_eq!(redact_present(""), "<unset>");
        assert_eq!(redact_present("x"), "<set>");
    }

    #[test]
    fn json_report_never_leaks_token() {
        let secret = "super-secret-pat-abc123XYZ";
        let cfg = cfg_with_token(secret);
        let report = build_report_json(&cfg);
        assert!(
            !report.contains(secret),
            "token value must NEVER appear in doctor --json output, got:\n{report}"
        );
        // Must be valid JSON with the redacted presence sentinel.
        let v: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(v["token"], "<set>");
        assert_eq!(v["endpoint"], "https://example.com");
    }

    #[test]
    fn json_report_shows_unset_for_empty_token() {
        let cfg = cfg_with_token("");
        let report = build_report_json(&cfg);
        let v: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(v["token"], "<unset>");
    }

    // -----------------------------------------------------------------------
    // P1 — doctor runs on a broken config: missing tenant+token are reported
    // as PROBLEMS (diagnostic lines), never a hard error.
    // -----------------------------------------------------------------------

    /// A leniently-resolved, half-configured config: empty tenant + empty token.
    fn cfg_broken() -> ResolvedConfig {
        ResolvedConfig {
            clw_config: Config {
                endpoint: "https://example.com".to_string(),
                tenant: String::new(),
                token: String::new(),
                cache_dir: PathBuf::from("/tmp/clw-cache"),
            },
        }
    }

    #[test]
    fn config_problems_reports_missing_tenant_and_token() {
        let problems = config_problems(&cfg_broken());
        assert_eq!(problems.len(), 2, "missing tenant + token = 2 problems");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("tenant") && p.contains("not set")),
            "must flag the missing tenant, got: {problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("token") && p.contains("not set")),
            "must flag the missing token, got: {problems:?}"
        );
    }

    #[test]
    fn config_problems_flags_invalid_tenant() {
        let cfg = ResolvedConfig {
            clw_config: Config {
                endpoint: "https://example.com".to_string(),
                // A path-injection tenant: present but invalid.
                tenant: "../other".to_string(),
                token: "tok".to_string(),
                cache_dir: PathBuf::from("/tmp/clw-cache"),
            },
        };
        let problems = config_problems(&cfg);
        assert!(
            problems.iter().any(|p| p.contains("tenant is invalid")),
            "an invalid present tenant must be a reported problem, got: {problems:?}"
        );
    }

    #[test]
    fn config_problems_empty_for_valid_config() {
        assert!(
            config_problems(&cfg_with_token("tok")).is_empty(),
            "a fully-configured valid config has no problems"
        );
    }

    #[test]
    fn report_on_broken_config_lists_problems_and_does_not_error() {
        // build_report is the exact value `run` prints; on a broken config it must
        // render the problems section (so doctor can still exit 0 and diagnose).
        let report = build_report(&cfg_broken());
        assert!(report.contains("problems:"), "must have a problems section");
        assert!(report.contains("tenant"), "must mention the missing tenant");
        assert!(report.contains("token"), "must mention the missing token");
    }

    #[test]
    fn json_report_on_broken_config_carries_problems_array() {
        let report = build_report_json(&cfg_broken());
        let v: serde_json::Value = serde_json::from_str(&report).unwrap();
        let problems = v["problems"].as_array().expect("problems must be an array");
        assert_eq!(problems.len(), 2, "missing tenant + token");
        assert_eq!(v["tenant"], "<unset>");
        assert_eq!(v["token"], "<unset>");
    }
}
