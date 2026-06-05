//! `clw hydrate <DEST> --name <NAME>` — materialize a workspace snapshot.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use clap::Args;
use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_hydrate::{hydrate, HydrateOptions};
use clw_types::ClwError;
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(about = "Materialize a workspace snapshot into a local directory")]
pub struct HydrateArgs {
    /// Destination directory (must not exist or be empty).
    #[arg(help = "Destination directory — must not exist or must be empty")]
    pub dest: PathBuf,

    /// Workspace name (ref key).
    #[arg(long, help = "Workspace name to hydrate")]
    pub name: String,
}

pub async fn run(args: HydrateArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let cache = LocalCache::new(&cfg.clw_config.cache_dir)?;
    let client = HttpClient::new(cfg.clw_config)?;
    let opts = HydrateOptions::new(&args.name);

    let report = hydrate(&args.dest, &client, &cache, &opts)
        .await
        .map_err(|e| map_hydrate_error(e, &args.name, &tenant, &args.dest))?;

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

/// Map hydrate-specific errors to actionable user-facing messages.
fn map_hydrate_error(err: ClwError, name: &str, tenant: &str, dest: &Path) -> ClwError {
    match &err {
        ClwError::Auth => ClwError::Transport(
            "authentication failed — check CLW_TOKEN or ~/.clw/config.toml".to_string(),
        ),
        ClwError::NotFound(_) => ClwError::Transport(format!(
            "workspace '{name}' not found for tenant '{tenant}' — run 'clw snapshot --name {name}' first or check --tenant",
        )),
        ClwError::Io(io_err) if io_err.kind() == ErrorKind::AlreadyExists => {
            let dest = dest.display();
            ClwError::Transport(format!(
                "destination '{dest}' is not empty — pass an empty directory or a new path that does not yet exist",
            ))
        }
        _ => err,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::ClwError;
    use std::io::ErrorKind;

    #[test]
    fn map_hydrate_error_auth() {
        let dest = PathBuf::from("/tmp/out");
        let mapped = map_hydrate_error(ClwError::Auth, "ws", "t1", &dest);
        let msg = mapped.to_string();
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "auth error must mention CLW_TOKEN or config.toml, got: {msg}"
        );
    }

    #[test]
    fn map_hydrate_error_not_found_is_actionable() {
        use clw_types::Digest;
        let dest = PathBuf::from("/tmp/out");
        let dummy_digest = Digest::of_bytes(b"test");
        let mapped = map_hydrate_error(ClwError::NotFound(dummy_digest), "my-ws", "acme", &dest);
        let msg = mapped.to_string();
        assert!(msg.contains("my-ws"), "must mention workspace name");
        assert!(msg.contains("acme"), "must mention tenant");
        assert!(
            msg.contains("clw snapshot"),
            "must suggest snapshot command"
        );
    }

    #[test]
    fn map_hydrate_error_already_exists_suggests_empty_dir() {
        let dest = PathBuf::from("/tmp/my-output");
        let io_err = std::io::Error::new(ErrorKind::AlreadyExists, "destination not empty");
        let mapped = map_hydrate_error(ClwError::Io(io_err), "ws", "t1", &dest);
        let msg = mapped.to_string();
        assert!(
            msg.contains("not empty") || msg.contains("empty"),
            "must mention that dest is not empty, got: {msg}"
        );
        assert!(
            msg.contains("/tmp/my-output"),
            "must mention the destination path, got: {msg}"
        );
    }

    #[test]
    fn map_hydrate_error_passthrough_for_other_errors() {
        let dest = PathBuf::from("/tmp/out");
        let original = ClwError::Transport("timeout".to_string());
        let mapped = map_hydrate_error(original, "ws", "t1", &dest);
        assert!(
            mapped.to_string().contains("timeout"),
            "other errors must pass through unchanged"
        );
    }
}
