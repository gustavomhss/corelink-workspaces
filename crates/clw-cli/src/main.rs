//! clw — `CoreLink` Workspaces CLI.
//!
//! Entry point: parses global flags + subcommand, resolves config (flag > env >
//! config file), builds the transport/cache, and dispatches to the matching
//! subcommand handler.

mod config;
mod subcmds;
// M5.3: the telemetry module ships the policy + consent gate; only
// `init_from_env_and_config` is wired today. The `record`/`Event` API and the
// consent helpers are deliberately not yet called from subcommands (the call
// sites land with the pluggable backend), so allow the as-yet-unused scaffolding.
#[allow(dead_code)]
mod telemetry;

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
        help = "CoreLink personal access token. WARNING: visible in `ps`/shell history — prefer CLW_TOKEN env or ~/.clw/config.toml"
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

    /// Disable ANSI color in human-readable output (accessibility).
    ///
    /// Color is ALSO disabled automatically when the `NO_COLOR` env var is set
    /// (https://no-color.org) or when stdout is not a TTY. clw currently emits
    /// plain text with no ANSI escapes, so this is a forward-looking guarantee:
    /// no command may emit color unless [`color_enabled`] returns true.
    #[arg(
        long,
        global = true,
        default_value_t = false,
        help = "Disable ANSI color output (also honored via NO_COLOR env and non-TTY stdout)"
    )]
    pub no_color: bool,

    /// Max concurrent CAS requests for snapshot/hydrate.
    #[arg(
        long,
        global = true,
        default_value_t = 8,
        help = "Max concurrent CAS requests for snapshot/hydrate (lower it if you hit rate limits)"
    )]
    pub concurrency: usize,

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
    /// Print a support-ready, redacted diagnostics bundle.
    Doctor(subcmds::doctor::DoctorArgs),
    /// Manage the locally-stored CoreLink identity (login/logout/whoami).
    Auth(subcmds::auth::AuthArgs),
}

// ---------------------------------------------------------------------------
// Color policy (M5.7 accessibility)
// ---------------------------------------------------------------------------

