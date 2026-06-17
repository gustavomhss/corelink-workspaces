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
                  regardless of ignore rules) or un-ignore it.\n\n\
                  The global --json and --concurrency flags DO NOT APPLY to `run`: it proxies the \
                  child's stdout/stderr verbatim (so there is no clw JSON payload to emit) and runs \
                  the single wrapped command (so there is nothing to parallelize). Passing --json \
                  with `run` prints a one-line note on stderr and is otherwise ignored."
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

/// P6: the one-line note printed (to STDERR, never stdout — that belongs to the
/// child) when `--json` is passed with `run`, which has no JSON form. Extracted as
/// a pure seam so the wording can be asserted without driving the child process.
const JSON_NOT_APPLICABLE_HINT: &str =
    "[clw] note: --json does not apply to `run` (it proxies the child's stdout/stderr); ignoring it";

/// Reserved exit code for a `clw run` clw-INTERNAL failure where the **child never
/// ran** (substrate unreachable, bad config, hydrate/cache error). Distinct from the
/// child's passed-through exit code so a caller (e.g. the CoreLink runner) can tell a
/// clw failure from a child result, and never caches it. 125 is the docker / `env` /
/// `timeout` "the wrapper itself failed" convention. FROZEN — see docs/CLI-CONTRACT.md.
const CLW_RUN_INTERNAL_NOT_RUN: i32 = 125;

pub async fn run(args: RunArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    // P6: `run` has no JSON form — it proxies the child's stdout/stderr verbatim,
    // so a global `--json` would be silently ignored. Emit a one-line stderr hint
    // (never stdout — that belongs to the child) so the user is not left guessing.
    if json {
        eprintln!("{JSON_NOT_APPLICABLE_HINT}");
    }

    let opts = RunOptions {
        inputs: args.input,
        command: args.cmd,
        env_keys: args.env,
    };

    // EXIT-CODE CONTRACT (frozen — see docs/CLI-CONTRACT.md "exit-code convention"):
    // `clw run` either runs the child (and passes the child's exit code through
    // verbatim below) OR fails BEFORE the child ever runs — substrate unreachable,
    // bad config, hydrate/cache error. That latter case is a clw-INTERNAL failure:
    // the child never executed, so its result must NOT be mistaken for a child exit
    // code and must NOT be cached. Signal it with the RESERVED exit code 125 — the
    // docker / `env` / `timeout` "the wrapper itself failed" convention — which is
    // distinct from the child passthrough. (A child SHOULD avoid emitting 125-127.)
    let outcome = match async move {
        let client = HttpClient::new(cfg.clw_config)?;
        let cwd = std::env::current_dir()?;
        run_memoized(&cwd, &client, &opts).await
    }
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            // clw-internal: child NOT run. Reserved code, NOT a child exit, NOT cached.
            eprintln!("[clw] internal error — child not run: {e}");
            std::process::exit(CLW_RUN_INTERNAL_NOT_RUN);
        }
    };

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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // P6 — the --json-with-run hint is actionable and clarifies why it's ignored.
    #[test]
    fn json_hint_explains_run_has_no_json_form() {
        assert!(JSON_NOT_APPLICABLE_HINT.contains("--json"));
        assert!(
            JSON_NOT_APPLICABLE_HINT.contains("does not apply"),
            "hint must say --json doesn't apply"
        );
        assert!(
            JSON_NOT_APPLICABLE_HINT.contains("proxies"),
            "hint must explain it proxies the child's streams"
        );
    }

    // P6 — `run --help` documents that --json/--concurrency don't apply.
    #[test]
    fn run_help_documents_json_and_concurrency_dont_apply() {
        use clap::Args as _;

        // Augment a bare command with RunArgs' derived surface (incl. the
        // long_about) and render its long help.
        let cmd = RunArgs::augment_args(clap::Command::new("run"));
        let long = cmd.clone().render_long_help().to_string();
        assert!(long.contains("--json"), "run --help must mention --json");
        assert!(
            long.contains("--concurrency"),
            "run --help must mention --concurrency"
        );
        assert!(
            long.contains("DO NOT APPLY") || long.contains("do not apply"),
            "run --help must state they don't apply, got:\n{long}"
        );
    }

    // CONTRACT TRIPWIRE: the CoreLink runner classifies a `clw run` exit of 125 as a
    // clw-internal failure (child never ran) and never caches it — see
    // docs/CLI-SURFACE-EXIT-FREEZE.md. Pin the value so it cannot drift silently.
    #[test]
    fn clw_run_internal_not_run_code_is_frozen_at_125() {
        assert_eq!(
            CLW_RUN_INTERNAL_NOT_RUN, 125,
            "the runner contract pins clw-internal (child-not-run) at exit 125"
        );
    }
}
