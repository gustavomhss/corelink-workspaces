//! `clw init` — write ~/.clw/config.toml from resolved config.

use clap::Args;
use clw_types::ClwError;
use serde_json::json;

use crate::config::{
    clw_config_path, ensure_config_parent_0700, merge_config_pairs, render_config,
    write_config_file_0600, ResolvedConfig,
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

// async to satisfy the uniform await-able subcommand dispatch signature (no async work here).
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

    // Preserve any unrelated keys already in the file (e.g. one a future config
    // feature adds) — only the four identity keys are overwritten. The token is
    // NEVER echoed; it is written to the 0600 file only.
    let c = &cfg.clw_config;
    let updates = vec![
        ("endpoint".to_string(), c.endpoint.clone()),
        ("tenant".to_string(), c.tenant.clone()),
        ("token".to_string(), c.token.clone()),
        ("cache_dir".to_string(), c.cache_dir.display().to_string()),
    ];
    let pairs = merge_config_pairs(&path, &updates);
    let content = render_config(&pairs);

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
    use crate::config::{
        harden_dir_0700, merge_config_pairs, read_config_pairs, render_config,
        write_config_file_0600,
    };

    // -----------------------------------------------------------------------
    // init must PRESERVE unrelated keys (any future config key) when rewriting.
    // -----------------------------------------------------------------------
    #[test]
    fn init_rewrite_preserves_unrelated_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        // Pre-existing file with an unrelated key plus a stale token.
        write_config_file_0600(
            &path,
            "# clw configuration\nunrelated_key = \"keep-me\"\ntoken = \"old-tok\"\n",
        )
        .unwrap();

        // Simulate the init rewrite: overwrite the four identity keys, merge.
        let updates = vec![
            ("endpoint".to_string(), "https://example.com".to_string()),
            ("tenant".to_string(), "acme".to_string()),
            ("token".to_string(), "new-tok".to_string()),
            ("cache_dir".to_string(), "/tmp/clw-cache".to_string()),
        ];
        let pairs = merge_config_pairs(&path, &updates);
        write_config_file_0600(&path, &render_config(&pairs)).unwrap();

        let mut after = read_config_pairs(&path);
        after.sort();
        let map: std::collections::HashMap<_, _> = after.into_iter().collect();
        assert_eq!(
            map.get("unrelated_key").map(String::as_str),
            Some("keep-me"),
            "an unrelated config key must survive an init rewrite"
        );
        assert_eq!(map.get("token").map(String::as_str), Some("new-tok"));
        assert_eq!(map.get("tenant").map(String::as_str), Some("acme"));
    }

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
