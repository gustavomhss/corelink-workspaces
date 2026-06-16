//! `clw run --input <PATH>... [--env <KEY>...] -- <CMD>...` — memoized command execution.
//!
//! Streams captured stdout/stderr to real stdout/stderr after completion.
//! Prints `[clw] cache hit` / `[clw] cache miss` to stderr.
//! Exits with the child's exit code.
//!
//! ## Hermeticity contract
//!
//! The memoization key captures ONLY: the declared `--input` paths (file/dir
//! content digests), the `--env` variable values, the full command vector, and
//! the resolved tool identity (the content digest of `command[0]`). Anything
//! NOT in that set is INVISIBLE to the cache: network access, the clock/`$NOW`,
//! random sources, and any file the command reads that was not passed via
//! `--input`. Such undeclared inputs are therefore NOT captured, so a hit may
//! replay a stale result for a non-deterministic command. Only memoize commands
//! that are deterministic given their declared inputs.

use std::io::Write as _;

use clap::Args;
use clw_client::HttpClient;
use clw_run::{run_memoized, RunOptions};
use clw_types::ClwError;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(
    about = "Run a command and memoize its output; replays stdout/stderr on cache hit",
    long_about = "Run a command and memoize its output, replaying stdout/stderr on a cache hit.\n\n\
                  The cache key is built from the --input paths, --env values, and the command \
                  vector. CAVEAT: input DIRECTORIES are walked respecting .gitignore/.clwignore, \
                  so an ignored file inside an input dir (e.g. a local .env) is NOT part of the \
                  key — changing it will not invalidate the cache and a run may replay a stale \
                  result. Pass such a file as an explicit --input <FILE> (named files are digested \
                  regardless of ignore rules) or un-ignore it."
)]
pub struct RunArgs {
    /// Input paths (files or directories) that key the memoization.
    #[arg(
        long = "input",
        required = true,
        help = "Input paths (files or dirs) that form the cache key; repeat for multiple. \
                Dirs respect .gitignore/.clwignore — ignored files are NOT in the key; \
                pass an ignored file as an explicit --input <FILE> to include it"
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
