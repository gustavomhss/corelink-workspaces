//! `clw rm --name <NAME>` — delete a workspace ref (or `--ref-key <KEY>` to delete a
//! dangling/corrupt ref by its raw key, with no usable name).
//!
//! The corelink-server `DELETE /v1/ac` capability is LIVE (D-1, conformance-
//! green). The 405/501 branch is a defensive guard: a LIVE capability returning
//! 405/501 is a server-side regression, so the transport surfaces it as an
//! actionable server-regression error rather than a silent success.

use clap::Args;
use clw_client::HttpClient;
use clw_types::{ref_key_in, AcTransport, ClwError, Digest, RefDomain};
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(about = "Delete a workspace ref — by name, or by raw key (--ref-key) for a dangling ref")]
pub struct RmArgs {
    /// Workspace name (ref key). Mutually exclusive with --ref-key.
    #[arg(long, conflicts_with = "ref_key", help = "Workspace name to delete")]
    pub name: Option<String>,

    /// Delete a ref by its raw 64-hex ref KEY directly — for removing a DANGLING or
    /// CORRUPT ref that `ls --all` / `prune` surfaced (they print the `ref_key`) but
    /// that has no parseable name. Mutually exclusive with --name.
    #[arg(
        long,
        value_name = "KEY",
        help = "Raw 64-hex ref key to delete (for a dangling/corrupt ref with no usable name)"
    )]
    pub ref_key: Option<String>,

    /// Skip the interactive confirmation prompt.
    #[arg(
        long,
        default_value_t = false,
        help = "Skip the y/N confirmation prompt (required for --json / non-interactive use)"
    )]
    pub yes: bool,
}

/// Resolve the (ref key, human label) to delete from `--name` XOR `--ref-key`.
/// Extracted so the name-vs-key contract is unit-testable without a live server.
fn resolve_rm_target(
    name: Option<&str>,
    ref_key: Option<&str>,
    domain: RefDomain,
) -> Result<(Digest, String), ClwError> {
    match (name, ref_key) {
        (Some(n), None) => Ok((ref_key_in(domain, n), format!("ref '{n}'"))),
        (None, Some(k)) => {
            let digest = Digest::from_hex(k).map_err(|_| {
                ClwError::Message(format!(
                    "--ref-key must be a 64-char lowercase hex ref key, got '{k}'"
                ))
            })?;
            Ok((digest, format!("ref-key {k}")))
        }
        (None, None) => Err(ClwError::Message(
            "specify the ref to delete: --name <NAME> or --ref-key <KEY>".to_string(),
        )),
        // clap's conflicts_with already rejects --name + --ref-key together.
        (Some(_), Some(_)) => unreachable!("clap conflicts_with rejects --name + --ref-key"),
    }
}

pub async fn run(args: RmArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    // Consent is required regardless of output format. `--json` only shapes the
    // OUTPUT, never bypasses consent: a delete needs `--yes` or an interactive
    // y/N confirm. In non-interactive contexts (no TTY, or `--json`) we require
    // `--yes` and never silently delete.
    //
    // EXIT-CODE CONTRACT (aligned with erase/uninstall): a DECLINED/aborted
    // confirmation exits NON-ZERO — automation that branches on exit code must be
    // able to tell "the delete did NOT happen" from a completed delete (they would
    // otherwise share exit 0). So in `--json` the structured `deleted:false` note
    // is printed FIRST, then an `Err` propagates carrying "left unchanged".
    let (rk, label) = resolve_rm_target(
        args.name.as_deref(),
        args.ref_key.as_deref(),
        cfg.ref_domain,
    )?;

    if !args.yes && !confirm_rm(&label)? {
        return declined(&label, json);
    }

    let client = HttpClient::new(cfg.clw_config)?;

    // FIX 3: the server DELETE is idempotent (a delete of an absent ref still
    // 204s), so a blind delete printed "deleted" for a ref that NEVER existed —
    // dishonest. GET the ref FIRST to learn whether it actually existed, then
    // report truthfully (`deleted:true` vs `deleted:false`). Still idempotent: rm
    // of an absent ref is SUCCESS (exit 0), it just says "nothing to delete".
    let existed = AcTransport::get(&client, &rk)
        .await
        .map_err(map_auth_error)?
        .is_some();

    match AcTransport::delete(&client, &rk).await {
        Ok(()) => {
            if json {
                // Preserve the `{name:…}` shape for the name case (contract); the
                // by-key case reports `{ref_key:…}`. `deleted` reflects whether the
                // ref actually existed (FIX 3).
                let out = match (&args.name, &args.ref_key) {
                    (Some(n), _) => json!({ "name": n, "deleted": existed }),
                    (None, Some(k)) => json!({ "ref_key": k, "deleted": existed }),
                    _ => json!({ "deleted": existed }),
                };
                println!("{out}");
            } else if existed {
                println!("deleted {label}");
            } else {
                println!("no such {label} (nothing to delete)");
            }
            Ok(())
        }
        // Defensive: DELETE /v1/ac is LIVE (D-1) — a 405/501 means a server-side
        // regression. ALWAYS fails (non-zero exit) in BOTH modes — a destructive
        // command that deleted nothing must never report success. In `--json` the
        // structured note is printed first, then the same error propagates
        // (mirrors ls/prune/erase).
        Err(e) if is_capability_unavailable(&e) => capability_unavailable(json),
        Err(e) => Err(map_auth_error(e)),
    }
}

