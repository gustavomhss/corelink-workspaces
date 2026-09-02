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
use std::future::Future;

use clap::Args;
use clw_client::HttpClient;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Entry, Manifest, RefRecord};
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::helpers::{capability_unavailable, is_capability_unavailable};
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

pub async fn run(
    args: PruneArgs,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
) -> Result<(), ClwError> {
    // Keep the scheduler and `HttpClient` governor at the same nonzero global
    // limit. `HttpClient::new` would silently start at eight, so it is unsuitable
    // when the caller explicitly requested another N. The governor may still
    // adapt downward on 429 responses.
    let concurrency = destructive_sweep_concurrency(concurrency);
    let client = HttpClient::with_concurrency(cfg.clw_config, concurrency)?;

    // ORDER MATTERS for GC correctness. Snapshot the blob CANDIDATE set FIRST
    // (at T0), THEN compute the live set (refs read at T1 > T0). With candidates
    // frozen before the mark, any blob that becomes live in the gap is reflected
    // in `live` (so it is never orphaned), and any blob written after T0 is simply
    // not a candidate. The reverse order (mark, then list blobs) would sweep a
    // concurrently-pushed ref's blobs as orphans — live-data loss.
    //
    // RESIDUAL (documented, not closed client-side): one skew case can still turn
    // a live blob into a false orphan, bounded by dry-run-by-default and the
    // store's last-write-wins semantics:
    //   1. A push whose blobs were already in CAS before T0 but whose ref write
    //      lands AFTER the mark read can still have those blobs swept.
    // The M10 case — a ref whose record GETs 404 mid-mark (AC replica-lag /
    // read-after-write skew, the ref exists but a lagging replica answered 404) —
    // is NO LONGER a residual: `compute_live_set` now ABORTS the whole prune with
    // a `live_set_incomplete` error rather than skipping the ref (which would have
    // swept its still-referenced blobs as false orphans under `--yes`). We cannot
    // read the ref, so we cannot know which blobs it protects; the only safe action
    // is to refuse to sweep and re-run when reads are consistent.
    // The fully-correct fix for residual (1) is a server-side GC lease/epoch (a
    // corelink-server item). Until then `prune` is dry-run-by-default: run it in a
    // low-write window, review the dry-run first, and rely on last-write-wins (a
    // concurrent re-push restores any blob a prune raced) to bound the exposure.
    let all_blobs = stable_dedupe_targets(match CasTransport::list_blobs(&client).await {
        Ok(b) => b,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "prune (CAS GC)", "D-8")
        }
        Err(e) => return Err(map_auth_error(e)),
    });
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

    // --- DELETE: bounded, drain-on-error, and audit-truthful ---------------
    // `try_buffer_unordered` would drop in-flight DELETE futures on the first
    // error. Instead this helper stops new submissions, drains every operation
    // already submitted, and reports only confirmed successes. `failed_at` is
    // chosen by the stable orphan input index rather than racing completion order.
    let client_ref = &client;
    let sweep = delete_bounded_draining(&orphans, concurrency, |digest| {
        let client = client_ref;
        async move {
            CasTransport::delete(client, &digest)
                .await
                .map_err(map_auth_error)
        }
    })
    .await;
    let deleted = sweep.succeeded.len();
    let failure = sweep.failure;

    if let Some((_, digest, err)) = failure {
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

/// Result of a bounded orphan-delete phase after it has drained. A failure is
/// deterministic by lowest target input index, even when a later target's error
/// reaches the client first.
#[derive(Debug)]
struct DrainedDeleteOutcome {
    succeeded: Vec<Digest>,
    failure: Option<(usize, Digest, ClwError)>,
}

/// Clamp the global CLI knob so `--concurrency 0` cannot stall a destructive
/// sweep. This exactly mirrors snapshot/hydrate's established contract.
fn destructive_sweep_concurrency(requested: usize) -> usize {
    requested.max(1)
}

/// Keep the list endpoint's first-seen order (the stable input order used to
/// choose `failed_at`) while ensuring a repeated listing row cannot issue two
/// DELETEs or inflate the reported confirmed-success count.
fn stable_dedupe_targets(targets: Vec<Digest>) -> Vec<Digest> {
    let mut seen = HashSet::with_capacity(targets.len());
    targets
        .into_iter()
        .filter(|digest| seen.insert(*digest))
        .collect()
}

async fn await_delete<Fut>(
    index: usize,
    digest: Digest,
    delete: Fut,
) -> (usize, Digest, Result<(), ClwError>)
where
    Fut: Future<Output = Result<(), ClwError>>,
{
    (index, digest, delete.await)
}

/// Run DELETEs with at most `concurrency` operations in flight. On the first
/// observed error it schedules no additional targets but
/// drains the operations already sent to the transport. That preserves truthful
/// partial counts and makes every result known before we return.
async fn delete_bounded_draining<F, Fut>(
    targets: &[Digest],
    concurrency: usize,
    delete: F,
) -> DrainedDeleteOutcome
where
    F: Fn(Digest) -> Fut,
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
    let mut failure: Option<(usize, Digest, ClwError)> = None;
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
        if failure.is_none() && next < targets.len() {
            let index = next;
            let digest = targets[index];
            in_flight.push(await_delete(index, digest, delete(digest)));
            next += 1;
        }
    }

    DrainedDeleteOutcome { succeeded, failure }
}

