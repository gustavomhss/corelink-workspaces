//! `clw ls --name <NAME>` — list ref info for one workspace.
//! `clw ls --all`         — enumerate every ref in the tenant.
//!
//! `--all` is OPTIMISTIC against a frozen contract: the corelink-server ref
//! enumeration capability is pending ratification (docs/REQUESTS-to-other-TLs.md
//! D-7). Until it deploys the real transport surfaces the 405/501 not-deployed
//! case as an actionable error rather than a silent empty list.

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
async fn run_all(cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let client = HttpClient::new(cfg.clw_config)?;

    let keys = match AcTransport::list_refs(&client).await {
        Ok(keys) => keys,
        Err(e) if is_not_deployed(&e) => return not_deployed_err(json, "ref enumeration", "D-7"),
        Err(e) => return Err(map_auth_error(e)),
    };

    let mut rows = Vec::with_capacity(keys.len());
    for key in &keys {
        // A ref deleted between enumeration and fetch simply drops out of the list.
        if let Some(bytes) = AcTransport::get(&client, key)
            .await
            .map_err(map_auth_error)?
        {
            let record = RefRecord::from_bytes(&bytes)?;
            rows.push(record);
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
        println!(
            "{}",
            json!({ "tenant": tenant, "count": rows.len(), "refs": arr })
        );
    } else if rows.is_empty() {
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
        println!("({} refs in tenant '{tenant}')", rows.len());
    }

    Ok(())
}

/// True if the error is the optimistic-pending "server capability not deployed"
/// case (the client maps 405/501 to a `Transport` error naming the gap).
fn is_not_deployed(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("not yet deployed"))
}

/// Render the optimistic-pending case. ALWAYS fails (non-zero exit) so a script
/// never mistakes "endpoint not deployed" for "tenant has no refs" — in `--json`
/// mode the structured note is printed first, then the error propagates.
fn not_deployed_err(json: bool, capability: &str, dep: &str) -> Result<(), ClwError> {
    let msg = format!(
        "{capability} requires a corelink-server capability not yet deployed — tracked ({dep}; see docs/RELAY-server-D1-D7-D8.md)"
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
    fn not_deployed_detects_pending_capability_message() {
        let err = ClwError::Transport(
            "ref enumeration requires a corelink-server capability not yet deployed (D-7)"
                .to_string(),
        );
        assert!(is_not_deployed(&err));
        assert!(!is_not_deployed(&ClwError::Transport("unrelated".into())));
        assert!(!is_not_deployed(&ClwError::Auth));
    }

    #[test]
    fn not_deployed_err_fails_in_both_modes() {
        // Both modes return an error (non-zero exit) so a script never mistakes
        // "endpoint not deployed" for "tenant has no refs".
        assert!(not_deployed_err(true, "ref enumeration", "D-7").is_err());
        assert!(not_deployed_err(false, "ref enumeration", "D-7").is_err());
    }
}
