//! `clw prune` — mark-and-sweep garbage collection over the tenant's CAS.
//!
//! Computes the LIVE set (every manifest root reachable from a ref, plus every
//! chunk those manifests reference) and deletes CAS blobs outside it. **Dry-run
//! by default**: it prints what it *would* delete and changes nothing unless
//! `--yes` is given.
//!
//! Ref enumeration (D-7) and CAS list/delete (D-8) are LIVE corelink-server
//! capabilities (conformance-green). The 405/501 branch is a defensive guard: a
//! LIVE capability returning 405/501 is a server-side regression, so the real
//! transport surfaces it as an actionable server-regression error.

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
    // RESIDUAL (documented, not closed client-side): two distinct skew cases can
    // turn a live blob into a false orphan, both bounded by dry-run-by-default and
    // the store's last-write-wins semantics:
    //   1. A push whose blobs were already in CAS before T0 but whose ref write
    //      lands AFTER the mark read can still have those blobs swept.
    //   2. A ref whose record GETs 404 mid-mark (see `compute_live_set`) is
    //      SKIPPED. That is usually a benign concurrent `rm`, but it can ALSO be
    //      AC replica-lag / read-after-write skew (the ref exists but a lagging
    //      replica answered 404). In that case the ref contributes nothing to the
    //      live set and its blobs become false orphans, deletable under `--yes`.
    // The fully-correct fix for both is a server-side GC lease/epoch (a
    // corelink-server item). Until then `prune` is dry-run-by-default: run it in a
    // low-write window, review the dry-run first, and rely on last-write-wins (a
    // concurrent re-push restores any blob a prune raced) to bound the exposure.
    let all_blobs = match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "prune (CAS GC)", "D-8")
        }
        Err(e) => return Err(map_auth_error(e)),
    };
    let live = match compute_live_set(&client).await {
        Ok(live) => live,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "prune (CAS GC)", "D-7/D-8")
        }
        // FAIL-SAFE abort (an unresolvable ref ⇒ incomplete live set). Surface the
        // ref-naming message verbatim and delete NOTHING — in --json the structured
        // note is printed first, then the same error propagates (non-zero exit).
        Err(e) if is_live_set_incomplete(&e) => {
            if json {
                println!(
                    "{}",
                    json!({ "error": "live-set-incomplete", "note": e.to_string() })
                );
            }
            return Err(e);
        }
        Err(e) => return Err(map_auth_error(e)),
    };

    let total_blobs = all_blobs.len();
    let orphans: Vec<Digest> = all_blobs
        .into_iter()
        .filter(|d| !live.contains(d))
        .collect();

    // SAFETY FLOOR: an empty live set turning 100% of a non-empty CAS into orphans
    // is indistinguishable from a transiently-empty/stale ref index (a D-7 200 with
    // `{"refs":[]}` during replica lag or a reindex). That "delete everything"
    // signature is as much the fingerprint of a mis-enumeration as of a truly-
    // orphaned tenant, so refuse it UNCONDITIONALLY (not only under `--yes`, and not
    // only at delete time): even the dry-run must not present a full wipe as a safe
    // plan. Point the user at `clw erase` (typed-tenant confirmation) for an
    // intentional delete-all. NOTHING is deleted on this path. In --json the
    // structured note is printed first, then the same error propagates.
    if live.is_empty() && !orphans.is_empty() {
        let msg = format!(
            "refusing to prune: the live set is empty but CAS holds {total_blobs} blob(s) — \
             pruning would delete the ENTIRE tenant, which usually means the ref index is \
             transiently empty/stale (D-7 returned no refs), not that every blob is truly \
             orphaned. Verify with `clw ls --all`; if the tenant is genuinely empty, use \
             `clw erase` (which requires typed-tenant confirmation) instead"
        );
        if json {
            println!(
                "{}",
                json!({ "error": "empty-live-set-refused", "note": msg })
            );
        }
        return Err(ClwError::Transport(msg));
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
    // Live counter on an interactive TTY (stderr only, so --json stdout stays
    // clean) so a long serial sweep isn't mistaken for a hang.
    let show_progress = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let total = orphans.len();
    let mut deleted = 0usize;
    let mut failure: Option<(Digest, ClwError)> = None;
    for d in &orphans {
        if let Err(e) = CasTransport::delete(&client, d).await {
            failure = Some((*d, map_auth_error(e)));
            break;
        }
        deleted += 1;
        if show_progress {
            eprint!("\rpruning {deleted}/{total}…");
            use std::io::Write as _;
            std::io::stderr().flush().ok();
        }
    }
    if show_progress && deleted > 0 {
        eprintln!("\rpruned {deleted}/{total} orphan(s).   ");
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
///
/// FAIL-SAFE: if ANY ref cannot be resolved to a parsed manifest (its record is
/// unreadable/corrupt, its manifest blob is missing, or either fails to parse),
/// the live set is INCOMPLETE — proceeding would sweep that ref's live blobs as
/// false orphans. We therefore ABORT the whole mark with a `live_set_incomplete`
/// error that NAMES the offending ref, rather than returning a partial set. A ref
/// that 404s on its record GET mid-walk is SKIPPED (it contributes nothing), not
/// treated as corruption — aborting on every concurrent `rm` would be too brittle.
/// CAVEAT: that 404 is USUALLY a benign concurrent `rm`, but it can ALSO be AC
/// replica-lag / read-after-write skew (the ref still exists; a lagging replica
/// answered 404). In the skew case the ref's blobs become false orphans, so the
/// skip is NOT unconditionally safe — the exposure is bounded by dry-run-by-default
/// and the store's last-write-wins (a re-push restores a raced blob). See the
/// RESIDUAL note in `run` for the full skew model.
async fn compute_live_set(client: &HttpClient) -> Result<HashSet<Digest>, ClwError> {
    let mut live = HashSet::new();

    for key in AcTransport::list_refs(client).await? {
        // A ref whose record GETs 404 mid-walk contributes nothing, so it is
        // skipped. USUALLY a benign concurrent `rm`; can also be AC replica-lag
        // (read-after-write skew), in which case its blobs become false orphans —
        // bounded by dry-run-default + last-write-wins (see the RESIDUAL note in `run`).
        let Some(bytes) = AcTransport::get(client, &key).await? else {
            continue;
        };
        // From here on, ANY failure to resolve this ref to a parsed manifest
        // means the live set is incomplete: abort and NAME the ref. Pruning on a
        // partial live set would delete live blobs as false orphans.
        let record = RefRecord::from_bytes(&bytes).map_err(|e| {
            live_set_incomplete(&key.to_hex(), &format!("ref record is corrupt ({e})"))
        })?;

        // The manifest root is itself a CAS blob and is always live.
        live.insert(record.root);

        // Every chunk the manifest references is live.
        let manifest_bytes = CasTransport::get(client, &record.root).await.map_err(|e| {
            live_set_incomplete(
                &record.name,
                &format!(
                    "its manifest root {} could not be fetched ({e})",
                    record.root.to_hex()
                ),
            )
        })?;
        let manifest = Manifest::from_bytes(&manifest_bytes).map_err(|e| {
            live_set_incomplete(
                &record.name,
                &format!(
                    "its manifest root {} is corrupt ({e})",
                    record.root.to_hex()
                ),
            )
        })?;
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

/// Build the fail-safe abort for an unresolvable ref. NAMES the offending ref and
/// explains the live set is incomplete so NO gc is safe — points the user at the
/// repair/removal path. Carried as a `Transport` error whose message contains the
/// `live set is incomplete` token (asserted by tests and detected by the caller).
fn live_set_incomplete(ref_label: &str, why: &str) -> ClwError {
    ClwError::Transport(format!(
        "refusing to prune: the live set is incomplete — ref '{ref_label}' {why}, so the set of \
         reachable blobs cannot be computed in full. Pruning now would delete live blobs as false \
         orphans. NOTHING was deleted. Repair or remove the offending ref (e.g. `clw rm --name \
         <NAME>` if it is dangling) and re-run prune."
    ))
}

/// True if the error is the fail-safe incomplete-live-set abort raised by
/// [`compute_live_set`] for an unresolvable ref.
fn is_live_set_incomplete(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("live set is incomplete"))
}

/// True if the error is the server-regression case (the client maps 405/501 to a
/// `Transport` error carrying the `405/501` token for a LIVE capability).
fn is_capability_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("405/501"))
}

/// Render the server-regression case. ALWAYS fails (non-zero exit) so a
/// destructive command that did nothing never reports success — in `--json` mode
/// the structured note is printed first, then the error propagates (same exit
/// code as human mode).
fn capability_unavailable(json: bool, capability: &str, dep: &str) -> Result<(), ClwError> {
    let msg = format!(
        "{capability} failed: corelink-server capability {dep} is LIVE but returned 405/501 — likely a server-side regression; retry or escalate (see docs/RELAY-server-D1-D7-D8.md)"
    );
    if json {
        println!(
            "{}",
            json!({ "error": "capability-unavailable", "note": msg })
        );
    }
    Err(ClwError::Transport(msg))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE (pass-2 audit de-vacuous): the previous `orphan_set_is_all_blobs_minus_live`,
    // `empty_live_set_makes_everything_an_orphan`, and `full_live_set_prunes_nothing`
    // tests reimplemented the `all_blobs.filter(!live.contains)` set difference IN THE
    // TEST and asserted on their own reimplementation — they never ran `run`'s real
    // sweep. The REAL orphan computation, the dry-run/`--yes` deletion, and the
    // keep-reachable/sweep-orphan invariant are now driven end-to-end against the live
    // binary by `prune_dry_run_then_yes_removes_only_orphans` in
    // `crates/clw-cli/tests/e2e.rs` (snapshots a tree, plants an orphan, asserts CAS
    // store state after both the dry-run and `--yes`). The genuine helper/predicate
    // tests that exercise REAL functions (`live_set_incomplete`,
    // `is_live_set_incomplete`, `is_capability_unavailable`, `capability_unavailable`)
    // are kept below.

    #[test]
    fn capability_unavailable_detects_405_501_message() {
        let err =
            ClwError::Transport("CAS listing failed: server returned 405/501 (D-8)".to_string());
        assert!(is_capability_unavailable(&err));
        assert!(!is_capability_unavailable(&ClwError::Transport(
            "unrelated".into()
        )));
        assert!(!is_capability_unavailable(&ClwError::Auth));
    }

    #[test]
    fn capability_unavailable_fails_in_both_modes() {
        // A destructive command that did nothing must never report success.
        assert!(capability_unavailable(true, "prune (CAS GC)", "D-8").is_err());
        assert!(capability_unavailable(false, "prune (CAS GC)", "D-8").is_err());
    }

    // FIX 1a — a corrupt/unresolvable ref aborts prune (deletes NOTHING) with an
    // error that NAMES the offending ref and explains the live set is incomplete.
    #[test]
    fn incomplete_live_set_aborts_and_names_the_ref() {
        let err = live_set_incomplete("payroll-prod", "ref record is corrupt (bad json)");
        // It is the fail-safe abort, detected by the caller.
        assert!(is_live_set_incomplete(&err));
        let msg = err.to_string();
        // Names the offending ref.
        assert!(
            msg.contains("payroll-prod"),
            "must name the ref, got: {msg}"
        );
        // Explains the live set is incomplete and that nothing was deleted.
        assert!(
            msg.contains("live set is incomplete"),
            "must say incomplete: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("nothing was deleted"),
            "must state nothing was deleted: {msg}"
        );
        // Points the user at the repair/removal path.
        assert!(
            msg.contains("clw rm"),
            "must point at the repair path: {msg}"
        );
    }

    #[test]
    fn is_live_set_incomplete_ignores_other_errors() {
        assert!(!is_live_set_incomplete(&ClwError::Transport(
            "unrelated".into()
        )));
        assert!(!is_live_set_incomplete(&ClwError::Auth));
        // The capability-unavailable abort is a DIFFERENT case and must not be
        // confused with the incomplete-live-set abort.
        let cap = ClwError::Transport("prune failed: returned 405/501".into());
        assert!(!is_live_set_incomplete(&cap));
    }

    // NOTE (pass-2 audit de-vacuous): the previous `empty_live_set_with_blobs_is_refused`
    // and `empty_live_set_with_empty_cas_does_not_trip_the_floor` tests reimplemented the
    // safety-floor condition (`live.is_empty() && !orphans.is_empty()`) and a copy-pasted
    // refusal message IN THE TEST, asserting only on their own reimplementation — they
    // never ran `run`'s real floor. The REAL floor (the exit-2 refusal over a non-empty
    // CAS with no refs, deleting NOTHING) is now driven end-to-end against the live binary
    // by `prune_empty_live_set_over_nonempty_cas_refuses` in `crates/clw-cli/tests/e2e.rs`,
    // which clears the AC store, runs `prune --yes`, asserts exit 2, and asserts every CAS
    // blob is still present. The benign empty-CAS case (nothing to prune, no refusal) is
    // covered by the same e2e suite via the verified-empty `erase` path.
}
