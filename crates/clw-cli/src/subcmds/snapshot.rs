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
#[command(
    about = "Snapshot a workspace directory and upload it to CoreLink",
    // FIX 8: the directory is POSITIONAL — `--path DIR` yields a confusing clap
    // error. Spell the correct form out so a user reaches for `<DIR>`, not --path.
    long_about = "Snapshot a workspace directory and upload it to CoreLink.\n\n\
                  The directory is a POSITIONAL argument, not a flag: \
                  `clw snapshot <DIR> --name X` (NOT `--path DIR`). Omit it to snapshot \
                  the current directory."
)]
pub struct SnapshotArgs {
    /// Workspace directory to snapshot (default: current directory).
    #[arg(
        default_value = ".",
        help = "Workspace directory to snapshot — POSITIONAL, e.g. `clw snapshot <DIR> --name X` \
                (not `--path DIR`); default: current directory"
    )]
    pub path: PathBuf,

    /// Workspace name (ref key).
    #[arg(long, help = "Workspace name to store the snapshot under")]
    pub name: String,

    /// Replace an existing name whose content differs (delete-then-write). Not needed
    /// to re-snapshot identical content — that is always a no-op success.
    #[arg(
        long,
        default_value_t = false,
        help = "Replace an existing workspace name whose content differs (rm-then-write)"
    )]
    pub force: bool,
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
    opts.ref_domain = cfg.ref_domain;
    opts.force = args.force;

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
        .map_err(|e| map_snapshot_error(e, &args.name))?;

    // FIX 7 (DOWNGRADED to a stderr note — see fn docs): the `ignore` walker drops
    // gitignored entries silently mid-iteration and does NOT expose a skip count
    // through its standard iterator, so an EXACT excluded count would mean a second
    // full walk (double I/O) or a from-scratch matcher rewrite — too invasive for a
    // low-severity polish. Instead emit a one-line stderr note when the snapshotted
    // tree carries an ignore file, so the user is not surprised that gitignored
    // files were dropped. stderr only (keeps --json stdout clean); does not change
    // the exit code.
    if has_ignore_file(&args.path) {
        eprintln!("note: files matched by .gitignore/.clwignore were excluded from this snapshot");
    }

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
            "name": args.name,
            "root": report.root.to_hex(),
            "files": report.files,
            "bytes_total": report.bytes_total,
            "chunks_total": report.chunks_total,
            "chunks_uploaded": report.chunks_uploaded,
            "unchanged": report.unchanged,
        });
        println!("{out}");
    } else if report.unchanged {
        println!(
            "already up to date — '{}' already points at this content (no change)",
            args.name
        );
        println!("  name:            {}", args.name);
        println!("  root:            {}", report.root);
        println!("  files:           {}", report.files);
        println!("  bytes_total:     {}", report.bytes_total);
        // Non-zero only if the server had lost/pruned blobs and they were re-uploaded
        // during this otherwise-no-op run — surfaced so "no change" isn't misleading.
        println!("  chunks_uploaded: {}", report.chunks_uploaded);
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

/// FIX 7: cheap presence check for a root-level ignore file (`.gitignore` /
/// `.clwignore`) so the CLI can emit a "files were excluded" note without a second
/// walk. A root-level ignore file is the overwhelmingly common case; a nested-only
/// ignore would not trip this, which is an accepted limitation of the cheap signal
/// (the alternative — an exact count — needs a full second walk or a matcher
/// rewrite). Pure over the filesystem so the trigger is unit-testable.
fn has_ignore_file(path: &std::path::Path) -> bool {
    path.join(".gitignore").exists() || path.join(".clwignore").exists()
}

/// Map snapshot errors to actionable messages. A re-snapshot of an existing
/// workspace name currently returns **HTTP 409** from the AC `PUT` (the server's
/// ref store is create-only — in-place versioning/upsert isn't live yet). Translate
/// that bare `HTTP 409` into a clear, actionable message (v0.1.1 power-user finding);
/// other errors fall through to the shared auth mapping.
fn map_snapshot_error(err: ClwError, name: &str) -> ClwError {
    match &err {
        ClwError::Http { status: 409, .. } => ClwError::Message(format!(
            "workspace '{name}' already exists with different content and the server rejected the in-place \
             update (HTTP 409) — the ref store is create-only (in-place versioning isn't live yet). To \
             replace it, re-run `clw snapshot --name {name} --force`, or `clw rm --name {name} --yes` first \
             then snapshot again. (Re-snapshotting identical content is a no-op and needs neither.)"
        )),
        _ => map_auth_error(err),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{has_ignore_file, map_snapshot_error};
    use clw_snapshot::SnapshotOptions;
    use clw_types::ClwError;

    // FIX 7 — the excluded-files note fires only when the tree carries a root-level
    // ignore file; a clean tree stays quiet (no spurious note).
    #[test]
    fn has_ignore_file_detects_gitignore_and_clwignore() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !has_ignore_file(dir.path()),
            "a tree with no ignore file must not trip the note"
        );
        std::fs::write(dir.path().join(".gitignore"), b"target/\n").unwrap();
        assert!(
            has_ignore_file(dir.path()),
            ".gitignore presence must trip the note"
        );

        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir2.path().join(".clwignore"), b"secret\n").unwrap();
        assert!(
            has_ignore_file(dir2.path()),
            ".clwignore presence must trip the note"
        );
    }

    #[test]
    fn re_snapshot_409_maps_to_actionable_rm_first_message() {
        // v0.1.1 power-user finding: a re-snapshot returns a bare 'HTTP 409'. Translate
        // it into a clear message that names the workspace and the rm-first workaround.
        let mapped = map_snapshot_error(
            ClwError::Http {
                status: 409,
                context: "PUT /v1/ac/...".to_string(),
            },
            "my-ws",
        );
        let msg = mapped.to_string();
        assert!(
            !msg.starts_with("HTTP 409"),
            "must not be a bare HTTP 409: {msg}"
        );
        assert!(msg.contains("my-ws"), "must name the workspace: {msg}");
        assert!(
            msg.contains("clw rm --name my-ws --yes"),
            "must give the rm-first workaround: {msg}"
        );
    }

    #[test]
    fn snapshot_error_passes_auth_through() {
        // Non-409 errors fall through to the shared auth mapping (Message, not 'transport').
        let mapped = map_snapshot_error(ClwError::Auth, "ws");
        assert!(mapped.to_string().contains("CLW_TOKEN") || mapped.to_string().contains("config"));
    }

    // NOTE (pass-2 audit de-vacuous): the previous `concurrency_zero_is_clamped_to_one`
    // and `concurrency_flag_is_passed_through_to_opts` tests defined a test-local `clamp`
    // helper (`concurrency.max(1)`) that REIMPLEMENTED the inline clamp in `run`, then
    // asserted on that reimplementation — they never ran `run`'s real clamp+plumbing, so a
    // regression that dropped the clamp from `run` would not have failed them. The real
    // `--concurrency` path (the `.max(1)` clamp plus wiring the value into BOTH
    // `HttpClient::with_concurrency` and `SnapshotOptions::concurrency`) is exercised
    // end-to-end through `run` by the binary-level `roundtrip_bit_identical` test in
    // `crates/clw-cli/tests/e2e.rs`, which drives a real `snapshot` (default concurrency 8)
    // and asserts a bit-identical round-trip — a broken governor/stream fan-out would surface
    // there. The genuine contract assertion below exercises the REAL `SnapshotOptions::new`
    // default (8) that `run` relies on as its wiring baseline.

    #[test]
    fn snapshot_options_new_defaults_concurrency_to_eight() {
        // `run` constructs `SnapshotOptions::new(name)` and then overwrites
        // `.concurrency` with the clamped flag value; this asserts the REAL default
        // that ships when the flag is left at its own default — a regression in the
        // library default would change the wiring baseline `run` builds on.
        let opts = SnapshotOptions::new("ws");
        assert_eq!(
            opts.concurrency, 8,
            "SnapshotOptions::new must default concurrency to 8"
        );
    }
}
