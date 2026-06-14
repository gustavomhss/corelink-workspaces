//! `clw init` — write ~/.clw/config.toml from resolved config.

use clap::Args;
use clw_types::ClwError;
use serde_json::json;

use crate::config::{
    clw_config_path, ensure_config_parent_0700, toml_escape, write_config_file_0600, ResolvedConfig,
};

#[derive(Debug, Args)]
#[command(about = "Write ~/.clw/config.toml from the resolved config values")]
pub struct InitArgs {
    /// Overwrite an existing config file.
    #[arg(
        long,
        default_value_t = false,
        help = "Overwrite an existing config file (default: error if file exists)"
    )]
    pub force: bool,
}

// All subcommand entry points are async to satisfy a uniform dispatch signature.
#[allow(clippy::unused_async)]
pub async fn run(args: InitArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let path = clw_config_path().ok_or_else(|| {
        ClwError::Transport("cannot determine home directory for config file".to_string())
    })?;

    if path.exists() && !args.force {
        return Err(ClwError::Transport(format!(
            "config file already exists at {}; use --force to overwrite",
            path.display()
        )));
    }

    // Ensure parent directory exists and is private (0700) — it holds the
    // plaintext PAT. Tighten perms even if ~/.clw pre-existed with a looser mode.
    ensure_config_parent_0700(&path)?;

    let content = format!(
        "# clw configuration\n\
         endpoint = \"{endpoint}\"\n\
         tenant = \"{tenant}\"\n\
         token = \"{token}\"\n\
         cache_dir = \"{cache_dir}\"\n",
        endpoint = toml_escape(&cfg.clw_config.endpoint),
        tenant = toml_escape(&cfg.clw_config.tenant),
        // NEVER print the token to stdout/stderr — write it to the config file only.
        token = toml_escape(&cfg.clw_config.token),
        cache_dir = toml_escape(&cfg.clw_config.cache_dir.display().to_string()),
    );

    // config.toml holds the plaintext token: force owner-only (0600) perms.
    write_config_file_0600(&path, &content)?;

    // The token is NEVER echoed; only the path is reported.
    if json {
        let out = json!({
            "wrote": path.display().to_string(),
            // token presence only — never the value.
            "token": if cfg.clw_config.token.is_empty() { "<unset>" } else { "<set>" },
        });
        println!("{out}");
    } else {
        eprintln!("wrote config to {}", path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::config::{harden_dir_0700, write_config_file_0600};

    // -----------------------------------------------------------------------
    // config.toml must be written 0600 (it holds the plaintext PAT).
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn write_config_file_is_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        write_config_file_0600(&path, "token = \"secret\"\n").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "config.toml must be owner-only (0600)");
    }

    // -----------------------------------------------------------------------
    // Re-writing an existing, world-readable file resets it back to 0600.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn write_config_file_resets_loose_mode_to_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        // Pre-create with a deliberately loose (world-readable) mode.
        std::fs::write(&path, "stale = \"old\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );

        // The privacy-preserving write must tighten it back to 0600.
        write_config_file_0600(&path, "token = \"secret\"\n").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "loose mode must be reset to 0600");

        // And the contents must be the new (truncated) content, not appended.
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "token = \"secret\"\n");
    }

    // -----------------------------------------------------------------------
    // The ~/.clw parent dir is hardened to 0700.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn harden_dir_is_0700() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::TempDir::new().unwrap();
        let sub = dir.path().join(".clw");
        std::fs::create_dir_all(&sub).unwrap();
        // Start from a loose, world-traversable mode.
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        harden_dir_0700(&sub).unwrap();

        let mode = std::fs::metadata(&sub).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "~/.clw must be owner-only (0700)");
    }
}
