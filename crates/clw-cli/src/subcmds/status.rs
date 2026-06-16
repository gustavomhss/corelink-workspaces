//! `clw status [PATH] --name <NAME>` — diff remote ref vs local working tree.
//!
//! Exit 0 = clean (no diff). Exit 1 = dirty (diff detected).
//! Exit 2 = error (propagated via the normal error path in main).

use std::path::PathBuf;

use clap::Args;
use clw_client::HttpClient;
use clw_manifest::diff;
use clw_snapshot::build_manifest_local;
use clw_types::{ref_key_in, AcTransport, CasTransport, ClwError, Digest, Manifest, RefRecord};
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(
    about = "Show diff between remote snapshot and local working tree (exit 0=clean, 1=dirty, 2=error)"
)]
pub struct StatusArgs {
    /// Local workspace directory to compare (default: current directory).
    #[arg(
        default_value = ".",
        help = "Local workspace directory to compare against the remote snapshot"
    )]
    pub path: PathBuf,

    /// Workspace name (ref key).
    #[arg(long, help = "Workspace name to compare against")]
    pub name: String,
}

pub async fn run(args: StatusArgs, cfg: ResolvedConfig, json: bool) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let client = HttpClient::new(cfg.clw_config)?;

    // --- Fetch remote ref + manifest ---
    let rk = ref_key_in(cfg.ref_domain, &args.name);
    let ref_bytes = AcTransport::get(&client, &rk)
        .await
        .map_err(map_auth_error)?
        .ok_or_else(|| {
            ClwError::Transport(format!(
                "workspace '{}' not found for tenant '{}' — run 'clw snapshot --name {}' first or check --tenant",
                args.name, tenant, args.name
            ))
        })?;
    let record = RefRecord::from_bytes(&ref_bytes)?;

    // Fetch manifest blob from CAS. The ref resolved but its manifest blob may be
    // missing (pruned / lost / replica-lag): surface a workspace-NAMED error
    // rather than a bare 64-hex `NotFound(<digest>)`.
    let manifest_bytes = CasTransport::get(&client, &record.root)
        .await
        .map_err(map_auth_error)
        .map_err(|e| map_missing_manifest(e, &args.name, &tenant, &record.root))?;
    let remote_manifest = Manifest::from_bytes(&manifest_bytes)?;

    // --- Build local manifest ---
    let local_manifest = build_manifest_local(&args.path).await?;

    // --- Diff ---
    let d = diff(&remote_manifest, &local_manifest);
    let is_clean = d.added.is_empty() && d.removed.is_empty() && d.changed.is_empty();

    if json {
        let out = json!({
            "clean": is_clean,
            "added": d.added,
            "removed": d.removed,
            "changed": d.changed,
        });
        println!("{out}");
    } else if is_clean {
        println!("clean — workspace matches remote ref '{}'", args.name);
    } else {
        if !d.added.is_empty() {
            println!("added:");
            for p in &d.added {
                println!("  + {p}");
            }
        }
        if !d.removed.is_empty() {
            println!("removed:");
            for p in &d.removed {
                println!("  - {p}");
            }
        }
        if !d.changed.is_empty() {
            println!("changed:");
            for p in &d.changed {
                println!("  ~ {p}");
            }
        }
    }

    if is_clean {
        Ok(())
    } else {
        // Exit 1 for dirty (not an error, but distinct from clean).
        std::process::exit(1);
    }
}

/// Map a 401 `Auth` error to an actionable message; pass other errors through.
pub fn map_auth_error(err: ClwError) -> ClwError {
    if matches!(err, ClwError::Auth) {
        ClwError::Transport(
            "authentication failed — check CLW_TOKEN or ~/.clw/config.toml".to_string(),
        )
    } else {
        err
    }
}

