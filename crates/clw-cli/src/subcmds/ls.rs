//! `clw ls --name <NAME>` — list ref info for one workspace.
//! `clw ls --all`         — enumerate every ref in the tenant.
//!
//! `--all` uses the corelink-server ref enumeration capability, which is LIVE
//! (D-7, conformance-green). The 405/501 branch is a defensive guard: a LIVE
//! capability returning 405/501 is a server-side regression, so the transport
//! surfaces it as an actionable server-regression error rather than a silent
//! empty list.

use clap::Args;
use clw_client::HttpClient;
use clw_types::{ref_key_in, AcTransport, CasTransport, ClwError, Config, Manifest, RefRecord};
use futures::stream::{self, StreamExt};
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::helpers::{capability_unavailable, is_capability_unavailable};
use crate::subcmds::status::{map_auth_error, map_missing_manifest};

// Keep the existing crate-local path used by the JSON contract golden while
// sourcing the builder from the shared generic renderer.
#[cfg(test)]
pub(crate) use crate::subcmds::helpers::build_capability_unavailable_json;

#[derive(Debug, Args)]
#[command(about = "List ref info for a workspace name, or every ref with --all")]
pub struct LsArgs {
    /// Workspace name (ref key). Mutually exclusive with `--all`.
    #[arg(long, conflicts_with = "all", help = "Workspace name to look up")]
    pub name: Option<String>,

    /// Enumerate every ref in the tenant (requires the server enumeration
    /// capability, D-7).
    #[arg(
        long,
        default_value_t = false,
        help = "List every ref in the tenant (instead of a single --name)"
    )]
    pub all: bool,
}

pub async fn run(
    args: LsArgs,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
) -> Result<(), ClwError> {
    run_with_client_factory(args, cfg, json, concurrency, HttpClient::with_concurrency).await
}

/// Production boundary for `ls`: the `--all` branch constructs its client through
/// this injected factory before handing it to the bounded listing dispatcher.
/// Keeping the factory at this boundary makes the CLI selection and dispatcher
/// testable together without a live HTTP server.
async fn run_with_client_factory<C, BuildClient>(
    args: LsArgs,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
    build_client: BuildClient,
) -> Result<(), ClwError>
where
    C: AcTransport,
    BuildClient: FnOnce(Config, usize) -> Result<C, ClwError>,
{
    match (&args.name, args.all) {
        (Some(name), false) => run_one(name, cfg, json).await,
        (None, true) => run_all(cfg.clw_config, json, concurrency, build_client).await,
        // clap's `conflicts_with` already rejects `--name X --all`; this guards
        // the `neither` case with an actionable message instead of a panic.
        // FIX 5: point a bare `clw ls` at `--help` so the user can discover the flags.
        _ => Err(ClwError::Message(
            "specify exactly one of --name <NAME> or --all (run `clw ls --help`)".to_string(),
        )),
    }
}

/// `clw ls --name <NAME>` — the single-workspace lookup (root digest, files, size).
async fn run_one(name: &str, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let client = HttpClient::new(cfg.clw_config)?;

    // Fetch ref record from AC.
    let rk = ref_key_in(cfg.ref_domain, name);
    let ref_bytes = AcTransport::get(&client, &rk)
        .await
        .map_err(map_auth_error)?
        .ok_or_else(|| ClwError::Message(not_found_msg(name, &tenant)))?;
    let record = RefRecord::from_bytes(&ref_bytes)?;

    // Fetch manifest to count files and total size. The ref exists but its
    // manifest blob may be missing (pruned / lost / replica-lag): surface a
    // workspace-NAMED error rather than a bare 64-hex `NotFound(<digest>)`.
    let manifest_bytes = CasTransport::get(&client, &record.root)
        .await
        .map_err(map_auth_error)
        .map_err(|e| map_missing_manifest(e, name, &tenant, &record.root))?;
    let manifest = Manifest::from_bytes(&manifest_bytes)?;

    let file_count = manifest
        .entries
        .iter()
        .filter(|e| matches!(e, clw_types::Entry::File { .. }))
        .count();

    let parent_str = record
        .parent
        .map_or_else(|| "none".to_string(), |d| d.to_hex());

    if json {
        let out = build_one_json(&record, file_count, manifest.total_size);
        println!("{out}");
    } else {
        println!("name:         {}", record.name);
        println!("root:         {}", record.root);
        // FIX 1: render a human UTC datetime; the raw epoch stays in `--json` only.
        println!("created_at:   {}", format_unix_utc(record.created_at_unix));
        println!("parent:       {parent_str}");
        println!("files:        {file_count}");
        println!("bytes_total:  {}", manifest.total_size);
    }

    Ok(())
}

