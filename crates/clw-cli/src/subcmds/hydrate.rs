//! `clw hydrate <DEST> --name <NAME>` (or `--manifest-digest <HEX>`) —
//! materialize a workspace snapshot, identified either by name or directly by
//! its content-addressed manifest digest.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use clap::Args;
use clw_cache::LocalCache;
use clw_client::HttpClient;
use clw_hydrate::{hydrate, HydrateOptions};
use clw_types::ClwError;
use serde_json::json;

use crate::config::ResolvedConfig;

#[derive(Debug, Args)]
#[command(
    about = "Materialize a workspace snapshot into a local directory",
    // FIX 8: the destination is POSITIONAL — `--path DIR` yields a confusing clap
    // error. Spell the correct form out so a user reaches for `<DEST>`, not --path.
    long_about = "Materialize a workspace snapshot into a local directory.\n\n\
                  The destination is a POSITIONAL argument, not a flag: \
                  `clw hydrate <DEST> --name X` (NOT `--path DEST`).\n\n\
                  Identify the snapshot EITHER by workspace name (`--name X`) \
                  OR directly by its manifest digest (`--manifest-digest <HEX>`), \
                  which skips the named-ref lookup and hydrates the content the \
                  digest pins (self-verifying: the bytes must hash to the digest)."
)]
pub struct HydrateArgs {
    /// Destination directory (must not exist or be empty).
    #[arg(
        help = "Destination directory — POSITIONAL, e.g. `clw hydrate <DEST> --name X` \
                (not `--path DEST`); must not exist or must be empty"
    )]
    pub dest: PathBuf,

    /// Workspace name (ref key). Mutually exclusive with --manifest-digest.
    #[arg(
        long,
        conflicts_with = "manifest_digest",
        required_unless_present = "manifest_digest",
        help = "Workspace name to hydrate (or use --manifest-digest)"
    )]
    pub name: Option<String>,

    /// Hydrate directly from a manifest digest (64-hex), bypassing the name→ref
    /// lookup. The fetched manifest is verified to hash to this digest.
    #[arg(
        long,
        value_name = "HEX",
        help = "Manifest digest (64-hex) to hydrate directly, skipping the named-ref lookup"
    )]
    pub manifest_digest: Option<String>,
}

pub async fn run(
    args: HydrateArgs,
    cfg: ResolvedConfig,
    json: bool,
    concurrency: usize,
) -> Result<(), ClwError> {
    let tenant = cfg.clw_config.tenant.clone();
    let cache = LocalCache::new(&cfg.clw_config.cache_dir)?;
    // Clamp to at least 1 so `--concurrency 0` can't stall the stream or starve
    // the client's AIMD governor.
    let concurrency = concurrency.max(1);
    // Govern the GET fan-out at the client too, so the AIMD limiter sees the
    // same ceiling the stream uses (M1.3).
    let client = HttpClient::with_concurrency(cfg.clw_config, concurrency)?;

    // Two ways to identify the snapshot: a workspace name (AC lookup) or a
    // manifest digest (content-addressed, skip the AC). clap guarantees exactly
    // one of `--name` / `--manifest-digest` is present.
    let mut opts = if let Some(hex) = args.manifest_digest.as_deref() {
        HydrateOptions::from_manifest_digest(parse_manifest_digest(hex)?)
    } else {
        // `required_unless_present` guarantees `--name` is Some here.
        HydrateOptions::new(args.name.as_deref().unwrap_or_default())
    };
    opts.concurrency = concurrency;
    opts.ref_domain = cfg.ref_domain;

    // A human-facing label for the source, used in progress + error messages.
    let label = match (&args.name, &args.manifest_digest) {
        (Some(n), _) => n.clone(),
        (_, Some(d)) => format!("manifest digest {d}"),
        _ => String::new(),
    };

    // Progress feedback on a long, otherwise-silent op (stderr only, TTY-gated):
    // tell an interactive user work has started so a slow hydrate over a throttled
    // link isn't mistaken for a hang. CI/pipelines stay quiet and --json stays clean.
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!(
            "hydrating '{}' into {} (silent until complete)…",
            label,
            args.dest.display()
        );
    }

    let report = hydrate(&args.dest, &client, &cache, &opts)
        .await
        .map_err(|e| {
            map_hydrate_error(
                e,
                &label,
                &tenant,
                &args.dest,
                args.manifest_digest.is_some(),
            )
        })?;

    if json {
        let out = json!({
            "root": report.root.to_hex(),
            "files": report.files,
            "bytes_total": report.bytes_total,
            "bytes_from_cache": report.bytes_from_cache,
            "bytes_downloaded": report.bytes_downloaded,
        });
        println!("{out}");
    } else {
        println!("hydrate complete");
        println!("  source:            {label}");
        println!("  root:              {}", report.root);
        println!("  files:             {}", report.files);
        println!("  bytes_total:       {}", report.bytes_total);
        println!("  bytes_from_cache:  {}", report.bytes_from_cache);
        println!("  bytes_downloaded:  {}", report.bytes_downloaded);
    }

    Ok(())
}

/// Parse a `--manifest-digest` value (64-hex) into a [`clw_types::Digest`],
/// mapping any malformed input to a clear, actionable CLI error.
fn parse_manifest_digest(hex: &str) -> Result<clw_types::Digest, ClwError> {
    clw_types::Digest::from_hex(hex).map_err(|_| {
        ClwError::Message(format!(
            "invalid --manifest-digest '{hex}' — expected a 64-character hex digest"
        ))
    })
}

