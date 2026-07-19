//! `clw-e2e` — the user-simulating E2E evidence runner.
//!
//! Drives the REAL `clw` binary as a user and writes an analyzable evidence report
//! (markdown + HTML). Backend target = `CLW_ENDPOINT` (prod for the real journeys, a
//! controllable hostile backend for the adversarial ones). Credentials come from the
//! env (`CLW_TENANT`/`CLW_TOKEN`), same as the live conformance suite; the token is a
//! `Secret` and never reaches the report.
//!
//! Env:
//!   CLW_E2E_BIN   path to the clw binary (default: `clw` next to this exe)
//!   CLW_E2E_OUT   report output dir (default: ./e2e-reports)
//!   CLW_ENDPOINT / CLW_TENANT / CLW_TOKEN  the target + identity (token optional →
//!                 prod journeys are SKIPPED, an offline-only report is still produced)
//!   CLW_E2E_REQUIRE_PROD=1  make a skipped prod journey a hard failure (CI uses this so
//!                 the pipeline can never silently stop exercising prod — canary-blind).
//!
//! Journeys shipped: `smoke-offline` (harness self-test) always; J1 onboarding + J2
//! lifecycle against the live backend when creds are present; J9 adversarial vs a local
//! hostile backend always. The remaining prod journeys (J3–J7, J8, J10/J11) land as
//! follow-on modules per the plan. A skip is counted separately from a pass, so an
//! absent-creds run can never be reported as green.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clw_e2e_evidence::hostile::Hostile;
use clw_e2e_evidence::journeys::{self, ProdCreds};
use clw_e2e_evidence::report::{render_html, render_markdown, Meta};
use clw_e2e_evidence::{ExpectExit, Journey, Observation, Runner, Secret, StepEvidence};

