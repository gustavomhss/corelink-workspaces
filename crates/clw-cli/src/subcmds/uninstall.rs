//! `clw uninstall` — clean removal of clw's on-disk footprint.
//!
//! clw persists two sensitive things on disk: the plaintext personal access
//! token (PAT) in `~/.clw/config.toml`, and a potentially LARGE cache of
//! customer file content under the resolved cache dir. `uninstall` removes both,
//! so a user can fully scrub the machine.
//!
//! INVARIANTS (match the rest of the CLI):
//! - The PAT is NEVER printed. We report PATHS that were/would be removed, never
//!   any file content or the token value.
//! - Paths are resolved via the shared [`home_dir`]/config helpers and the
//!   already-resolved `cache_dir` — NEVER a hardcoded `~/.clw`. The cache dir may
//!   be configured OUTSIDE `~/.clw`, so we remove the actual resolved dir too.
//! - Destruction requires consent: interactive `y/N` unless `--yes`. In `--json`
//!   or any non-interactive (non-TTY) context we do NOT prompt and REQUIRE
//!   `--yes` — `--json` never bypasses consent (mirrors `clw auth logout --all`).
//! - We do NOT self-delete the running binary: removing a running executable is
//!   fragile and cross-platform-unsafe. We PRINT its path and the exact command
//!   to remove it, and let the user run that.

use clap::Args;
use clw_types::ClwError;
use serde_json::json;

use crate::config::home_dir;

#[derive(Debug, Args)]
#[command(about = "Remove clw's on-disk footprint (~/.clw config + PAT, cache)")]
pub struct UninstallArgs {
    /// Skip the confirmation prompt (REQUIRED in --json/non-interactive mode).
    #[arg(
        long,
        default_value_t = false,
        help = "Confirm removal without prompting (required with --json or when non-interactive)"
    )]
    pub yes: bool,

    /// List what would be removed without deleting anything.
    #[arg(
        long,
        default_value_t = false,
        help = "Show what would be removed without deleting anything"
    )]
    pub dry_run: bool,
}

/// A removal target: a path and whether it currently exists on disk.
struct Target {
    path: std::path::PathBuf,
    /// Human label for the non-JSON summary (e.g. "config dir").
    label: &'static str,
}

/// Resolve the set of paths `uninstall` operates on.
///
/// PURE over its inputs (the resolved config dir + cache dir), so it can be unit
/// tested without touching `$HOME`. Returns the config dir (`~/.clw`, holding the
/// PAT) and the resolved cache dir. The cache dir is only
/// listed separately when it is NOT already inside the config dir (the default
/// `~/.clw/cache` is removed transitively by removing `~/.clw`).
fn resolve_targets(
    config_dir: Option<std::path::PathBuf>,
    cache_dir: &std::path::Path,
) -> Vec<Target> {
    let mut targets = Vec::new();

    if let Some(cfg) = &config_dir {
        targets.push(Target {
            path: cfg.clone(),
            label: "config dir (config.toml with PAT)",
        });
    }

    // Only add the cache dir as a SEPARATE target when it is not nested under the
    // config dir — otherwise removing ~/.clw already takes it. This keeps the
    // honored-resolved-cache-dir-outside-~/.clw case covered without double-listing.
    let cache_inside_config = config_dir
        .as_ref()
        .is_some_and(|cfg| cache_dir.starts_with(cfg));
    if !cache_inside_config {
        targets.push(Target {
            path: cache_dir.to_path_buf(),
            label: "cache dir (customer file content)",
        });
    }

    targets
}

/// Return `~/.clw` (the config DIR, parent of config.toml) via the shared
/// `home_dir()` helper. NEVER hardcoded — `None` if no HOME/USERPROFILE.
fn clw_config_dir() -> Option<std::path::PathBuf> {
    home_dir().map(|h| h.join(".clw"))
}

/// Whether we may prompt interactively (stdin is a TTY).
fn is_interactive() -> bool {
    use std::io::IsTerminal as _;
    std::io::stdin().is_terminal()
}

/// Best-effort path to the running binary, for the self-removal instruction.
fn current_exe_display() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "the clw binary".to_string())
}

