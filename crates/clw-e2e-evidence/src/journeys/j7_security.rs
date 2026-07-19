//! J7 — security, observed through the real binary. Each property is a thing an attacker
//! or a careless paste could exploit; each is asserted from the OUTSIDE (exit code, the
//! `leaked_secret` pre-redaction signal, on-disk state), never by reading clw's source.
//!
//! - the PAT never appears even in the `-vv` DIAGNOSTIC channel;
//! - an endpoint's `user:pass@` userinfo is redacted (and not leaked raw);
//! - a plaintext `http://` non-loopback endpoint is REFUSED (no PAT over the wire in clear);
//! - consent gates: `rm` / `auth logout --all` refuse without `--yes` and take NO action.
//!
//! The token-redaction check is non-vacuous because `leaked_secret` is computed BEFORE the
//! harness scrubs anything (the scrub would otherwise hide a real clw leak).

use std::path::Path;

use super::{pseudo_bytes_seeded, ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner, Secret, StepEvidence};

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let ws = Workspace::new("j7");
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let name = format!("e2e-j7-{pid}-{nonce}");

    ws.write(
        "src/data.bin",
        &pseudo_bytes_seeded(256 * 1024, pid as u64 ^ nonce),
    );
    let src = ws.dir("src").to_string_lossy().into_owned();

    let r = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache"),
    );

    let mut j = Journey::new(
        "J7-security",
        "security properties observed through the binary: redaction, plaintext refusal, consent gates",
        creds.target(),
    );

    // 1. The PAT must not appear even in the -vv DIAGNOSTIC channel (which logs the
    //    request lifecycle). leaked_secret is checked PRE-scrub, so this is non-vacuous.
    let mut verbose = r.step(
        "snapshot -vv (verbose diagnostics) → PAT never logged",
        &["-vv", "--json", "snapshot", &src, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    verbose.expect_no_secret_leak(
        "the -vv diagnostic channel logs the request lifecycle — it must never echo the PAT",
    );
    j.push(verbose);

    // 2. An endpoint carrying user:pass@ userinfo must be redacted by clw itself (doctor is
    //    offline, so it echoes the resolved endpoint). We register the bare password too, so
    //    leaked_secret catches even a partial leak.
    let home = ws.dir("home2").to_string_lossy().into_owned();
    let mut r_userinfo = Runner::new(
        bin,
        "https://e2euser:e2esecret@corelink-api.humangr.com",
        "prod (userinfo endpoint)",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-ui"),
    );
    r_userinfo.add_redaction(&Secret::new("e2esecret"));
    let mut doc = r_userinfo.step(
        "doctor with a user:pass@ endpoint → userinfo redacted",
        &["--json", "doctor"],
        &[("HOME", &home, false)],
        ExpectExit::Eq(0),
    );
    let endpoint_field = doc
        .json
        .as_ref()
        .and_then(|v| v.get("endpoint"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    // NOTE (confirmatory, Info — NOT the guard): clw's redaction marker "<redacted>" is
    // the SAME literal the harness scrubs with, so a redacted field and a scrubbed-leak
    // field are indistinguishable in post-scrub output. This is recorded for the reader;
    // the real non-vacuous guard is `expect_no_secret_leak` below (pre-scrub leaked_secret).
    doc.observe(
        Observation::security(
            "clw's echoed endpoint shows the <redacted> userinfo marker",
            "confirmatory only — the load-bearing check is the pre-scrub no-leak assertion below",
        )
        .info(format!("endpoint field = {endpoint_field}")),
    );
    doc.expect_no_secret_leak(
        "THE guard: the embedded endpoint credential must not appear raw anywhere in the bundle (pre-scrub)",
    );
    j.push(doc);

    // 3. A plaintext http:// non-loopback endpoint must be REFUSED before any PAT is sent.
    let r_plain = Runner::new(
        bin,
        "http://corelink-api.humangr.com",
        "plaintext endpoint",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-plain"),
    );
    let mut plain = r_plain.step(
        "snapshot to an http:// non-loopback endpoint → refused",
        &["snapshot", &src, "--name", &format!("{name}-plain")],
        &[],
        ExpectExit::NonZero,
    );
    assert_stderr(
        &mut plain,
        &["plaintext", "loopback"],
        "clw refuses to send the PAT over plaintext http:// to a non-loopback host",
        "sending a bearer PAT over cleartext exposes it to any on-path attacker",
    );
    j.push(plain);

    // 4. Consent gate — rm without --yes (non-interactive) refuses and deletes NOTHING.
    let mut rm_noyes = r.step(
        "rm without --yes (non-interactive) → refused, nothing deleted",
        &["--json", "rm", "--name", &name],
        &[],
        ExpectExit::NonZero,
    );
    let deleted = rm_noyes
        .json
        .as_ref()
        .and_then(|v| v.get("deleted"))
        .and_then(serde_json::Value::as_bool);
    rm_noyes.observe(
        Observation::security(
            "a destructive rm without --yes is refused and deletes nothing (deleted=false)",
            "an unconfirmed non-interactive delete must never destroy data — consent is mandatory",
        )
        .check(
            deleted == Some(false) && rm_noyes.stderr.contains("declined"),
            format!(
                "deleted={deleted:?}, stderr mentions declined={}",
                rm_noyes.stderr.contains("declined")
            ),
        ),
    );
    j.push(rm_noyes);

    // 5. Consent gate — auth logout --all without --yes refuses AND leaves the config on disk
    //    (the load-bearing check: the file survives, so consent genuinely prevented deletion).
    let home5_path = ws.dir("home5");
    let home5 = home5_path.to_string_lossy().into_owned();
    let init = r.step(
        "init a sandbox config (for the logout consent test)",
        &["init", "--force"],
        &[("HOME", &home5, false)],
        ExpectExit::Eq(0),
    );
    j.push(init);
    let cfg_file = home5_path.join(".clw").join("config.toml");
    let mut logout = r.step(
        "auth logout --all without --yes → refused, config preserved",
        &["auth", "logout", "--all"],
        &[("HOME", &home5, false)],
        ExpectExit::NonZero,
    );
    // NON-VACUOUS on-disk check: a CONSENTED `logout --all --yes` truncates the config to
    // a bare header (it does NOT unlink the file), so `.exists()` can't tell honored from
    // cleared. Instead read the content and confirm the IDENTITY KEYS survived. The content
    // holds the plaintext PAT, so we only check for KEY NAMES and report booleans — the
    // file content (and the token) is NEVER put into the evidence.
    let cfg_body = std::fs::read_to_string(&cfg_file).unwrap_or_default();
    let intact = cfg_body.contains("tenant") && cfg_body.contains("endpoint");
    logout.observe(
        Observation::security(
            "logout --all without --yes refuses AND leaves the stored identity intact on disk",
            "clearing all credentials is destructive — it must require explicit consent and never act silently",
        )
        .check(
            logout.stderr.contains("refusing to clear") && intact,
            format!(
                "stderr refuses={}, config still has its identity keys={intact}",
                logout.stderr.contains("refusing to clear")
            ),
        ),
    );
    j.push(logout);

    // 6. No CLW_TOKEN fallback in the runner ref-domain — soft-noted (exercised by the
    //    cred-ticket journey J8 / live_journeys). Documented here so the security catalog
    //    is complete without overclaiming.
    let mut nofallback = StepEvidence {
        label: "no CLW_TOKEN fallback in runner-domain (soft-noted)".to_string(),
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
    nofallback.observe(
        Observation::security(
            "the runner ref-domain does not fall back to CLW_TOKEN (cred-ticket only)",
            "a runner must authenticate with its single-use ticket, never a long-lived PAT it shouldn't hold",
        )
        .info("exercised by the cred-ticket journey (J8) against the real fabric"),
    );
    j.push(nofallback);

    // 7. Cleanup.
    let rm = r.step(
        "rm the ref (cleanup)",
        &["--json", "rm", "--name", &name, "--yes"],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(rm);

    j
}

/// Assert the step's stderr contains every `needle` (non-vacuous: a missing needle fails).
fn assert_stderr(ev: &mut StepEvidence, needles: &[&str], claim: &str, why: &str) {
    let missing: Vec<&str> = needles
        .iter()
        .copied()
        .filter(|n| !ev.stderr.contains(n))
        .collect();
    ev.observe(Observation::security(claim, why).check(
        missing.is_empty(),
        if missing.is_empty() {
            format!("stderr contains all of {needles:?}")
        } else {
            format!("stderr MISSING {missing:?}")
        },
    ));
}