/// Map a `NotFound(<manifest root>)` while fetching a workspace's manifest blob
/// to a workspace-NAMED, actionable message — mirroring the ref-not-found
/// contextualization in [`run`]/`ls::run_one`. The ref RESOLVED, but its manifest
/// `record.root` blob is missing/unreachable (pruned, lost, or replica-lag), so a
/// bare 64-hex `NotFound(<digest>)` would name nothing the user recognizes.
/// Non-`NotFound` errors (and a `NotFound` on a different digest) pass through.
pub fn map_missing_manifest(err: ClwError, name: &str, tenant: &str, root: &Digest) -> ClwError {
    match &err {
        ClwError::NotFound(d) if d == root => ClwError::Transport(format!(
            "workspace '{name}' (tenant '{tenant}') is missing its manifest blob {root} — the ref \
             resolves but its manifest is unreachable (pruned, lost, or replica lag); re-run \
             'clw snapshot --name {name}' to repair it"
        )),
        _ => err,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::ClwError;

    #[test]
    fn map_auth_error_rewrites_auth_variant() {
        let mapped = map_auth_error(ClwError::Auth);
        let msg = mapped.to_string();
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "mapped auth error must mention CLW_TOKEN or config.toml, got: {msg}"
        );
    }

    #[test]
    fn map_auth_error_passes_through_other_errors() {
        let original = ClwError::Transport("network timeout".to_string());
        let mapped = map_auth_error(original);
        assert!(
            mapped.to_string().contains("network timeout"),
            "non-auth errors must pass through unchanged"
        );
    }

    #[test]
    fn status_args_defaults() {
        use clap::Parser;

        #[derive(Parser)]
        struct Cmd {
            #[command(flatten)]
            args: StatusArgs,
        }

        let cmd = Cmd::try_parse_from(["cmd", "--name", "my-ws"]).unwrap();
        assert_eq!(cmd.args.path, PathBuf::from("."));
        assert_eq!(cmd.args.name, "my-ws");
    }

    #[test]
    fn json_shape_matches_spec() {
        // Verify the JSON shape we produce has the correct keys.
        let out = json!({
            "clean": true,
            "added": Vec::<String>::new(),
            "removed": Vec::<String>::new(),
            "changed": Vec::<String>::new(),
        });
        assert!(out.get("clean").is_some(), "must have 'clean'");
        assert!(out.get("added").is_some(), "must have 'added'");
        assert!(out.get("removed").is_some(), "must have 'removed'");
        assert!(out.get("changed").is_some(), "must have 'changed'");
        assert!(out["clean"].as_bool().is_some(), "'clean' must be a bool");
        assert!(
            out["added"].as_array().is_some(),
            "'added' must be an array"
        );
    }

    // P3 — a NotFound on the manifest root maps to a workspace-NAMED error, not a
    // bare 64-hex digest.
    #[test]
    fn missing_manifest_maps_to_workspace_named_error() {
        let root = Digest::of_bytes(b"manifest-root");
        let mapped = map_missing_manifest(ClwError::NotFound(root), "my-ws", "acme", &root);
        let msg = mapped.to_string();
        assert!(
            !matches!(mapped, ClwError::NotFound(_)),
            "a missing manifest must be re-mapped away from the bare NotFound digest"
        );
        assert!(msg.contains("my-ws"), "must NAME the workspace, got: {msg}");
        assert!(msg.contains("acme"), "must name the tenant, got: {msg}");
        assert!(
            msg.contains("manifest"),
            "must say the manifest is missing, got: {msg}"
        );
        assert!(
            msg.contains("clw snapshot"),
            "must suggest re-snapshotting to repair, got: {msg}"
        );
    }

    #[test]
    fn missing_manifest_passes_through_unrelated_errors() {
        let root = Digest::of_bytes(b"root");
        // A NotFound on a DIFFERENT digest is not the manifest root — pass through.
        let other = Digest::of_bytes(b"other");
        let mapped = map_missing_manifest(ClwError::NotFound(other), "ws", "t", &root);
        assert!(
            matches!(mapped, ClwError::NotFound(d) if d == other),
            "a NotFound on a different digest must pass through unchanged"
        );
        // A non-NotFound error passes through verbatim.
        let mapped2 = map_missing_manifest(ClwError::Transport("blip".into()), "ws", "t", &root);
        assert!(matches!(mapped2, ClwError::Transport(m) if m == "blip"));
    }

    #[test]
    fn not_found_error_message_is_actionable() {
        // Simulate the NotFound-mapped error message that run() would produce.
        let name = "my-workspace";
        let tenant = "acme";
        let msg = format!(
            "workspace '{name}' not found for tenant '{tenant}' — run 'clw snapshot --name {name}' first or check --tenant",
        );
        assert!(msg.contains("not found"), "must say 'not found'");
        assert!(msg.contains(name), "must mention workspace name");
        assert!(msg.contains(tenant), "must mention tenant");
        assert!(msg.contains("clw snapshot"), "must suggest clw snapshot");
    }
}