#[allow(clippy::unused_async)]
pub async fn run(
    args: UninstallArgs,
    cache_dir: std::path::PathBuf,
    json: bool,
) -> Result<(), ClwError> {
    let targets = resolve_targets(clw_config_dir(), &cache_dir);

    // --- consent ----------------------------------------------------------
    // A real (non-dry-run) deletion needs consent. `--json` and non-interactive
    // contexts must NOT be prompted — they REQUIRE `--yes`. Consent is gated on
    // the OUTPUT shape only via the message, never bypassed by it.
    if !args.dry_run && !args.yes {
        if json || !is_interactive() {
            return Err(ClwError::Transport(
                "refusing to uninstall without confirmation: pass --yes \
                 (required with --json or when non-interactive)"
                    .to_string(),
            ));
        }
        if !confirm(&targets)? {
            if json {
                // (Unreachable: json path already returned above, but keep the
                // shape consistent if the guard ever changes.)
                println!(
                    "{}",
                    json!({ "removed": [], "note": "aborted — confirmation declined" })
                );
            } else {
                eprintln!("aborted — nothing was removed");
            }
            return Ok(());
        }
    }

    // --- act --------------------------------------------------------------
    // Collect what exists (would be removed) and perform removal unless dry-run.
    let mut acted: Vec<(&Target, bool)> = Vec::new(); // (target, existed)
    for t in &targets {
        let existed = t.path.exists();
        if existed && !args.dry_run {
            // Directories are removed recursively; a stray file is removed too.
            let res = if t.path.is_dir() {
                std::fs::remove_dir_all(&t.path)
            } else {
                std::fs::remove_file(&t.path)
            };
            if let Err(e) = res {
                return Err(ClwError::Transport(format!(
                    "failed to remove {}: {e}",
                    t.path.display()
                )));
            }
        }
        acted.push((t, existed));
    }

    let exe = current_exe_display();
    report(&acted, &exe, args.dry_run, json);
    Ok(())
}