fn main() {
    let bin = clw_bin();
    if !bin.exists() {
        eprintln!(
            "clw binary not found at {} — set CLW_E2E_BIN or `cargo build -p clw-cli`",
            bin.display()
        );
        std::process::exit(2);
    }
    let out_dir = std::env::var("CLW_E2E_OUT").unwrap_or_else(|_| "e2e-reports".to_string());
    std::fs::create_dir_all(&out_dir).expect("create report dir");

    let clw_version = probe_version(&bin);
    let mut journeys = Vec::new();
    // Harness self-test: the credential-free surface (always runs, proves the harness).
    journeys.push(smoke_offline(&bin));

    // The prod journeys (Onda 2) — driven against the live backend when creds are
    // present, else a truthful "skipped" entry so an offline run still yields a report.
    // Each is guarded: a setup panic becomes a FAILED journey (not a lost report), so
    // smoke-offline + the other journeys' evidence always survives.
    match ProdCreds::from_env() {
        Some(creds) => {
            journeys.push(guarded("J1-onboarding", || {
                journeys::j1_onboarding::run(&bin, &creds)
            }));
            journeys.push(guarded("J2-lifecycle", || {
                journeys::j2_lifecycle::run(&bin, &creds)
            }));
            journeys.push(guarded("J3-content-addressing", || {
                journeys::j3_content_addressing::run(&bin, &creds)
            }));
            // J4 needs a SECOND identity (CLW_TOKEN_B) on the same tenant.
            match journeys::second_token() {
                Some(token_b) => journeys.push(guarded("J4-team-concurrency", || {
                    journeys::j4_team::run(&bin, &creds, &token_b)
                })),
                None => journeys.push(journeys::skipped_journey(
                    "J4-team-concurrency",
                    "two identities on one tenant + concurrent writes",
                    "CLW_TOKEN_B (a second identity PAT)",
                )),
            }
            journeys.push(guarded("J6-errors", || {
                journeys::j6_errors::run(&bin, &creds)
            }));
            journeys.push(guarded("J7-security", || {
                journeys::j7_security::run(&bin, &creds)
            }));
            // J5 is DESTRUCTIVE (real erase). It runs ONLY against a dedicated throwaway
            // tenant that is not the primary — the loader fences both conditions.
            match journeys::erase_creds(&creds.tenant) {
                Some(ec) => journeys.push(guarded("J5-destructive", || {
                    journeys::j5_destructive::run(&bin, &ec)
                })),
                None => journeys.push(journeys::skipped_journey(
                    "J5-destructive",
                    "prune + erase on a dedicated throwaway tenant",
                    "CLW_E2E_ERASE_TENANT (≠ primary) + CLW_E2E_ERASE_TOKEN",
                )),
            }
        }
        None => {
            eprintln!(
                "NOTE: CLW_TOKEN/CLW_TENANT absent → prod journeys skipped (offline-only report)"
            );
            let need = "CLW_TOKEN + CLW_TENANT";
            journeys.push(journeys::skipped_journey(
                "J1-onboarding",
                "the first surface a user touches",
                need,
            ));
            journeys.push(journeys::skipped_journey(
                "J2-lifecycle",
                "snapshot → status → ls → hydrate → run → rm",
                need,
            ));
            journeys.push(journeys::skipped_journey(
                "J3-content-addressing",
                "dedup across a cold cache + pin-by-manifest-digest",
                need,
            ));
            journeys.push(journeys::skipped_journey(
                "J4-team-concurrency",
                "two identities on one tenant + concurrent writes",
                need,
            ));
            journeys.push(journeys::skipped_journey(
                "J6-errors",
                "inducible failures return the right exit code + actionable message",
                need,
            ));
            journeys.push(journeys::skipped_journey(
                "J7-security",
                "redaction, plaintext refusal, consent gates — observed via the binary",
                need,
            ));
            journeys.push(journeys::skipped_journey(
                "J5-destructive",
                "prune + erase on a dedicated throwaway tenant",
                need,
            ));
        }
    }

    // Adversarial (J9): needs no creds — the real binary vs a local hostile backend.
    journeys.push(adversarial(&bin));

    let meta = Meta {
        title: "clw — user-simulating E2E evidence".to_string(),
        clw_version,
        generated_at: now_iso(),
        os: std::env::consts::OS.to_string(),
    };
    let md = render_markdown(&journeys, &meta);
    let html = render_html(&journeys, &meta);
    let md_path = Path::new(&out_dir).join("evidence.md");
    let html_path = Path::new(&out_dir).join("evidence.html");
    std::fs::write(&md_path, &md).expect("write md");
    std::fs::write(&html_path, &html).expect("write html");

    // Count passes / skips / failures SEPARATELY — a skipped journey (no creds) is never
    // folded into "passed", so an absent-creds run can never masquerade as green (the
    // canary-blind failure mode). `CLW_E2E_REQUIRE_PROD=1` (set in CI) makes a skipped
    // prod journey a hard failure, so the pipeline can't silently stop exercising prod.
    let passed = journeys.iter().filter(|j| !j.skipped && j.passed()).count();
    let failed = journeys
        .iter()
        .filter(|j| !j.skipped && !j.passed())
        .count();
    let skipped = journeys.iter().filter(|j| j.skipped).count();
    let require_prod = std::env::var("CLW_E2E_REQUIRE_PROD").ok().as_deref() == Some("1");
    let require_violated = require_prod && skipped > 0;
    eprintln!(
        "evidence: {passed} passed, {skipped} skipped, {failed} failed → {} · {}",
        md_path.display(),
        html_path.display()
    );
    if require_violated {
        eprintln!(
            "FAIL: CLW_E2E_REQUIRE_PROD=1 but {skipped} prod journey(s) were skipped (creds absent)"
        );
    }
    std::process::exit(if failed == 0 && !require_violated {
        0
    } else {
        1
    });
}

