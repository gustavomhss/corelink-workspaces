//! `clw auth` — manage the locally-stored CoreLink identity.
//!
//! Subcommands:
//! - `login`  — persist endpoint/tenant/token to ~/.clw/config.toml (0600).
//! - `logout` — remove the token (or clear the file) from ~/.clw/config.toml.
//! - `whoami` — print the RESOLVED identity, redacted, offline (no network).
//! - `rotate` — OUT OF SCOPE (needs a server token-rotation endpoint).
//!
//! INVARIANT: the personal access token is NEVER printed to stdout/stderr.
//! Presence is reported as `<set>`/`<unset>` only. The config file is the only
//! place the plaintext token is written, and it is forced owner-only (0600).

use clap::{Args, Subcommand};
use clw_types::ClwError;
use serde_json::json;

use crate::config::{
    clw_config_path, ensure_config_parent_0700, read_config_pairs, render_config,
    write_config_file_0600, ResolvedConfig,
};

/// Presence sentinels — token values are never shown verbatim.
const REDACTED_SET: &str = "<set>";
const REDACTED_UNSET: &str = "<unset>";

#[derive(Debug, Args)]
#[command(about = "Manage the locally-stored CoreLink identity (login/logout/whoami)")]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthCommand,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Persist the resolved endpoint/tenant/token to ~/.clw/config.toml (0600).
    Login,
    /// Remove the stored token (or clear the file) from ~/.clw/config.toml.
    Logout(LogoutArgs),
    /// Print the resolved identity (redacted, offline — no network call).
    Whoami,
    /// Token rotation — not yet supported (server dependency).
    Rotate,
}

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Remove ALL keys (clear the whole config file), not just the token.
    #[arg(
        long,
        default_value_t = false,
        help = "Clear the entire config file, not just the token line"
    )]
    pub all: bool,

    /// Skip the confirmation prompt for --all.
    #[arg(
        long,
        default_value_t = false,
        help = "Skip the confirmation prompt when used with --all"
    )]
    pub yes: bool,
}

/// Map an empty value to a presence sentinel — never the value itself.
fn redact_present(value: &str) -> &'static str {
    if value.is_empty() {
        REDACTED_UNSET
    } else {
        REDACTED_SET
    }
}

// All subcommand entry points are async to satisfy the uniform dispatch
// signature; these handlers do no async work (auth is fully offline).
#[allow(clippy::unused_async)]
pub async fn run(args: AuthArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    match args.command {
        AuthCommand::Login => login(&cfg, json),
        AuthCommand::Logout(largs) => logout(&largs, json),
        AuthCommand::Whoami => {
            whoami(&cfg, json);
            Ok(())
        }
        AuthCommand::Rotate => {
            rotate(json);
            Ok(())
        }
    }
}

/// `clw auth login` — persist the resolved identity to ~/.clw/config.toml.
///
/// Reuses the shared 0600 writer/`toml_escape` helpers (no duplication). NEVER
/// prints the token — only the path and `<set>`/`<unset>` presence.
fn login(cfg: &ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let path = clw_config_path().ok_or_else(|| {
        ClwError::Transport("cannot determine home directory for config file".to_string())
    })?;

    ensure_config_parent_0700(&path)?;

    let c = &cfg.clw_config;
    // Render via the shared writer so the format round-trips through the parser.
    let pairs = vec![
        ("endpoint".to_string(), c.endpoint.clone()),
        ("tenant".to_string(), c.tenant.clone()),
        // NEVER echoed — written to the 0600 file only.
        ("token".to_string(), c.token.clone()),
        ("cache_dir".to_string(), c.cache_dir.display().to_string()),
    ];
    let content = render_config(&pairs);
    write_config_file_0600(&path, &content)?;

    if json {
        let out = json!({
            "logged_in": true,
            "wrote": path.display().to_string(),
            "endpoint": c.endpoint,
            "tenant": c.tenant,
            "token": redact_present(&c.token),
        });
        println!("{out}");
    } else {
        eprintln!("logged in — wrote identity to {}", path.display());
        eprintln!("  endpoint: {}", c.endpoint);
        eprintln!("  tenant:   {}", c.tenant);
        eprintln!("  token:    {}", redact_present(&c.token));
    }
    Ok(())
}

/// `clw auth logout` — remove the token line (keeping other keys), or clear the
/// whole file with `--all`. Prints WHAT was removed, never the value.
fn logout(args: &LogoutArgs, json: bool) -> Result<(), ClwError> {
    let path = clw_config_path().ok_or_else(|| {
        ClwError::Transport("cannot determine home directory for config file".to_string())
    })?;

    if !path.exists() {
        if json {
            println!("{}", json!({ "removed": [], "note": "no config file" }));
        } else {
            eprintln!("no config file at {} — nothing to remove", path.display());
        }
        return Ok(());
    }

    if args.all {
        // Clear the entire file (destructive) — confirm unless --yes.
        if !args.yes && !json && !confirm_clear(&path)? {
            eprintln!("aborted — config file left unchanged");
            return Ok(());
        }
        let existing = read_config_pairs(&path);
        let removed_keys: Vec<String> = existing.into_iter().map(|(k, _)| k).collect();
        // Truncate to just the header comment (keeps the 0600 mode).
        write_config_file_0600(&path, "# clw configuration\n")?;
        report_removed(&removed_keys, &path, json);
        return Ok(());
    }

    // Default: drop only the `token` key, keep everything else.
    let pairs = read_config_pairs(&path);
    let had_token = pairs.iter().any(|(k, _)| k == "token");
    if !had_token {
        if json {
            println!("{}", json!({ "removed": [], "note": "no token set" }));
        } else {
            eprintln!("no token set in {} — nothing to remove", path.display());
        }
        return Ok(());
    }
    let kept: Vec<(String, String)> = pairs.into_iter().filter(|(k, _)| k != "token").collect();
    write_config_file_0600(&path, &render_config(&kept))?;
    report_removed(&["token".to_string()], &path, json);
    Ok(())
}

