//! `clw run --input <PATH>... [--env <KEY>...] -- <CMD>...` — memoized command execution.
//!
//! Streams captured stdout/stderr to real stdout/stderr after completion.
//! Prints `[clw] cache hit` / `[clw] cache miss` to stderr.
//! Exits with the child's exit code.

use std::io::Write as _;

use clap::Args;
use clw_client::HttpClient;
use clw_run::{run_memoized, RunOptions};
use clw_types::ClwError;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(about = "Run a command and memoize its output; replays stdout/stderr on cache hit")]
pub struct RunArgs {
    /// Input paths (files or directories) that key the memoization.
    #[arg(
        long = "input",
        required = true,
        help = "Input paths (files or dirs) that form the cache key; repeat for multiple"
    )]
    pub input: Vec<String>,

    /// Environment variable names to include in the cache key.
    #[arg(
        long = "env",
        help = "Environment variable names to fold into the cache key; repeat for multiple"
    )]
    pub env: Vec<String>,

    /// The command to run (after `--`).
    #[arg(
        last = true,
        required = true,
        help = "Command and arguments to run (pass after --)"
    )]
    pub cmd: Vec<String>,
}

pub async fn run(args: RunArgs, cfg: ResolvedConfig) -> Result<(), ClwError> {
    let client = HttpClient::new(cfg.clw_config)?;
    let cwd = std::env::current_dir()?;

    let opts = RunOptions {
        inputs: args.input,
        command: args.cmd,
        env_keys: args.env,
    };

    let outcome = run_memoized(&cwd, &client, &opts).await?;

    if outcome.hit {
        eprintln!("[clw] cache hit");
    } else {
        eprintln!("[clw] cache miss");
    }

    // Stream captured output to real stdout/stderr.
    std::io::stdout().write_all(&outcome.stdout).ok();
    std::io::stderr().write_all(&outcome.stderr).ok();

    // Propagate child exit code.
    std::process::exit(outcome.exit_code);
}
