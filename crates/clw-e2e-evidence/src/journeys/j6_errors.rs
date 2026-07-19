//! J6 — errors: every user-inducible failure mode returns the right exit code AND an
//! actionable message. The contract a script/CI branches on is the exit code; the human
//! fix is the message — so each step asserts BOTH against the real binary.
//!
//! Covered (single PAT, against live prod): 200-same no-op, 409 create-only divergent,
//! 401 bad token, hydrate-into-non-empty, invalid --manifest-digest, clap missing-arg.
//! 402 (monthly-quota wall) needs a quota-capped tenant — it is soft-noted here and
//! exercised by the metered-402 journey in live_journeys.rs.

use std::path::Path;

use super::{pseudo_bytes_seeded, ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner, Secret, StepEvidence};

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let ws = Workspace::new("j6");
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seed = pid as u64 ^ nonce;
    let name = format!("e2e-j6-{pid}-{nonce}");

    // Two distinct workspaces: X (seeded under `name`) and Y (a divergent re-snapshot).
    ws.write("srcx/data.bin", &pseudo_bytes_seeded(512 * 1024, seed));
    ws.write(
        "srcy/data.bin",
        &pseudo_bytes_seeded(512 * 1024, seed.wrapping_add(1)),
    );
    let srcx = ws.dir("srcx").to_string_lossy().into_owned();
    let srcy = ws.dir("srcy").to_string_lossy().into_owned();

    let r = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache"),
    );

    let mut j = Journey::new(
        "J6-errors",
        "each user-inducible failure returns the right exit code AND an actionable message",
        creds.target(),
    );

    // 1. Seed X under `name` (the baseline for the no-op and 409 steps).
    let seed_step = r.step(
        "seed: snapshot X",
        &["--json", "snapshot", &srcx, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(seed_step);

    // 2. 200-same: re-snapshot the SAME content → a benign no-op (unchanged=true), exit 0.
    let mut noop = r.step(
        "re-snapshot identical content → no-op (200 same)",
        &["--json", "snapshot", &srcx, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    let unchanged = noop
        .json
        .as_ref()
        .and_then(|v| v.get("unchanged"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    noop.observe(
        Observation::behavior(
            "re-snapshotting identical content is a no-op (unchanged=true, exit 0)",
            "the create-only ref store must treat an identical write as success, not a 409",
        )
        .check(unchanged, format!("unchanged={unchanged}")),
    );
    j.push(noop);

    // 3. 409 create-only: snapshot DIFFERENT content under the same name (no --force).
    let mut conflict = r.step(
        "snapshot divergent content under the same name → 409 create-only",
        &["snapshot", &srcy, "--name", &name],
        &[],
        ExpectExit::NonZero,
    );
    assert_stderr(
        &mut conflict,
        &["already exists with different content", "409", "create-only"],
        "a divergent re-snapshot is rejected create-only (no silent last-write-wins) with a fix hint",
        "last-write-wins would silently destroy a teammate's snapshot — the 409 + message is the guardrail",
    );
    j.push(conflict);

    // 4. 401: a bad token is rejected with an auth error naming CLW_TOKEN.
    let r_bad = Runner::new(
        bin,
        &creds.endpoint,
        "prod (bad token)",
        &creds.tenant,
        Secret::new("core-e2e-invalid-token-deadbeef"),
        ws.dir("cache-bad"),
    );
    let mut unauth = r_bad.step(
        "snapshot with an invalid token → 401",
        &["snapshot", &srcx, "--name", &format!("{name}-401")],
        &[],
        ExpectExit::NonZero,
    );
    assert_stderr(
        &mut unauth,
        &["authentication failed", "CLW_TOKEN"],
        "an invalid token fails with an actionable auth error that names the fix (CLW_TOKEN)",
        "a cryptic auth failure wastes hours; the message must point straight at the credential",
    );
    j.push(unauth);

    // 5. hydrate into a NON-EMPTY destination → refused (never clobbers existing files).
    let dst = ws.dir("dst-nonempty");
    ws.write("dst-nonempty/prior.txt", b"pre-existing\n");
    let dst_s = dst.to_string_lossy().into_owned();
    let mut notempty = r.step(
        "hydrate into a non-empty dir → refused",
        &["hydrate", &dst_s, "--name", &name],
        &[],
        ExpectExit::NonZero,
    );
    assert_stderr(
        &mut notempty,
        &["not empty"],
        "hydrate refuses a non-empty destination instead of clobbering it",
        "silently overwriting a populated directory would destroy the user's local work",
    );
    j.push(notempty);

    // 6. invalid --manifest-digest → a clear validation error (before any network work).
    let dst2 = ws.dir("dst2").to_string_lossy().into_owned();
    let mut baddigest = r.step(
        "hydrate --manifest-digest <not-hex> → validation error",
        &["hydrate", &dst2, "--manifest-digest", "nothex"],
        &[],
        ExpectExit::NonZero,
    );
    assert_stderr(
        &mut baddigest,
        &["invalid --manifest-digest", "64-character hex"],
        "a malformed digest is rejected with a precise format error",
        "a typo'd pin must fail loudly with the expected format, not a confusing not-found",
    );
    j.push(baddigest);

    // 7. clap usage error: a missing required flag → exit non-zero with a usage message.
    let mut usage = r.step(
        "snapshot without --name → usage error",
        &["snapshot", &srcx],
        &[],
        ExpectExit::NonZero,
    );
    assert_stderr(
        &mut usage,
        &["required arguments", "--name"],
        "a missing required flag is a clear usage error, not a crash",
        "CLI misuse must be diagnosable from the message alone",
    );
    j.push(usage);

    // 8. 402 monthly-quota wall — needs a quota-capped tenant; soft-noted (Info) here and
    //    exercised by the metered-402 journey in live_journeys.rs.
    let mut quota = StepEvidence {
        label: "402 monthly-quota wall (soft-noted)".to_string(),
        cmd: String::new(),
        env_shown: Vec::new(),
        expected_exit: ExpectExit::Any,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        json: None,
        duration_ms: 0,
        leaked_secret: false,
        observations: Vec::new(),
    };
    quota.observe(
        Observation::behavior(
            "402 (monthly usage quota) is covered by the metered-402 journey, not inducible here",
            "the $-ceiling 402 needs a quota-capped tenant; inducing it on a healthy tenant is not possible",
        )
        .info("run against a metered/probe tenant to see the actionable billing message"),
    );
    j.push(quota);

    // 9. Cleanup. The 401 step should create NO ref (auth is rejected before any write),
    //    so `{name}-401` normally does not exist — but rm it best-effort (Any) for symmetry
    //    so a regression of the 401-before-write invariant can never leave a stray ref.
    let rm = r.step(
        "rm the ref (cleanup)",
        &["--json", "rm", "--name", &name, "--yes"],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(rm);
    let rm401 = r.step(
        "rm the 401 ref if any (best-effort cleanup)",
        &["--json", "rm", "--name", &format!("{name}-401"), "--yes"],
        &[],
        ExpectExit::Any,
    );
    j.push(rm401);

    j
}

/// Assert the step's stderr contains every `needle` (a behavior observation). Non-vacuous:
/// a missing needle fails the observation and thus the step.
fn assert_stderr(ev: &mut StepEvidence, needles: &[&str], claim: &str, why: &str) {
    let missing: Vec<&str> = needles
        .iter()
        .copied()
        .filter(|n| !ev.stderr.contains(n))
        .collect();
    ev.observe(Observation::behavior(claim, why).check(
        missing.is_empty(),
        if missing.is_empty() {
            format!("stderr contains all of {needles:?}")
        } else {
            format!("stderr MISSING {missing:?}")
        },
    ));
}
