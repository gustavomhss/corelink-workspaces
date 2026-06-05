//! `clw status [PATH] --name <NAME>` — diff remote ref vs local working tree.
//!
//! Exit 0 = clean (no diff). Exit 1 = dirty (diff detected).
//! Exit 2 = error (propagated via the normal error path in main).

use std::path::PathBuf;

use clap::Args;
use clw_client::HttpClient;
use clw_manifest::diff;
use clw_snapshot::build_manifest_local;
use clw_types::{ref_key, AcTransport, ClwError, Manifest, RefRecord};
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Local workspace directory to compare (default: current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Workspace name (ref key).
    #[arg(long)]
    pub name: String,
}

pub async fn run(args: StatusArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let client = HttpClient::new(cfg.clw_config)?;

    // --- Fetch remote ref + manifest ---
    let rk = ref_key(&args.name);
    let ref_bytes = AcTransport::get(&client, &rk)
        .await?
        .ok_or(ClwError::NotFound(rk))?;
    let record = RefRecord::from_bytes(&ref_bytes)?;

    // Fetch manifest blob from CAS.
    use clw_types::CasTransport;
    let manifest_bytes = CasTransport::get(&client, &record.root).await?;
    let remote_manifest = Manifest::from_bytes(&manifest_bytes)?;

    // --- Build local manifest ---
    let local_manifest = build_manifest_local(&args.path).await?;

    // --- Diff ---
    let d = diff(&remote_manifest, &local_manifest);
    let is_clean = d.added.is_empty() && d.removed.is_empty() && d.changed.is_empty();

    if json {
        let out = json!({
            "clean": is_clean,
            "added": d.added,
            "removed": d.removed,
            "changed": d.changed,
        });
        println!("{out}");
    } else if is_clean {
        println!("clean — workspace matches remote ref '{}'", args.name);
    } else {
        if !d.added.is_empty() {
            println!("added:");
            for p in &d.added {
                println!("  + {p}");
            }
        }
        if !d.removed.is_empty() {
            println!("removed:");
            for p in &d.removed {
                println!("  - {p}");
            }
        }
        if !d.changed.is_empty() {
            println!("changed:");
            for p in &d.changed {
                println!("  ~ {p}");
            }
        }
    }

    if is_clean {
        Ok(())
    } else {
        // Exit 1 for dirty (not an error, but distinct from clean).
        std::process::exit(1);
    }
}
