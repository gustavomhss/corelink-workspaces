//! `clw erase` — irreversibly delete ALL of a tenant's data (every ref + every
//! CAS blob). This is the GDPR Art.17 "right to erasure" path.
//!
//! Three independent safety rails, because this is the single most destructive
//! command in the CLI:
//!   1. **Dry-run by default** — without `--yes` it only reports what it would
//!      delete and changes nothing.
//!   2. **`--yes`** — explicit intent to actually delete.
//!   3. **Typed-tenant confirmation** — the operator must reproduce the exact
//!      tenant id, interactively or via `--confirm-tenant <TENANT>` for
//!      non-interactive/`--json` use. A mismatch aborts. This makes erasing the
//!      wrong tenant a deliberate two-factor act, not a fat-fingered flag.
//!
//! OPTIMISTIC against a frozen contract: ref enumeration (D-7) and CAS
//! list/delete (D-8) are pending corelink-server ratification
//! (docs/REQUESTS-to-other-TLs.md). Until they deploy, the real transport
//! surfaces the 405/501 not-deployed case as an actionable error.

use clap::Args;
use clw_client::HttpClient;
use clw_types::{AcTransport, CasTransport, ClwError};
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::status::map_auth_error;

#[derive(Debug, Args)]
#[command(
    about = "Irreversibly delete ALL data for a tenant (GDPR erasure; dry-run unless --yes)",
    long_about = "Delete every ref and every CAS blob for the tenant — the GDPR Art.17 \
                  erasure path. Dry-run by default. Executing requires --yes AND a typed \
                  tenant confirmation (interactive prompt, or --confirm-tenant for non-interactive)."
)]
pub struct EraseArgs {
    /// Explicit intent to actually delete (without this, erase is a dry run).
    #[arg(
        long,
        default_value_t = false,
        help = "Perform the deletion (default is a dry run that only reports)"
    )]
    pub yes: bool,

    /// Non-interactive typed-tenant confirmation. Must equal the resolved tenant
    /// id or the command aborts. Required for `--json`/non-TTY execution.
    #[arg(
        long,
        value_name = "TENANT",
        help = "Reproduce the tenant id to confirm erasure (required for non-interactive use)"
    )]
    pub confirm_tenant: Option<String>,
}

pub async fn run(args: EraseArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let client = HttpClient::new(cfg.clw_config)?;

    // Enumerate the targets up front (also drives the dry-run report).
    let ref_keys = match AcTransport::list_refs(&client).await {
        Ok(k) => k,
        Err(e) if is_not_deployed(&e) => return not_deployed(json, "erase", "D-7"),
        Err(e) => return Err(map_auth_error(e)),
    };
    let blobs = match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_not_deployed(&e) => return not_deployed(json, "erase", "D-8"),
        Err(e) => return Err(map_auth_error(e)),
    };

    // Dry run (default): report the blast radius and stop.
    if !args.yes {
        if json {
            println!(
                "{}",
                json!({
                    "dry_run": true,
                    "tenant": tenant,
                    "refs": ref_keys.len(),
                    "blobs": blobs.len(),
                })
            );
        } else {
            println!(
                "DRY RUN — would IRREVERSIBLY erase tenant '{tenant}': {} ref(s) + {} blob(s).",
                ref_keys.len(),
                blobs.len()
            );
            println!("Re-run with --yes and confirm the tenant id to proceed.");
        }
        return Ok(());
    }

    // Rail 3: typed-tenant confirmation. `--confirm-tenant` wins (non-interactive);
    // otherwise prompt. A mismatch — or a non-TTY without the flag — aborts.
    if !confirm_tenant(&tenant, args.confirm_tenant.as_deref())? {
        return abort(json, &tenant);
    }

    // Echo the blast radius immediately before deleting, so even when the typed
    // confirmation passes (e.g. a wrong-but-confirmed tenant in CI) the target and
    // scale are auditable on screen and in the job log.
    eprintln!(
        "erase: IRREVERSIBLY deleting tenant '{tenant}' — {} ref(s) + {} blob(s)…",
        ref_keys.len(),
        blobs.len()
    );

    // Execute: delete every REF first, then every blob. Refs-first keeps the store
    // consistent on a partial failure — a failure after the ref loop leaves only
    // UNREFERENCED blobs (orphans, reclaimable by `prune` or an idempotent re-run),
    // never a live ref pointing at already-deleted content.
    let mut refs_deleted = 0usize;
    for k in &ref_keys {
        AcTransport::delete(&client, k)
            .await
            .map_err(map_auth_error)?;
        refs_deleted += 1;
    }
    let mut blobs_deleted = 0usize;
    for d in &blobs {
        CasTransport::delete(&client, d)
            .await
            .map_err(map_auth_error)?;
        blobs_deleted += 1;
    }

    if json {
        println!(
            "{}",
            json!({
                "dry_run": false,
                "tenant": tenant,
                "refs_deleted": refs_deleted,
                "blobs_deleted": blobs_deleted,
            })
        );
    } else {
        println!(
            "erased tenant '{tenant}': {refs_deleted} ref(s) + {blobs_deleted} blob(s) deleted"
        );
    }
    Ok(())
}

