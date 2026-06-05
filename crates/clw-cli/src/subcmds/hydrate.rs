//! `clw hydrate <DEST> --name <NAME>` — materialize a workspace snapshot.

use std::path::PathBuf;

use clap::Args;
use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_hydrate::{hydrate, HydrateOptions};
use clw_types::ClwError;
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
pub struct HydrateArgs {
    /// Destination directory (must not exist or be empty).
    pub dest: PathBuf,

    /// Workspace name (ref key).
    #[arg(long)]
    pub name: String,
}

pub async fn run(args: HydrateArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let cache = LocalCache::new(&cfg.clw_config.cache_dir)?;
    let client = HttpClient::new(cfg.clw_config)?;
    let opts = HydrateOptions::new(&args.name);

    let report = hydrate(&args.dest, &client, &cache, &opts).await?;

    if json {
        let out = json!({
            "root": report.root.to_hex(),
            "files": report.files,
            "bytes_total": report.bytes_total,
            "bytes_from_cache": report.bytes_from_cache,
            "bytes_downloaded": report.bytes_downloaded,
        });
        println!("{out}");
    } else {
        println!("hydrate complete");
        println!("  name:              {}", args.name);
        println!("  root:              {}", report.root);
        println!("  files:             {}", report.files);
        println!("  bytes_total:       {}", report.bytes_total);
        println!("  bytes_from_cache:  {}", report.bytes_from_cache);
        println!("  bytes_downloaded:  {}", report.bytes_downloaded);
    }

    Ok(())
}
