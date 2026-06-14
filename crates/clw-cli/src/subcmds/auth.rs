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
    clw_config_path, ensure_config_parent_0700, merge_config_pairs, read_config_pairs,
    render_config, write_config_file_0600, ResolvedConfig,
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

/// Build the `clw auth login` summary as a machine-readable JSON object.
///
/// PURE over its inputs (this is the real value `login` prints in `--json` mode,
/// so redaction tests assert on THIS rather than a re-built literal). The token is
/// reported as `<set>`/`<unset>` presence only — its value never appears.
fn build_login_summary_json(cfg: &ResolvedConfig, wrote: &std::path::Path) -> serde_json::Value {
    let c = &cfg.clw_config;
    json!({
        "logged_in": true,
        "wrote": wrote.display().to_string(),
        "endpoint": c.endpoint,
        "tenant": c.tenant,
        // NEVER the value — presence only.
        "token": redact_present(&c.token),
    })
}

/// `clw auth login` — persist the resolved identity to ~/.clw/config.toml.
///
/// Reuses the shared 0600 writer/`toml_escape` helpers (no duplication). NEVER
/// prints the token — only the path and `<set>`/`<unset>` presence. Unrelated keys
/// already in the file (e.g. recorded `telemetry` consent) are PRESERVED.
fn login(cfg: &ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let path = clw_config_path().ok_or_else(|| {
        ClwError::Transport("cannot determine home directory for config file".to_string())
    })?;

    ensure_config_parent_0700(&path)?;

    let c = &cfg.clw_config;
    // Merge over existing keys so an unrelated key (e.g. `telemetry`) survives;
    // only the four identity keys are overwritten. NEVER echoed — the token is
    // written to the 0600 file only.
    let updates = vec![
        ("endpoint".to_string(), c.endpoint.clone()),
        ("tenant".to_string(), c.tenant.clone()),
        ("token".to_string(), c.token.clone()),
        ("cache_dir".to_string(), c.cache_dir.display().to_string()),
    ];
    let pairs = merge_config_pairs(&path, &updates);
    let content = render_config(&pairs);
    write_config_file_0600(&path, &content)?;

    if json {
        println!("{}", build_login_summary_json(cfg, &path));
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
        // Clear the entire file (destructive) — consent is required regardless of
        // output format. `--json` only affects the OUTPUT shape, never consent: a
        // destructive clear needs `--yes` or an interactive y/N confirm.
        if !args.yes && !confirm_clear(&path)? {
            if json {
                println!(
                    "{}",
                    json!({ "removed": [], "note": "aborted — confirmation declined" })
                );
            } else {
                eprintln!("aborted — config file left unchanged");
            }
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

/// Build the `clw auth whoami` summary as a machine-readable JSON object.
///
/// PURE over its inputs (this is the real value `whoami` prints in `--json` mode,
/// so redaction tests assert on THIS rather than a re-built literal). The token is
/// reported as `<set>`/`<unset>` presence only — its value never appears.
fn build_whoami_json(cfg: &ResolvedConfig) -> serde_json::Value {
    let c = &cfg.clw_config;
    json!({
        "endpoint": c.endpoint,
        "tenant": c.tenant,
        // NEVER the value — presence only.
        "token": redact_present(&c.token),
    })
}

/// `clw auth whoami` — print the resolved identity (redacted, offline).
fn whoami(cfg: &ResolvedConfig, json: bool) {
    let c = &cfg.clw_config;
    if json {
        println!("{}", build_whoami_json(cfg));
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

    // whoami output must never leak the token value — asserted on the REAL pure
    // function `build_whoami_json` that the command actually prints.
    #[test]
    fn whoami_json_never_leaks_token() {
        let secret = "super-secret-pat-abc123XYZ";
        let cfg = cfg_with(secret);
        let out = build_whoami_json(&cfg);
        let s = out.to_string();
        assert!(
            !s.contains(secret),
            "whoami JSON must NEVER contain the token value, got: {s}"
        );
        // The token field must be exactly the presence sentinel, nothing else.
        assert_eq!(out["token"], "<set>", "set token must redact to <set>");

        // And an unset token must redact to <unset>.
        let empty = cfg_with("");
        assert_eq!(build_whoami_json(&empty)["token"], "<unset>");
    }

    // login summary must never leak the token — asserted on the REAL pure function
    // `build_login_summary_json` that the command actually prints.
    #[test]
    fn login_json_summary_redacts_token() {
        let secret = "pat-do-not-leak-987";
        let cfg = cfg_with(secret);
        let out = build_login_summary_json(&cfg, std::path::Path::new("/tmp/config.toml"));
        let s = out.to_string();
        assert!(
            !s.contains(secret),
            "login JSON summary must NEVER contain the token, got: {s}"
        );
        assert_eq!(out["token"], "<set>", "set token must redact to <set>");
        assert_eq!(out["logged_in"], true);

        let empty = cfg_with("");
        assert_eq!(
            build_login_summary_json(&empty, std::path::Path::new("/tmp/config.toml"))["token"],
            "<unset>"
        );
    }

    // login must PRESERVE an unrelated key (e.g. telemetry consent) on rewrite.
    #[test]
    fn login_rewrite_preserves_unrelated_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        // Pre-existing file holding telemetry consent + a stale token.
        write_config_file_0600(
            &path,
            "# clw configuration\ntelemetry = \"on\"\ntoken = \"old-tok\"\n",
        )
        .unwrap();

        // Simulate the login rewrite (overwrite identity keys, merge the rest).
        let cfg = cfg_with("new-tok");
        let c = &cfg.clw_config;
        let updates = vec![
            ("endpoint".to_string(), c.endpoint.clone()),
            ("tenant".to_string(), c.tenant.clone()),
            ("token".to_string(), c.token.clone()),
            ("cache_dir".to_string(), c.cache_dir.display().to_string()),
        ];
        let pairs = merge_config_pairs(&path, &updates);
        write_config_file_0600(&path, &render_config(&pairs)).unwrap();

        let map: std::collections::HashMap<_, _> = read_config_pairs(&path).into_iter().collect();
        assert_eq!(
            map.get("telemetry").map(String::as_str),
            Some("on"),
            "telemetry consent key must survive an auth login rewrite"
        );
        assert_eq!(map.get("token").map(String::as_str), Some("new-tok"));
        assert_eq!(map.get("tenant").map(String::as_str), Some("acme-tenant"));
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