/// `clw ls --all` — enumerate every ref in the tenant.
///
/// `list_refs` returns ref KEYS (one-way `BLAKE3(domain‖name)`), so we fetch each
/// record to recover its human name + root. A key that 404s between enumeration
/// and fetch (a concurrent `rm`) is skipped rather than failing the whole listing.
///
/// TOLERANT BY DESIGN (the opposite of prune's fail-safe): `ls` is READ-ONLY, so a
/// single corrupt/unparseable ref record must NOT blind the operator to every
/// other ref. We SKIP the bad record, emit a `tracing::warn!` that NAMES it (by its
/// ref key, shown with `-v`), record it for the `--json` `errors` array + an
/// end-of-listing summary, and continue — but NOT a per-record stderr line, which
/// would flood a large/degraded tenant. `clw run` memo records (the AC-backed run
/// cache) legitimately share the keyspace; they are recognized via
/// [`clw_run::is_run_record`], counted as `run_cache_entries`, and never warned.
/// Likewise, the tenant AC keyspace is SHARED with native/Bazel ActionCache, whose
/// values are arbitrary NON-JSON bytes; a ref/run record is always JSON, so bytes
/// that fail to parse as JSON at all are a foreign AC value — counted as
/// `foreign_ac_entries` and never warned. Only bytes that ARE valid JSON but do not
/// match the `RefRecord`/`RunRecord` shape are treated as a genuinely corrupt ref.
/// In `--json` each genuinely-corrupt record is a structured `{key, error}` entry
/// under `errors` so a machine caller sees the partial-listing rather than a hard
/// failure. Partial output is strictly better than no output for a listing.
async fn run_all<C, BuildClient>(
    cfg: Config,
    json: bool,
    requested_concurrency: usize,
    build_client: BuildClient,
) -> Result<(), ClwError>
where
    C: AcTransport,
    BuildClient: FnOnce(Config, usize) -> Result<C, ClwError>,
{
    let tenant = cfg.tenant.clone();
    let concurrency = all_fetch_concurrency(requested_concurrency);
    let client = build_client(cfg, concurrency.http_client_limit)?;

    let keys = match AcTransport::list_refs(&client).await {
        Ok(keys) => keys,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "ref enumeration", "D-7")
        }
        Err(e) => return Err(map_auth_error(e)),
    };

    let mut rows = Vec::with_capacity(keys.len());
    // Per-record skips, carried so --json can report them structurally and human
    // mode can summarize. Covers BOTH a corrupt/unparseable record AND a transient
    // per-record TRANSPORT error (a network blip fetching one record) — for a
    // read-only listing, neither should be fatal to the whole command.
    let mut skipped: Vec<(String, String)> = Vec::new();
    // `clw run` memo records (the AC-backed run cache) share this tenant's AC keyspace
    // with workspace refs, so enumeration surfaces them too. They are NOT corrupt — count
    // them separately and never warn about them (they would otherwise flood the listing
    // and grow with every `clw run`).
    let mut run_cache = 0usize;
    // Native/Bazel ActionCache values (arbitrary non-JSON bytes) also share this
    // tenant's AC keyspace. They are NOT corrupt refs — count them separately and
    // stay silent, exactly like run-cache entries.
    let mut foreign = 0usize;

    // Fetch every record to recover its name/root. The keys are INDEPENDENT, so we fetch
    // with BOUNDED CONCURRENCY instead of one-at-a-time. The old sequential loop paid N
    // sequential round-trips — on a real 46-ref tenant that was ~46 back-to-back requests
    // (and with a ~1s/request backend, ~46 s). The client's AIMD Governor still caps the
    // true in-flight count and applies 429 backpressure, so this is a latency win, not a
    // thundering herd. The same user-selected limit is supplied to the HTTP
    // governor and this dispatcher: neither layer may silently widen or narrow
    // `--concurrency`. Results are re-sorted to enumeration order for a
    // deterministic list.

    // Per-key outcome. `Box<RefRecord>` keeps the enum small (clippy large-variant).
    enum Fetched {
        Row(Box<RefRecord>),
        Skip(String, String), // (key_hex, error) — corrupt record OR transient transport blip
        RunCache,             // a `clw run` memo record legitimately sharing the keyspace
        Foreign,              // a non-JSON AC value (native/Bazel ActionCache) sharing the keyspace
        Gone,                 // Ok(None): deleted between enumeration and fetch
        Auth,                 // 401: the WHOLE listing is unauthorized → abort
    }

    let mut indexed: Vec<(usize, Fetched)> = stream::iter(keys.iter().enumerate().map(
        |(i, key)| {
            let client = &client;
            async move {
                let out = match AcTransport::get(client, key).await {
                    Ok(Some(bytes)) => match RefRecord::from_bytes(&bytes) {
                        Ok(record) => Fetched::Row(Box::new(record)),
                        // A `clw run` memo record is NOT corrupt — count it, stay silent.
                        Err(_) if clw_run::is_run_record(&bytes) => {
                            tracing::debug!(ref_key = %key.to_hex(), "skipping run-cache entry in ref listing");
                            Fetched::RunCache
                        }
                        // The tenant AC keyspace is SHARED with native/Bazel ActionCache,
                        // whose values are arbitrary (non-JSON) bytes — NOT corrupt refs.
                        // A ref/run record is always JSON; so bytes that are not valid JSON
                        // at all are a foreign AC value, benign like a run-cache entry: count
                        // it, stay silent (no warn, no --json `errors` line).
                        Err(_) if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() => {
                            tracing::debug!(ref_key = %key.to_hex(), "skipping foreign (non-JSON) AC value in ref listing");
                            Fetched::Foreign
                        }
                        // Valid JSON but NOT a RefRecord/RunRecord shape: a genuinely corrupt
                        // ref — diagnostic warn (shown with -v) + structured --json error +
                        // summary. No per-record stderr flood.
                        Err(e) => {
                            tracing::warn!(ref_key = %key.to_hex(), error = %e, "skipping unparseable ref record");
                            Fetched::Skip(key.to_hex(), e.to_string())
                        }
                    },
                    Ok(None) => Fetched::Gone,
                    Err(ClwError::Auth) => Fetched::Auth,
                    // A transient per-record transport blip is TOLERATED like a corrupt
                    // record (read-only listing must not fail wholesale on one blip).
                    Err(e) => {
                        tracing::warn!(ref_key = %key.to_hex(), error = %e, "skipping ref record after a transport error");
                        Fetched::Skip(key.to_hex(), e.to_string())
                    }
                };
                (i, out)
            }
        },
    ))
    .buffer_unordered(concurrency.fetch_dispatcher_limit)
    .collect()
    .await;

    // Restore enumeration order so the listing is deterministic regardless of the
    // completion order of the concurrent fetches.
    indexed.sort_by_key(|(i, _)| *i);
    for (_, out) in indexed {
        match out {
            Fetched::Row(record) => rows.push(*record),
            Fetched::Skip(key_hex, err) => skipped.push((key_hex, err)),
            Fetched::RunCache => run_cache += 1,
            Fetched::Foreign => foreign += 1,
            Fetched::Gone => {}
            // AUTH (401) means the WHOLE listing is unauthorized: abort with the
            // actionable auth message (the same outcome the sequential loop produced).
            Fetched::Auth => return Err(map_auth_error(ClwError::Auth)),
        }
    }

    if json {
        println!(
            "{}",
            build_all_json(&tenant, &rows, &skipped, run_cache, foreign)
        );
    } else if rows.is_empty() && skipped.is_empty() && run_cache == 0 && foreign == 0 {
        println!("no refs for tenant '{tenant}'");
    } else {
        for r in &rows {
            // FIX 1: human UTC datetime in the table column (epoch kept in --json).
            println!(
                "{}  {}  {}",
                &r.root.to_hex()[..16],
                format_unix_utc(r.created_at_unix),
                r.name
            );
        }
        let mut summary = format!("({} refs in tenant '{tenant}'", rows.len());
        if run_cache > 0 {
            summary.push_str(&format!("; {run_cache} run-cache entries"));
        }
        if foreign > 0 {
            summary.push_str(&format!("; {foreign} non-ref AC entries"));
        }
        if !skipped.is_empty() {
            summary.push_str(&format!(
                "; {} corrupt record(s) skipped — run with -v or --json for details",
                skipped.len()
            ));
        }
        summary.push(')');
        println!("{summary}");
    }

    Ok(())
}

