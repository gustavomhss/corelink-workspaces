//! `clw prune` — mark-and-sweep garbage collection over the tenant's CAS.
//!
//! Computes the LIVE set (every manifest root reachable from a ref, plus every
//! chunk those manifests reference) and deletes CAS blobs outside it. **Dry-run
//! by default**: it prints what it *would* delete and changes nothing unless
//! `--yes` is given.
//!
//! OPTIMISTIC against a frozen contract: ref enumeration (D-7) and CAS
//! list/delete (D-8) are pending corelink-server ratification
//! (docs/REQUESTS-to-other-TLs.md). Until they deploy, the real transport
//! surfaces the 405/501 not-deployed case as an actionable error.

use std::collections::HashSet;

use clap::Args;
use clw_client::HttpClient;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Entry, Manifest, RefRecord};
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::status::map_auth_error;

#[derive(Debug, Args)]
#[command(
    about = "Garbage-collect unreferenced CAS blobs (dry-run unless --yes)",
    long_about = "Delete CAS blobs not reachable from any ref. Dry-run by default: \
                  prints what would be deleted and changes nothing without --yes."
)]
pub struct PruneArgs {
    /// Actually delete the orphaned blobs (without this, prune is a dry run).
    #[arg(
        long,
        default_value_t = false,
        help = "Perform the deletion (default is a dry run that only reports)"
    )]
    pub yes: bool,
}

pub async fn run(args: PruneArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let client = HttpClient::new(cfg.clw_config)?;

    // ORDER MATTERS for GC correctness. Snapshot the blob CANDIDATE set FIRST
    // (at T0), THEN compute the live set (refs read at T1 > T0). With candidates
    // frozen before the mark, any blob that becomes live in the gap is reflected
    // in `live` (so it is never orphaned), and any blob written after T0 is simply
    // not a candidate. The reverse order (mark, then list blobs) would sweep a
    // concurrently-pushed ref's blobs as orphans — live-data loss.
    //
    // RESIDUAL (documented, not closed client-side): a push whose blobs were
    // already in CAS before T0 but whose ref write lands AFTER the mark read can
    // still have those blobs swept. The fully-correct fix is a server-side GC
    // lease/epoch (a corelink-server item). Until then `prune` is dry-run-by-
    // default; run it in a low-write window and review the dry-run first.
    let all_blobs = match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_not_deployed(&e) => return not_deployed(json, "prune (CAS GC)", "D-8"),
        Err(e) => return Err(map_auth_error(e)),
    };
    let live = match compute_live_set(&client).await {
        Ok(live) => live,
        Err(e) if is_not_deployed(&e) => return not_deployed(json, "prune (CAS GC)", "D-7/D-8"),
        Err(e) => return Err(map_auth_error(e)),
    };

    let total_blobs = all_blobs.len();
    let orphans: Vec<Digest> = all_blobs
        .into_iter()
        .filter(|d| !live.contains(d))
        .collect();

    // SAFETY FLOOR: an empty live set turning 100% of a non-empty CAS into orphans
    // is indistinguishable from a transiently-empty/stale ref index (a D-7 200 with
    // `{"refs":[]}` during replica lag or a reindex). Refuse the full wipe rather
    // than risk deleting a live tenant on a spurious empty page.
    if args.yes && live.is_empty() && !orphans.is_empty() {
        return Err(ClwError::Transport(format!(
            "refusing to prune: the live set is empty but CAS holds {total_blobs} blob(s) — \
             pruning would delete the ENTIRE tenant, which usually means the ref index is \
             transiently empty/stale (D-7 returned no refs), not that every blob is truly \
             orphaned. Verify with `clw ls --all`; if the tenant is genuinely empty, use \
             `clw erase` (which requires typed-tenant confirmation) instead"
        )));
    }

    // Dry run (default): report and stop. A delete this broad must be explicit.
    if !args.yes {
        if json {
            println!(
                "{}",
                json!({
                    "dry_run": true,
                    "live": live.len(),
                    "orphans": orphans.len(),
                    "would_delete": orphans.iter().map(Digest::to_hex).collect::<Vec<_>>(),
                })
            );
        } else if orphans.is_empty() {
            println!("nothing to prune — {} live blobs, 0 orphans", live.len());
        } else {
            println!(
                "DRY RUN — would delete {} orphaned blob(s) ({} live). Re-run with --yes to delete:",
                orphans.len(),
                live.len()
            );
            for d in &orphans {
                println!("  {}", d.to_hex());
            }
        }
        return Ok(());
    }

    // --- DELETE: remove each orphan, never discarding progress on failure ----
    let mut deleted = 0usize;
    let mut failure: Option<(Digest, ClwError)> = None;
    for d in &orphans {
        if let Err(e) = CasTransport::delete(&client, d).await {
            failure = Some((*d, map_auth_error(e)));
            break;
        }
        deleted += 1;
    }

    if let Some((digest, err)) = failure {
        // Partial GC: report what WAS deleted (resumable/auditable) then fail.
        if json {
            println!(
                "{}",
                json!({ "dry_run": false, "live": live.len(), "deleted": deleted, "failed_at": digest.to_hex() })
            );
        } else {
            eprintln!(
                "prune: deleted {deleted} of {} orphan(s) before failing at {}; re-run to resume",
                orphans.len(),
                digest.to_hex()
            );
        }
        return Err(err);
    }

    if json {
        println!(
            "{}",
            json!({ "dry_run": false, "live": live.len(), "deleted": deleted })
        );
    } else {
        println!("pruned {deleted} orphaned blob(s) ({} live)", live.len());
    }
    Ok(())
}

