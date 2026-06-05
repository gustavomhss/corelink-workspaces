//! clw — `CoreLink` Workspaces CLI.
//!
//! Entry point: parses global flags + subcommand, resolves config (flag > env >
//! config file), builds the transport/cache, and dispatches to the matching
//! subcommand handler.

mod config;
mod subcmds;

use clap::Parser;
use clw_types::ClwError;

use config::ResolvedConfig;

// ---------------------------------------------------------------------------
// CLI surface (clap derive)
// ---------------------------------------------------------------------------

/// `CoreLink` Workspaces client — snapshot, hydrate, and memoize your workspace.
#[derive(Debug, Parser)]
#[command(name = "clw", version, about, long_about = None)]
pub struct Cli {
    /// Remote API endpoint (no trailing slash).
    #[arg(
        long,
        global = true,
        env = "CLW_ENDPOINT",
        help = "Remote API endpoint URL (no trailing slash)"
    )]
    pub endpoint: Option<String>,

    /// Tenant ID used to namespace all CAS/AC keys.
    #[arg(
        long,
        global = true,
        env = "CLW_TENANT",
        help = "Tenant ID (set via --tenant, CLW_TENANT env, or config)"
    )]
    pub tenant: Option<String>,

    /// Personal access token (never printed).
    #[arg(
        long,
        global = true,
        env = "CLW_TOKEN",
        help = "CoreLink personal access token (set via --token, CLW_TOKEN env, or config)"
    )]
    pub token: Option<String>,

    /// Local cache directory.
    #[arg(
        long,
        global = true,
        env = "CLW_CACHE_DIR",
        help = "Local blob cache directory (default: ~/.clw/cache)"
    )]
    pub cache_dir: Option<String>,

    /// Emit machine-readable JSON to stdout.
    #[arg(
        long,
        global = true,
        default_value_t = false,
        help = "Emit machine-readable JSON on stdout; errors still go to stderr"
    )]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
    /// Write ~/.clw/config.toml from resolved config values.
    Init(subcmds::init::InitArgs),
    /// Snapshot a workspace directory to `CoreLink`.
    Snapshot(subcmds::snapshot::SnapshotArgs),
    /// Hydrate a workspace snapshot into a local directory.
    Hydrate(subcmds::hydrate::HydrateArgs),
    /// Show diff between remote snapshot and local working tree.
    Status(subcmds::status::StatusArgs),
    /// Run a command, memoizing its output to `CoreLink`.
    Run(subcmds::run::RunArgs),
    /// List ref info for a workspace name.
    Ls(subcmds::ls::LsArgs),
}

// ---------------------------------------------------------------------------
// Error exit helper
// ---------------------------------------------------------------------------

fn exit_err(err: &ClwError) -> ! {
    let msg = actionable_error(err);
    eprintln!("error: {msg}");
    std::process::exit(2);
}

