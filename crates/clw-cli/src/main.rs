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
///
/// `Debug` is hand-written (NOT derived) so the global `--token`/`CLW_TOKEN` PAT
/// is redacted: the derived `Debug` would print `token: Some("<the PAT>")`. The
/// hand-written route keeps clap parsing untouched (no newtype / `FromStr`).
#[derive(Parser)]
#[command(name = "clw", version, about, long_about = None)]
pub struct Cli {
    /// Remote API endpoint (no trailing slash).
    #[arg(
        long,
        global = true,
        env = "CLW_ENDPOINT",
        help = "Remote API endpoint URL (no trailing slash). Env: CLW_ENDPOINT (or CORELINK_ENDPOINT)"
    )]
    pub endpoint: Option<String>,

    /// Tenant ID used to namespace all CAS/AC keys.
    #[arg(
        long,
        global = true,
        env = "CLW_TENANT",
        help = "Tenant ID (set via --tenant, CLW_TENANT/CORELINK_TENANT env, or config)"
    )]
    pub tenant: Option<String>,

    /// Personal access token (never printed).
    #[arg(
        long,
        global = true,
        env = "CLW_TOKEN",
        help = "CoreLink personal access token. WARNING: visible in `ps`/shell history — prefer CLW_TOKEN/CORELINK_TOKEN env or ~/.clw/config.toml"
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

    /// Ref keyspace selector — `user` (default) or `runner`.
    ///
    /// `runner` writes/reads refs under a reserved, collision-proof keyspace
    /// (`clw/ref/runner/v1/`), disjoint from user refs by construction. Set by
    /// the corelink-runners fabric at dispatch time; users never need it.
    #[arg(
        long,
        global = true,
        env = "CLW_REF_DOMAIN",
        help = "Ref keyspace: 'user' (default) or 'runner' (runner-produced refs)"
    )]
    pub ref_domain: Option<String>,

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

    /// Increase diagnostic verbosity on stderr (repeatable: -v=info, -vv=debug).
    ///
    /// Drives a `tracing` diagnostic layer that writes to STDERR only, so stdout
    /// (including `--json`) stays clean. `RUST_LOG`, when set, overrides this.
    #[arg(
        short = 'v',
        long,
        global = true,
        action = clap::ArgAction::Count,
        help = "Increase diagnostic logging on stderr (-v=info, -vv=debug); RUST_LOG overrides"
    )]
    pub verbose: u8,

    /// Suppress diagnostics below ERROR on stderr. Conflicts with `-v`.
    #[arg(
        short = 'q',
        long,
        global = true,
        default_value_t = false,
        conflicts_with = "verbose",
        help = "Quiet: only ERROR-level diagnostics on stderr; RUST_LOG overrides"
    )]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

// Manual Debug impl: redact the `--token`/`CLW_TOKEN` PAT (rendered as
// `<redacted>`/`<unset>`) while showing every other field normally. The derived
// Debug would print `token: Some("<the PAT>")`, leaking the secret into any
// `{cli:?}` diagnostic.
impl std::fmt::Debug for Cli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let token = if self.token.is_some() {
            "<redacted>"
        } else {
            "<unset>"
        };
        f.debug_struct("Cli")
            .field("endpoint", &self.endpoint)
            .field("tenant", &self.tenant)
            .field("token", &token)
            .field("cache_dir", &self.cache_dir)
            .field("ref_domain", &self.ref_domain)
            .field("json", &self.json)
            .field("no_color", &self.no_color)
            .field("concurrency", &self.concurrency)
            .field("verbose", &self.verbose)
            .field("quiet", &self.quiet)
            .field("command", &self.command)
            .finish()
    }
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
    /// Delete a workspace ref by name.
    Rm(subcmds::rm::RmArgs),
    /// Garbage-collect unreferenced CAS blobs (dry-run unless --yes).
    Prune(subcmds::prune::PruneArgs),
    /// Irreversibly delete ALL of a tenant's data (GDPR erasure; dry-run unless --yes).
    Erase(subcmds::erase::EraseArgs),
    /// Print a support-ready, redacted diagnostics bundle.
    Doctor(subcmds::doctor::DoctorArgs),
    /// Manage the locally-stored CoreLink identity (login/logout/whoami).
    Auth(subcmds::auth::AuthArgs),
    /// Remove clw's on-disk footprint (config + PAT, cache).
    Uninstall(subcmds::uninstall::UninstallArgs),
    /// Generate a shell-completion script (bash|zsh|fish|powershell|elvish).
    Completions(subcmds::completions::CompletionsArgs),
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
// Diagnostic tracing layer (M5.1)
// ---------------------------------------------------------------------------