/// The one concurrency decision for `ls --all`'s HTTP-client builder and its
/// bounded record-fetch dispatcher. Keep both fields equal: a different value
/// would make the CLI report one limit while the other layer silently imposes
/// another. Like the other network commands, zero is clamped to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllFetchConcurrency {
    http_client_limit: usize,
    fetch_dispatcher_limit: usize,
}

fn all_fetch_concurrency(requested: usize) -> AllFetchConcurrency {
    let limit = requested.max(1);
    AllFetchConcurrency {
        http_client_limit: limit,
        fetch_dispatcher_limit: limit,
    }
}

/// FIX 4: the workspace-not-found message, shared with hydrate/status. Beyond
/// "snapshot first or check --tenant" it now points at `--ref-domain`: a
/// runner-produced ref lives in the `runner` keyspace, so a user-domain lookup of
/// it (or vice versa) "not found" is most often a ref-domain mismatch, not a
/// missing snapshot.
pub fn not_found_msg(name: &str, tenant: &str) -> String {
    format!(
        "workspace '{name}' not found for tenant '{tenant}' — run 'clw snapshot --name {name}' \
         first or check --tenant — if this is a runner-produced workspace, set \
         CLW_REF_DOMAIN=runner (or --ref-domain runner)"
    )
}

/// FIX 1: render a Unix epoch (seconds) as `YYYY-MM-DD HH:MM:SS UTC` WITHOUT a new
/// crate dependency — chrono/time are not in the dep tree and the machine-facing
/// epoch stays in `--json`, so a hand-rolled civil-from-days is the right weight.
/// Uses Howard Hinnant's `civil_from_days` (days since the 1970-01-01 epoch → Y-M-D).
fn format_unix_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (hour, min, sec) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    // civil_from_days: days-since-epoch → (year, month, day). Shift the epoch to
    // 0000-03-01 (era arithmetic) so leap days fall at the END of each 400-year era.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02} UTC")
}

