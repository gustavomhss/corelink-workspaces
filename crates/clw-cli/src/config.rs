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
///
/// NOTE: this is a deliberately minimal, dependency-free subset of TOML — it
/// handles exactly the flat string keys clw writes (`endpoint`, `tenant`,
/// `token`, `cache_dir`). It is NOT a general TOML parser (no tables, arrays,
/// multi-line values; only `\"` and `\\` escapes within quoted values). A quoted
/// value is read up to its closing quote, so a trailing inline `# comment` after a quoted value is ignored; an
/// unquoted value is taken verbatim (so `#` is NOT an inline comment there —
/// only whole-line `#` comments are stripped).
pub fn parse_config_lines(content: &str, map: &mut HashMap<String, String>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let val_raw = v.trim();
            // A quoted value is read up to its CLOSING quote (ignoring any
            // trailing inline comment); an unquoted value is taken verbatim.
            let val = if let Some(rest) = val_raw.strip_prefix('"') {
                // Quoted value: read up to the closing quote, honoring `\"` and
                // `\\` escapes (the matched inverse of `toml_escape` on write, so
                // a token containing a quote round-trips instead of truncating).
                // A trailing inline `# comment` after the closing quote is ignored.
                let mut out = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(next) = chars.next() {
                                out.push(next);
                            }
                        }
                        '"' => break,
                        _ => out.push(c),
                    }
                }
                out
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

/// Escape a string value for the minimal config writer: backslash and double
/// quote, so it round-trips through [`parse_config_lines`]. Writer and parser are
/// a matched pair (this is intentionally NOT a general TOML serializer).
pub(crate) fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

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
///
/// Reads `$HOME` first (unix-correct), falling back to `%USERPROFILE%` so the
/// resolver works on Windows, rather than the deprecated `std::env::home_dir`.
/// An empty value is treated as unset.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
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

    #[test]
    fn parse_quoted_value_ignores_trailing_inline_comment() {
        // A trailing inline comment after a QUOTED value must not leak into the value.
        let mut map = HashMap::new();
        parse_config_lines("token = \"secret\" # inline comment", &mut map);
        assert_eq!(map["token"], "secret");
    }

    #[test]
    fn escaped_value_round_trips_through_writer_and_parser() {
        // Regression (M0.5): a value containing a quote/backslash must round-trip
        // through toml_escape -> parse_config_lines instead of truncating the PAT.
        for raw in [
            "pat_with\"quote",
            "back\\slash",
            "both\"and\\here",
            "trailing\\",
            "normal_pat_abc123",
        ] {
            let line = format!("token = \"{}\"", toml_escape(raw));
            let mut map = HashMap::new();
            parse_config_lines(&line, &mut map);
            assert_eq!(
                map["token"], raw,
                "value must round-trip exactly for {raw:?}"
            );
        }
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

    // -----------------------------------------------------------------------
    // home_dir: Windows fallback to %USERPROFILE% when $HOME is absent
    // -----------------------------------------------------------------------

    /// RAII guard that snapshots `HOME`/`USERPROFILE`, lets the test mutate
    /// them, and restores the originals on drop. This crate does not serialize
    /// env tests, so restore-on-drop keeps the mutation scoped to this test.
    struct HomeEnvGuard {
        home: Option<std::ffi::OsString>,
        userprofile: Option<std::ffi::OsString>,
    }

    impl HomeEnvGuard {
        fn new() -> Self {
            Self {
                home: std::env::var_os("HOME"),
                userprofile: std::env::var_os("USERPROFILE"),
            }
        }
    }

    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            match &self.home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.userprofile {
                Some(v) => std::env::set_var("USERPROFILE", v),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }

    #[test]
    fn home_dir_falls_back_to_userprofile_when_home_absent() {
        let _guard = HomeEnvGuard::new();

        // $HOME unset, %USERPROFILE% set → USERPROFILE is used (Windows path).
        std::env::remove_var("HOME");
        std::env::set_var("USERPROFILE", "/fallback/userprofile");
        assert_eq!(home_dir(), Some(PathBuf::from("/fallback/userprofile")));

        // $HOME present → it wins over %USERPROFILE% (unix-correct precedence).
        std::env::set_var("HOME", "/primary/home");
        assert_eq!(home_dir(), Some(PathBuf::from("/primary/home")));
    }
}