/// Interactive confirmation prompt (stderr), listing the dirs to be removed.
fn confirm(targets: &[Target]) -> Result<bool, ClwError> {
    use std::io::Write as _;
    eprintln!("clw uninstall will remove (this cannot be undone):");
    for t in targets {
        eprintln!("  {} — {}", t.path.display(), t.label);
    }
    eprint!("proceed? [y/N]: ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

/// Print the result. Paths only — never the token or any file content.
fn report(acted: &[(&Target, bool)], exe: &str, dry_run: bool, json: bool) {
    if json {
        println!("{}", build_report_json(acted, exe, dry_run));
        return;
    }

    if dry_run {
        eprintln!("clw uninstall --dry-run — nothing was removed. Would remove:");
    } else {
        eprintln!("clw uninstall — removed:");
    }
    for (t, existed) in acted {
        let status = if dry_run {
            if *existed {
                "would remove"
            } else {
                "would remove (absent)"
            }
        } else if *existed {
            "removed"
        } else {
            "absent (nothing to do)"
        };
        eprintln!("  {} — {} [{}]", t.path.display(), t.label, status);
    }

    // Self-removal instruction — we never delete the running binary ourselves.
    eprintln!();
    eprintln!(
        "the clw binary itself was NOT removed (a running executable cannot safely self-delete)."
    );
    eprintln!("to remove it, run:");
    eprintln!("  rm {exe}");
}

/// Build the machine-readable report. Reports PATHS only — by construction it
/// never contains the token (we never read or echo config.toml content here).
fn build_report_json(acted: &[(&Target, bool)], exe: &str, dry_run: bool) -> serde_json::Value {
    let removed: Vec<serde_json::Value> = acted
        .iter()
        .map(|(t, existed)| {
            json!({
                "path": t.path.display().to_string(),
                "existed": existed,
                // In dry-run nothing is deleted; otherwise an existing target was.
                "removed": !dry_run && *existed,
            })
        })
        .collect();

    json!({
        "dry_run": dry_run,
        "targets": removed,
        // The running binary is reported but NOT deleted; the caller removes it.
        "binary": {
            "path": exe,
            "removed": false,
            "remove_command": format!("rm {exe}"),
        },
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // resolve_targets
    // -----------------------------------------------------------------------

    #[test]
    fn default_cache_inside_config_is_not_double_listed() {
        // Default layout: cache lives under ~/.clw, so removing the config dir
        // takes it transitively — only ONE target (the config dir).
        let cfg = PathBuf::from("/home/u/.clw");
        let cache = PathBuf::from("/home/u/.clw/cache");
        let targets = resolve_targets(Some(cfg.clone()), &cache);
        assert_eq!(
            targets.len(),
            1,
            "cache under ~/.clw must not be listed twice"
        );
        assert_eq!(targets[0].path, cfg);
    }

    #[test]
    fn cache_outside_config_is_listed_separately() {
        // A cache dir configured OUTSIDE ~/.clw must be removed as its own target.
        let cfg = PathBuf::from("/home/u/.clw");
        let cache = PathBuf::from("/var/tmp/clw-cache");
        let targets = resolve_targets(Some(cfg.clone()), &cache);
        assert_eq!(
            targets.len(),
            2,
            "external cache dir must be a separate target"
        );
        assert!(targets.iter().any(|t| t.path == cfg));
        assert!(targets.iter().any(|t| t.path == cache));
    }

    #[test]
    fn no_home_still_lists_cache() {
        // If HOME can't be resolved we still scrub the resolved cache dir.
        let cache = PathBuf::from("/var/tmp/clw-cache");
        let targets = resolve_targets(None, &cache);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, cache);
    }

    // -----------------------------------------------------------------------
    // build_report_json: paths only, never the token; dry-run removes nothing
    // -----------------------------------------------------------------------

    fn targets_fixture() -> (Target, Target) {
        (
            Target {
                path: PathBuf::from("/home/u/.clw"),
                label: "config dir (config.toml with PAT)",
            },
            Target {
                path: PathBuf::from("/var/tmp/clw-cache"),
                label: "cache dir (customer file content)",
            },
        )
    }

    #[test]
    fn dry_run_json_lists_paths_removes_nothing_and_no_token() {
        let secret = "super-secret-pat-abc123XYZ";
        let (a, b) = targets_fixture();
        let acted = vec![(&a, true), (&b, true)];
        let out = build_report_json(&acted, "/usr/local/bin/clw", true);
        let s = out.to_string();

        // Never contains a token (we only ever carry PATHS here).
        assert!(
            !s.contains(secret),
            "report must never contain a token, got: {s}"
        );

        // dry-run: nothing is marked removed.
        assert_eq!(out["dry_run"], true);
        for t in out["targets"].as_array().unwrap() {
            assert_eq!(t["removed"], false, "dry-run must remove nothing");
        }
        // The two resolved paths are present.
        let paths: Vec<String> = out["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["path"].as_str().unwrap().to_string())
            .collect();
        assert!(paths.contains(&"/home/u/.clw".to_string()));
        assert!(paths.contains(&"/var/tmp/clw-cache".to_string()));

        // The binary is reported but never auto-removed.
        assert_eq!(out["binary"]["removed"], false);
        assert!(out["binary"]["remove_command"]
            .as_str()
            .unwrap()
            .contains("/usr/local/bin/clw"));
    }

    #[test]
    fn real_run_json_marks_existing_targets_removed() {
        let (a, b) = targets_fixture();
        // a existed, b did not.
        let acted = vec![(&a, true), (&b, false)];
        let out = build_report_json(&acted, "/usr/local/bin/clw", false);
        assert_eq!(out["dry_run"], false);
        let arr = out["targets"].as_array().unwrap();
        let a_obj = arr.iter().find(|t| t["path"] == "/home/u/.clw").unwrap();
        let b_obj = arr
            .iter()
            .find(|t| t["path"] == "/var/tmp/clw-cache")
            .unwrap();
        assert_eq!(a_obj["removed"], true, "existing target must be removed");
        assert_eq!(b_obj["removed"], false, "absent target removes nothing");
    }
}
