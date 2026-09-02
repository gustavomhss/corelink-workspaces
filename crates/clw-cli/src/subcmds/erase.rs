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

use std::future::Future;

use clap::Args;
use clw_client::HttpClient;
use clw_types::{AcTransport, CasTransport, ClwError};
use futures::stream::{FuturesUnordered, StreamExt};
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

pub async fn run(
    args: EraseArgs,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    // The same user-selected, nonzero limit bounds both the local work queue and
    // the HTTP governor. In particular, erase does not claim N=16 while a default
    // eight-permit client silently caps it; a 429 can still adapt the live window
    // downward as intended.
    let concurrency = destructive_sweep_concurrency(concurrency);
    let client = HttpClient::with_concurrency(cfg.clw_config, concurrency)?;

    // Enumerate the targets up front (also drives the dry-run report).
    let ref_keys = stable_dedupe_targets(match AcTransport::list_refs(&client).await {
        Ok(k) => k,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "erase", "D-7")
        }
        Err(e) => return Err(map_auth_error(e)),
    });
    let blobs = stable_dedupe_targets(match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "erase", "D-8")
        }
        Err(e) => return Err(map_auth_error(e)),
    });

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
    } = match erase_with_verification(&client, ref_keys, blobs, concurrency).await {
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
            return Err(ClwError::Message(format!(
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
        return Err(ClwError::Message(format!(
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

/// Results of a bounded destructive phase after *all already-started* deletes
/// have completed. `failure`, when present, is the lowest input index that
/// failed, not simply the first completion that happened to return an error.
///
/// That distinction is load-bearing for the partial-report contract: completion
/// order is intentionally nondeterministic, while the target vector's input
/// order is stable. Once any failure is observed we schedule no further work,
/// but we keep polling the existing futures to completion. Thus every item in
/// `succeeded` is a confirmed successful DELETE and no in-flight operation is
/// silently cancelled or omitted from the audit count.
#[derive(Debug)]
struct DrainedDeleteOutcome {
    succeeded: Vec<clw_types::Digest>,
    failure: Option<(usize, clw_types::Digest, ClwError)>,
}

/// Clamp the global CLI knob so `--concurrency 0` cannot stall a destructive
/// sweep. This exactly mirrors snapshot/hydrate's established contract.
fn destructive_sweep_concurrency(requested: usize) -> usize {
    requested.max(1)
}

/// Keep only the first appearance of each target. The list endpoint's order is
/// our input order for deterministic failure selection, so do not sort here.
/// Deduplication happens before every phase (including every verify pass) so a
/// duplicate listing can neither issue a duplicate DELETE nor inflate a count.
fn stable_dedupe_targets(targets: Vec<clw_types::Digest>) -> Vec<clw_types::Digest> {
    let mut seen = std::collections::HashSet::with_capacity(targets.len());
    targets
        .into_iter()
        .filter(|digest| seen.insert(*digest))
        .collect()
}

async fn await_delete<Fut>(
    index: usize,
    digest: clw_types::Digest,
    delete: Fut,
) -> (usize, clw_types::Digest, Result<(), ClwError>)
where
    Fut: Future<Output = Result<(), ClwError>>,
{
    (index, digest, delete.await)
}

/// Execute one destructive phase with bounded concurrency and drain semantics.
///
/// A failure stops *new* submissions but never drops work already handed to the
/// transport. This is deliberately not `try_buffer_unordered`: that adaptor
/// returns early and drops in-flight DELETE futures, leaving their outcome
/// unknown. Callers must inspect the returned failure before starting a later
/// phase; in particular, erase never starts a blob DELETE until this has fully
/// drained and the refs phase succeeded.
async fn delete_bounded_draining<F, Fut>(
    targets: &[clw_types::Digest],
    concurrency: usize,
    delete: F,
) -> DrainedDeleteOutcome
where
    F: Fn(clw_types::Digest) -> Fut,
    Fut: Future<Output = Result<(), ClwError>>,
{
    let concurrency = destructive_sweep_concurrency(concurrency);
    let mut next = 0usize;
    let mut in_flight = FuturesUnordered::new();
    while next < targets.len() && in_flight.len() < concurrency {
        let index = next;
        let digest = targets[index];
        in_flight.push(await_delete(index, digest, delete(digest)));
        next += 1;
    }

    let mut succeeded = Vec::with_capacity(targets.len());
    let mut failure: Option<(usize, clw_types::Digest, ClwError)> = None;
    while let Some((index, digest, result)) = in_flight.next().await {
        match result {
            Ok(()) => succeeded.push(digest),
            Err(error) => {
                if failure
                    .as_ref()
                    .is_none_or(|(lowest_index, _, _)| index < *lowest_index)
                {
                    failure = Some((index, digest, error));
                }
            }
        }

        // Do not start a target after the first observed error. The loop still
        // drains every future that was already in flight above.
        if failure.is_none() && next < targets.len() {
            let index = next;
            let digest = targets[index];
            in_flight.push(await_delete(index, digest, delete(digest)));
            next += 1;
        }
    }

    DrainedDeleteOutcome { succeeded, failure }
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
    concurrency: usize,
) -> Result<EraseOutcome, MidDeleteError>
where
    C: AcTransport + CasTransport,
{
    const MAX_PASSES: usize = 3;
    let concurrency = destructive_sweep_concurrency(concurrency);
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
        // PHASE BARRIER: this helper drains every started ref DELETE and returns
        // only after each has a known result. A failed refs phase returns below,
        // before even one blob DELETE can be submitted.
        let refs = delete_bounded_draining(&pending_refs, concurrency, |key| async move {
            AcTransport::delete(client, &key)
                .await
                .map_err(map_auth_error)
        })
        .await;
        deleted_refs.extend(refs.succeeded);
        if let Some((_, _, source)) = refs.failure {
            return Err(MidDeleteError {
                refs_deleted: deleted_refs.len(),
                blobs_deleted: deleted_blobs.len(),
                source,
            });
        }

        // This starts only after the successful, fully drained refs phase above.
        let blobs = delete_bounded_draining(&pending_blobs, concurrency, |digest| async move {
            CasTransport::delete(client, &digest)
                .await
                .map_err(map_auth_error)
        })
        .await;
        deleted_blobs.extend(blobs.succeeded);
        if let Some((_, _, source)) = blobs.failure {
            return Err(MidDeleteError {
                refs_deleted: deleted_refs.len(),
                blobs_deleted: deleted_blobs.len(),
                source,
            });
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
        // Enumeration can contain duplicate rows during eventual consistency.
        // Preserve its first-seen order but never submit/count a duplicate target.
        // This includes the final bounded-pass report: it must say how many
        // DISTINCT objects remain, rather than how many rows a faulty listing
        // happened to return.
        let next_refs = stable_dedupe_targets(r);
        let next_blobs = stable_dedupe_targets(b);
        if (next_refs.is_empty() && next_blobs.is_empty()) || pass >= MAX_PASSES {
            break (next_refs.len(), next_blobs.len());
        }
        pending_refs = next_refs;
        pending_blobs = next_blobs;
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
    use clw_types::{Config, Digest, Result as ClwResult};
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::{Duration, Instant};
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

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

        let outcome = erase_with_verification(&client, ref_keys, blobs, 8)
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

        let err = erase_with_verification(&client, ref_keys, blobs, 8)
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

    // -----------------------------------------------------------------------
    // WP-27 destructive-sweep harness
    // -----------------------------------------------------------------------

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DeleteEventKind {
        RefStarted,
        RefFinished,
        BlobStarted,
        BlobFinished,
    }

    #[derive(Clone, Debug)]
    struct DeleteEvent {
        kind: DeleteEventKind,
        digest: Digest,
        sequence: usize,
        at: Instant,
    }

    /// Delayed transport for exercising the real erase phase barrier. It records
    /// a monotonically ordered event log as well as wall-clock instants, so this
    /// test cannot be satisfied by a merely coincidental completion ordering.
    #[derive(Clone, Default)]
    struct DelayedPhaseTransport {
        events: Arc<Mutex<Vec<DeleteEvent>>>,
        next_sequence: Arc<AtomicUsize>,
        refs_in_flight: Arc<AtomicUsize>,
        blobs_in_flight: Arc<AtomicUsize>,
        max_refs_in_flight: Arc<AtomicUsize>,
        max_blobs_in_flight: Arc<AtomicUsize>,
        failing_ref: Arc<Mutex<Option<Digest>>>,
    }

    impl DelayedPhaseTransport {
        fn record(&self, kind: DeleteEventKind, digest: Digest) {
            self.events.lock().unwrap().push(DeleteEvent {
                kind,
                digest,
                sequence: self.next_sequence.fetch_add(1, Ordering::SeqCst),
                at: Instant::now(),
            });
        }
    }

    #[async_trait::async_trait]
    impl AcTransport for DelayedPhaseTransport {
        async fn get(&self, _key: &Digest) -> ClwResult<Option<Vec<u8>>> {
            Ok(None)
        }
        async fn put(&self, _key: &Digest, _value: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, key: &Digest) -> ClwResult<()> {
            let in_flight = self.refs_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_refs_in_flight
                .fetch_max(in_flight, Ordering::SeqCst);
            self.record(DeleteEventKind::RefStarted, *key);
            tokio::time::sleep(Duration::from_millis(2)).await;
            self.refs_in_flight.fetch_sub(1, Ordering::SeqCst);
            self.record(DeleteEventKind::RefFinished, *key);
            if *self.failing_ref.lock().unwrap() == Some(*key) {
                Err(ClwError::Transport("delayed ref failure".into()))
            } else {
                Ok(())
            }
        }
        async fn list_refs(&self) -> ClwResult<Vec<Digest>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl CasTransport for DelayedPhaseTransport {
        async fn exists(&self, _digest: &Digest) -> ClwResult<bool> {
            Ok(false)
        }
        async fn get(&self, digest: &Digest) -> ClwResult<Vec<u8>> {
            Err(ClwError::NotFound(*digest))
        }
        async fn put(&self, _digest: &Digest, _bytes: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, digest: &Digest) -> ClwResult<()> {
            let in_flight = self.blobs_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_blobs_in_flight
                .fetch_max(in_flight, Ordering::SeqCst);
            self.record(DeleteEventKind::BlobStarted, *digest);
            tokio::time::sleep(Duration::from_millis(2)).await;
            self.blobs_in_flight.fetch_sub(1, Ordering::SeqCst);
            self.record(DeleteEventKind::BlobFinished, *digest);
            Ok(())
        }
        async fn list_blobs(&self) -> ClwResult<Vec<Digest>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn erase_uses_bounded_phases_and_never_starts_blob_before_refs_drain() {
        let client = DelayedPhaseTransport::default();
        let refs: Vec<Digest> = (0..200u16)
            .map(|i| Digest::of_bytes(&i.to_le_bytes()))
            .collect();
        let blobs: Vec<Digest> = (0..200u16)
            .map(|i| {
                let [lo, hi] = i.to_le_bytes();
                Digest::of_bytes(&[b'b', lo, hi])
            })
            .collect();

        let outcome = erase_with_verification(&client, refs.clone(), blobs.clone(), 16)
            .await
            .expect("all delayed deletes succeed");
        assert_eq!(outcome.refs_deleted, refs.len());
        assert_eq!(outcome.blobs_deleted, blobs.len());
        assert_eq!(
            client.max_refs_in_flight.load(Ordering::SeqCst),
            16,
            "the refs phase must use, but never exceed, N=16"
        );
        assert_eq!(
            client.max_blobs_in_flight.load(Ordering::SeqCst),
            16,
            "the blobs phase must independently use, but never exceed, N=16"
        );

        let events = client.events.lock().unwrap();
        let last_ref = events
            .iter()
            .filter(|event| event.kind == DeleteEventKind::RefFinished)
            .max_by_key(|event| event.sequence)
            .expect("every ref must finish");
        let first_blob = events
            .iter()
            .filter(|event| event.kind == DeleteEventKind::BlobStarted)
            .min_by_key(|event| event.sequence)
            .expect("every blob must start after the refs phase");
        assert!(
            last_ref.sequence < first_blob.sequence && last_ref.at <= first_blob.at,
            "all ref DELETE completions must precede every blob DELETE start; \
             last ref={last_ref:?}, first blob={first_blob:?}"
        );
    }

    #[tokio::test]
    async fn ref_failure_drains_started_refs_and_starts_zero_blobs() {
        let client = DelayedPhaseTransport::default();
        let refs: Vec<Digest> = (0..32u8).map(|i| Digest::of_bytes(&[i])).collect();
        let blobs: Vec<Digest> = (0..32u8).map(|i| Digest::of_bytes(&[b'b', i])).collect();
        *client.failing_ref.lock().unwrap() = Some(refs[0]);

        let error = erase_with_verification(&client, refs, blobs, 16)
            .await
            .expect_err("a ref failure must abort before the blobs phase");
        assert_eq!(error.blobs_deleted, 0);
        assert!(error.source.to_string().contains("delayed ref failure"));

        let events = client.events.lock().unwrap();
        let ref_starts = events
            .iter()
            .filter(|event| event.kind == DeleteEventKind::RefStarted)
            .count();
        let ref_finishes = events
            .iter()
            .filter(|event| event.kind == DeleteEventKind::RefFinished)
            .count();
        assert_eq!(
            ref_starts, ref_finishes,
            "the error path must drain every ref DELETE already started"
        );
        assert!(
            events
                .iter()
                .all(|event| event.kind != DeleteEventKind::BlobStarted),
            "a failed refs phase must submit zero blob DELETEs"
        );
    }

    #[tokio::test]
    async fn erase_matches_the_committed_golden_deletion_set() {
        // This literal set was captured from the serial implementation's fixed
        // fixture before this concurrent implementation replaced it. Sorting the
        // transport log intentionally compares a set, not completion order.
        const GOLDEN: &[&str] = &[
            "0f54d5b220ece7ac1cd64c6e29df6d1681dd471246defe8e0d6cd965e9541e96",
            "24004b57f238810d7347bcd10983a86647dc4dbe35641888a6650101598f4ee7",
            "317203ca06a3514b2d451bec912c0cef5fdbc4da29e3190ca484cb875ff7b82b",
            "6b1db08ed76ad9d9d69bc03e29b6e9d518238bc14467f870b0449b6019d5a5cd",
            "6b6bbc3303eadbef68e901c8834f89a4c78f1fdbac3329b26daf7bc5c16d8457",
            "7131b2368eed74ef4b08db374d795cf05bf42b156c19753f23b8fd14a5566727",
            "9fb881b6606a5ca35449f448b7a148c7ffb76137c0347315175bbad01c56cd03",
            "bde2f620a92985cd675870f455ad73f2878fa88a8758c254a7de1670f758e731",
        ];
        let client = DelayedPhaseTransport::default();
        let refs: Vec<Digest> = (0..4u8)
            .map(|i| Digest::of_bytes(format!("wp27-golden-ref-{i}").as_bytes()))
            .collect();
        let blobs: Vec<Digest> = (0..4u8)
            .map(|i| Digest::of_bytes(format!("wp27-golden-blob-{i}").as_bytes()))
            .collect();

        erase_with_verification(&client, refs, blobs, 16)
            .await
            .expect("golden fixture must erase completely");
        let mut actual: Vec<String> = client
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    DeleteEventKind::RefFinished | DeleteEventKind::BlobFinished
                )
            })
            .map(|event| event.digest.to_hex())
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, GOLDEN);
    }

    #[derive(Clone)]
    struct DelayedWireDeleteResponder {
        delay: Duration,
        active_until: Arc<Mutex<Vec<Instant>>>,
        peak: Arc<AtomicUsize>,
        deleted: Arc<Mutex<Vec<String>>>,
    }

    impl Respond for DelayedWireDeleteResponder {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            // `ResponseTemplate::set_delay` keeps this request live on wiremock's
            // async server. Track overlapping request windows independently so the
            // acceptance test observes the actual HTTP fan-out as well as elapsed
            // time. A completed response cannot trigger its replacement before its
            // delay has elapsed, so expiring windows here never overstates N.
            let now = Instant::now();
            let mut active = self.active_until.lock().unwrap();
            active.retain(|until| *until > now);
            active.push(now + self.delay);
            self.peak.fetch_max(active.len(), Ordering::SeqCst);
            drop(active);

            let digest = request
                .url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .expect("DELETE URL has a digest segment")
                .to_owned();
            self.deleted.lock().unwrap().push(digest);
            ResponseTemplate::new(204).set_delay(self.delay)
        }
    }

    async fn wiremock_500_ref_sweep(
        concurrency: usize,
        targets: &[Digest],
    ) -> (Duration, usize, Vec<String>) {
        const DELAY: Duration = Duration::from_millis(8);
        let server = MockServer::start().await;
        let responder = DelayedWireDeleteResponder {
            delay: DELAY,
            active_until: Arc::new(Mutex::new(Vec::new())),
            peak: Arc::new(AtomicUsize::new(0)),
            deleted: Arc::new(Mutex::new(Vec::new())),
        };
        Mock::given(method("DELETE"))
            .and(path_regex("/v1/ac/bench/[a-f0-9]{64}$"))
            .respond_with(responder.clone())
            .mount(&server)
            .await;
        // `erase_with_verification` re-lists both namespaces before it can make
        // the completed-erasure claim. Both need an empty, valid pagination page.
        Mock::given(method("GET"))
            .and(path("/v1/ac/bench"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "refs": [], "next_cursor": serde_json::Value::Null,
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/cas/bench"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "blobs": [], "next_cursor": serde_json::Value::Null,
            })))
            .mount(&server)
            .await;

        let client = HttpClient::with_concurrency(
            Config {
                endpoint: server.uri(),
                tenant: "bench".into(),
                token: "bench-token".into(),
                cache_dir: PathBuf::from("/nonexistent/wp27-wiremock-cache"),
            },
            concurrency,
        )
        .expect("loopback WireMock endpoint is an allowed client endpoint");
        let start = Instant::now();
        let result = erase_with_verification(&client, targets.to_vec(), Vec::new(), concurrency)
            .await
            .expect("all delayed WireMock DELETEs return 204");
        assert_eq!(result.refs_deleted, targets.len());
        assert_eq!(result.blobs_deleted, 0);
        let elapsed = start.elapsed();
        let peak = responder.peak.load(Ordering::SeqCst);
        let deleted = responder.deleted.lock().unwrap().clone();
        (elapsed, peak, deleted)
    }

    #[tokio::test]
    async fn wiremock_500_ref_sweep_is_bounded_and_at_least_four_times_faster_at_n16() {
        // Reproducible end-to-end scheduler benchmark: same 500 unique HTTP
        // DELETEs, same delayed WireMock responder, only N differs. This exercises
        // both `delete_bounded_draining` and the real HttpClient governor.
        let targets: Vec<Digest> = (0..500u16)
            .map(|i| Digest::of_bytes(&i.to_le_bytes()))
            .collect();
        let mut golden: Vec<String> = targets.iter().map(Digest::to_hex).collect();
        golden.sort_unstable();

        let (n1_elapsed, n1_peak, mut n1_deleted) = wiremock_500_ref_sweep(1, &targets).await;
        let (n16_elapsed, n16_peak, mut n16_deleted) = wiremock_500_ref_sweep(16, &targets).await;
        n1_deleted.sort_unstable();
        n16_deleted.sort_unstable();
        assert_eq!(n1_deleted, golden, "N=1 must delete the golden input set");
        assert_eq!(n16_deleted, golden, "N=16 must delete the golden input set");
        assert_eq!(n1_peak, 1, "N=1 baseline must remain serial on the wire");
        assert!(
            n16_peak > 1 && n16_peak <= 16,
            "WireMock observed invalid N=16 fan-out: peak={n16_peak}"
        );
        assert!(
            n16_elapsed.saturating_mul(4) <= n1_elapsed,
            "WireMock N=16 regressed below 4x: N=1={n1_elapsed:?}, N=16={n16_elapsed:?}"
        );
    }

    #[test]
    fn destructive_sweep_concurrency_clamps_zero_and_keeps_explicit_n() {
        assert_eq!(destructive_sweep_concurrency(0), 1);
        assert_eq!(destructive_sweep_concurrency(1), 1);
        assert_eq!(destructive_sweep_concurrency(16), 16);
    }

    #[test]
    fn stable_dedupe_targets_preserves_first_seen_input_order() {
        let first = Digest::of_bytes(b"first");
        let second = Digest::of_bytes(b"second");
        let third = Digest::of_bytes(b"third");
        assert_eq!(
            stable_dedupe_targets(vec![second, first, second, third, first]),
            vec![second, first, third]
        );
    }

    // A server whose enumeration NEVER converges to empty: every `list_refs`/
    // `list_blobs` keeps returning a "ghost" the delete can't clear (a server-side
    // enumeration fault, or a concurrent writer re-creating data). This is the
    // adversarial input behind the GDPR Art.17 truthfulness guarantee.
    struct NeverEmptiesTransport {
        ghost_ref: Digest,
        ghost_blob: Digest,
        ref_deletes: Mutex<usize>,
        blob_deletes: Mutex<usize>,
    }
    impl NeverEmptiesTransport {
        fn new() -> Self {
            Self {
                ghost_ref: Digest::of_bytes(b"stuck-ref"),
                ghost_blob: Digest::of_bytes(b"stuck-blob"),
                ref_deletes: Mutex::new(0),
                blob_deletes: Mutex::new(0),
            }
        }
    }
    #[async_trait::async_trait]
    impl AcTransport for NeverEmptiesTransport {
        async fn get(&self, _k: &Digest) -> ClwResult<Option<Vec<u8>>> {
            Ok(None)
        }
        async fn put(&self, _k: &Digest, _v: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, _k: &Digest) -> ClwResult<()> {
            *self.ref_deletes.lock().unwrap() += 1;
            Ok(())
        }
        async fn list_refs(&self) -> ClwResult<Vec<Digest>> {
            // A duplicated final enumeration row must count as one remaining
            // distinct ref in the incomplete-erasure report.
            Ok(vec![self.ghost_ref, self.ghost_ref]) // never empties
        }
    }
    #[async_trait::async_trait]
    impl CasTransport for NeverEmptiesTransport {
        async fn exists(&self, _d: &Digest) -> ClwResult<bool> {
            Ok(false)
        }
        async fn get(&self, d: &Digest) -> ClwResult<Vec<u8>> {
            Err(ClwError::NotFound(*d))
        }
        async fn put(&self, _d: &Digest, _b: Vec<u8>) -> ClwResult<()> {
            Ok(())
        }
        async fn delete(&self, _d: &Digest) -> ClwResult<()> {
            *self.blob_deletes.lock().unwrap() += 1;
            Ok(())
        }
        async fn list_blobs(&self) -> ClwResult<Vec<Digest>> {
            // Ditto for CAS blobs: the final count is an object count, not a
            // raw response-row count.
            Ok(vec![self.ghost_blob, self.ghost_blob]) // never empties
        }
    }

    // GDPR Art.17 guarantee (the detection half of "fail loud if not empty"): when
    // enumeration never converges to empty, `erase_with_verification` must NOT spin
    // forever — it bounds at MAX_PASSES and reports the tenant is STILL NOT empty.
    // That non-empty outcome is exactly what makes `run()` fail loud (exit non-zero,
    // `complete:false`) instead of recording a completed erasure while PII remains.
    #[tokio::test]
    async fn erase_final_duplicate_enumeration_reports_distinct_remaining_counts() {
        let client = NeverEmptiesTransport::new();
        let ref_keys = vec![client.ghost_ref];
        let blobs = vec![client.ghost_blob];

        let outcome = erase_with_verification(&client, ref_keys, blobs, 8)
            .await
            .expect("a never-empty enumeration is not a transport error — it verifies non-empty");

        // Bounded: exactly MAX_PASSES (3) passes — it does not loop forever.
        assert_eq!(
            outcome.passes, 3,
            "the verify loop must bound at MAX_PASSES on a never-converging store, not spin"
        );
        // Reports the DISTINCT remaining objects, not duplicated response rows
        // → this is what drives run()'s loud, truthful INCOMPLETE error.
        assert_eq!(
            outcome.remaining_refs, 1,
            "duplicate final enumeration rows must not inflate remaining refs"
        );
        assert_eq!(
            outcome.remaining_blobs, 1,
            "duplicate final enumeration rows must not inflate remaining blobs"
        );
        // Deletes were attempted every pass (idempotently), never silently skipped.
        assert!(*client.ref_deletes.lock().unwrap() >= 3);
        assert!(*client.blob_deletes.lock().unwrap() >= 3);
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
