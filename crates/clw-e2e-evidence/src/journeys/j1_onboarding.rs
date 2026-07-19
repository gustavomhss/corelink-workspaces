//! J1 — onboarding: the first surface a user touches, end to end.
//!
//! `--version` → `completions` → `doctor` → `init` → `auth whoami` → config precedence.
//! Every step is the real binary; the security-critical evidence is that the diagnostics
//! surface never leaks the PAT and that `init` persists the identity owner-only (0600
//! file / 0700 dir). Runs against the live endpoint when creds are present.

use std::path::Path;

use super::{unix_mode, ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner};

/// A syntactically-valid but obviously-fake tenant used ONLY to prove flag-beats-env
/// precedence in `doctor` (offline; never sent to the backend).
const PRECEDENCE_SENTINEL: &str = "00000000-0000-4000-8000-00000000face";

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let ws = Workspace::new("j1");
    let r = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.cache(),
    );
    // A sandbox $HOME shared by EVERY config-reading step, so no step ever reads or
    // writes the operator's real ~/.clw (init writes it; doctor/whoami read it as the
    // layer-3 config source). This also keeps the doctor evidence deterministic.
    let home = ws.home();
    let home_s = home.to_string_lossy().into_owned();
    let home_env: &[(&str, &str, bool)] = &[("HOME", &home_s, false)];
    let mut j = Journey::new(
        "J1-onboarding",
        "the first surface a user touches: version, completions, doctor, init, whoami, precedence",
        creds.target(),
    );

    // 1. --version: identifies the build (a user's first sanity check).
    let mut v = r.step("clw --version", &["--version"], &[], ExpectExit::Eq(0));
    v.expect_output_contains("clw", "the version string must identify the build");
    j.push(v);

    // 2. completions: offline, config-free shell integration.
    let mut comp = r.step(
        "clw completions bash",
        &["completions", "bash"],
        &[],
        ExpectExit::Eq(0),
    );
    comp.observe(
        Observation::behavior(
            "emits a non-empty completion script",
            "shell completion must work offline and config-free",
        )
        .check(
            comp.stdout.len() > 100 && comp.stdout.contains("clw"),
            format!("{} bytes on stdout", comp.stdout.len()),
        ),
    );
    j.push(comp);

    // 3. doctor --json: the support bundle. It is routinely pasted into tickets, so the
    //    PAT must appear only as a presence marker — never the value.
    let mut doc = r.step(
        "clw --json doctor",
        &["--json", "doctor"],
        home_env,
        ExpectExit::Eq(0),
    );
    let token_marker = doc
        .json
        .as_ref()
        .and_then(|v| v.get("token"))
        .and_then(|v| v.as_str())
        .map(|s| s == "<set>" || s == "<unset>")
        .unwrap_or(false);
    let token_field = doc
        .json
        .as_ref()
        .and_then(|v| v.get("token"))
        .map(ToString::to_string)
        .unwrap_or_default();
    doc.observe(
        Observation::security(
            "doctor reports the token as <set>/<unset>, never the value",
            "a diagnostics bundle is pasted into tickets — a leaked PAT is full account access",
        )
        .check(token_marker, format!("token field = {token_field}")),
    );
    // Non-vacuous: clw's RAW bundle carried no PAT (checked before the harness scrubbed).
    doc.expect_no_secret_leak(
        "the whole diagnostics bundle is pasted into tickets — no field may carry the PAT",
    );
    j.push(doc);

    // 4. init: persist the identity to a SANDBOX ~/.clw (never the operator's real one).
    //    Security: the file holds the plaintext PAT, so it MUST be 0600 and its dir 0700,
    //    and init itself must not print the token.
    let mut init = r.step(
        "clw init (sandbox HOME)",
        &["init", "--force"],
        home_env,
        ExpectExit::Eq(0),
    );
    let cfg_path = home.join(".clw").join("config.toml");
    let dir_path = home.join(".clw");
    let cfg_mode = unix_mode(&cfg_path);
    let dir_mode = unix_mode(&dir_path);
    init.observe(
        Observation::security(
            "the persisted config.toml is owner-only (0600)",
            "config.toml holds the plaintext PAT — group/world-readable would leak it",
        )
        .check(
            cfg_mode.map(|m| m == 0o600).unwrap_or(cfg!(not(unix))),
            match cfg_mode {
                Some(m) => format!("mode {m:04o}"),
                None => "not unix — DACL checked in J10".to_string(),
            },
        ),
    );
    init.observe(
        Observation::security(
            "the ~/.clw directory is owner-only (0700)",
            "a world-listable identity dir leaks tenant/endpoint metadata",
        )
        .check(
            dir_mode.map(|m| m == 0o700).unwrap_or(cfg!(not(unix))),
            match dir_mode {
                Some(m) => format!("mode {m:04o}"),
                None => "not unix — DACL checked in J10".to_string(),
            },
        ),
    );
    init.expect_no_secret_leak("onboarding output is frequently screen-shared / logged");
    j.push(init);

    // 5. auth whoami: the resolved identity, redacted, offline (no network call).
    let mut who = r.step(
        "clw --json auth whoami (sandbox HOME)",
        &["--json", "auth", "whoami"],
        home_env,
        ExpectExit::Eq(0),
    );
    let who_tenant = who
        .json
        .as_ref()
        .and_then(|v| v.get("tenant"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    who.observe(
        Observation::behavior(
            "whoami resolves the configured tenant",
            "the user must be able to confirm which tenant they are operating on",
        )
        .check(who_tenant == creds.tenant, format!("tenant = {who_tenant}")),
    );
    who.expect_no_secret_leak(
        "whoami output is a natural thing to screenshot when asking for help",
    );
    j.push(who);

    // 6. precedence: an explicit --tenant flag must beat the CLW_TENANT env the Runner
    //    sets. doctor is offline and reports the resolved tenant_value, so it is the
    //    clean probe for flag > env > file > default.
    let mut prec = r.step(
        "clw --tenant <sentinel> --json doctor (flag beats env)",
        &["--tenant", PRECEDENCE_SENTINEL, "--json", "doctor"],
        home_env,
        ExpectExit::Eq(0),
    );
    let resolved = prec
        .json
        .as_ref()
        .and_then(|v| v.get("tenant_value"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    prec.observe(
        Observation::behavior(
            "an explicit --tenant flag overrides the CLW_TENANT env",
            "config precedence (flag > env > file > default) must be honored deterministically",
        )
        .check(
            resolved == PRECEDENCE_SENTINEL,
            format!(
                "resolved tenant_value = {resolved} (env was {})",
                creds.tenant
            ),
        ),
    );
    j.push(prec);

    j
}