/// Rail 3 — typed-tenant confirmation. With `--confirm-tenant` we compare the
/// supplied value (non-interactive). Otherwise we prompt for it on a TTY. A
/// non-interactive call WITHOUT the flag returns `false` (refuse, never erase).
fn confirm_tenant(expected: &str, supplied: Option<&str>) -> Result<bool, ClwError> {
    use std::io::{IsTerminal as _, Write as _};

    if let Some(given) = supplied {
        return Ok(given == expected);
    }
    if !std::io::stdin().is_terminal() {
        // Non-interactive without --confirm-tenant: refuse.
        return Ok(false);
    }
    eprint!("type the tenant id '{expected}' to IRREVERSIBLY erase it: ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim() == expected)
}

/// Emit the aborted-confirmation outcome and FAIL with a non-zero exit, so
/// automation can distinguish a refused erase from a completed one (they would
/// otherwise share exit 0). In `--json` mode the structured note is still printed
/// to stdout before the error; in human mode `exit_err` prints the message.
fn abort(json: bool, tenant: &str) -> Result<(), ClwError> {
    if json {
        println!(
            "{}",
            json!({ "tenant": tenant, "erased": false, "note": "aborted — tenant confirmation did not match" })
        );
    }
    Err(ClwError::Transport(format!(
        "aborted — tenant confirmation did not match; '{tenant}' left unchanged"
    )))
}

/// True if the error is the optimistic-pending "server capability not deployed"
/// case (the client maps 405/501 to a `Transport` error naming the gap).
fn is_not_deployed(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("not yet deployed"))
}

/// Render the optimistic-pending case. ALWAYS fails (non-zero exit) so a
/// destructive command that did nothing never reports success — in `--json` mode
/// the structured note is printed first, then the error propagates (same exit
/// code as human mode).
fn not_deployed(json: bool, capability: &str, dep: &str) -> Result<(), ClwError> {
    let msg = format!(
        "{capability} requires corelink-server capabilities not yet deployed — tracked ({dep}; see docs/RELAY-server-D1-D7-D8.md)"
    );
    if json {
        println!("{}", json!({ "error": "not-deployed", "note": msg }));
    }
    Err(ClwError::Transport(msg))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_tenant_flag_must_match_exactly() {
        assert!(confirm_tenant("acme", Some("acme")).unwrap());
        assert!(!confirm_tenant("acme", Some("acm")).unwrap());
        assert!(!confirm_tenant("acme", Some("ACME")).unwrap());
        assert!(!confirm_tenant("acme", Some("")).unwrap());
    }

    #[test]
    fn not_deployed_detects_pending_capability_message() {
        let err = ClwError::Transport(
            "erase requires a corelink-server capability not yet deployed (D-8)".to_string(),
        );
        assert!(is_not_deployed(&err));
        assert!(!is_not_deployed(&ClwError::Transport("unrelated".into())));
        assert!(!is_not_deployed(&ClwError::Auth));
    }

    #[test]
    fn not_deployed_fails_in_both_modes() {
        // A destructive command that did nothing must NEVER report success —
        // both JSON and human mode return an error (non-zero exit).
        assert!(not_deployed(true, "erase", "D-7").is_err());
        assert!(not_deployed(false, "erase", "D-7").is_err());
    }

    #[test]
    fn abort_fails_in_both_modes() {
        // A refused/mismatched confirmation must exit non-zero so automation can
        // distinguish it from a completed erase.
        assert!(abort(true, "acme").is_err());
        assert!(abort(false, "acme").is_err());
    }
}
