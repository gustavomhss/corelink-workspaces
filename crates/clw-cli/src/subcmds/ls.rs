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
use clw_types::{ref_key, AcTransport, CasTransport, ClwError, Manifest, RefRecord};
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::status::map_auth_error;

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

pub async fn run(args: LsArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    match (&args.name, args.all) {
        (Some(name), false) => run_one(name, cfg, json).await,
        (None, true) => run_all(cfg, json).await,
        // clap's `conflicts_with` already rejects `--name X --all`; this guards
        // the `neither` case with an actionable message instead of a panic.
        _ => Err(ClwError::Transport(
            "specify exactly one of --name <NAME> or --all".to_string(),
        )),
    }
}

/// `clw ls --name <NAME>` — the single-workspace lookup (root digest, files, size).
async fn run_one(name: &str, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let client = HttpClient::new(cfg.clw_config)?;

    // Fetch ref record from AC.
    let rk = ref_key(name);
    let ref_bytes = AcTransport::get(&client, &rk)
        .await
        .map_err(map_auth_error)?
        .ok_or_else(|| {
            ClwError::Transport(format!(
                "workspace '{name}' not found for tenant '{tenant}' — run 'clw snapshot --name {name}' first or check --tenant",
            ))
        })?;
    let record = RefRecord::from_bytes(&ref_bytes)?;

    // Fetch manifest to count files and total size.
    let manifest_bytes = CasTransport::get(&client, &record.root)
        .await
        .map_err(map_auth_error)?;
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
        let out = json!({
            "name": record.name,
            "root": record.root.to_hex(),
            "created_at_unix": record.created_at_unix,
            "parent": record.parent.map(|d| d.to_hex()),
            "files": file_count,
            "bytes": manifest.total_size,
        });
        println!("{out}");
    } else {
        println!("name:         {}", record.name);
        println!("root:         {}", record.root);
        println!("created_at:   {}", record.created_at_unix);
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
/// other ref. We SKIP the bad record, emit a `tracing::warn!`/stderr note that
/// NAMES it (by its ref key), and continue. In `--json` each skipped record is
/// reported as a structured `{key, error}` entry under `errors` so a machine
/// caller sees the partial-listing rather than a hard failure. Partial output is
/// strictly better than no output for a listing.
async fn run_all(cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let client = HttpClient::new(cfg.clw_config)?;

    let keys = match AcTransport::list_refs(&client).await {
        Ok(keys) => keys,
        Err(e) if is_capability_unavailable(&e) => {
            return capability_unavailable(json, "ref enumeration", "D-7")
        }
        Err(e) => return Err(map_auth_error(e)),
    };

    let mut rows = Vec::with_capacity(keys.len());
    // Per-record skips (corrupt/unparseable records), carried so --json can report
    // them structurally and human mode can summarize.
    let mut skipped: Vec<(String, String)> = Vec::new();
    for key in &keys {
        // A ref deleted between enumeration and fetch simply drops out of the list.
        let Some(bytes) = AcTransport::get(&client, key)
            .await
            .map_err(map_auth_error)?
        else {
            continue;
        };
        match RefRecord::from_bytes(&bytes) {
            Ok(record) => rows.push(record),
            // A corrupt record is SKIPPED (read-only ⇒ tolerant): warn + continue
            // so every good ref still lists. Name the offender by its ref key.
            Err(e) => {
                let key_hex = key.to_hex();
                tracing::warn!(ref_key = %key_hex, error = %e, "skipping unparseable ref record");
                eprintln!("warning: skipping unparseable ref record {key_hex}: {e}");
                skipped.push((key_hex, e.to_string()));
            }
        }
    }

    if json {
        let arr: Vec<_> = rows
            .iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "root": r.root.to_hex(),
                    "created_at_unix": r.created_at_unix,
                    "parent": r.parent.map(|d| d.to_hex()),
                })
            })
            .collect();
        let errors: Vec<_> = skipped
            .iter()
            .map(|(key, err)| json!({ "ref_key": key, "error": err }))
            .collect();
        println!(
            "{}",
            json!({
                "tenant": tenant,
                "count": rows.len(),
                "refs": arr,
                "skipped": errors.len(),
                "errors": errors,
            })
        );
    } else if rows.is_empty() && skipped.is_empty() {
        println!("no refs for tenant '{tenant}'");
    } else {
        for r in &rows {
            println!(
                "{}  {}  {}",
                &r.root.to_hex()[..16],
                r.created_at_unix,
                r.name
            );
        }
        if skipped.is_empty() {
            println!("({} refs in tenant '{tenant}')", rows.len());
        } else {
            println!(
                "({} refs in tenant '{tenant}'; {} record(s) skipped — see warnings above)",
                rows.len(),
                skipped.len()
            );
        }
    }

    Ok(())
}

