//! `clw ls --name <NAME>` — list ref info for a workspace.

use clap::Args;
use clw_client::HttpClient;
use clw_types::{ref_key, AcTransport, CasTransport, ClwError, Manifest, RefRecord};
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
pub struct LsArgs {
    /// Workspace name (ref key).
    #[arg(long)]
    pub name: String,
}

pub async fn run(args: LsArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let client = HttpClient::new(cfg.clw_config)?;

    // Fetch ref record from AC.
    let rk = ref_key(&args.name);
    let ref_bytes = AcTransport::get(&client, &rk)
        .await?
        .ok_or(ClwError::NotFound(rk))?;
    let record = RefRecord::from_bytes(&ref_bytes)?;

    // Fetch manifest to count files and total size.
    let manifest_bytes = CasTransport::get(&client, &record.root).await?;
    let manifest = Manifest::from_bytes(&manifest_bytes)?;

    let file_count = manifest
        .entries
        .iter()
        .filter(|e| matches!(e, clw_types::Entry::File { .. }))
        .count();

    let parent_str = record
        .parent
        .map(|d| d.to_hex())
        .unwrap_or_else(|| "none".to_string());

    if json {
        let out = json!({
            "name": record.name,
            "root": record.root.to_hex(),
            "created_at_unix": record.created_at_unix,
            "parent": record.parent.map(|d| d.to_hex()),
            "files": file_count,
            "bytes_total": manifest.total_size,
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
