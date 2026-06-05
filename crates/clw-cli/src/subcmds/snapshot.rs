//! `clw snapshot [PATH] --name <NAME>` — snapshot a workspace directory.

use std::path::PathBuf;

use clap::Args;
use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_snapshot::{snapshot, SnapshotOptions};
use clw_types::ClwError;
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
pub struct SnapshotArgs {
    /// Workspace directory to snapshot (default: current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Workspace name (ref key).
    #[arg(long)]
    pub name: String,
}

pub async fn run(args: SnapshotArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let cache = LocalCache::new(&cfg.clw_config.cache_dir)?;
    let client = HttpClient::new(cfg.clw_config)?;
    let opts = SnapshotOptions::new(&args.name);

    let report = snapshot(&args.path, &client, &cache, &opts).await?;

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
