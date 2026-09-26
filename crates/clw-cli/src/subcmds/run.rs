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
use clw_run::{initialize_execution_state, run_memoized, run_memoized_with_state_file, RunOptions};
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
/// ran** (substrate unreachable, bad config, hydrate/cache error). A child may also
/// return 125, so callers use the optional execution receipt rather than treating
/// this numeric value as proof. This wrapper error is never cached. 125 is the docker / `env` /
/// `timeout` "the wrapper itself failed" convention. FROZEN — see docs/CLI-CONTRACT.md.
///
/// `pub` so `main.rs` can route a `clw run` that fails DURING config/identity
/// resolution (before dispatch ever reaches this subcommand) through the SAME
/// reserved code — a single source of truth for the frozen value.
pub const CLW_RUN_INTERNAL_NOT_RUN: i32 = 125;

/// Exit code for a `clw run` whose child DID run but was KILLED BY clw because its
/// captured output exceeded the CAS blob cap (`ClwError::TooLarge`). This is a
/// DISTINCT clw-error exit — deliberately NOT the reserved [`CLW_RUN_INTERNAL_NOT_RUN`]
/// (125) "child never ran" sentinel, because here the child ran to the point clw had
/// to terminate it. clw kills the oversized child with SIGKILL, so we surface the
/// POSIX 128+signal convention for a SIGKILL (`128 + 9 = 137`) — the same convention
/// `clw-run` uses for any signal-killed child — which is a truthful "the child was
/// killed by SIGKILL" signal, plainly separate from the 125 wrapper-never-ran code.
/// Like every non-zero `clw run` outcome it is NEVER cached (clw-run only stores on a
/// clean exit 0, and this is an error return that never reaches memoization).
pub const CLW_RUN_OUTPUT_TOO_LARGE: i32 = 137;

/// Map a `run_memoized` failure to the `clw run` exit code. The child NEVER ran →
/// the reserved [`CLW_RUN_INTERNAL_NOT_RUN`] (125); the child RAN but clw killed it
/// for exceeding the output cap ([`ClwError::TooLarge`]) → the distinct
/// [`CLW_RUN_OUTPUT_TOO_LARGE`] (137). Pure so the frozen mapping is unit-tested
/// without driving a real child through `std::process::exit`.
/// The run layer reports an oversized initial AC response as a lookup Transport
/// failure before this mapper; it is not captured child output (#305).
fn run_err_exit_code(e: &ClwError) -> i32 {
    match e {
        ClwError::TooLarge { .. } => CLW_RUN_OUTPUT_TOO_LARGE,
        _ => CLW_RUN_INTERNAL_NOT_RUN,
    }
}

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
    // `clw run` passes a completed child's exit code verbatim or returns a wrapper
    // error code. A child may return the same numeric value (notably 125), so callers
    // that need retry safety use the optional out-of-band execution-state receipt.
    let state_file = std::env::var_os("CLW_RUN_STATE_FILE").map(std::path::PathBuf::from);
    if let Some(path) = state_file.as_deref() {
        if let Err(error) = initialize_execution_state(path) {
            eprintln!("[clw] could not initialize run execution receipt: {error}");
            std::process::exit(CLW_RUN_INTERNAL_NOT_RUN);
        }
    }
    let outcome = match async move {
        let client = HttpClient::new(cfg.clw_config)?;
        let cwd = std::env::current_dir()?;
        if let Some(path) = state_file.as_deref() {
            run_memoized_with_state_file(&cwd, &client, &opts, path).await
        } else {
            run_memoized(&cwd, &client, &opts).await
        }
    }
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            // Two distinct clw-error exits (see `run_err_exit_code`):
            //   * TooLarge — the child DID run but clw killed it for exceeding the
            //     output cap → 137 (SIGKILL), NOT the reserved 125 child-never-ran
            //     sentinel, so the runner does not mis-classify a real killed child.
            //   * everything else — clw-internal, the child NEVER ran → reserved 125.
            // Both are error returns that never reached memoization, so NOT cached.
            match &e {
                ClwError::TooLarge { size, cap } => {
                    eprintln!(
                        "[clw] child output exceeded the cap ({size} bytes > {cap}); child killed"
                    );
                }
                // Route through `actionable_error` so a clw-internal failure on the
                // `run` path (hydrate/lookup) surfaces the ACTIONABLE message — most
                // importantly a 402 renders the quota/plan/upgrade guidance instead of
                // a bare `HTTP 402 from …`. Message text only: the reserved exit-125
                // "child never ran" contract below is unchanged.
                _ => eprintln!(
                    "[clw] internal error — child not run: {}",
                    crate::actionable_error(&e)
                ),
            }
            std::process::exit(run_err_exit_code(&e));
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

    // LOW — oversized output is a child that RAN and was killed by clw, NOT a
    // child-never-ran failure. Its exit must be DISTINCT from the reserved 125
    // sentinel. RED-on-revert: mapping TooLarge back to CLW_RUN_INTERNAL_NOT_RUN
    // (125) makes both assertions below fail.
    #[test]
    fn oversized_output_maps_to_distinct_non_125_exit() {
        let too_large = ClwError::TooLarge {
            size: (5 * 1024 * 1024) + 1,
            cap: 5 * 1024 * 1024,
        };
        let code = run_err_exit_code(&too_large);
        assert_ne!(
            code, CLW_RUN_INTERNAL_NOT_RUN,
            "a child that RAN but was killed for oversized output must NOT surface \
             the reserved 125 child-never-ran sentinel"
        );
        assert_eq!(
            code, CLW_RUN_OUTPUT_TOO_LARGE,
            "oversized output must map to the distinct clw-error exit (137)"
        );
        assert_eq!(
            CLW_RUN_OUTPUT_TOO_LARGE, 137,
            "SIGKILL convention (128 + 9)"
        );

        // Any OTHER error is the child-never-ran path → the reserved 125.
        assert_eq!(
            run_err_exit_code(&ClwError::Config("bad".into())),
            CLW_RUN_INTERNAL_NOT_RUN,
            "a non-TooLarge (child-never-ran) error keeps the reserved 125 code"
        );
    }

    // FIX #3 (LOW-MED): a 402 quota wall hit DURING `clw run`'s hydrate/lookup must
    // surface the actionable plan/upgrade guidance, not the raw `HTTP 402 from …`
    // Display. The internal-failure arm routes the error through
    // `crate::actionable_error`, so its message must carry the quota/plan wording.
    // RED-on-revert: reverting the arm to the raw `{e}` prints `e.to_string()` (the
    // "HTTP 402 from …" Display), which contains none of these words → this fails.
    #[test]
    fn run_internal_failure_routes_402_through_actionable_error() {
        let e = ClwError::Http {
            status: 402,
            context: "cas.example/tenant/blobs".to_string(),
        };
        let msg = crate::actionable_error(&e);

        // The actionable message names the plan/billing wall and the next step.
        assert!(
            msg.contains("quota"),
            "402 run-path message must mention the usage quota, got: {msg}"
        );
        assert!(
            msg.contains("plan") || msg.contains("upgrade"),
            "402 run-path message must give plan/upgrade guidance, got: {msg}"
        );

        // And it must NOT be the bare Display the raw `{e}` arm would have printed.
        assert_ne!(
            msg,
            e.to_string(),
            "the run path must render the actionable message, not the raw HTTP Display"
        );
        assert!(
            !msg.contains("HTTP 402 from"),
            "the actionable message must replace the raw `HTTP 402 from …` Display, got: {msg}"
        );
    }
}