/// True if the error is the server-regression case (the client maps 405/501 to a
/// `Transport` error carrying the `405/501` token for a LIVE capability).
fn is_capability_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("405/501"))
}

/// Render the server-regression case. ALWAYS fails (non-zero exit) so a script
/// never mistakes a server regression for "tenant has no refs" — in `--json`
/// mode the structured note is printed first, then the error propagates.
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
    use clw_types::ClwError;

    #[test]
    fn ls_json_shape_matches_spec() {
        // Verify the JSON shape has the correct keys per spec:
        // {name, root, created_at_unix, parent, files, bytes}
        let out = json!({
            "name": "my-ws",
            "root": "a".repeat(64),
            "created_at_unix": 1_700_000_000_u64,
            "parent": serde_json::Value::Null,
            "files": 3_u64,
            "bytes": 1024_u64,
        });
        assert!(out.get("name").is_some(), "must have 'name'");
        assert!(out.get("root").is_some(), "must have 'root'");
        assert!(
            out.get("created_at_unix").is_some(),
            "must have 'created_at_unix'"
        );
        assert!(out.get("parent").is_some(), "must have 'parent'");
        assert!(out.get("files").is_some(), "must have 'files'");
        assert!(out.get("bytes").is_some(), "must have 'bytes'");
    }

    #[test]
    fn not_found_error_is_actionable() {
        let name = "ws-x";
        let tenant = "t1";
        let msg = format!(
            "workspace '{name}' not found for tenant '{tenant}' — run 'clw snapshot --name {name}' first or check --tenant",
        );
        assert!(msg.contains("not found"), "must mention not found");
        assert!(msg.contains(name), "must mention workspace name");
        assert!(msg.contains(tenant), "must mention tenant");
        assert!(msg.contains("--tenant"), "must mention --tenant flag");
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

    use clw_types::{Digest, RefRecord};

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

    /// Mirror of `run_all`'s per-record loop: parse each fetched record, push the
    /// good ones, SKIP (collect) the corrupt ones. This is the exact tolerance the
    /// command relies on — a single bad record must not blind the rest.
    fn partition(records: &[(Digest, Vec<u8>)]) -> (Vec<RefRecord>, Vec<(String, String)>) {
        let mut rows = Vec::new();
        let mut skipped = Vec::new();
        for (key, bytes) in records {
            match RefRecord::from_bytes(bytes) {
                Ok(r) => rows.push(r),
                Err(e) => skipped.push((key.to_hex(), e.to_string())),
            }
        }
        (rows, skipped)
    }

    // FIX 2 — a listing with one bad record still returns the good ones, and the
    // bad record is reported (skipped), never aborting the whole `--all` listing.
    #[test]
    fn one_bad_record_still_lists_the_good_ones() {
        let bad_key = Digest::of_bytes(b"corrupt");
        let records = vec![
            (Digest::of_bytes(b"a"), good_record("alpha")),
            (bad_key, b"{ this is not valid json".to_vec()),
            (Digest::of_bytes(b"b"), good_record("bravo")),
        ];
        let (rows, skipped) = partition(&records);

        // Both good refs survive — partial output beats no output.
        assert_eq!(rows.len(), 2, "good records must still list");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"bravo"));

        // The one bad record is skipped and named by its ref key.
        assert_eq!(skipped.len(), 1, "exactly one record skipped");
        assert_eq!(
            skipped[0].0,
            bad_key.to_hex(),
            "skip entry names the ref key"
        );

        // The --json shape carries the skip as a structured {ref_key, error} entry.
        let errors: Vec<_> = skipped
            .iter()
            .map(|(key, err)| json!({ "ref_key": key, "error": err }))
            .collect();
        let out =
            json!({ "count": rows.len(), "refs": [], "skipped": errors.len(), "errors": errors });
        assert_eq!(out["count"], 2);
        assert_eq!(out["skipped"], 1);
        assert_eq!(out["errors"][0]["ref_key"], bad_key.to_hex());
    }
}