/// MARK phase: every digest reachable from a ref — the manifest roots plus every
/// chunk those manifests reference. Anything in CAS outside this set is garbage.
///
/// FAIL-SAFE: if ANY ref cannot be resolved to a parsed manifest (its record is
/// unreadable/corrupt, its manifest blob is missing, or either fails to parse),
/// the live set is INCOMPLETE — proceeding would sweep that ref's live blobs as
/// false orphans. We therefore ABORT the whole mark with a `live_set_incomplete`
/// error that NAMES the offending ref, rather than returning a partial set. A ref
/// that 404s on its record GET mid-walk ALSO aborts (M10): a 404 makes the live
/// set NOT provably complete, and we cannot read the ref to learn which blobs it
/// protects. The 404 is USUALLY a benign concurrent `rm`, but it can ALSO be AC
/// replica-lag / read-after-write skew (the ref still exists; a lagging replica
/// answered 404) — in which case SKIPPING it would sweep its still-referenced
/// blobs as false orphans under `--yes` (live-data loss). Because we cannot tell
/// the two apart from a 404 alone, the fail-safe action is to refuse to sweep and
/// re-run when reads are consistent. See the RESIDUAL note in `run`.
async fn compute_live_set<C: AcTransport + CasTransport>(
    client: &C,
) -> Result<HashSet<Digest>, ClwError> {
    let mut live = HashSet::new();

    for key in AcTransport::list_refs(client).await? {
        // FAIL-SAFE (M10): a ref that `list_refs` enumerated but whose record GET
        // returns 404 mid-mark makes the computed live set NOT provably complete.
        // We CANNOT read the ref, so we cannot know which blobs it protects. Under
        // read-after-write / AC replica lag the ref STILL EXISTS on the primary (a
        // lagging replica answered 404), so skipping it would omit its still-
        // referenced blobs from the live set and `prune --yes` would sweep them as
        // false orphans — live-data loss. The only safe action is to refuse to
        // sweep: ABORT the whole mark (deletes NOTHING) and name the offending ref.
        let Some(bytes) = AcTransport::get(client, &key).await? else {
            return Err(live_set_incomplete_lagging_replica(&key.to_hex()));
        };
        // From here on, ANY failure to resolve this ref to a parsed manifest
        // means the live set is incomplete: abort and NAME the ref. Pruning on a
        // partial live set would delete live blobs as false orphans.
        //
        // EXCEPT: `clw run` memo records (RunRecords) legitimately share this AC
        // keyspace, so they DON'T parse as a RefRecord — but they are NOT corrupt.
        // A RunRecord points at its captured stdout/stderr CAS blobs, which are
        // run-cache LIVE data: insert both into the live set and skip on. Treating
        // a RunRecord as corruption would abort prune entirely; SKIPPING it without
        // marking its blobs would orphan them, so `prune --yes` would DELETE the run
        // cache's stdout/stderr blobs (cache corruption). Only bytes that are NEITHER
        // a RefRecord NOR a RunRecord are genuinely corrupt.
        let record = match RefRecord::from_bytes(&bytes) {
            Ok(record) => record,
            Err(e) => {
                if let Some((stdout, stderr)) = clw_run::run_record_blobs(&bytes) {
                    live.insert(stdout);
                    live.insert(stderr);
                    continue;
                }
                return Err(live_set_incomplete(
                    &key.to_hex(),
                    &format!("ref record is corrupt ({e})"),
                ));
            }
        };

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
    ClwError::Message(format!(
        "refusing to prune: the live set is incomplete — ref '{ref_label}' {why}, so the set of \
         reachable blobs cannot be computed in full. Pruning now would delete live blobs as false \
         orphans. NOTHING was deleted. Repair or remove the offending ref (e.g. `clw rm --name \
         <NAME>` if it is dangling) and re-run prune."
    ))
}

/// Build the fail-safe abort (M10) for a ref that `list_refs` enumerated but
/// whose record GET returned 404 mid-mark. Under read-after-write / AC replica
/// lag the ref still exists on the primary while a lagging replica answers 404,
/// so its still-referenced blobs must NOT be swept as false orphans. Since the
/// ref is unreadable we cannot know which blobs it protects — the only safe
/// action is to refuse to sweep and re-run when reads are consistent. Carries the
/// `live set is incomplete` token so the caller's fail-safe branch catches it and
/// deletes nothing.
fn live_set_incomplete_lagging_replica(ref_label: &str) -> ClwError {
    ClwError::Message(format!(
        "refusing to prune: the live set is incomplete — ref '{ref_label}' read returned 404 \
         mid-mark, so the set of reachable blobs cannot be computed in full. A lagging replica \
         may still reference its blobs (read-after-write skew), which would then be swept as \
         false orphans. NOTHING was deleted. Re-run prune when reads are consistent."
    ))
}

/// True if the error is the fail-safe incomplete-live-set abort raised by
/// [`compute_live_set`] for an unresolvable ref.
fn is_live_set_incomplete(err: &ClwError) -> bool {
    matches!(err, ClwError::Message(msg) if msg.contains("live set is incomplete"))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{ChunkRef, Manifest, Result as ClwResult, MANIFEST_VERSION};
    use std::collections::HashMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // In-memory fake transport for driving the REAL `compute_live_set`.
    // -----------------------------------------------------------------------
    #[derive(Clone, Default)]
    struct FakeStore {
        cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        ac: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
        // Ref keys that `list_refs` REPORTS but that `AcTransport::get` 404s on
        // (they are absent from `ac`) — simulates read-after-write / AC replica
        // lag: the ref was enumerated on the primary but a lagging replica answers
        // 404 on its record GET.
        lagging_ref_keys: Arc<Mutex<Vec<Digest>>>,
    }

    #[async_trait::async_trait]
    impl CasTransport for FakeStore {
        async fn exists(&self, digest: &Digest) -> ClwResult<bool> {
            Ok(self.cas.lock().unwrap().contains_key(digest))
        }
        async fn get(&self, digest: &Digest) -> ClwResult<Vec<u8>> {
            self.cas
                .lock()
                .unwrap()
                .get(digest)
                .cloned()
                .ok_or(ClwError::NotFound(*digest))
        }
        async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> ClwResult<()> {
            self.cas.lock().unwrap().insert(*digest, bytes);
            Ok(())
        }
        async fn delete(&self, digest: &Digest) -> ClwResult<()> {
            self.cas.lock().unwrap().remove(digest);
            Ok(())
        }
        async fn list_blobs(&self) -> ClwResult<Vec<Digest>> {
            Ok(self.cas.lock().unwrap().keys().copied().collect())
        }
    }

    #[async_trait::async_trait]
    impl AcTransport for FakeStore {
        async fn get(&self, key: &Digest) -> ClwResult<Option<Vec<u8>>> {
            Ok(self.ac.lock().unwrap().get(key).cloned())
        }
        async fn put(&self, key: &Digest, value: Vec<u8>) -> ClwResult<()> {
            self.ac.lock().unwrap().insert(*key, value);
            Ok(())
        }
        async fn delete(&self, key: &Digest) -> ClwResult<()> {
            self.ac.lock().unwrap().remove(key);
            Ok(())
        }
        async fn list_refs(&self) -> ClwResult<Vec<Digest>> {
            let mut keys: Vec<Digest> = self.ac.lock().unwrap().keys().copied().collect();
            // Lagging-replica keys are enumerated but NOT in `ac`, so a subsequent
            // `get` 404s (returns None) — the read-after-write skew of M10.
            keys.extend(self.lagging_ref_keys.lock().unwrap().iter().copied());
            Ok(keys)
        }
    }

    // FIX #2 — a RunRecord shares the AC keyspace with refs. `compute_live_set`
    // must (a) NOT abort prune on the RunRecord (it is not corruption), and (b)
    // mark the RunRecord's stdout/stderr CAS blobs as LIVE so a `prune --yes`
    // never orphans + deletes the run cache. We plant BOTH a real workspace ref
    // (manifest root + a file chunk) AND a RunRecord (whose stdout/stderr blobs
    // are in CAS), drive the REAL `compute_live_set`, and assert: it does not
    // abort, and the run-cache blobs are in the live set (so NOT would-be orphans).
    #[tokio::test]
    async fn run_records_do_not_abort_prune_and_their_blobs_stay_live() {
        let store = FakeStore::default();

        // --- A real workspace ref: a file chunk + a manifest root in CAS. ---
        let chunk = Digest::of_bytes(b"a live file chunk");
        CasTransport::put(&store, &chunk, b"a live file chunk".to_vec())
            .await
            .unwrap();
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: 17,
            entries: vec![Entry::File {
                path: "f.txt".into(),
                mode: 0o644,
                size: 17,
                chunks: vec![ChunkRef {
                    digest: chunk,
                    size: 17,
                }],
            }],
        };
        let manifest_bytes = manifest.canonical_bytes().unwrap();
        let root = Digest::of_bytes(&manifest_bytes);
        CasTransport::put(&store, &root, manifest_bytes)
            .await
            .unwrap();
        let ref_rec = RefRecord {
            name: "ws".into(),
            root,
            manifest_size: 1,
            created_at_unix: 0,
            parent: None,
            tool_version: "t".into(),
        };
        AcTransport::put(
            &store,
            &Digest::of_bytes(b"refkey"),
            ref_rec.to_bytes().unwrap(),
        )
        .await
        .unwrap();

        // --- A `clw run` memo record sharing the AC keyspace. Its stdout/stderr
        // blobs live in CAS and MUST be treated as live (never orphaned). ---
        let out_blob = Digest::of_bytes(b"captured stdout");
        let err_blob = Digest::of_bytes(b"captured stderr");
        CasTransport::put(&store, &out_blob, b"captured stdout".to_vec())
            .await
            .unwrap();
        CasTransport::put(&store, &err_blob, b"captured stderr".to_vec())
            .await
            .unwrap();
        // Build the RunRecord bytes via clw-run's own serializer surrogate: it is a
        // JSON object {exit_code, stdout, stderr}. We synthesize the exact wire form
        // and confirm clw_run recognizes it.
        let run_bytes = format!(
            r#"{{"exit_code":0,"stdout":"{}","stderr":"{}"}}"#,
            out_blob.to_hex(),
            err_blob.to_hex()
        )
        .into_bytes();
        assert!(
            clw_run::run_record_blobs(&run_bytes).is_some(),
            "the synthesized RunRecord must be recognized by clw-run"
        );
        AcTransport::put(&store, &Digest::of_bytes(b"runkey"), run_bytes)
            .await
            .unwrap();

        // --- Drive the REAL mark phase. It must NOT abort on the RunRecord. ---
        let live = compute_live_set(&store)
            .await
            .expect("a RunRecord must not abort prune as 'live set incomplete'");

        // The ref's manifest root + chunk are live (the normal path).
        assert!(live.contains(&root), "manifest root must be live");
        assert!(live.contains(&chunk), "the file chunk must be live");
        // The crux: the run-cache stdout/stderr blobs are LIVE — so the orphan set
        // (all_blobs - live) excludes them and `prune --yes` would NOT delete them.
        assert!(
            live.contains(&out_blob),
            "RunRecord stdout blob must be live (never orphaned)"
        );
        assert!(
            live.contains(&err_blob),
            "RunRecord stderr blob must be live (never orphaned)"
        );

        // Concretely: with this live set, no run-cache blob appears as an orphan.
        let all_blobs = CasTransport::list_blobs(&store).await.unwrap();
        let orphans: Vec<Digest> = all_blobs
            .into_iter()
            .filter(|d| !live.contains(d))
            .collect();
        assert!(
            !orphans.contains(&out_blob) && !orphans.contains(&err_blob),
            "run-cache blobs must NOT be in the would-delete/orphan set"
        );
    }

    // A genuinely-corrupt AC entry (neither RefRecord nor RunRecord) still aborts.
    #[tokio::test]
    async fn genuinely_corrupt_ac_entry_still_aborts_prune() {
        let store = FakeStore::default();
        AcTransport::put(
            &store,
            &Digest::of_bytes(b"bad"),
            b"{not a record}".to_vec(),
        )
        .await
        .unwrap();
        let err = compute_live_set(&store).await.unwrap_err();
        assert!(
            is_live_set_incomplete(&err),
            "neither-RefRecord-nor-RunRecord bytes must abort as incomplete, got: {err:?}"
        );
    }

    // M10 — a ref that `list_refs` enumerated but whose record GET 404s mid-mark
    // (AC replica lag / read-after-write skew) must ABORT the live-set mark, NOT be
    // silently skipped. Skipping omits the ref's still-referenced blobs from the
    // live set, so they appear as orphans and `prune --yes` DELETES them — live-data
    // loss. Since the ref is unreadable we cannot know which blobs it protects, so
    // the only safe action is to refuse to sweep. RED-on-revert: restoring the old
    // `continue` skip makes `compute_live_set` return Ok (a partial live set), so
    // `expect_err` below fails; and `run` reaches its delete loop and sweeps the
    // protected blob.
    #[tokio::test]
    async fn lagging_replica_404_ref_aborts_prune_and_deletes_nothing() {
        let store = FakeStore::default();

        // A blob a still-live-but-lagging ref would protect. Under the old skip it
        // is absent from the live set → an orphan → `prune --yes` would delete it.
        let protected = Digest::of_bytes(b"still-referenced by a lagging ref");
        CasTransport::put(
            &store,
            &protected,
            b"still-referenced by a lagging ref".to_vec(),
        )
        .await
        .unwrap();

        // The ref EXISTS on the primary (list_refs reports it) but a lagging replica
        // answers 404 on its record GET (absent from `ac`).
        let lagging_key = Digest::of_bytes(b"lagging-ref-key");
        store.lagging_ref_keys.lock().unwrap().push(lagging_key);

        // Drive the REAL mark phase. A 404 mid-mark must ABORT, not return a partial
        // (skipped) live set.
        let err = compute_live_set(&store)
            .await
            .expect_err("a 404 mid-mark must abort as 'live set incomplete', not skip");

        // It is the fail-safe abort the caller catches to delete NOTHING: `run` only
        // reaches its delete loop when `compute_live_set` returns Ok, so this Err
        // means `prune --yes` deletes no blob.
        assert!(
            is_live_set_incomplete(&err),
            "404 mid-mark must abort as incomplete, got: {err:?}"
        );

        // The error is actionable: names the ref, flags the lagging-replica cause,
        // the 404, that nothing was deleted, and the re-run guidance.
        let msg = err.to_string();
        assert!(
            msg.contains(&lagging_key.to_hex()),
            "must name the offending ref: {msg}"
        );
        assert!(msg.contains("404"), "must state the 404: {msg}");
        assert!(
            msg.to_lowercase().contains("replica"),
            "must flag the lagging-replica cause: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("nothing was deleted"),
            "must state nothing was deleted: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("re-run"),
            "must give re-run guidance: {msg}"
        );

        // Concretely, no blob was swept: `compute_live_set` does not delete, and its
        // abort stops `run` before the delete loop, so the protected blob survives.
        assert!(
            CasTransport::exists(&store, &protected).await.unwrap(),
            "the protected blob must NOT be deleted under replica lag"
        );
    }

    #[tokio::test]
    async fn orphan_sweep_drains_mixed_failures_deterministically_for_50_runs() {
        // The later failure finishes first on purpose. Every run must still name
        // the lower input index, count only confirmed successes, drain all 16
        // initially submitted deletes, and never exceed the bound.
        let targets: Vec<Digest> = (0..32u8).map(|i| Digest::of_bytes(&[i])).collect();
        for attempt in 0..50 {
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(AtomicUsize::new(0));
            let completed = Arc::new(AtomicUsize::new(0));
            let confirmed = Arc::new(Mutex::new(Vec::new()));
            let low_failure = targets[0];
            let high_failure = targets[1];

            let outcome = delete_bounded_draining(&targets, 16, {
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                let started = Arc::clone(&started);
                let completed = Arc::clone(&completed);
                let confirmed = Arc::clone(&confirmed);
                move |digest| {
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&peak);
                    let started = Arc::clone(&started);
                    let completed = Arc::clone(&completed);
                    let confirmed = Arc::clone(&confirmed);
                    async move {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        started.fetch_add(1, Ordering::SeqCst);
                        // Index 1 fails first; index 0 is deliberately delayed
                        // and must nevertheless win deterministic error selection.
                        let delay = if digest == low_failure {
                            Duration::from_millis(6)
                        } else if digest == high_failure {
                            Duration::from_millis(1)
                        } else {
                            Duration::from_millis(3)
                        };
                        tokio::time::sleep(delay).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        completed.fetch_add(1, Ordering::SeqCst);
                        if digest == low_failure {
                            Err(ClwError::Transport("low-index failure".into()))
                        } else if digest == high_failure {
                            Err(ClwError::Transport("high-index failure".into()))
                        } else {
                            confirmed.lock().unwrap().push(digest);
                            Ok(())
                        }
                    }
                }
            })
            .await;

            let (index, digest, error) = outcome.failure.expect("mixed failures must fail");
            assert_eq!(index, 0, "attempt {attempt} selected the wrong failure");
            assert_eq!(digest, low_failure);
            assert!(error.to_string().contains("low-index failure"));
            assert_eq!(
                completed.load(Ordering::SeqCst),
                16,
                "attempt {attempt} must drain all work already in flight"
            );
            assert_eq!(
                started.load(Ordering::SeqCst),
                16,
                "attempt {attempt} must not submit a replacement after the fast failure"
            );
            assert_eq!(active.load(Ordering::SeqCst), 0);
            assert!(
                peak.load(Ordering::SeqCst) <= 16,
                "attempt {attempt} exceeded the orphan-delete bound"
            );

            let mut reported: Vec<String> = outcome.succeeded.iter().map(Digest::to_hex).collect();
            let mut transport_confirmed: Vec<String> = confirmed
                .lock()
                .unwrap()
                .iter()
                .map(Digest::to_hex)
                .collect();
            reported.sort_unstable();
            transport_confirmed.sort_unstable();
            assert_eq!(
                reported, transport_confirmed,
                "attempt {attempt} must report exactly the deletes that completed successfully"
            );
        }
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
