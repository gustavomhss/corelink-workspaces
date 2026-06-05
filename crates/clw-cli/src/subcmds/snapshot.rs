//! `clw snapshot [PATH] --name <NAME>` — snapshot a workspace directory.

use std::path::PathBuf;

use clap::Args;
use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_snapshot::{snapshot, SnapshotOptions};
use clw_types::ClwError;
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::status::map_auth_error;

#[derive(Debug, Args)]
#[command(about = "Snapshot a workspace directory and upload it to CoreLink")]
pub struct SnapshotArgs {
    /// Workspace directory to snapshot (default: current directory).
    #[arg(
        default_value = ".",
        help = "Workspace directory to snapshot (default: current directory)"
    )]
    pub path: PathBuf,

    /// Workspace name (ref key).
    #[arg(long, help = "Workspace name to store the snapshot under")]
    pub name: String,
}

pub async fn run(args: SnapshotArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let cache = LocalCache::new(&cfg.clw_config.cache_dir)?;
    let client = HttpClient::new(cfg.clw_config)?;
    let opts = SnapshotOptions::new(&args.name);

    let report = snapshot(&args.path, &client, &cache, &opts)
        .await
        .map_err(map_auth_error)?;

    if json {
        let out = json!({
            "root": report.root.to_hex(),
            "files": report.files,
            "bytes_total": report.bytes_total,
            "chunks_total": report.chunks_total,
            "chunks_uploaded": report.chunks_uploaded,
        });
        println!("{out}");
    } else {
        println!("snapshot complete");
        println!("  name:            {}", args.name);
        println!("  root:            {}", report.root);
        println!("  files:           {}", report.files);
        println!("  bytes_total:     {}", report.bytes_total);
        println!("  chunks_total:    {}", report.chunks_total);
        println!("  chunks_uploaded: {}", report.chunks_uploaded);
    }

    Ok(())
}
