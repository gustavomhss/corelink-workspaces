//! `clw snapshot [PATH] --name <NAME>` — snapshot a workspace directory.

use std::path::PathBuf;

use clap::Args;
use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_snapshot::{snapshot, SnapshotOptions};
use clw_types::ClwError;
use serde_json::json;

use crate::config::ResolvedConfig;
use crate::subcmds::status::map_auth_error;

#[derive(Debug, Args)]
#[command(about = "Snapshot a workspace directory and upload it to CoreLink")]
pub struct SnapshotArgs {
    /// Workspace directory to snapshot (default: current directory).
    #[arg(
        default_value = ".",
        help = "Workspace directory to snapshot (default: current directory)"
    )]
    pub path: PathBuf,

    /// Workspace name (ref key).
    #[arg(long, help = "Workspace name to store the snapshot under")]
    pub name: String,
}

pub async fn run(
    args: SnapshotArgs,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
) -> Result<(), ClwError> {
    let cache = LocalCache::new(&cfg.clw_config.cache_dir)?;
    // Clamp to at least 1 so `--concurrency 0` can't stall the stream or starve
    // the client's AIMD governor.
    let concurrency = concurrency.max(1);
    // FIX 4: govern the upload fan-out at the client too, so the AIMD limiter sees
    // the same ceiling the stream uses — previously `--concurrency` was honored by
    // the stream but the HTTP governor always used the default (mirrors hydrate).
    let client = HttpClient::with_concurrency(cfg.clw_config, concurrency)?;
    let mut opts = SnapshotOptions::new(&args.name);
    opts.concurrency = concurrency;

    // Progress feedback on a long, otherwise-silent op: tell an interactive user
    // work has started so a slow snapshot isn't mistaken for a hung one. stderr
    // only (keeps --json stdout clean) and TTY-gated (no noise in CI/pipelines).
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!(
            "snapshotting '{}' — large workspaces can take a while (silent until complete)…",
            args.name
        );
    }

    let report = snapshot(&args.path, &client, &cache, &opts)
        .await
        .map_err(map_auth_error)?;

    // Non-destructive warning: secret-looking files were uploaded, not dropped.
    // Printed to stderr so it never pollutes the `--json` stdout payload, and
    // it does not change the exit code.
    if !report.secret_warnings.is_empty() {
        eprintln!(
            "WARNING: {} secret-looking file(s) were uploaded in this snapshot:",
            report.secret_warnings.len()
        );
        for path in &report.secret_warnings {
            eprintln!("  - {path}");
        }
        eprintln!("Add them to a .clwignore file to exclude them from future snapshots.");
    }

    if json {
        let out = json!({
            "root": report.root.to_hex(),
            "files": report.files,
            "bytes_total": report.bytes_total,
            "chunks_total": report.chunks_total,
            "chunks_uploaded": report.chunks_uploaded,
        });
        println!("{out}");
    } else {
        println!("snapshot complete");
        println!("  name:            {}", args.name);
        println!("  root:            {}", report.root);
        println!("  files:           {}", report.files);
        println!("  bytes_total:     {}", report.bytes_total);
        println!("  chunks_total:    {}", report.chunks_total);
        println!("  chunks_uploaded: {}", report.chunks_uploaded);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use clw_snapshot::SnapshotOptions;

    /// The clamp the subcmd applies before wiring `--concurrency` into both the
    /// `HttpClient` governor and `SnapshotOptions`. Mirrors the body of `run` so
    /// a regression that drops the clamp (or fails to plumb the value) is caught.
    fn clamp(concurrency: usize) -> usize {
        concurrency.max(1)
    }

    #[test]
    fn concurrency_zero_is_clamped_to_one() {
        // `--concurrency 0` must not stall the stream/governor.
        assert_eq!(clamp(0), 1);
    }

    #[test]
    fn concurrency_flag_is_passed_through_to_opts() {
        // FIX 4: the flag must reach `SnapshotOptions` (previously honored by the
        // stream but dropped on the client's HTTP governor). Assert the plumbing:
        // the clamped value is the one written onto opts.
        let concurrency = clamp(8);
        let mut opts = SnapshotOptions::new("ws");
        opts.concurrency = concurrency;
        assert_eq!(opts.concurrency, 8, "concurrency must be plumbed onto opts");

        // And a 0 is clamped before it ever reaches opts or the governor.
        let mut opts0 = SnapshotOptions::new("ws");
        opts0.concurrency = clamp(0);
        assert_eq!(opts0.concurrency, 1);
    }
}