/// Run a journey builder, converting a panic (e.g. a fixture-setup failure) into a
/// single FAILED-step journey rather than unwinding through `main` and losing the
/// whole report (smoke-offline + the other journeys' evidence).
fn guarded(name: &str, build: impl FnOnce() -> Journey + std::panic::UnwindSafe) -> Journey {
    match std::panic::catch_unwind(build) {
        Ok(j) => j,
        Err(_) => {
            let mut j = Journey::new(
                name,
                "journey aborted before completion",
                "prod (setup failed)",
            );
            let mut ev = StepEvidence {
                label: "journey setup".to_string(),
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
            ev.observe(
                Observation::behavior(
                    "the journey panicked during setup",
                    "a fixture/FS failure must be a loud FAIL, not a lost report",
                )
                .check(false, "journey builder panicked — see stderr"),
            );
            j.push(ev);
            j
        }
    }
}

/// Offline journey — no creds, no network: version, help, completions, doctor. Proves
/// the harness + the always-available user surface.
fn smoke_offline(bin: &Path) -> Journey {
    let mut j = Journey::new(
        "smoke-offline",
        "the credential-free surface a user hits first",
        "none (offline)",
    );
    // A runner with an empty token + a throwaway cache; offline steps ignore them.
    let cache = std::env::temp_dir().join("clw-e2e-smoke-cache");
    let r = Runner::new(
        bin,
        "https://corelink-api.humangr.com",
        "offline",
        "00000000-0000-4000-8000-000000000000",
        Secret::new(""),
        &cache,
    );

    let v = r.step("clw --version", &["--version"], &[], ExpectExit::Eq(0));
    let mut v = v;
    v.expect_output_contains("clw", "the version string identifies the build");
    j.push(v);

    let mut comp = r.step(
        "completions bash",
        &["completions", "bash"],
        &[],
        ExpectExit::Eq(0),
    );
    comp.observe(
        Observation::behavior(
            "emits a non-empty completion script",
            "shell completion is offline + config-free",
        )
        .check(
            comp.stdout.len() > 100,
            format!("{} bytes on stdout", comp.stdout.len()),
        ),
    );
    j.push(comp);

    // doctor runs on an empty config and MUST report the token as a marker, never the
    // value. (The tenant IS shown in full by design — that is not a secret.) HOME is
    // sandboxed to an empty temp dir so this self-test reads NO operator config and stays
    // deterministic — the report never bleeds the machine's real ~/.clw.
    let sandbox_home =
        std::env::temp_dir().join(format!("clw-e2e-smoke-home-{}", std::process::id()));
    std::fs::create_dir_all(&sandbox_home).ok();
    let sandbox_home_s = sandbox_home.to_string_lossy().into_owned();
    let mut doc = r.step(
        "doctor --json",
        &["--json", "doctor"],
        &[("HOME", &sandbox_home_s, false)],
        ExpectExit::Eq(0),
    );
    let marker =
        doc.stdout.contains("\"token\":\"<set>\"") || doc.stdout.contains("\"token\":\"<unset>\"");
    doc.observe(
        Observation::security(
            "doctor reports token as <set>/<unset>, never the value",
            "a diagnostics bundle is often pasted into tickets — a leaked PAT is full account access",
        )
        .check(
            marker,
            format!("token field = {}", doc.json.as_ref().and_then(|v| v.get("token")).map(|v| v.to_string()).unwrap_or_default()),
        ),
    );
    j.push(doc);

    let _ = std::fs::remove_dir_all(&sandbox_home);
    j
}

/// J9 — the real `clw` binary vs a controllable HOSTILE backend: the security /
/// robustness defenses prod cannot produce on demand (SSRF/redirect, 5xx, 413, 429).
/// Needs no creds — a fake token; the hostile backend ignores auth. A short op-deadline
/// bounds each scenario and proves "backs off then fails cleanly", never a hang.
fn adversarial(bin: &Path) -> Journey {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let mut j = Journey::new(
        "J9-adversarial",
        "the real binary vs a controllable hostile backend — defenses prod can't produce on demand",
        "hostile local backend (loopback http)",
    );
    let workdir = std::env::temp_dir().join(format!("clw-e2e-adv-{}", std::process::id()));
    let src = workdir.join("src");
    std::fs::create_dir_all(&src).ok();
    std::fs::write(src.join("f.txt"), "adversarial-probe\n").ok();
    let src_s = src.to_string_lossy().into_owned();
    let tenant = "00000000-0000-4000-8000-00000000dead";
    let fake = Secret::new("core-e2e-adversarial-fake-token");
    let cache = workdir.join("cache");
    // A short op-deadline: the evidence is "bounded backoff then a clean error", and
    // it keeps the retryable scenarios (5xx/429) from taking the full 300 s default.
    let deadline: &[(&str, &str, bool)] = &[("CLW_OP_DEADLINE_SECS", "10", false)];
    let snap = |name: &str| {
        [
            "--json".to_string(),
            "snapshot".to_string(),
            src_s.clone(),
            "--name".to_string(),
            name.to_string(),
        ]
    };

    // --- 5xx everywhere: retries within the deadline, then a clear error, no hang. ---
    {
        let h = rt.block_on(Hostile::always_5xx());
        let r = Runner::new(
            bin,
            h.endpoint(),
            "hostile:5xx",
            tenant,
            fake.clone(),
            cache.join("5xx"),
        );
        let a = snap("adv5xx");
        let mut s = r.step(
            "snapshot vs a backend that 503s every request",
            &a.iter().map(String::as_str).collect::<Vec<_>>(),
            deadline,
            ExpectExit::NonZero,
        );
        s.observe(
            Observation::behavior(
                "the op-deadline bounds the retries (no hang) and surfaces a clear, actionable error",
                "a 5xx / black-hole backend must never wedge the client — CLW_OP_DEADLINE_SECS is the ceiling",
            )
            .check(
                s.exit_code.map(|c| c != 0).unwrap_or(false)
                    && s.duration_ms < 20_000
                    && s.stderr.contains("deadline")
                    && s.stderr.contains("error:"),
                format!("exit {:?} in {} ms; error names the deadline", s.exit_code, s.duration_ms),
            ),
        );
        j.push(s);
    }

    // --- SSRF: 302 → internal host; PROVE the binary never followed it. ---
    {
        let h = rt.block_on(Hostile::ssrf_redirect());
        let r = Runner::new(
            bin,
            h.endpoint(),
            "hostile:redirect",
            tenant,
            fake.clone(),
            cache.join("ssrf"),
        );
        let a = snap("advssrf");
        let mut s = r.step(
            "snapshot vs a backend that 302-redirects to an internal host",
            &a.iter().map(String::as_str).collect::<Vec<_>>(),
            deadline,
            ExpectExit::NonZero,
        );
        let paths = rt.block_on(h.received_paths());
        let followed = paths.iter().any(|p| p.contains("internal-metadata"));
        // Guard against a vacuous pass: the binary must have ACTUALLY reached the
        // backend and received the 302 (a non-internal request) before "did not follow"
        // means anything. A clw no-op / connect failure would otherwise pass falsely.
        let reached = paths.iter().any(|p| !p.contains("internal-metadata"));
        s.observe(
            Observation::security(
                "no-redirect: reached the backend, got the 302, and NEVER contacted the internal host",
                "following a server-chosen redirect is an SSRF / credential-exfiltration vector",
            )
            .check(
                reached && !followed,
                format!(
                    "server saw {} request(s) (reached backend: {reached}); internal-metadata hit: {followed}",
                    paths.len()
                ),
            ),
        );
        s.observe(
            Observation::behavior(
                "surfaces the redirect as an error, does not chase it",
                "a create-only content store must never be redirected out from under the user",
            )
            .check(
                s.exit_code.map(|c| c != 0).unwrap_or(false),
                format!("exit {:?}", s.exit_code),
            ),
        );
        j.push(s);
    }

    // --- 413 on PUT: a refused/oversized blob is a clean error, not a hang/silent drop. ---
    {
        let h = rt.block_on(Hostile::put_413());
        let r = Runner::new(
            bin,
            h.endpoint(),
            "hostile:413",
            tenant,
            fake.clone(),
            cache.join("413"),
        );
        let a = snap("adv413");
        let mut s = r.step(
            "snapshot vs a backend that 413s every PUT",
            &a.iter().map(String::as_str).collect::<Vec<_>>(),
            deadline,
            ExpectExit::NonZero,
        );
        let put_seen = rt
            .block_on(h.received_methods())
            .iter()
            .any(|m| m.eq_ignore_ascii_case("PUT"));
        s.observe(
            Observation::behavior(
                "actually issued a PUT and surfaced the 413 as a clean error (no hang, no silent drop)",
                "the client must reach the upload and fail loud on a refused/oversized blob",
            )
            .check(
                put_seen && s.exit_code.map(|c| c != 0).unwrap_or(false) && s.duration_ms < 20_000,
                format!(
                    "PUT reached backend: {put_seen}; exit {:?} in {} ms",
                    s.exit_code, s.duration_ms
                ),
            ),
        );
        j.push(s);
    }

    // --- 429 everywhere: backs off (AIMD) then exhausts to a clear error, no tight loop. ---
    {
        let h = rt.block_on(Hostile::always_429());
        let r = Runner::new(
            bin,
            h.endpoint(),
            "hostile:429",
            tenant,
            fake.clone(),
            cache.join("429"),
        );
        let a = snap("adv429");
        let mut s = r.step(
            "snapshot vs a backend that 429s every request",
            &a.iter().map(String::as_str).collect::<Vec<_>>(),
            deadline,
            ExpectExit::NonZero,
        );
        let backed_off = s.stderr.contains("backing off") || s.stderr.contains("rate-limited");
        s.observe(
            Observation::behavior(
                "actually backs off on 429 (AIMD), then exhausts to a clean error — no hammering, no hang",
                "rate-limit backpressure must be respected, not ignored or infinitely retried",
            )
            .check(
                s.exit_code.map(|c| c != 0).unwrap_or(false) && backed_off && s.duration_ms < 20_000,
                format!(
                    "exit {:?} in {} ms; observed AIMD backoff in the client log: {}",
                    s.exit_code, s.duration_ms, backed_off
                ),
            ),
        );
        j.push(s);
    }

    let _ = std::fs::remove_dir_all(&workdir);
    j
}

fn clw_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CLW_E2E_BIN") {
        return PathBuf::from(p);
    }
    // Sibling of this exe: target/<profile>/clw[.exe].
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = if cfg!(windows) { "clw.exe" } else { "clw" };
    dir.join(name)
}

fn probe_version(bin: &Path) -> String {
    std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn now_iso() -> String {
    // Epoch seconds → a coarse UTC label (no chrono dep). Good enough for a report header.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch {secs} (UTC)")
}
