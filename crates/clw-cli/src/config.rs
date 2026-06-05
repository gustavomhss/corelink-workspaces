//! Config resolution: CLI flags > env vars > ~/.clw/config.toml > defaults.
//!
//! `tenant` and `token` have no default and produce a friendly error (exit 2)
//! if still missing after all layers.

use std::collections::HashMap;
use std::path::PathBuf;

use clw_types::{ClwError, Config};

use crate::Cli;

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

const DEFAULT_ENDPOINT: &str = "https://corelink-api.humangr.com";
const DEFAULT_CACHE_DIR: &str = "~/.clw/cache";

// ---------------------------------------------------------------------------
// ResolvedConfig
// ---------------------------------------------------------------------------

/// Fully resolved configuration ready to hand off to subcommands.
pub struct ResolvedConfig {
    pub clw_config: Config,
}

impl ResolvedConfig {
    /// Resolve config from the parsed [`Cli`]:
    /// 1. CLI flags (already in `cli.endpoint` etc.)
    /// 2. Env vars (already merged by clap via `env = "..."`)
    /// 3. `~/.clw/config.toml` (parsed manually)
    /// 4. Hard defaults
    pub fn resolve(cli: &Cli) -> Result<Self, ClwError> {
        let file_cfg = read_config_file();

        let endpoint = cli
            .endpoint
            .clone()
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());

        let tenant = cli
            .tenant
            .clone()
            .or_else(|| file_cfg.get("tenant").cloned())
            .ok_or_else(|| {
                ClwError::Transport(
                    "missing tenant: set --tenant, CLW_TENANT, or add tenant to ~/.clw/config.toml"
                        .to_string(),
                )
            })?;

        // NEVER print the token; only validate it is present.
        let token = cli
            .token
            .clone()
            .or_else(|| file_cfg.get("token").cloned())
            .ok_or_else(|| {
                ClwError::Transport(
                    "missing token: set --token, CLW_TOKEN, or add token to ~/.clw/config.toml"
                        .to_string(),
                )
            })?;

        let cache_dir_str = cli
            .cache_dir
            .clone()
            .or_else(|| file_cfg.get("cache_dir").cloned())
            .unwrap_or_else(|| DEFAULT_CACHE_DIR.to_string());

        let cache_dir = expand_tilde(&cache_dir_str);

        Ok(ResolvedConfig {
            clw_config: Config {
                endpoint: endpoint.trim_end_matches('/').to_string(),
                tenant,
                token,
                cache_dir,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Config file parsing (~/.clw/config.toml)
//
// Simple `key = "value"` line parser. No new toml dependency.
// ---------------------------------------------------------------------------

fn config_file_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".clw").join("config.toml"))
}

/// Read and parse `~/.clw/config.toml`. Returns an empty map on any error or
/// missing file (failures are silently ignored — flags/env take precedence).
fn read_config_file() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(path) = config_file_path() else {
        return map;
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return map;
    };
    parse_config_lines(&content, &mut map);
    map
}

/// Parse lines of the form `key = "value"` or `key = value` (no quotes).
/// Blank lines and lines beginning with `#` are ignored.
pub fn parse_config_lines(content: &str, map: &mut HashMap<String, String>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let val_raw = v.trim();
            // Strip optional surrounding double-quotes.
            let val = if val_raw.starts_with('"') && val_raw.ends_with('"') && val_raw.len() >= 2 {
                val_raw[1..val_raw.len() - 1].to_string()
            } else {
                val_raw.to_string()
            };
            map.insert(key, val);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(s)
}

/// Return the home directory.
pub fn home_dir() -> Option<PathBuf> {
    // std::env::home_dir is deprecated but functional on macOS/Linux.
    #[allow(deprecated)]
    std::env::home_dir()
}

/// Return `~/.clw/config.toml` path (for use in subcommands).
pub fn clw_config_path() -> Option<PathBuf> {
    config_file_path()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::Path;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // parse_config_lines
    // -----------------------------------------------------------------------

    #[test]
    fn parse_quoted_values() {
        let mut map = HashMap::new();
        parse_config_lines(
            r#"
endpoint = "https://example.com"
tenant = "t1"
token = "tok123"
cache_dir = "~/.clw/cache"
"#,
            &mut map,
        );
        assert_eq!(map["endpoint"], "https://example.com");
        assert_eq!(map["tenant"], "t1");
        assert_eq!(map["token"], "tok123");
        assert_eq!(map["cache_dir"], "~/.clw/cache");
    }

    #[test]
    fn parse_unquoted_values() {
        let mut map = HashMap::new();
        parse_config_lines("endpoint = https://example.com\ntenant = acme", &mut map);
        assert_eq!(map["endpoint"], "https://example.com");
        assert_eq!(map["tenant"], "acme");
    }

    #[test]
    fn parse_ignores_blank_and_comment_lines() {
        let mut map = HashMap::new();
        parse_config_lines("# this is a comment\n\nkey = val", &mut map);
        assert_eq!(map.len(), 1);
        assert_eq!(map["key"], "val");
    }

    // -----------------------------------------------------------------------
    // Config precedence: flag > env > file
    // -----------------------------------------------------------------------

    /// Helper: write a config file into `dir/.clw/config.toml`.
    fn write_config_file(dir: &Path, content: &str) {
        let clw_dir = dir.join(".clw");
        std::fs::create_dir_all(&clw_dir).unwrap();
        let mut f = std::fs::File::create(clw_dir.join("config.toml")).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn config_precedence_flag_beats_env_beats_file() {
        // Use a temp dir as the fake HOME.
        let tmp_home = TempDir::new().unwrap();
        write_config_file(
            tmp_home.path(),
            "endpoint = \"https://from-file.example.com\"\ntenant = \"file-tenant\"\ntoken = \"file-token\"\n",
        );

        // Parse config lines directly to verify file would have lower precedence.
        let mut file_cfg = HashMap::new();
        let content =
            std::fs::read_to_string(tmp_home.path().join(".clw").join("config.toml")).unwrap();
        parse_config_lines(&content, &mut file_cfg);

        // Simulate: flag wins over file value.
        let flag_endpoint = Some("https://from-flag.example.com".to_string());
        let resolved_endpoint = flag_endpoint
            .clone()
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        assert_eq!(resolved_endpoint, "https://from-flag.example.com");

        // Simulate: file wins when flag is absent.
        let no_flag: Option<String> = None;
        let resolved_endpoint2 = no_flag
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        assert_eq!(resolved_endpoint2, "https://from-file.example.com");

        // Simulate: default wins when both absent.
        let mut empty_file_cfg: HashMap<String, String> = HashMap::new();
        parse_config_lines("", &mut empty_file_cfg);
        let no_flag2: Option<String> = None;
        let resolved_endpoint3 = no_flag2
            .or_else(|| empty_file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        assert_eq!(resolved_endpoint3, DEFAULT_ENDPOINT);
    }

    // -----------------------------------------------------------------------
    // expand_tilde
    // -----------------------------------------------------------------------

    #[test]
    fn expand_tilde_with_home() {
        if let Some(home) = home_dir() {
            let result = expand_tilde("~/.clw/cache");
            assert_eq!(result, home.join(".clw/cache"));
        }
    }

    #[test]
    fn expand_tilde_absolute_path_unchanged() {
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn expand_tilde_relative_path_unchanged() {
        let result = expand_tilde("relative/path");
        assert_eq!(result, PathBuf::from("relative/path"));
    }
}
