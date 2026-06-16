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
//! Ref enumeration (D-7) and CAS list/delete (D-8) are LIVE corelink-server
//! capabilities (conformance-green). The 405/501 branch is a defensive guard: a
//! LIVE capability returning 405/501 is a server-side regression, so the real
//! transport surfaces it as an actionable server-regression error.

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
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "erase", "D-7")
        }
        Err(e) => return Err(map_auth_error(e)),
    };
    let blobs = match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "erase", "D-8")
        }
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

    // Execute WITH VERIFICATION (see `erase_with_verification`).
    let EraseOutcome {
        refs_deleted,
        blobs_deleted,
        remaining_refs,
        remaining_blobs,
        passes: pass,
    } = match erase_with_verification(&client, ref_keys, blobs).await {
        Ok(o) => o,
        // FIX F3: a mid-delete transport failure carries the partial-erasure
        // counts. In --json, print a structured progress note to stdout BEFORE the
        // error propagates (same shape as the other failure paths), so an audit
        // log records exactly how much was erased before the abort.
        Err(MidDeleteError {
            refs_deleted,
            blobs_deleted,
            source,
        }) => {
            if json {
                println!(
                    "{}",
                    json!({
                        "dry_run": false,
                        "tenant": tenant,
                        "refs_deleted": refs_deleted,
                        "blobs_deleted": blobs_deleted,
                        "complete": false,
                        "note": "erasure failed mid-delete; counts reflect progress before the failure",
                    })
                );
            }
            return Err(ClwError::Transport(format!(
                "erasure FAILED mid-delete for tenant '{tenant}': {refs_deleted} ref(s) + \
                 {blobs_deleted} blob(s) were already erased before the error below. Re-run \
                 `clw erase` to resume; do NOT record this tenant as erased until a run \
                 completes. Underlying error: {source}"
            )));
        }
    };

    // Truthful Art.17 outcome: only claim success when enumeration confirms the
    // tenant is empty. Otherwise FAIL loud (non-zero exit) so a caller never
    // records a completed erasure while data — i.e. PII — still remains.
    if remaining_refs > 0 || remaining_blobs > 0 {
        if json {
            println!(
                "{}",
                json!({
                    "dry_run": false,
                    "tenant": tenant,
                    "refs_deleted": refs_deleted,
                    "blobs_deleted": blobs_deleted,
                    "remaining_refs": remaining_refs,
                    "remaining_blobs": remaining_blobs,
                    "complete": false,
                })
            );
        }
        return Err(ClwError::Transport(format!(
            "erasure INCOMPLETE for tenant '{tenant}': deleted {refs_deleted} ref(s) + {blobs_deleted} blob(s), but {remaining_refs} ref(s) + {remaining_blobs} blob(s) still remain after {pass} pass(es) — likely concurrent writes or a server-side enumeration fault. Re-run `clw erase` or escalate; do NOT record this tenant as erased."
        )));
    }

    if json {
        println!(
            "{}",
            json!({
                "dry_run": false,
                "tenant": tenant,
                "refs_deleted": refs_deleted,
                "blobs_deleted": blobs_deleted,
                "complete": true,
            })
        );
    } else {
        println!(
            "erased tenant '{tenant}': {refs_deleted} ref(s) + {blobs_deleted} blob(s) deleted; tenant verified empty"
        );
    }
    Ok(())
}

/// Result of the verified-erase loop. `refs_deleted`/`blobs_deleted` are the
/// counts of DISTINCT objects erased (the GDPR Art.17 audit numbers), NOT the
/// number of delete calls. `remaining_*` are what enumeration still saw on the
/// final pass; `passes` is how many passes ran.
#[derive(Debug)]
struct EraseOutcome {
    refs_deleted: usize,
    blobs_deleted: usize,
    remaining_refs: usize,
    remaining_blobs: usize,
    passes: usize,
}