/// Produce a user-actionable error message for any `ClwError`.
///
/// Falls back to the `Display` impl for errors that are already clear.
#[must_use]
pub fn actionable_error(err: &ClwError) -> String {
    match err {
        ClwError::Auth => {
            "authentication failed — check CLW_TOKEN or add token to ~/.clw/config.toml".to_string()
        }
        // NotFound at the top-level prints the raw digest — subcommands override
        // this with workspace-name context before returning to main.
        _ => err.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Resolve config from flags, env, and file.
    let resolved = match ResolvedConfig::resolve(&cli) {
        Ok(c) => c,
        Err(e) => {
            exit_err(&e);
        }
    };

    let result = dispatch(cli.command, resolved, cli.json).await;
    if let Err(e) = result {
        // `run` subcommand propagates child exit code differently (handled in subcmd).
        exit_err(&e);
    }
}

async fn dispatch(cmd: Commands, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    match cmd {
        Commands::Init(args) => subcmds::init::run(args, cfg).await,
        Commands::Snapshot(args) => subcmds::snapshot::run(args, cfg, json).await,
        Commands::Hydrate(args) => subcmds::hydrate::run(args, cfg, json).await,
        Commands::Status(args) => subcmds::status::run(args, cfg, json).await,
        Commands::Run(args) => subcmds::run::run(args, cfg).await,
        Commands::Ls(args) => subcmds::ls::run(args, cfg, json).await,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_init_basic() {
        let cli = Cli::try_parse_from(["clw", "init"]).unwrap();
        assert!(matches!(cli.command, Commands::Init(_)));
        assert!(!cli.json);
    }

    #[test]
    fn parse_init_force() {
        let cli = Cli::try_parse_from(["clw", "init", "--force"]).unwrap();
        assert!(matches!(cli.command, Commands::Init(ref a) if a.force));
    }

    #[test]
    fn parse_snapshot_with_name() {
        let cli = Cli::try_parse_from(["clw", "snapshot", "--name", "my-ws"]).unwrap();
        match &cli.command {
            Commands::Snapshot(a) => {
                assert_eq!(a.name, "my-ws");
                assert_eq!(a.path, std::path::PathBuf::from("."));
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }

    #[test]
    fn parse_snapshot_with_path_and_json() {
        let cli =
            Cli::try_parse_from(["clw", "--json", "snapshot", "/tmp/foo", "--name", "ws"]).unwrap();
        assert!(cli.json);
        match &cli.command {
            Commands::Snapshot(a) => {
                assert_eq!(a.path, std::path::PathBuf::from("/tmp/foo"));
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }

    #[test]
    fn parse_hydrate() {
        let cli = Cli::try_parse_from(["clw", "hydrate", "/out", "--name", "ws"]).unwrap();
        match &cli.command {
            Commands::Hydrate(a) => {
                assert_eq!(a.dest, std::path::PathBuf::from("/out"));
                assert_eq!(a.name, "ws");
            }
            other => panic!("expected Hydrate, got {other:?}"),
        }
    }

    #[test]
    fn parse_status_defaults() {
        let cli = Cli::try_parse_from(["clw", "status", "--name", "ws"]).unwrap();
        match &cli.command {
            Commands::Status(a) => {
                assert_eq!(a.path, std::path::PathBuf::from("."));
                assert_eq!(a.name, "ws");
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_basic() {
        let cli =
            Cli::try_parse_from(["clw", "run", "--input", "src/", "--", "cargo", "build"]).unwrap();
        match &cli.command {
            Commands::Run(a) => {
                assert_eq!(a.input, vec!["src/"]);
                assert_eq!(a.cmd, vec!["cargo", "build"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_with_env() {
        let cli = Cli::try_parse_from([
            "clw", "run", "--input", "a", "--input", "b", "--env", "CC", "--env", "GOPATH", "--",
            "make",
        ])
        .unwrap();
        match &cli.command {
            Commands::Run(a) => {
                assert_eq!(a.input, vec!["a", "b"]);
                assert_eq!(a.env, vec!["CC", "GOPATH"]);
                assert_eq!(a.cmd, vec!["make"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_ls() {
        let cli = Cli::try_parse_from(["clw", "ls", "--name", "some-ws"]).unwrap();
        match &cli.command {
            Commands::Ls(a) => assert_eq!(a.name, "some-ws"),
            other => panic!("expected Ls, got {other:?}"),
        }
    }

    #[test]
    fn global_flags_parsed() {
        let cli = Cli::try_parse_from([
            "clw",
            "--endpoint",
            "https://example.com",
            "--tenant",
            "t1",
            "--token",
            "tok",
            "--cache-dir",
            "/tmp/cache",
            "ls",
            "--name",
            "ws",
        ])
        .unwrap();
        assert_eq!(cli.endpoint.as_deref(), Some("https://example.com"));
        assert_eq!(cli.tenant.as_deref(), Some("t1"));
        assert_eq!(cli.token.as_deref(), Some("tok"));
        assert_eq!(cli.cache_dir.as_deref(), Some("/tmp/cache"));
    }

    // -----------------------------------------------------------------------
    // actionable_error mapping
    // -----------------------------------------------------------------------

    #[test]
    fn actionable_error_auth_is_user_friendly() {
        let err = ClwError::Auth;
        let msg = actionable_error(&err);
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "auth error must mention CLW_TOKEN or config.toml, got: {msg}"
        );
        assert!(
            msg.contains("authentication failed"),
            "auth error must say 'authentication failed', got: {msg}"
        );
    }

    #[test]
    fn actionable_error_transport_falls_back_to_display() {
        let err = ClwError::Transport("some transport issue".to_string());
        let msg = actionable_error(&err);
        // Falls back to Display, which is the transport error string.
        assert!(
            msg.contains("some transport issue"),
            "transport error must include original message, got: {msg}"
        );
    }

    #[test]
    fn version_flag_parses() {
        // `--version` causes clap to print version and exit; verify it is
        // recognised as a valid flag (no UnknownArgument error).
        let result = Cli::try_parse_from(["clw", "--version"]);
        // clap returns Err with kind=DisplayVersion for --version.
        if let Err(e) = &result {
            assert_eq!(
                e.kind(),
                clap::error::ErrorKind::DisplayVersion,
                "expected DisplayVersion, got: {e}"
            );
        }
    }
}