/// Initialize the diagnostic `tracing` subscriber, writing to STDERR only.
///
/// This is a SEPARATE channel from user-facing command output (which stays on
/// stdout, including `--json`). It exists for support/debugging via `-v`/`-q`
/// and `RUST_LOG`.
///
/// Level policy: if `RUST_LOG` is set it wins (full `env-filter` syntax);
/// otherwise the level derives from verbosity — default WARN, `-v`=INFO,
/// `-vv`(or more)=DEBUG, `-q`=ERROR. ANSI color follows the same policy as the
/// rest of the CLI: disabled by `--no-color`, `NO_COLOR`, or a non-TTY stderr.
///
/// Safe to call exactly once at startup; uses `try_init` so a double-init (e.g.
/// a test harness that already installed a subscriber) is a silent no-op rather
/// than a panic.
fn init_tracing(verbose: u8, quiet: bool, no_color_flag: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    // RUST_LOG (if present) overrides the -v/-q knobs entirely.
    let filter = match EnvFilter::try_from_default_env() {
        Ok(f) => f,
        Err(_) => {
            let level = if quiet {
                "error"
            } else {
                match verbose {
                    0 => "warn",
                    1 => "info",
                    _ => "debug",
                }
            };
            EnvFilter::new(level)
        }
    };

    // Color on stderr follows --no-color / NO_COLOR / non-TTY stderr.
    let ansi = stderr_color_enabled(no_color_flag);

    // try_init: a no-op if a global subscriber is already installed.
    let _ = fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_ansi(ansi)
        .try_init();
}

