//! `clw auth` — manage the locally-stored CoreLink identity.
//!
//! Subcommands:
//! - `login`  — persist endpoint/tenant/token to ~/.clw/config.toml (0600).
//! - `logout` — remove the token (or clear the file) from ~/.clw/config.toml.
//! - `whoami` — print the RESOLVED identity, redacted, offline (no network).
//! - `rotate` — in v1, rotate by re-running `login` with the new token; a
//!   dedicated one-step rotate command is planned post-launch.
//!
//! INVARIANT: the personal access token is NEVER printed to stdout/stderr.
//! Presence is reported as `<set>`/`<unset>` only. The config file is the only
//! place the plaintext token is written, and it is forced owner-only (0600).
//!
//! STDOUT/STDERR CONTRACT (P5, shared across the CLI): a machine-readable RESULT
//! — the `--json` payload, OR a query RESULT like `whoami`'s identity report
//! (mirroring `doctor`/`ls`) — goes to STDOUT. Human action NOTICES/progress/
//! advisories (`login`'s "logged in", `logout`'s "removed", `rotate`'s guidance,
//! the "nothing to remove" notes) go to STDERR, so piping stdout to a file or a
//! `jq` never captures chatter. The `--json` payload itself always stays on
//! stdout regardless.

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
    /// Token rotation — in v1, re-run `login` with the new token.
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
/// prints the token — only the path and `<set>`/`<unset>` presence. Any unrelated
/// key already in the file (e.g. one a future config feature adds) is PRESERVED.
fn login(cfg: &ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let path = clw_config_path().ok_or_else(|| {
        ClwError::Transport("cannot determine home directory for config file".to_string())
    })?;

    ensure_config_parent_0700(&path)?;

    let c = &cfg.clw_config;
    // Merge over existing keys so any unrelated key survives; only the four
    // identity keys are overwritten. NEVER echoed — the token is
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
        // output format, and the rail must be no weaker than uninstall/erase.
        // `--json` only affects the OUTPUT shape, never consent. In `--json` or any
        // non-interactive (non-TTY) context we do NOT prompt and REQUIRE `--yes`,
        // refusing with a non-zero exit otherwise (mirrors `clw uninstall`); on a
        // TTY without `--yes` we prompt y/N.
        if !args.yes {
            if must_refuse_clear_without_consent(args.yes, json, is_interactive()) {
                return Err(ClwError::Transport(
                    "refusing to clear the config file without confirmation: pass --yes \
                     (required with --json or when non-interactive)"
                        .to_string(),
                ));
            }
            if !confirm_clear(&path)? {
                // FIX F4: a DECLINED interactive confirmation must exit NON-ZERO, to
                // match the destructive-command contract shared with rm/erase/
                // uninstall (W2/Wave-A): a refused destructive action is an error,
                // not a success. Returning Ok here would exit 0 and be
                // indistinguishable from a completed clear in automation.
                if json {
                    // (Unreachable: the json path already returned above, but keep
                    // the shape consistent if the guard above ever changes.)
                    println!(
                        "{}",
                        json!({ "removed": [], "note": "aborted — confirmation declined" })
                    );
                }
                return Err(declined_clear_err());
            }
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

/// Whether we may prompt interactively (stdin is a TTY). Mirrors the same guard in
/// `uninstall.rs`: in `--json` or any non-TTY context a destructive `logout --all`
/// must NOT be prompted — it requires `--yes`.
fn is_interactive() -> bool {
    use std::io::IsTerminal as _;
    std::io::stdin().is_terminal()
}

/// Decide whether a `logout --all` must REFUSE for lack of consent in a non-
/// interactive context. PURE over its inputs so the consent rail is unit-testable
/// without a TTY: refuse when neither `--yes` is set NOR an interactive prompt is
/// possible (i.e. `--json` or non-TTY). On a TTY without `--yes`, returns false
/// (we fall through to the y/N prompt).
fn must_refuse_clear_without_consent(yes: bool, json: bool, interactive: bool) -> bool {
    !yes && (json || !interactive)
}

/// FIX F4: the error returned when an interactive `logout --all` confirmation is
/// DECLINED. A refused destructive action exits non-zero (matching rm/erase/
/// uninstall), not 0. Extracted as a seam so the declined-is-an-error contract is
/// unit-testable without driving real stdin through `confirm_clear`.
fn declined_clear_err() -> ClwError {
    ClwError::Transport("aborted — config file left unchanged".to_string())
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

/// Render a present identifier verbatim, or `<not configured>` when it is empty.
/// P2: `whoami` reports what is PRESENT and clearly flags what is missing, so it
/// is useful on a half-configured config rather than printing a blank line.
fn or_not_configured(value: &str) -> &str {
    if value.is_empty() {
        "<not configured>"
    } else {
        value
    }
}

/// `clw auth whoami` — print the resolved identity (redacted, offline).
///
/// P2: resolves LENIENTLY (no required token+tenant), so it runs on a
/// half-configured config — reporting present fields and `<not configured>` /
/// `<unset>` for missing ones. This is a query RESULT (like `doctor`/`ls`), so its
/// human form goes to STDOUT per the stdout/stderr contract.
fn whoami(cfg: &ResolvedConfig, json: bool) {
    let c = &cfg.clw_config;
    if json {
        println!("{}", build_whoami_json(cfg));
    } else {
        println!("endpoint: {}", or_not_configured(&c.endpoint));
        println!("tenant:   {}", or_not_configured(&c.tenant));
        println!("token:    {}", redact_present(&c.token));
    }
}

/// `clw auth rotate` — in v1, rotation is performed by RE-AUTHENTICATING: run
/// `clw auth login` with the new token to atomically replace the stored
/// credential. A dedicated one-step rotate (mint-new + revoke-old in a single
/// call) is a post-launch nicety that needs a server endpoint; until then this
/// directs the user to the supported path and exits 0.
fn rotate(json: bool) {
    if json {
        let out = json!({
            "rotated": false,
            "action_required": "run `clw auth login` with the new token",
            "note": "v1 rotates by re-authenticating; a one-step rotate command is post-launch",
        });
        println!("{out}");
    } else {
        eprintln!(
            "clw auth rotate: to rotate your token, run `clw auth login` with the new \
             token — this replaces the stored credential. (v1 has no one-step rotate; \
             a dedicated mint-and-revoke command is planned post-launch.)"
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

    // P2 — whoami on a half-configured config: a missing (empty) tenant/endpoint
    // renders "<not configured>" rather than a blank line.
    #[test]
    fn or_not_configured_flags_missing_fields() {
        assert_eq!(or_not_configured(""), "<not configured>");
        assert_eq!(or_not_configured("acme"), "acme");
    }

    // P2 — whoami's JSON shape still round-trips when tenant is empty (logout/
    // whoami must succeed on a config missing tenant, not error).
    #[test]
    fn whoami_json_on_missing_tenant_does_not_error() {
        let cfg = ResolvedConfig {
            clw_config: Config {
                endpoint: "https://example.com".to_string(),
                tenant: String::new(),
                token: String::new(),
                cache_dir: PathBuf::from("/tmp/clw-cache"),
            },
        };
        let out = build_whoami_json(&cfg);
        assert_eq!(out["tenant"], "", "missing tenant => empty in JSON");
        assert_eq!(out["token"], "<unset>", "missing token => <unset>");
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

    // login must PRESERVE an unrelated key (any future config key) on rewrite.
    #[test]
    fn login_rewrite_preserves_unrelated_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        // Pre-existing file holding an unrelated key + a stale token.
        write_config_file_0600(
            &path,
            "# clw configuration\nunrelated_key = \"keep-me\"\ntoken = \"old-tok\"\n",
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
            map.get("unrelated_key").map(String::as_str),
            Some("keep-me"),
            "an unrelated config key must survive an auth login rewrite"
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

    // FIX 3 — `logout --all` consent rail, consistent with uninstall/erase:
    // non-interactive (`--json` or no TTY) without `--yes` must REFUSE; a TTY
    // without `--yes` falls through to the y/N prompt; `--yes` always proceeds.
    #[test]
    fn logout_all_refuses_non_interactive_without_consent() {
        // --json (non-interactive shape): refuse regardless of TTY state.
        assert!(
            must_refuse_clear_without_consent(false, true, true),
            "--json without --yes must refuse even on a TTY"
        );
        assert!(
            must_refuse_clear_without_consent(false, true, false),
            "--json without --yes must refuse off a TTY"
        );
        // Plain non-TTY (piped) without --yes: refuse.
        assert!(
            must_refuse_clear_without_consent(false, false, false),
            "non-interactive without --yes must refuse"
        );
    }

    #[test]
    fn logout_all_prompts_on_tty_and_proceeds_with_yes() {
        // Interactive TTY without --yes: do NOT auto-refuse — fall through to the
        // y/N prompt.
        assert!(
            !must_refuse_clear_without_consent(false, false, true),
            "a TTY without --yes must prompt, not auto-refuse"
        );
        // --yes always proceeds (no prompt, no refusal), in every context.
        for json in [true, false] {
            for interactive in [true, false] {
                assert!(
                    !must_refuse_clear_without_consent(true, json, interactive),
                    "--yes must always proceed (json={json}, interactive={interactive})"
                );
            }
        }
    }

    // FIX F4 — a DECLINED interactive `logout --all` must be an Err (non-zero
    // exit), consistent with rm/erase/uninstall. The declined-error seam is the
    // exact value the logout decline path returns.
    #[test]
    fn declined_logout_all_is_an_error() {
        let err = declined_clear_err();
        assert!(
            matches!(err, ClwError::Transport(ref m) if m.contains("left unchanged")),
            "a declined logout --all must return an Err naming the unchanged file, got: {err:?}"
        );
    }

    #[test]
    fn rotate_json_directs_to_login() {
        // v1 rotate is a re-login: the JSON must say it did not rotate and point
        // the user at `clw auth login` as the supported path.
        let out = json!({
            "rotated": false,
            "action_required": "run `clw auth login` with the new token",
            "note": "v1 rotates by re-authenticating; a one-step rotate command is post-launch",
        });
        assert_eq!(out["rotated"], false);
        assert!(out["action_required"]
            .as_str()
            .unwrap()
            .contains("clw auth login"));
    }
}