/// FIX F3: a mid-delete failure carrying the partial-erasure counts (distinct
/// refs + blobs erased BEFORE the failing delete) alongside the underlying error.
/// Surfacing the counts keeps the GDPR Art.17 audit trail truthful about how much
/// was actually erased when a run aborts. The caller renders both the structured
/// `--json` note and the human-readable error from these fields.
#[derive(Debug)]
struct MidDeleteError {
    refs_deleted: usize,
    blobs_deleted: usize,
    source: ClwError,
}

/// Delete every ref then every blob, RE-ENUMERATE, and repeat until empty or
/// `MAX_PASSES`. Refs-first keeps the store consistent on a partial failure — a
/// crash after the ref loop leaves only UNREFERENCED orphan blobs, reclaimable
/// by `prune` or an idempotent re-run, never a live ref pointing at deleted
/// content. The verify loop exists because a GDPR Art.17 claim MUST NOT report
/// success until enumeration confirms the tenant is actually empty — a single
/// pass can miss data written concurrently or carried by a late pagination page.
/// Bounded passes prevent an unbounded loop against a misbehaving server.
///
/// COUNT DISTINCT objects erased, NOT delete CALLS: DELETE is idempotent
/// (404→Ok), so an eventually-consistent server can re-return a just-deleted key
/// on the next enumeration. Re-DELETEing it must NOT inflate the reported total —
/// that total is the audit number and must be truthful. We accumulate the
/// keys/hashes we issued a delete for into HashSets, so the distinct count holds
/// regardless of re-listing.
async fn erase_with_verification<C>(
    client: &C,
    ref_keys: Vec<clw_types::Digest>,
    blobs: Vec<clw_types::Digest>,
) -> Result<EraseOutcome, MidDeleteError>
where
    C: AcTransport + CasTransport,
{
    const MAX_PASSES: usize = 3;
    let mut deleted_refs: std::collections::HashSet<clw_types::Digest> =
        std::collections::HashSet::new();
    let mut deleted_blobs: std::collections::HashSet<clw_types::Digest> =
        std::collections::HashSet::new();
    let mut pending_refs = ref_keys;
    let mut pending_blobs = blobs;
    let mut pass = 0usize;
    // FIX F3: any transport failure in this loop carries the partial-erasure
    // counts (distinct refs + blobs erased SO FAR) so the caller can report
    // progress before aborting — the GDPR Art.17 audit trail must record what was
    // actually erased, not just an opaque error.
    let (remaining_refs, remaining_blobs) = loop {
        pass += 1;
        for k in &pending_refs {
            AcTransport::delete(client, k)
                .await
                .map_err(|e| MidDeleteError {
                    refs_deleted: deleted_refs.len(),
                    blobs_deleted: deleted_blobs.len(),
                    source: map_auth_error(e),
                })?;
            deleted_refs.insert(*k);
        }
        for d in &pending_blobs {
            CasTransport::delete(client, d)
                .await
                .map_err(|e| MidDeleteError {
                    refs_deleted: deleted_refs.len(),
                    blobs_deleted: deleted_blobs.len(),
                    source: map_auth_error(e),
                })?;
            deleted_blobs.insert(*d);
        }
        // Verify: re-enumerate to confirm nothing remains. A failure here is also a
        // mid-erase abort and carries the same progress counts.
        let r = AcTransport::list_refs(client)
            .await
            .map_err(|e| MidDeleteError {
                refs_deleted: deleted_refs.len(),
                blobs_deleted: deleted_blobs.len(),
                source: map_auth_error(e),
            })?;
        let b = CasTransport::list_blobs(client)
            .await
            .map_err(|e| MidDeleteError {
                refs_deleted: deleted_refs.len(),
                blobs_deleted: deleted_blobs.len(),
                source: map_auth_error(e),
            })?;
        if (r.is_empty() && b.is_empty()) || pass >= MAX_PASSES {
            break (r.len(), b.len());
        }
        pending_refs = r;
        pending_blobs = b;
    };
    Ok(EraseOutcome {
        refs_deleted: deleted_refs.len(),
        blobs_deleted: deleted_blobs.len(),
        remaining_refs,
        remaining_blobs,
        passes: pass,
    })
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
    use clw_types::{Digest, Result as ClwResult};
    use std::sync::Mutex;

    // FIX 1 — a fake transport whose enumeration is EVENTUALLY CONSISTENT: on the
    // SECOND `list_refs`/`list_blobs` call it re-returns one already-deleted key,
    // then reports empty thereafter. This reproduces the double-count: pass 2
    // re-DELETEs the re-listed key, and a call-counting implementation would
    // inflate the audit number. The distinct-set count must NOT inflate.
    struct EventuallyConsistentTransport {
        // The single ref/blob that gets re-listed once after being deleted.
        ghost_ref: Digest,
        ghost_blob: Digest,
        ref_list_calls: Mutex<usize>,
        blob_list_calls: Mutex<usize>,
        ref_deletes: Mutex<usize>,
        blob_deletes: Mutex<usize>,
    }

    impl EventuallyConsistentTransport {
        fn new() -> Self {
            Self {
                ghost_ref: Digest::of_bytes(b"ghost-ref"),
                ghost_blob: Digest::of_bytes(b"ghost-blob"),
                ref_list_calls: Mutex::new(0),
                blob_list_calls: Mutex::new(0),
                ref_deletes: Mutex::new(0),
                blob_deletes: Mutex::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl AcTransport for EventuallyConsistentTransport {
        async fn get(&self, _key: &Digest) -> ClwResult<Option<Vec<u8>>> {
            Ok(None)
        }
        async fn put(&self, _key: &Digest, _value: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, _key: &Digest) -> ClwResult<()> {
            *self.ref_deletes.lock().unwrap() += 1;
            Ok(()) // idempotent — re-deleting the ghost is Ok, like a real 404→Ok.
        }
        async fn list_refs(&self) -> ClwResult<Vec<Digest>> {
            let mut n = self.ref_list_calls.lock().unwrap();
            *n += 1;
            // 1st verify (after pass 1) STILL sees the just-deleted ghost; from
            // the 2nd verify on, it's gone.
            if *n == 1 {
                Ok(vec![self.ghost_ref])
            } else {
                Ok(vec![])
            }
        }
    }

    #[async_trait::async_trait]
    impl CasTransport for EventuallyConsistentTransport {
        async fn exists(&self, _digest: &Digest) -> ClwResult<bool> {
            Ok(false)
        }
        async fn get(&self, digest: &Digest) -> ClwResult<Vec<u8>> {
            Err(ClwError::NotFound(*digest))
        }
        async fn put(&self, _digest: &Digest, _bytes: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, _digest: &Digest) -> ClwResult<()> {
            *self.blob_deletes.lock().unwrap() += 1;
            Ok(())
        }
        async fn list_blobs(&self) -> ClwResult<Vec<Digest>> {
            let mut n = self.blob_list_calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Ok(vec![self.ghost_blob])
            } else {
                Ok(vec![])
            }
        }
    }

    #[tokio::test]
    async fn erase_counts_distinct_objects_not_delete_calls() {
        let client = EventuallyConsistentTransport::new();
        // Initial enumeration found exactly one ref + one blob (the ghosts).
        let ref_keys = vec![client.ghost_ref];
        let blobs = vec![client.ghost_blob];

        let outcome = erase_with_verification(&client, ref_keys, blobs)
            .await
            .unwrap();

        // Two passes ran (pass 1 deletes; the re-list forces pass 2 which
        // re-deletes the ghost), so the DELETE CALL count is 2 each…
        assert_eq!(
            *client.ref_deletes.lock().unwrap(),
            2,
            "the ghost ref was DELETEd twice (idempotently)"
        );
        assert_eq!(*client.blob_deletes.lock().unwrap(), 2);

        // …but the AUDIT number is the DISTINCT count: exactly ONE ref + ONE blob.
        assert_eq!(
            outcome.refs_deleted, 1,
            "must report DISTINCT refs erased, not delete calls"
        );
        assert_eq!(
            outcome.blobs_deleted, 1,
            "must report DISTINCT blobs erased, not delete calls"
        );

        // Erasure correctness is unchanged: the loop still gated on emptiness and
        // the tenant ended up verified empty.
        assert_eq!(outcome.remaining_refs, 0);
        assert_eq!(outcome.remaining_blobs, 0);
        assert_eq!(outcome.passes, 2);
    }

    // FIX F3 — a transport that errors after N successful ref-deletes must yield a
    // MidDeleteError whose counts reflect what was erased BEFORE the failure.
    struct FailAfterNTransport {
        // Number of ref deletes to allow before erroring.
        allow_ref_deletes: usize,
        ref_deletes: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl AcTransport for FailAfterNTransport {
        async fn get(&self, _key: &Digest) -> ClwResult<Option<Vec<u8>>> {
            Ok(None)
        }
        async fn put(&self, _key: &Digest, _value: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, _key: &Digest) -> ClwResult<()> {
            let mut n = self.ref_deletes.lock().unwrap();
            if *n >= self.allow_ref_deletes {
                return Err(ClwError::Transport("simulated mid-delete failure".into()));
            }
            *n += 1;
            Ok(())
        }
        async fn list_refs(&self) -> ClwResult<Vec<Digest>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl CasTransport for FailAfterNTransport {
        async fn exists(&self, _digest: &Digest) -> ClwResult<bool> {
            Ok(false)
        }
        async fn get(&self, digest: &Digest) -> ClwResult<Vec<u8>> {
            Err(ClwError::NotFound(*digest))
        }
        async fn put(&self, _digest: &Digest, _bytes: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, _digest: &Digest) -> ClwResult<()> {
            Ok(())
        }
        async fn list_blobs(&self) -> ClwResult<Vec<Digest>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn mid_delete_failure_reports_count_erased_before_failure() {
        // 5 refs to delete; the transport allows 2 then errors on the 3rd.
        let client = FailAfterNTransport {
            allow_ref_deletes: 2,
            ref_deletes: Mutex::new(0),
        };
        let ref_keys: Vec<Digest> = (0..5u8).map(|i| Digest::of_bytes(&[i])).collect();
        let blobs: Vec<Digest> = vec![];

        let err = erase_with_verification(&client, ref_keys, blobs)
            .await
            .expect_err("a mid-delete failure must abort with an error");

        // Exactly 2 distinct refs were erased before the 3rd delete failed.
        assert_eq!(
            err.refs_deleted, 2,
            "must report the 2 refs erased before the failure"
        );
        assert_eq!(err.blobs_deleted, 0);

        // The rendered message (as the caller builds it) names the partial counts.
        let rendered = format!(
            "{} ref(s) + {} blob(s) were already erased",
            err.refs_deleted, err.blobs_deleted
        );
        assert!(rendered.contains("2 ref(s) + 0 blob(s)"));
    }

    #[test]
    fn confirm_tenant_flag_must_match_exactly() {
        assert!(confirm_tenant("acme", Some("acme")).unwrap());
        assert!(!confirm_tenant("acme", Some("acm")).unwrap());
        assert!(!confirm_tenant("acme", Some("ACME")).unwrap());
        assert!(!confirm_tenant("acme", Some("")).unwrap());
    }

    #[test]
    fn capability_unavailable_detects_405_501_message() {
        let err = ClwError::Transport("erase failed: server returned 405/501 (D-8)".to_string());
        assert!(is_capability_unavailable(&err));
        assert!(!is_capability_unavailable(&ClwError::Transport(
            "unrelated".into()
        )));
        assert!(!is_capability_unavailable(&ClwError::Auth));
    }

    #[test]
    fn capability_unavailable_fails_in_both_modes() {
        // A destructive command that did nothing must NEVER report success —
        // both JSON and human mode return an error (non-zero exit).
        assert!(capability_unavailable(true, "erase", "D-7").is_err());
        assert!(capability_unavailable(false, "erase", "D-7").is_err());
    }

    #[test]
    fn abort_fails_in_both_modes() {
        // A refused/mismatched confirmation must exit non-zero so automation can
        // distinguish it from a completed erase.
        assert!(abort(true, "acme").is_err());
        assert!(abort(false, "acme").is_err());
    }
}