/// Emit the declined-confirmation outcome and FAIL with a non-zero exit. Aligns
/// `rm` with `erase`/`uninstall`: a DECLINED destructive confirmation must exit
/// non-zero so automation can tell "the delete did NOT happen" from a completed
/// delete (they would otherwise share exit 0). In `--json` the structured
/// `deleted:false` note is printed FIRST, then the `Err` propagates carrying
/// "left unchanged". (Was Ok(()) / exit 0 before the 2026-06-15 audit; the rm
/// declined-exit tests were updated to assert this Err in both modes.)
fn declined(label: &str, json: bool) -> Result<(), ClwError> {
    if json {
        println!(
            "{}",
            json!({
                "ref": label,
                "deleted": false,
                "note": "aborted — confirmation declined (use --yes for non-interactive delete)"
            })
        );
    }
    // Message (not Transport): a declined confirmation is not a transport failure.
    Err(ClwError::Message(format!(
        "aborted — confirmation declined; {label} left unchanged"
    )))
}

/// Render the server-regression case. ALWAYS returns `Err` so the process exits
/// non-zero regardless of `--json`; in JSON mode the structured note is emitted
/// to stdout first. Identical exit-code contract to `ls`/`prune`/`erase` — the
/// brutal-audit (2026-06-15) caught `rm` as the lone outlier that PR #34 missed.
fn capability_unavailable(json: bool) -> Result<(), ClwError> {
    let msg = "ref deletion failed: the corelink-server DELETE /v1/ac capability is LIVE (D-1) but returned 405/501 — likely a server-side regression; retry or escalate (see docs/RELAY-server-D1-D7-D8.md)".to_string();
    if json {
        println!(
            "{}",
            json!({ "deleted": false, "error": "capability-unavailable", "note": msg })
        );
    }
    Err(ClwError::Transport(msg))
}

/// True if the error is the server-regression case (the client maps 405/501 to a
/// `Transport` error whose message carries the `405/501` token for a LIVE
/// capability).
fn is_capability_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("405/501"))
}

/// Rewrite a raw `Auth` error into an actionable transport message (mirrors
/// `ls`/`status`); pass everything else through.
fn map_auth_error(err: ClwError) -> ClwError {
    if matches!(err, ClwError::Auth) {
        ClwError::Message(
            "authentication failed — check CLW_TOKEN or ~/.clw/config.toml".to_string(),
        )
    } else {
        err
    }
}

/// Prompt for confirmation before deleting a ref. Returns `false` immediately if
/// stdin is not a TTY (non-interactive), so `--json`/pipelines never delete
/// without an explicit `--yes`.
fn confirm_rm(label: &str) -> Result<bool, ClwError> {
    use std::io::{IsTerminal as _, Write as _};
    if !std::io::stdin().is_terminal() {
        // Non-interactive without --yes: refuse to delete.
        return Ok(false);
    }
    eprint!("delete {label}? this cannot be undone [y/N]: ");
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
    fn capability_unavailable_detects_405_501_message() {
        let err = ClwError::Transport(
            "ref deletion failed: server returned 405/501 for DELETE /v1/ac".to_string(),
        );
        assert!(is_capability_unavailable(&err));
    }

    #[test]
    fn resolve_rm_target_by_name_or_ref_key() {
        // by name → the domain-folded ref key + a "ref '<name>'" label.
        let (k_name, label) = resolve_rm_target(Some("my-ws"), None, RefDomain::User).unwrap();
        assert_eq!(label, "ref 'my-ws'");
        assert_eq!(k_name, ref_key_in(RefDomain::User, "my-ws"));
        // by raw ref-key (64-hex) → that exact digest + a "ref-key …" label (v0.1.1
        // power-user finding: lets a user delete a dangling/corrupt ref with no name).
        let hex = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let (k_key, label2) = resolve_rm_target(None, Some(hex), RefDomain::User).unwrap();
        assert_eq!(k_key, Digest::from_hex(hex).unwrap());
        assert!(label2.contains(hex));
        // invalid hex → actionable Message error (not a panic / not "transport").
        let bad = resolve_rm_target(None, Some("not-hex"), RefDomain::User).unwrap_err();
        assert!(!bad.to_string().starts_with("transport error"));
        // neither flag → error naming BOTH options.
        let none = resolve_rm_target(None, None, RefDomain::User).unwrap_err();
        assert!(none.to_string().contains("--name") && none.to_string().contains("--ref-key"));
    }

    #[test]
    fn capability_unavailable_ignores_other_transport_errors() {
        let err = ClwError::Transport("some unrelated transport issue".to_string());
        assert!(!is_capability_unavailable(&err));
        assert!(!is_capability_unavailable(&ClwError::Auth));
    }

    #[test]
    fn capability_unavailable_fails_in_both_modes() {
        // A destructive delete that did nothing must NEVER report success — both
        // JSON and human mode return an error (non-zero exit). Regression for the
        // brutal-audit CRIT/HIGH: rm was the sibling PR #34 missed.
        assert!(capability_unavailable(true).is_err());
        assert!(capability_unavailable(false).is_err());
    }

    // FIX 4 — a DECLINED rm confirmation must exit NON-ZERO in BOTH modes, aligned
    // with erase/uninstall, so automation can distinguish "delete did not happen"
    // from a completed delete. (Previously returned Ok(()) / exit 0; this test
    // documents the intentionally-changed behavior.)
    #[test]
    fn declined_confirmation_is_err_in_both_modes() {
        let json_res = declined("payroll", true);
        let human_res = declined("payroll", false);
        assert!(
            json_res.is_err(),
            "--json declined must be an Err (non-zero exit)"
        );
        assert!(
            human_res.is_err(),
            "human declined must be an Err (non-zero exit)"
        );
        // The error message says the ref was left unchanged and names the ref.
        let msg = human_res.unwrap_err().to_string();
        assert!(
            msg.contains("left unchanged"),
            "must say left unchanged: {msg}"
        );
        assert!(msg.contains("payroll"), "must name the ref: {msg}");
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