/// Map hydrate-specific errors to actionable user-facing messages.
///
/// `is_digest` selects the not-found wording: a named-ref miss suggests
/// `clw snapshot`, whereas a `--manifest-digest` miss means the manifest blob is
/// absent from the CAS (a snapshot suggestion would be nonsensical there).
fn map_hydrate_error(
    err: ClwError,
    label: &str,
    tenant: &str,
    dest: &Path,
    is_digest: bool,
) -> ClwError {
    match &err {
        ClwError::Auth => ClwError::Message(
            "authentication failed — check CLW_TOKEN or ~/.clw/config.toml".to_string(),
        ),
        // A not-found means different things per source: an absent named ref vs.
        // an absent manifest blob in the CAS.
        ClwError::NotFound(_) if is_digest => ClwError::Message(format!(
            "{label} not found in the CAS for tenant '{tenant}' — the manifest blob is absent \
             (wrong digest, wrong tenant, or not yet pushed)"
        )),
        // FIX 4: shared not-found message — now also hints at --ref-domain (the
        // likely cause when a runner-domain ref is looked up in the user domain).
        ClwError::NotFound(_) => {
            ClwError::Message(crate::subcmds::ls::not_found_msg(label, tenant))
        }
        ClwError::Io(io_err) if io_err.kind() == ErrorKind::AlreadyExists => {
            let dest = dest.display();
            ClwError::Message(format!(
                "destination '{dest}' is not empty — pass an empty directory or a new path that does not yet exist",
            ))
        }
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
    use std::io::ErrorKind;

    #[test]
    fn parse_manifest_digest_accepts_valid_64_hex() {
        use clw_types::Digest;
        let hex = Digest::of_bytes(b"some manifest").to_hex();
        let parsed = parse_manifest_digest(&hex).expect("valid 64-hex must parse");
        assert_eq!(parsed.to_hex(), hex, "round-trips to the same digest");
    }

    #[test]
    fn parse_manifest_digest_rejects_malformed_with_actionable_error() {
        // Too short, and a non-hex char — both must hit the clear error branch.
        for bad in ["deadbeef", "not-hex", "", &"zz".repeat(32)] {
            let err = parse_manifest_digest(bad).expect_err("malformed hex must error");
            let msg = err.to_string();
            assert!(
                msg.contains("--manifest-digest") && msg.contains("64-character hex"),
                "error must name the flag and the 64-hex expectation, got: {msg}"
            );
        }
    }

    #[test]
    fn map_hydrate_error_auth() {
        let dest = PathBuf::from("/tmp/out");
        let mapped = map_hydrate_error(ClwError::Auth, "ws", "t1", &dest, false);
        let msg = mapped.to_string();
        assert!(
            msg.contains("CLW_TOKEN") || msg.contains("config.toml"),
            "auth error must mention CLW_TOKEN or config.toml, got: {msg}"
        );
    }

    #[test]
    fn map_hydrate_error_not_found_is_actionable() {
        use clw_types::Digest;
        let dest = PathBuf::from("/tmp/out");
        let dummy_digest = Digest::of_bytes(b"test");
        let mapped = map_hydrate_error(
            ClwError::NotFound(dummy_digest),
            "my-ws",
            "acme",
            &dest,
            false,
        );
        let msg = mapped.to_string();
        assert!(msg.contains("my-ws"), "must mention workspace name");
        assert!(msg.contains("acme"), "must mention tenant");
        assert!(
            msg.contains("clw snapshot"),
            "must suggest snapshot command"
        );
        // FIX 4 — not-found now hints at the ref-domain mismatch case.
        assert!(
            msg.contains("CLW_REF_DOMAIN=runner") && msg.contains("--ref-domain"),
            "must hint at --ref-domain for runner-produced refs, got: {msg}"
        );
    }

    #[test]
    fn map_hydrate_error_not_found_digest_does_not_suggest_snapshot() {
        use clw_types::Digest;
        let dest = PathBuf::from("/tmp/out");
        let dummy_digest = Digest::of_bytes(b"test");
        // In digest mode a missing manifest is a CAS-absent blob, NOT a missing
        // named ref — suggesting `clw snapshot --name <hex>` would be nonsense.
        let mapped = map_hydrate_error(
            ClwError::NotFound(dummy_digest),
            "manifest digest abc123",
            "acme",
            &dest,
            true,
        );
        let msg = mapped.to_string();
        assert!(
            msg.contains("CAS") && msg.contains("absent"),
            "digest not-found must say the manifest blob is absent from the CAS, got: {msg}"
        );
        assert!(
            !msg.contains("clw snapshot"),
            "digest not-found must NOT suggest `clw snapshot`, got: {msg}"
        );
        assert!(msg.contains("acme"), "must mention tenant");
    }

    #[test]
    fn map_hydrate_error_already_exists_suggests_empty_dir() {
        let dest = PathBuf::from("/tmp/my-output");
        let io_err = std::io::Error::new(ErrorKind::AlreadyExists, "destination not empty");
        let mapped = map_hydrate_error(ClwError::Io(io_err), "ws", "t1", &dest, false);
        let msg = mapped.to_string();
        assert!(
            msg.contains("not empty") || msg.contains("empty"),
            "must mention that dest is not empty, got: {msg}"
        );
        assert!(
            msg.contains("/tmp/my-output"),
            "must mention the destination path, got: {msg}"
        );
    }

    #[test]
    fn map_hydrate_error_passthrough_for_other_errors() {
        let dest = PathBuf::from("/tmp/out");
        let original = ClwError::Transport("timeout".to_string());
        let mapped = map_hydrate_error(original, "ws", "t1", &dest, false);
        assert!(
            mapped.to_string().contains("timeout"),
            "other errors must pass through unchanged"
        );
    }
}