/// Build the single-reference JSON response.
///
/// `bytes` is retained as the legacy field for the current deprecation cycle;
/// new callers should read `bytes_total`. Keeping this in one production builder
/// lets the versioned JSON-contract golden exercise the exact emitted value.
pub(crate) fn build_one_json(
    record: &RefRecord,
    file_count: usize,
    bytes_total: u64,
) -> serde_json::Value {
    json!({
        "name": record.name,
        "root": record.root.to_hex(),
        "created_at_unix": record.created_at_unix,
        "parent": record.parent.map(|d| d.to_hex()),
        "files": file_count,
        "bytes": bytes_total,
        "bytes_total": bytes_total,
    })
}

/// Build the aggregate `ls --all` JSON response from its real listing outcomes.
pub(crate) fn build_all_json(
    tenant: &str,
    rows: &[RefRecord],
    skipped: &[(String, String)],
    run_cache: usize,
    foreign: usize,
) -> serde_json::Value {
    let refs: Vec<_> = rows
        .iter()
        .map(|record| {
            json!({
                "name": record.name,
                "root": record.root.to_hex(),
                "created_at_unix": record.created_at_unix,
                "parent": record.parent.map(|digest| digest.to_hex()),
            })
        })
        .collect();
    let errors: Vec<_> = skipped
        .iter()
        .map(|(key, error)| json!({ "ref_key": key, "error": error }))
        .collect();

    json!({
        "tenant": tenant,
        "count": rows.len(),
        "refs": refs,
        "run_cache_entries": run_cache,
        "foreign_ac_entries": foreign,
        "skipped": errors.len(),
        "errors": errors,
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::ClwError;

    #[test]
    fn ls_json_shape_matches_spec() {
        // Exercise the production builder: both spellings are deliberately
        // present through the `bytes` deprecation cycle.
        let out = build_one_json(
            &RefRecord {
                name: "my-ws".to_string(),
                root: Digest::of_bytes(b"root"),
                manifest_size: 0,
                created_at_unix: 1_700_000_000,
                parent: None,
                tool_version: "test".to_string(),
            },
            3,
            1024,
        );
        assert!(out.get("name").is_some(), "must have 'name'");
        assert!(out.get("root").is_some(), "must have 'root'");
        assert!(
            out.get("created_at_unix").is_some(),
            "must have 'created_at_unix'"
        );
        assert!(out.get("parent").is_some(), "must have 'parent'");
        assert!(out.get("files").is_some(), "must have 'files'");
        assert!(out.get("bytes").is_some(), "must have 'bytes'");
        assert!(
            out.get("bytes_total").is_some(),
            "must have preferred 'bytes_total'"
        );
        assert_eq!(out["bytes"], out["bytes_total"]);
    }

    #[test]
    fn not_found_error_is_actionable() {
        let msg = not_found_msg("ws-x", "t1");
        assert!(msg.contains("not found"), "must mention not found");
        assert!(msg.contains("ws-x"), "must mention workspace name");
        assert!(msg.contains("t1"), "must mention tenant");
        assert!(msg.contains("--tenant"), "must mention --tenant flag");
        // FIX 4 — the not-found hint now points at the ref-domain mismatch case.
        assert!(
            msg.contains("CLW_REF_DOMAIN=runner") && msg.contains("--ref-domain"),
            "must hint at --ref-domain/CLW_REF_DOMAIN for runner-produced refs, got: {msg}"
        );
    }

    // FIX 1 — the hand-rolled civil-from-days renderer matches `date -u` for known
    // epochs (verified against `date -u -r <epoch>`); the epoch boundary and a
    // mid-2026 timestamp both land exactly.
    #[test]
    fn format_unix_utc_matches_known_epochs() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_unix_utc(1_781_980_856), "2026-06-20 18:40:56 UTC");
        // A leap day (2024-02-29) — exercises the era leap-handling.
        assert_eq!(format_unix_utc(1_709_208_000), "2024-02-29 12:00:00 UTC");
    }

    #[test]
    fn auth_error_mapped_to_actionable_message() {
        let mapped = map_auth_error(ClwError::Auth);
        let msg = mapped.to_string();
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "auth error message must mention CLW_TOKEN or config.toml, got: {msg}"
        );
    }

    #[test]
    fn capability_unavailable_detects_405_501_message() {
        let err = ClwError::Transport(
            "ref enumeration failed: server returned 405/501 (D-7)".to_string(),
        );
        assert!(is_capability_unavailable(&err));
        assert!(!is_capability_unavailable(&ClwError::Transport(
            "unrelated".into()
        )));
        assert!(!is_capability_unavailable(&ClwError::Auth));
    }

    #[test]
    fn capability_unavailable_fails_in_both_modes() {
        // Both modes return an error (non-zero exit) so a script never mistakes
        // a server regression for "tenant has no refs".
        assert!(capability_unavailable(true, "ref enumeration", "D-7").is_err());
        assert!(capability_unavailable(false, "ref enumeration", "D-7").is_err());
    }

    #[test]
    fn ls_all_concurrency_clamps_zero_and_propagates_one_value_to_builder_and_dispatcher() {
        assert_eq!(
            all_fetch_concurrency(0),
            AllFetchConcurrency {
                http_client_limit: 1,
                fetch_dispatcher_limit: 1,
            }
        );
        assert_eq!(
            all_fetch_concurrency(1),
            AllFetchConcurrency {
                http_client_limit: 1,
                fetch_dispatcher_limit: 1,
            }
        );
        assert_eq!(
            all_fetch_concurrency(13),
            AllFetchConcurrency {
                http_client_limit: 13,
                fetch_dispatcher_limit: 13,
            }
        );
    }

    use clw_types::{Digest, RefDomain, RefRecord};

    #[derive(Clone)]
    struct BlockingListClient {
        keys: Vec<Digest>,
        started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        entered: tokio::sync::mpsc::UnboundedSender<()>,
        release: std::sync::Arc<tokio::sync::Semaphore>,
    }

    #[async_trait::async_trait]
    impl AcTransport for BlockingListClient {
        async fn get(&self, _key: &Digest) -> Result<Option<Vec<u8>>, ClwError> {
            self.started
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = self.entered.send(());
            let permit = self.release.acquire().await.map_err(|_| {
                ClwError::Transport("test dispatcher release semaphore was closed".to_string())
            })?;
            drop(permit);
            Ok(None)
        }

        async fn put(&self, _key: &Digest, _value: Vec<u8>) -> Result<(), ClwError> {
            unreachable!("ls --all does not write AC records")
        }

        async fn delete(&self, _key: &Digest) -> Result<(), ClwError> {
            unreachable!("ls --all does not delete AC records")
        }

        async fn list_refs(&self) -> Result<Vec<Digest>, ClwError> {
            Ok(self.keys.clone())
        }
    }

    fn list_test_config() -> Config {
        Config {
            endpoint: "http://127.0.0.1:9".to_string(),
            tenant: "test-tenant".to_string(),
            token: "test-token".to_string(),
            cache_dir: std::path::PathBuf::from("/nonexistent/ls-all-test-cache"),
        }
    }

    fn resolved_list_test_config() -> ResolvedConfig {
        ResolvedConfig {
            clw_config: list_test_config(),
            ref_domain: RefDomain::User,
            ref_domain_invalid: None,
            broker: None,
        }
    }

    /// Exercises the production `run` boundary and `run_all` dispatcher through
    /// the client-builder seam. The fake's `get` futures never complete until
    /// this test releases them, so the observed starts are an exact scheduler
    /// bound, not a timing estimate. This goes red if either the builder falls
    /// back to `HttpClient::new` or the dispatcher regresses to a fixed bound
    /// such as 16.
    #[tokio::test]
    async fn production_ls_all_forwards_clamped_concurrency_to_builder_and_dispatcher() {
        const KEY_COUNT: usize = 17;

        let harness = async {
            for (requested, expected) in [(0, 1), (1, 1), (13, 13)] {
                let keys = (0..KEY_COUNT)
                    .map(|n| Digest::of_bytes(format!("key-{n}").as_bytes()))
                    .collect();
                let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
                let release = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
                let client = BlockingListClient {
                    keys,
                    started: std::sync::Arc::clone(&started),
                    entered,
                    release: std::sync::Arc::clone(&release),
                };
                let built_with =
                    std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
                let built_with_for_factory = std::sync::Arc::clone(&built_with);

                let listing = run_with_client_factory(
                    LsArgs {
                        name: None,
                        all: true,
                    },
                    resolved_list_test_config(),
                    false,
                    requested,
                    move |_cfg, limit| {
                        built_with_for_factory.store(limit, std::sync::atomic::Ordering::SeqCst);
                        Ok(client)
                    },
                );
                tokio::pin!(listing);

                for _ in 0..expected {
                    tokio::select! {
                        entry = entered_rx.recv() => {
                            entry.expect("the production dispatcher must start each allowed fetch");
                        }
                        result = &mut listing => {
                            panic!("the blocked production listing returned early: {result:?}");
                        }
                    }
                }
                assert_eq!(
                    built_with.load(std::sync::atomic::Ordering::SeqCst),
                    expected,
                    "requested {requested} must reach the HTTP-client builder"
                );
                assert_eq!(
                    started.load(std::sync::atomic::Ordering::SeqCst),
                    expected,
                    "requested {requested} must bound the production fetch dispatcher"
                );

                release.add_permits(KEY_COUNT);
                listing
                    .await
                    .expect("released fake ref fetches must complete");
            }
        };

        // Guard only against a broken scheduler causing this test harness to
        // wait forever; it is NOT a throughput or latency assertion.
        tokio::time::timeout(std::time::Duration::from_secs(1), harness)
            .await
            .expect("bounded ls --all concurrency harness timed out");
    }

    fn good_record(name: &str) -> Vec<u8> {
        RefRecord {
            name: name.to_string(),
            root: Digest::of_bytes(name.as_bytes()),
            manifest_size: 10,
            created_at_unix: 1_700_000_000,
            parent: None,
            tool_version: "test".to_string(),
        }
        .to_bytes()
        .unwrap()
    }

    // NOTE (pass-2 audit de-vacuous): the previous `one_bad_record_still_lists_the_good_ones`
    // test drove a test-local `partition` helper whose doc literally read "Mirror of
    // run_all's per-record loop" — it reimplemented the `RefRecord::from_bytes` ⇒
    // push-good/skip-corrupt tolerance IN THE TEST and asserted on its own reimplementation,
    // never running `run_all`. The REAL `--all` tolerance (good refs still list while a
    // corrupt AC record is reported as `skipped` with its `ref_key`, exit 0) is now driven
    // end-to-end against the live binary by `ls_all_lists_refs_tolerates_corruption_and_reflects_rm`
    // in `crates/clw-cli/tests/e2e.rs`, which snapshots a real ref, injects a non-decodable
    // value under a fake ref key in the AC store, and asserts the real JSON `count`/`skipped`/
    // `errors[0].ref_key` against the binary's stdout. The `partition` mirror helper is
    // deleted; the `collect_listing` helper below is RETAINED because it exercises the
    // distinct per-record fetch-RESULT branching (Auth-fatal vs transport-skip vs Ok(None))
    // that the e2e store cannot easily inject mid-fetch, calling REAL `map_auth_error`.

    /// Exact mirror of `run_all`'s per-record fetch loop (FIX 4 logic): given each
    /// key's fetch RESULT, AUTH aborts the whole listing; Ok(None) drops out;
    /// Ok(Some(corrupt)) and any non-auth transport Err are both SKIPPED (collected
    /// + named) and the loop keeps going. Returns Err only for the fatal auth case.
    #[allow(clippy::type_complexity)]
    fn collect_listing(
        fetches: Vec<(Digest, Result<Option<Vec<u8>>, ClwError>)>,
    ) -> Result<(Vec<RefRecord>, Vec<(String, String)>), ClwError> {
        let mut rows = Vec::new();
        let mut skipped = Vec::new();
        for (key, fetched) in fetches {
            let bytes = match fetched {
                Ok(b) => b,
                Err(ClwError::Auth) => return Err(map_auth_error(ClwError::Auth)),
                Err(e) => {
                    skipped.push((key.to_hex(), e.to_string()));
                    continue;
                }
            };
            let Some(bytes) = bytes else { continue };
            match RefRecord::from_bytes(&bytes) {
                Ok(r) => rows.push(r),
                Err(e) => skipped.push((key.to_hex(), e.to_string())),
            }
        }
        Ok((rows, skipped))
    }

    // FIX 4 — a per-record TRANSPORT error (a transient blip) is SKIPPED+reported
    // just like a corrupt record; the rest of the listing still comes back.
    #[test]
    fn one_transport_error_still_lists_the_others() {
        let blip_key = Digest::of_bytes(b"blip");
        let fetches = vec![
            (Digest::of_bytes(b"a"), Ok(Some(good_record("alpha")))),
            (
                blip_key,
                Err(ClwError::Transport("connection reset by peer".into())),
            ),
            (Digest::of_bytes(b"b"), Ok(Some(good_record("bravo")))),
        ];

        let (rows, skipped) = collect_listing(fetches).expect("a transport blip must NOT be fatal");

        // Both good refs survive — partial output beats no output.
        assert_eq!(rows.len(), 2, "good records must still list");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"bravo"));

        // The blipped record is skipped and named by its ref key.
        assert_eq!(skipped.len(), 1, "exactly one record skipped");
        assert_eq!(skipped[0].0, blip_key.to_hex(), "skip names the ref key");
        assert!(
            skipped[0].1.contains("connection reset"),
            "skip carries the transport error text"
        );
    }

    /// The `run_all` classifier for a byte value fetched from the SHARED AC keyspace,
    /// extracted as a pure fn so the exact branch order (Ref → RunRecord → non-JSON
    /// foreign → JSON-but-corrupt) is unit-testable without a live fetch. MUST mirror
    /// the `match RefRecord::from_bytes` arms in `run_all`.
    #[derive(Debug, PartialEq)]
    enum Class {
        Ref,
        RunCache,
        Foreign,
        Corrupt,
    }
    fn classify(bytes: &[u8]) -> Class {
        match RefRecord::from_bytes(bytes) {
            Ok(_) => Class::Ref,
            Err(_) if clw_run::is_run_record(bytes) => Class::RunCache,
            Err(_) if serde_json::from_slice::<serde_json::Value>(bytes).is_err() => Class::Foreign,
            Err(_) => Class::Corrupt,
        }
    }

    // The keyspace-sharing fix: a native/Bazel ActionCache value (arbitrary non-JSON
    // bytes, e.g. the `ac-roundtrip-value-*` values seen live) is classified FOREIGN
    // (benign, silent), NOT a corrupt ref. A real ref stays a Ref; only valid-JSON
    // that is not a RefRecord/RunRecord shape is Corrupt (the genuine warn signal).
    #[test]
    fn foreign_ac_value_is_not_a_corrupt_ref() {
        // A raw native AC value observed live in a shared dogfood tenant.
        assert_eq!(
            classify(b"ac-roundtrip-value-000012e618bfbfe8288c5064"),
            Class::Foreign
        );
        // Empty body and other non-JSON bytes are also foreign, not corrupt.
        assert_eq!(classify(b""), Class::Foreign);
        assert_eq!(classify(&[0xff, 0x00, 0x01]), Class::Foreign);
        // A real ref is still a ref.
        assert_eq!(classify(&good_record("alpha")), Class::Ref);
        // Valid JSON that is neither a RefRecord nor a RunRecord IS a corrupt ref
        // (the only case that should warn).
        assert_eq!(classify(br#"{"unexpected":"shape"}"#), Class::Corrupt);
    }

    // FIX 4 — an AUTH (401) error fetching a record is STILL fatal: a 401 means the
    // whole listing is unauthorized, so it must abort with the actionable message.
    #[test]
    fn auth_error_on_a_record_is_fatal() {
        let fetches = vec![
            (Digest::of_bytes(b"a"), Ok(Some(good_record("alpha")))),
            (Digest::of_bytes(b"x"), Err(ClwError::Auth)),
        ];
        let err = collect_listing(fetches).expect_err("a 401 must abort the whole listing");
        let msg = err.to_string();
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "auth abort must use the actionable message, got: {msg}"
        );
    }
}
