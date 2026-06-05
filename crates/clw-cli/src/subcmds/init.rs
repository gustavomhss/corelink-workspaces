//! `clw init` — write ~/.clw/config.toml from resolved config.

use std::io::Write as _;

use clap::Args;
use clw_types::ClwError;

use crate::config::{clw_config_path, ResolvedConfig};

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Overwrite an existing config file.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

pub async fn run(args: InitArgs, cfg: ResolvedConfig) -> Result<(), ClwError> {
    let path = clw_config_path().ok_or_else(|| {
        ClwError::Transport("cannot determine home directory for config file".to_string())
    })?;

    if path.exists() && !args.force {
        return Err(ClwError::Transport(format!(
            "config file already exists at {}; use --force to overwrite",
            path.display()
        )));
    }

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = format!(
        "# clw configuration\n\
         endpoint = \"{endpoint}\"\n\
         tenant = \"{tenant}\"\n\
         token = \"{token}\"\n\
         cache_dir = \"{cache_dir}\"\n",
        endpoint = cfg.clw_config.endpoint,
        tenant = cfg.clw_config.tenant,
        // NEVER print the token to stdout/stderr — write it to the config file only.
        token = cfg.clw_config.token,
        cache_dir = cfg.clw_config.cache_dir.display(),
    );

    let mut file = std::fs::File::create(&path)?;
    file.write_all(content.as_bytes())?;

    eprintln!("wrote config to {}", path.display());
    Ok(())
}