/// Whether ANSI color may be emitted on stdout.
///
/// Color is enabled ONLY when ALL of the following hold:
/// 1. `--no-color` was not passed,
/// 2. the `NO_COLOR` env var is unset (https://no-color.org), and
/// 3. stdout is a TTY (not a pipe/file/`--json` consumer).
///
/// clw currently emits no ANSI escapes anywhere; this is the single gate every
/// future colorized output must consult, so the `--no-color`/`NO_COLOR`/non-TTY
/// contract holds by construction.
#[must_use]
pub fn color_enabled(no_color_flag: bool) -> bool {
    use std::io::IsTerminal as _;
    if no_color_flag {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
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

    // Resolve the color policy once (flag > NO_COLOR env > non-TTY). clw emits
    // no ANSI today, so this currently only documents/guarantees the contract.
    let _color = color_enabled(cli.no_color);

    // Resolve the telemetry gate once (M5.3). Off by default; honors
    // DO_NOT_TRACK / CLW_TELEMETRY kill-switches and recorded consent. This is
    // no-op-safe: it never errors, prompts, or blocks, so it is safe in CI and
    // scripts. The sink is a documented no-op until a backend is plugged in.
    telemetry::init_from_env_and_config();

    let result = dispatch(cli.command, resolved, cli.json, cli.concurrency).await;
    if let Err(e) = result {
        // `run` subcommand propagates child exit code differently (handled in subcmd).
        exit_err(&e);
    }
}

async fn dispatch(
    cmd: Commands,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
) -> Result<(), ClwError> {
    match cmd {
        Commands::Init(args) => subcmds::init::run(args, cfg, json).await,
        Commands::Snapshot(args) => subcmds::snapshot::run(args, cfg, json, concurrency).await,
        Commands::Hydrate(args) => subcmds::hydrate::run(args, cfg, json, concurrency).await,
        Commands::Status(args) => subcmds::status::run(args, cfg, json).await,
        // `run` streams the child's stdout/stderr verbatim and propagates its
        // exit code, so there is no clw-level payload to render — it has no
        // `--json` form by design (the wrapped command owns stdout).
        Commands::Run(args) => subcmds::run::run(args, cfg).await,
        Commands::Ls(args) => subcmds::ls::run(args, cfg, json).await,
        Commands::Doctor(args) => subcmds::doctor::run(args, cfg, json).await,
        Commands::Auth(args) => subcmds::auth::run(args, cfg, json).await,
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
    fn parse_concurrency_flag() {
        let cli =
            Cli::try_parse_from(["clw", "--concurrency", "4", "snapshot", "--name", "ws"]).unwrap();
        assert_eq!(cli.concurrency, 4);
        assert!(matches!(cli.command, Commands::Snapshot(_)));
    }

    #[test]
    fn parse_concurrency_defaults_to_8() {
        let cli = Cli::try_parse_from(["clw", "snapshot", "--name", "ws"]).unwrap();
        assert_eq!(cli.concurrency, 8);
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
    fn parse_doctor() {
        let cli = Cli::try_parse_from(["clw", "doctor"]).unwrap();
        assert!(matches!(cli.command, Commands::Doctor(_)));
    }

    // -----------------------------------------------------------------------
    // M6.6 — `clw auth` parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_auth_login() {
        use subcmds::auth::AuthCommand;
        let cli = Cli::try_parse_from(["clw", "auth", "login"]).unwrap();
        match cli.command {
            Commands::Auth(a) => assert!(matches!(a.command, AuthCommand::Login)),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn parse_auth_logout_default() {
        use subcmds::auth::AuthCommand;
        let cli = Cli::try_parse_from(["clw", "auth", "logout"]).unwrap();
        match cli.command {
            Commands::Auth(a) => match a.command {
                AuthCommand::Logout(l) => {
                    assert!(!l.all);
                    assert!(!l.yes);
                }
                other => panic!("expected Logout, got {other:?}"),
            },
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn parse_auth_logout_all_yes() {
        use subcmds::auth::AuthCommand;
        let cli = Cli::try_parse_from(["clw", "auth", "logout", "--all", "--yes"]).unwrap();
        match cli.command {
            Commands::Auth(a) => match a.command {
                AuthCommand::Logout(l) => {
                    assert!(l.all);
                    assert!(l.yes);
                }
                other => panic!("expected Logout, got {other:?}"),
            },
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn parse_auth_whoami() {
        use subcmds::auth::AuthCommand;
        let cli = Cli::try_parse_from(["clw", "auth", "whoami"]).unwrap();
        match cli.command {
            Commands::Auth(a) => assert!(matches!(a.command, AuthCommand::Whoami)),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn parse_auth_whoami_with_json() {
        let cli = Cli::try_parse_from(["clw", "--json", "auth", "whoami"]).unwrap();
        assert!(cli.json);
        assert!(matches!(cli.command, Commands::Auth(_)));
    }

    #[test]
    fn parse_auth_rotate() {
        use subcmds::auth::AuthCommand;
        let cli = Cli::try_parse_from(["clw", "auth", "rotate"]).unwrap();
        match cli.command {
            Commands::Auth(a) => assert!(matches!(a.command, AuthCommand::Rotate)),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // M5.7 — --no-color / color policy
    // -----------------------------------------------------------------------

    #[test]
    fn parse_no_color_flag() {
        let cli = Cli::try_parse_from(["clw", "--no-color", "doctor"]).unwrap();
        assert!(cli.no_color);
    }

    #[test]
    fn no_color_defaults_false() {
        let cli = Cli::try_parse_from(["clw", "doctor"]).unwrap();
        assert!(!cli.no_color);
    }

    #[test]
    fn color_disabled_when_flag_set() {
        // --no-color always wins, regardless of TTY/env.
        assert!(!color_enabled(true));
    }

    // -----------------------------------------------------------------------
    // Redaction: no auth output path includes the token value.
    // -----------------------------------------------------------------------

    #[test]
    fn auth_help_does_not_mention_token_value() {
        // Sanity: the auth subcommand surface never takes a positional token, so
        // a token value can't leak via usage/help text. (The token comes from the
        // global --token/CLW_TOKEN/config, redacted everywhere it is echoed.)
        let err = Cli::try_parse_from(["clw", "auth", "--help"]).unwrap_err();
        let help = err.to_string();
        assert!(
            !help.to_lowercase().contains("token value"),
            "auth help must not surface a token value, got: {help}"
        );
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