/// MARK phase: every digest reachable from a ref — the manifest roots plus every
/// chunk those manifests reference. Anything in CAS outside this set is garbage.
async fn compute_live_set(client: &HttpClient) -> Result<HashSet<Digest>, ClwError> {
    let mut live = HashSet::new();

    for key in AcTransport::list_refs(client).await? {
        // A ref deleted mid-walk simply contributes nothing.
        let Some(bytes) = AcTransport::get(client, &key).await? else {
            continue;
        };
        let record = RefRecord::from_bytes(&bytes)?;

        // The manifest root is itself a CAS blob and is always live.
        live.insert(record.root);

        // Every chunk the manifest references is live.
        let manifest_bytes = CasTransport::get(client, &record.root).await?;
        let manifest = Manifest::from_bytes(&manifest_bytes)?;
        for entry in &manifest.entries {
            if let Entry::File { chunks, .. } = entry {
                for c in chunks {
                    live.insert(c.digest);
                }
            }
        }
    }

    Ok(live)
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

    fn digest(byte: u8) -> Digest {
        Digest::of_bytes(&[byte])
    }

    #[test]
    fn orphan_set_is_all_blobs_minus_live() {
        // The sweep is exactly set difference: blobs not in `live` are orphans.
        let live: HashSet<Digest> = [digest(1), digest(2)].into_iter().collect();
        let all = vec![digest(1), digest(2), digest(3), digest(4)];
        let orphans: Vec<Digest> = all.into_iter().filter(|d| !live.contains(d)).collect();
        assert_eq!(orphans, vec![digest(3), digest(4)]);
    }

    #[test]
    fn empty_live_set_makes_everything_an_orphan() {
        let live: HashSet<Digest> = HashSet::new();
        let all = vec![digest(7), digest(8)];
        let orphans: Vec<Digest> = all.into_iter().filter(|d| !live.contains(d)).collect();
        assert_eq!(orphans.len(), 2);
    }

    #[test]
    fn full_live_set_prunes_nothing() {
        let live: HashSet<Digest> = [digest(7), digest(8)].into_iter().collect();
        let all = vec![digest(7), digest(8)];
        let orphans: Vec<Digest> = all.into_iter().filter(|d| !live.contains(d)).collect();
        assert!(orphans.is_empty());
    }

    #[test]
    fn not_deployed_detects_pending_capability_message() {
        let err = ClwError::Transport(
            "CAS listing requires a corelink-server capability not yet deployed (D-8)".to_string(),
        );
        assert!(is_not_deployed(&err));
        assert!(!is_not_deployed(&ClwError::Transport("unrelated".into())));
        assert!(!is_not_deployed(&ClwError::Auth));
    }

    #[test]
    fn not_deployed_fails_in_both_modes() {
        // A destructive command that did nothing must never report success.
        assert!(not_deployed(true, "prune (CAS GC)", "D-8").is_err());
        assert!(not_deployed(false, "prune (CAS GC)", "D-8").is_err());
    }
}
