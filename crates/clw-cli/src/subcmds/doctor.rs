//! `clw doctor` — print a support-ready, REDACTED diagnostics bundle.
//!
//! Emits environment, config, and cache information useful when filing a bug
//! report. The personal access token is NEVER printed — only its presence is
//! reported as `<set>`/`<unset>`. Always exits 0.

use clap::Args;
use clw_types::ClwError;

use crate::config::{clw_config_path, ResolvedConfig};

#[derive(Debug, Args)]
#[command(about = "Print a support-ready, redacted diagnostics bundle (never prints the token)")]
pub struct DoctorArgs {}

/// Sentinel shown in place of any secret value. Used so tests can assert the
/// token never appears verbatim in the output.
const REDACTED_SET: &str = "<set>";
const REDACTED_UNSET: &str = "<unset>";

pub async fn run(_args: DoctorArgs, cfg: ResolvedConfig) -> Result<(), ClwError> {
    let report = build_report(&cfg);
    print!("{report}");
    Ok(())
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

    // --- reporting note ----------------------------------------------------
    out.push_str("-----------------------------------\n");
    out.push_str(
        "note: this bundle is redacted; your token is never shown. To report a bug, see \
         SECURITY.md for responsible-disclosure of security issues, or open an issue and \
         paste this output.\n",
    );

    out
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
}
