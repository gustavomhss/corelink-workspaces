use super::{
    global_tool_digest_cache, prepare_run_with_tool_digest_cache, RunOptions, RunOutcome, RunRecord,
};
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use std::path::Path;

/// Versioned behavior advertised independently of the clw package version.
pub const REQUIRED_HIT_CAPABILITY: &str = "required-hit/v1";

/// Look up and verify a memoized result without ever spawning the wrapped command.
/// An incomplete, malformed, or unverifiable entry is a refusal (`Ok(None)`).
/// Transport and key-preparation failures remain errors and are also fail-closed.
pub async fn run_memoized_required_hit<C: CasTransport + AcTransport>(
    cwd: &Path,
    client: &C,
    opts: &RunOptions,
) -> Result<Option<RunOutcome>> {
    if opts.command.is_empty() {
        return Err(ClwError::InvalidManifest("empty command".to_string()));
    }

    let key = prepare_run_with_tool_digest_cache(cwd, opts, global_tool_digest_cache())
        .await?
        .key;
    let Some(record_bytes) = AcTransport::get(client, &key).await? else {
        return Ok(None);
    };
    let Ok(record) = serde_json::from_slice::<RunRecord>(&record_bytes) else {
        return Ok(None);
    };
    let stdout = match CasTransport::get(client, &record.stdout).await {
        Ok(bytes) if Digest::of_bytes(&bytes) == record.stdout => bytes,
        Ok(_) | Err(ClwError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    let stderr = match CasTransport::get(client, &record.stderr).await {
        Ok(bytes) if Digest::of_bytes(&bytes) == record.stderr => bytes,
        Ok(_) | Err(ClwError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };

    Ok(Some(RunOutcome {
        key,
        hit: true,
        exit_code: record.exit_code,
        stdout,
        stderr,
    }))
}