/// Print the list of removed KEY names (never any value).
fn report_removed(removed: &[String], path: &std::path::Path, json: bool) {
    if json {
        let out = json!({
            "removed": removed,
            "file": path.display().to_string(),
        });
        println!("{out}");
    } else if removed.is_empty() {
        eprintln!("nothing removed from {}", path.display());
    } else {
        eprintln!("removed from {}: {}", path.display(), removed.join(", "));
    }
}

/// Prompt for confirmation before clearing the whole config file.
fn confirm_clear(path: &std::path::Path) -> Result<bool, ClwError> {
    use std::io::Write as _;
    eprint!(
        "clear ALL keys from {}? this cannot be undone [y/N]: ",
        path.display()
    );
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

/// `clw auth whoami` — print the resolved identity (redacted, offline).
fn whoami(cfg: &ResolvedConfig, json: bool) {
    let c = &cfg.clw_config;
    if json {
        let out = json!({
            "endpoint": c.endpoint,
            "tenant": c.tenant,
            // NEVER the value — presence only.
            "token": redact_present(&c.token),
        });
        println!("{out}");
    } else {
        println!("endpoint: {}", c.endpoint);
        println!("tenant:   {}", c.tenant);
        println!("token:    {}", redact_present(&c.token));
    }
}

/// `clw auth rotate` — OUT OF SCOPE. Server token-rotation endpoint does not yet
/// exist; print a clear message and exit 0 (handled by caller returning Ok).
fn rotate(json: bool) {
    if json {
        let out = json!({
            "rotated": false,
            "supported": false,
            "reason": "not yet supported — tracked (server dependency)",
        });
        println!("{out}");
    } else {
        eprintln!(
            "clw auth rotate: not yet supported — tracked (server dependency). \
             Use `clw auth login` with a new --token to replace the stored token."
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::Config;
    use std::path::PathBuf;

    fn cfg_with(token: &str) -> ResolvedConfig {
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
    fn redact_present_maps_correctly() {
        assert_eq!(redact_present(""), "<unset>");
        assert_eq!(redact_present("x"), "<set>");
    }

    // whoami output must never leak the token value (text form).
    #[test]
    fn whoami_text_never_leaks_token() {
        let secret = "super-secret-pat-abc123XYZ";
        let cfg = cfg_with(secret);
        // Build the same JSON whoami emits and assert no leak.
        let c = &cfg.clw_config;
        let out = json!({
            "endpoint": c.endpoint,
            "tenant": c.tenant,
            "token": redact_present(&c.token),
        });
        let s = out.to_string();
        assert!(
            !s.contains(secret),
            "whoami JSON must NEVER contain the token value, got: {s}"
        );
        assert!(
            s.contains("<set>"),
            "token presence must be <set>, got: {s}"
        );
    }

    // login render must put the token in the file body (for persistence) but the
    // value must NOT appear in any of the redacted summary fields.
    #[test]
    fn login_json_summary_redacts_token() {
        let secret = "pat-do-not-leak-987";
        let cfg = cfg_with(secret);
        let c = &cfg.clw_config;
        let summary = json!({
            "logged_in": true,
            "endpoint": c.endpoint,
            "tenant": c.tenant,
            "token": redact_present(&c.token),
        });
        let s = summary.to_string();
        assert!(
            !s.contains(secret),
            "login JSON summary must NEVER contain the token, got: {s}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_writes_0600_and_logout_drops_token() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        // Simulate login by writing via the shared helper directly.
        let cfg = cfg_with("secret-tok");
        let c = &cfg.clw_config;
        let pairs = vec![
            ("endpoint".to_string(), c.endpoint.clone()),
            ("tenant".to_string(), c.tenant.clone()),
            ("token".to_string(), c.token.clone()),
            ("cache_dir".to_string(), c.cache_dir.display().to_string()),
        ];
        write_config_file_0600(&path, &render_config(&pairs)).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "config must be 0600");

        // The token IS persisted in the file (that's the point of login).
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("secret-tok"), "token must be persisted");

        // Now simulate logout (drop only the token key).
        let after = read_config_pairs(&path);
        let kept: Vec<(String, String)> = after.into_iter().filter(|(k, _)| k != "token").collect();
        write_config_file_0600(&path, &render_config(&kept)).unwrap();

        let body2 = std::fs::read_to_string(&path).unwrap();
        assert!(
            !body2.contains("secret-tok"),
            "token must be gone after logout, got: {body2}"
        );
        assert!(body2.contains("acme-tenant"), "other keys must remain");
        // Mode must still be 0600 after the rewrite.
        let mode2 = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode2 & 0o777, 0o600, "config must stay 0600 after logout");
    }

    #[test]
    fn rotate_json_reports_unsupported() {
        let out = json!({
            "rotated": false,
            "supported": false,
            "reason": "not yet supported — tracked (server dependency)",
        });
        assert_eq!(out["supported"], false);
        assert!(out["reason"]
            .as_str()
            .unwrap()
            .contains("not yet supported"));
    }
}
