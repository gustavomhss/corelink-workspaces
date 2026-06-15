//! `clw rm --name <NAME>` — delete a workspace ref.
//!
//! OPTIMISTIC against a frozen contract: the corelink-server `DELETE /v1/ac`
//! capability is pending ratification (docs/REQUESTS-to-other-TLs.md D-1). Until
//! it deploys, the real transport surfaces the 405/501 not-deployed case as an
//! actionable error rather than a silent success.

use clap::Args;
use clw_client::HttpClient;
use clw_types::{ref_key, AcTransport, ClwError};
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(about = "Delete a workspace ref by name (idempotent)")]
pub struct RmArgs {
    /// Workspace name (ref key).
    #[arg(long, help = "Workspace name to delete")]
    pub name: String,

    /// Skip the interactive confirmation prompt.
    #[arg(
        long,
        default_value_t = false,
        help = "Skip the y/N confirmation prompt (required for --json / non-interactive use)"
    )]
    pub yes: bool,
}

pub async fn run(args: RmArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    // Consent is required regardless of output format. `--json` only shapes the
    // OUTPUT, never bypasses consent: a delete needs `--yes` or an interactive
    // y/N confirm. In non-interactive contexts (no TTY, or `--json`) we require
    // `--yes` and never silently delete.
    if !args.yes && !confirm_rm(&args.name)? {
        if json {
            println!(
                "{}",
                json!({
                    "name": args.name,
                    "deleted": false,
                    "note": "aborted — confirmation declined (use --yes for non-interactive delete)"
                })
            );
        } else {
            eprintln!("aborted — ref '{}' left unchanged", args.name);
        }
        return Ok(());
    }

    let client = HttpClient::new(cfg.clw_config)?;
    let rk = ref_key(&args.name);

    match AcTransport::delete(&client, &rk).await {
        Ok(()) => {
            if json {
                println!("{}", json!({ "name": args.name, "deleted": true }));
            } else {
                println!("deleted ref '{}'", args.name);
            }
            Ok(())
        }
        // Optimistic-pending: the server lacks the DELETE handler. ALWAYS fails
        // (non-zero exit) in BOTH modes — a destructive command that deleted
        // nothing must never report success. In `--json` the structured note is
        // printed first, then the same error propagates (mirrors ls/prune/erase).
        Err(e) if is_not_deployed(&e) => not_deployed(json),
        Err(e) => Err(map_auth_error(e)),
    }
}

/// Render the optimistic-pending case. ALWAYS returns `Err` so the process exits
/// non-zero regardless of `--json`; in JSON mode the structured note is emitted
/// to stdout first. Identical exit-code contract to `ls`/`prune`/`erase` — the
/// brutal-audit (2026-06-15) caught `rm` as the lone outlier that PR #34 missed.
fn not_deployed(json: bool) -> Result<(), ClwError> {
    let msg = "ref deletion requires a corelink-server capability not yet deployed — tracked (DELETE /v1/ac; see docs/REQUESTS-to-other-TLs.md D-1)".to_string();
    if json {
        println!(
            "{}",
            json!({ "deleted": false, "error": "not-deployed", "note": msg })
        );
    }
    Err(ClwError::Transport(msg))
}

/// True if the error is the optimistic-pending "server delete not deployed" case
/// (the client maps 405/501 to a `Transport` error whose message names the
/// pending capability).
fn is_not_deployed(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("not yet deployed"))
}

/// Rewrite a raw `Auth` error into an actionable transport message (mirrors
/// `ls`/`status`); pass everything else through.
fn map_auth_error(err: ClwError) -> ClwError {
    if matches!(err, ClwError::Auth) {
        ClwError::Transport(
            "authentication failed — check CLW_TOKEN or ~/.clw/config.toml".to_string(),
        )
    } else {
        err
    }
}

/// Prompt for confirmation before deleting a ref. Returns `false` immediately if
/// stdin is not a TTY (non-interactive), so `--json`/pipelines never delete
/// without an explicit `--yes`.
fn confirm_rm(name: &str) -> Result<bool, ClwError> {
    use std::io::{IsTerminal as _, Write as _};
    if !std::io::stdin().is_terminal() {
        // Non-interactive without --yes: refuse to delete.
        return Ok(false);
    }
    eprint!("delete ref '{name}'? this cannot be undone [y/N]: ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_deployed_detects_pending_capability_message() {
        let err = ClwError::Transport(
            "ref deletion requires a corelink-server capability not yet deployed (DELETE /v1/ac)"
                .to_string(),
        );
        assert!(is_not_deployed(&err));
    }

    #[test]
    fn not_deployed_ignores_other_transport_errors() {
        let err = ClwError::Transport("some unrelated transport issue".to_string());
        assert!(!is_not_deployed(&err));
        assert!(!is_not_deployed(&ClwError::Auth));
    }

    #[test]
    fn not_deployed_fails_in_both_modes() {
        // A destructive delete that did nothing must NEVER report success — both
        // JSON and human mode return an error (non-zero exit). Regression for the
        // brutal-audit CRIT/HIGH: rm was the sibling PR #34 missed.
        assert!(not_deployed(true).is_err());
        assert!(not_deployed(false).is_err());
    }

    #[test]
    fn map_auth_error_rewrites_auth_variant() {
        let mapped = map_auth_error(ClwError::Auth);
        let msg = mapped.to_string();
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "auth error must mention CLW_TOKEN or config.toml, got: {msg}"
        );
    }

    #[test]
    fn map_auth_error_passes_through_other_errors() {
        let mapped = map_auth_error(ClwError::Transport("x".to_string()));
        assert!(matches!(mapped, ClwError::Transport(_)));
    }
}