/// Whether ANSI color may be emitted on STDERR (the diagnostic channel).
///
/// Mirrors [`color_enabled`] but gates on stderr being a TTY, since the
/// diagnostic layer writes there rather than to stdout.
fn stderr_color_enabled(no_color_flag: bool) -> bool {
    use std::io::IsTerminal as _;
    if no_color_flag {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stderr().is_terminal()
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

/// Whether `cmd` is a LOCAL-ONLY command that must run even on a broken/
/// half-configured config (missing tenant/token), resolving leniently instead of
/// erroring out. Covers `doctor` (diagnostics — documented to always exit 0) and
/// the offline `auth logout`/`auth whoami` (which repair/report local creds).
///
/// `auth login`/`auth rotate` are deliberately EXCLUDED: `login` persists the
/// resolved identity, so it should still require a resolvable token+tenant.
/// `uninstall` has its own dedicated lenient path above and is handled before
/// this gate is consulted.
fn command_runs_on_broken_config(cmd: &Commands) -> bool {
    use subcmds::auth::AuthCommand;
    match cmd {
        Commands::Doctor(_) => true,
        Commands::Auth(a) => matches!(a.command, AuthCommand::Logout(_) | AuthCommand::Whoami),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Bring up the diagnostic tracing layer (stderr only) before anything that
    // might emit an event. Honors -v/-q and RUST_LOG; never touches stdout.
    init_tracing(cli.verbose, cli.quiet, cli.no_color);

    // `completions` is a pure, offline, client-only command: it reflects the
    // derived clap surface and needs NO endpoint/tenant/token. Handle it before
    // config resolution so a fresh user (no config, no token) can still generate
    // completions. (It is a pure offline reflection of the clap surface.)
    if let Commands::Completions(args) = cli.command {
        if let Err(e) = subcmds::completions::run(args).await {
            exit_err(&e);
        }
        return;
    }

    // Resolve config from flags, env, and file. A handful of LOCAL-ONLY commands
    // must work even when no token/tenant has ever been set:
    //   - `uninstall` is the cleanup path — it resolves ONLY the cache dir
    //     leniently rather than failing on a missing token.
    //   - `doctor`/`auth logout`/`auth whoami` operate on whatever is configured
    //     and make NO network call; they must run on the exact broken/half-
    //     configured config a user invokes them to diagnose or repair, resolving
    //     missing tenant/token to `<unset>` rather than erroring out.
    let resolved = match ResolvedConfig::resolve(&cli) {
        Ok(c) => c,
        Err(e) => {
            if matches!(cli.command, Commands::Uninstall(_)) {
                let cache_dir = ResolvedConfig::resolve_cache_dir_lenient(&cli);
                let json = cli.json;
                let Commands::Uninstall(args) = cli.command else {
                    unreachable!("guarded by matches! above")
                };
                if let Err(e) = subcmds::uninstall::run(args, cache_dir, json).await {
                    exit_err(&e);
                }
                return;
            }
            // `doctor` and the local-only `auth logout`/`whoami` fall back to a
            // LENIENT resolution (missing tenant/token => empty) so they run on a
            // broken config instead of erroring before they can diagnose/repair it.
            if command_runs_on_broken_config(&cli.command) {
                ResolvedConfig::resolve_lenient(&cli)
            } else {
                exit_err(&e);
            }
        }
    };

    // Resolve the color policy once (flag > NO_COLOR env > non-TTY). clw emits
    // no ANSI today, so this currently only documents/guarantees the contract.
    let _color = color_enabled(cli.no_color);

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
        // `--json` form by design (the wrapped command owns stdout), and it does
        // not honor the global `--concurrency`. `json` is threaded in only so it
        // can emit a one-line stderr hint when `--json` is passed (P6).
        Commands::Run(args) => subcmds::run::run(args, cfg, json).await,
        Commands::Ls(args) => subcmds::ls::run(args, cfg, json).await,
        Commands::Rm(args) => subcmds::rm::run(args, cfg, json).await,
        Commands::Prune(args) => subcmds::prune::run(args, cfg, json).await,
        Commands::Erase(args) => subcmds::erase::run(args, cfg, json).await,
        Commands::Doctor(args) => subcmds::doctor::run(args, cfg, json).await,
        Commands::Auth(args) => subcmds::auth::run(args, cfg, json).await,
        Commands::Uninstall(args) => {
            subcmds::uninstall::run(args, cfg.clw_config.cache_dir, json).await
        }
        // `completions` is handled before config resolution in `main` (it needs
        // no config); it never reaches `dispatch`.
        Commands::Completions(_) => Ok(()),
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
            Commands::Ls(a) => {
                assert_eq!(a.name.as_deref(), Some("some-ws"));
                assert!(!a.all);
            }
            other => panic!("expected Ls, got {other:?}"),
        }
    }

    #[test]
    fn parse_ls_all() {
        let cli = Cli::try_parse_from(["clw", "ls", "--all"]).unwrap();
        match &cli.command {
            Commands::Ls(a) => {
                assert!(a.all);
                assert!(a.name.is_none());
            }
            other => panic!("expected Ls, got {other:?}"),
        }
    }

    #[test]
    fn parse_ls_name_and_all_conflict() {
        // `--name` and `--all` are mutually exclusive (clap rejects the combo).
        assert!(Cli::try_parse_from(["clw", "ls", "--name", "x", "--all"]).is_err());
    }

    #[test]
    fn parse_rm() {
        let cli = Cli::try_parse_from(["clw", "rm", "--name", "some-ws"]).unwrap();
        match &cli.command {
            Commands::Rm(a) => {
                assert_eq!(a.name, "some-ws");
                assert!(!a.yes);
            }
            other => panic!("expected Rm, got {other:?}"),
        }
    }

    #[test]
    fn parse_rm_yes() {
        let cli = Cli::try_parse_from(["clw", "rm", "--name", "ws", "--yes"]).unwrap();
        match &cli.command {
            Commands::Rm(a) => assert!(a.yes),
            other => panic!("expected Rm, got {other:?}"),
        }
    }

    #[test]
    fn parse_prune_defaults_to_dry_run() {
        let cli = Cli::try_parse_from(["clw", "prune"]).unwrap();
        match &cli.command {
            Commands::Prune(a) => assert!(!a.yes, "prune must default to dry-run"),
            other => panic!("expected Prune, got {other:?}"),
        }
        let cli = Cli::try_parse_from(["clw", "prune", "--yes"]).unwrap();
        match &cli.command {
            Commands::Prune(a) => assert!(a.yes),
            other => panic!("expected Prune, got {other:?}"),
        }
    }

    #[test]
    fn parse_erase_defaults_to_dry_run() {
        let cli = Cli::try_parse_from(["clw", "erase"]).unwrap();
        match &cli.command {
            Commands::Erase(a) => {
                assert!(!a.yes, "erase must default to dry-run");
                assert!(a.confirm_tenant.is_none());
            }
            other => panic!("expected Erase, got {other:?}"),
        }
    }

    #[test]
    fn parse_erase_yes_with_confirm_tenant() {
        let cli =
            Cli::try_parse_from(["clw", "erase", "--yes", "--confirm-tenant", "acme"]).unwrap();
        match &cli.command {
            Commands::Erase(a) => {
                assert!(a.yes);
                assert_eq!(a.confirm_tenant.as_deref(), Some("acme"));
            }
            other => panic!("expected Erase, got {other:?}"),
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
    // clw uninstall + clw completions parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_uninstall_defaults() {
        let cli = Cli::try_parse_from(["clw", "uninstall"]).unwrap();
        match cli.command {
            Commands::Uninstall(a) => {
                assert!(!a.yes);
                assert!(!a.dry_run);
            }
            other => panic!("expected Uninstall, got {other:?}"),
        }
    }

    #[test]
    fn parse_uninstall_yes_and_dry_run() {
        let cli = Cli::try_parse_from(["clw", "uninstall", "--yes", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Uninstall(a) => {
                assert!(a.yes);
                assert!(a.dry_run);
            }
            other => panic!("expected Uninstall, got {other:?}"),
        }
    }

    #[test]
    fn parse_uninstall_with_json() {
        let cli = Cli::try_parse_from(["clw", "--json", "uninstall", "--yes"]).unwrap();
        assert!(cli.json);
        assert!(matches!(cli.command, Commands::Uninstall(_)));
    }

    #[test]
    fn parse_completions_each_shell() {
        for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
            let cli = Cli::try_parse_from(["clw", "completions", shell]).unwrap();
            assert!(
                matches!(cli.command, Commands::Completions(_)),
                "completions {shell} must parse"
            );
        }
    }

    #[test]
    fn parse_completions_rejects_unknown_shell() {
        let err = Cli::try_parse_from(["clw", "completions", "tcsh"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
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
    fn cli_debug_redacts_token() {
        // FIX 3: a `{cli:?}` diagnostic must never contain the PAT value.
        let cli = Cli::try_parse_from([
            "clw",
            "--tenant",
            "acme",
            "--token",
            "super-secret-pat-value",
            "ls",
            "--name",
            "ws",
        ])
        .unwrap();
        let dbg = format!("{cli:?}");
        assert!(
            !dbg.contains("super-secret-pat-value"),
            "Cli Debug must NOT contain the token value, got: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "Cli Debug must show '<redacted>' for a set token, got: {dbg}"
        );
        // Other fields are still shown.
        assert!(dbg.contains("acme"));

        // An unset token renders '<unset>', not '<redacted>'.
        let cli_no_tok =
            Cli::try_parse_from(["clw", "--tenant", "acme", "ls", "--name", "ws"]).unwrap();
        let dbg2 = format!("{cli_no_tok:?}");
        assert!(
            dbg2.contains("<unset>"),
            "Cli Debug must show '<unset>' when no token is set, got: {dbg2}"
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

    // -----------------------------------------------------------------------
    // M5.1 — diagnostic verbosity flags (-v/-q)
    // -----------------------------------------------------------------------

    #[test]
    fn verbose_counts_repeats() {
        let cli = Cli::try_parse_from(["clw", "-vv", "doctor"]).unwrap();
        assert_eq!(cli.verbose, 2);
        assert!(!cli.quiet);
    }

    #[test]
    fn verbose_defaults_to_zero() {
        let cli = Cli::try_parse_from(["clw", "doctor"]).unwrap();
        assert_eq!(cli.verbose, 0);
        assert!(!cli.quiet);
    }

    #[test]
    fn quiet_flag_parses() {
        let cli = Cli::try_parse_from(["clw", "-q", "doctor"]).unwrap();
        assert!(cli.quiet);
    }

    #[test]
    fn verbose_and_quiet_conflict() {
        // -q conflicts with -v: clap must reject combining them.
        let err = Cli::try_parse_from(["clw", "-v", "-q", "doctor"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn stderr_color_disabled_by_flag() {
        // --no-color always disables ANSI on the diagnostic (stderr) channel.
        assert!(!stderr_color_enabled(true));
    }

    // -----------------------------------------------------------------------
    // P1/P2 — which commands run on a broken/half-configured config
    // -----------------------------------------------------------------------

    #[test]
    fn doctor_and_local_auth_run_on_broken_config() {
        // doctor (P1) and the offline auth logout/whoami (P2) must fall back to a
        // lenient resolution so they run on a config missing tenant/token.
        let doctor = Cli::try_parse_from(["clw", "doctor"]).unwrap();
        assert!(command_runs_on_broken_config(&doctor.command));

        let logout = Cli::try_parse_from(["clw", "auth", "logout"]).unwrap();
        assert!(
            command_runs_on_broken_config(&logout.command),
            "auth logout must run on a half-configured config"
        );

        let whoami = Cli::try_parse_from(["clw", "auth", "whoami"]).unwrap();
        assert!(
            command_runs_on_broken_config(&whoami.command),
            "auth whoami must run on a half-configured config"
        );
    }

    #[test]
    fn network_and_persisting_commands_still_require_full_config() {
        // auth login PERSISTS the resolved identity, so it keeps the strict gate;
        // rotate and network commands likewise are NOT lenient.
        let login = Cli::try_parse_from(["clw", "auth", "login"]).unwrap();
        assert!(!command_runs_on_broken_config(&login.command));

        let rotate = Cli::try_parse_from(["clw", "auth", "rotate"]).unwrap();
        assert!(!command_runs_on_broken_config(&rotate.command));

        let ls = Cli::try_parse_from(["clw", "ls", "--name", "ws"]).unwrap();
        assert!(!command_runs_on_broken_config(&ls.command));

        let snap = Cli::try_parse_from(["clw", "snapshot", "--name", "ws"]).unwrap();
        assert!(!command_runs_on_broken_config(&snap.command));
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
