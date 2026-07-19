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
//!                 offline-only smoke)
//!
//! This binary currently ships the SMOKE journey that proves the harness end-to-end
//! (offline steps always; a prod snapshot→hydrate→rm when creds are present). The full
//! journey set (J1–J11) lands as follow-on modules per the plan.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clw_e2e_evidence::hostile::Hostile;
use clw_e2e_evidence::report::{render_html, render_markdown, Meta};
use clw_e2e_evidence::{ExpectExit, Journey, Observation, Runner, Secret};

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
    journeys.push(smoke_offline(&bin));
    if let Some(j) = smoke_prod(&bin) {
        journeys.push(j);
    } else {
        eprintln!("NOTE: CLW_TOKEN/CLW_TENANT absent → prod smoke skipped (offline-only report)");
    }
    // Adversarial: needs no creds — the real binary vs a local hostile backend.
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

    let all_pass = journeys.iter().all(Journey::passed);
    let (jp, jt): (usize, usize) = (
        journeys.iter().filter(|j| j.passed()).count(),
        journeys.len(),
    );
    eprintln!(
        "evidence: {}/{} journeys passed → {} · {}",
        jp,
        jt,
        md_path.display(),
        html_path.display()
    );
    std::process::exit(if all_pass { 0 } else { 1 });
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

    // doctor runs on a broken/empty config and MUST report the token as a marker,
    // never the value. (The tenant IS shown in full by design — that is not a secret.)
    let mut doc = r.step(
        "doctor --json",
        &["--json", "doctor"],
        &[],
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

    j
}

/// Prod smoke — a real snapshot→hydrate→rm round-trip against the real backend, if
/// creds are present. Returns `None` (skip) when `CLW_TOKEN`/`CLW_TENANT` is absent.
fn smoke_prod(bin: &Path) -> Option<Journey> {
    let token = std::env::var("CLW_TOKEN").ok().filter(|s| !s.is_empty())?;
    let tenant = std::env::var("CLW_TENANT").ok().filter(|s| !s.is_empty())?;
    let endpoint = std::env::var("CLW_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://corelink-api.humangr.com".to_string());

    let workdir = std::env::temp_dir().join(format!("clw-e2e-{}", std::process::id()));
    let src = workdir.join("src");
    let dst = workdir.join("dst");
    std::fs::create_dir_all(&src).ok()?;
    std::fs::write(
        src.join("small.txt"),
        format!("e2e-{}\n", std::process::id()),
    )
    .ok()?;
    let big: Vec<u8> = (0u32..(3 * 1024 * 1024))
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    std::fs::write(src.join("big.bin"), &big).ok()?;

    let cache = workdir.join("cache");
    let r = Runner::new(bin, &endpoint, "prod", &tenant, Secret::new(token), &cache);
    let name = format!("e2e-smoke-{}", std::process::id());
    let mut j = Journey::new(
        "smoke-prod",
        "a real snapshot→hydrate→rm round-trip against the live backend",
        format!("prod · {}", tenant),
    );

    let src_s = src.to_string_lossy().into_owned();
    let dst_s = dst.to_string_lossy().into_owned();

    let mut snap = r.step(
        "snapshot a fresh workspace",
        &["--json", "snapshot", &src_s, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    if let Some(v) = &snap.json {
        let uploaded = v.get("chunks_uploaded").and_then(|x| x.as_u64());
        snap.observe(
            Observation::behavior(
                "fresh snapshot uploads chunks",
                "a first snapshot must populate the content-addressed store",
            )
            .check(
                uploaded.unwrap_or(0) > 0,
                format!("chunks_uploaded={uploaded:?}"),
            ),
        );
    }
    let snap_ok = snap.passed();
    j.push(snap);

    if snap_ok {
        let mut hyd = r.step(
            "hydrate into an empty dest",
            &["--json", "hydrate", &dst_s, "--name", &name],
            &[],
            ExpectExit::Eq(0),
        );
        // Byte-identity: compare the hydrated big.bin to the source.
        let identical = std::fs::read(dst.join("big.bin"))
            .map(|b| b == big)
            .unwrap_or(false);
        hyd.observe(
            Observation::behavior(
                "hydrated tree is byte-identical to the source",
                "content-addressing must round-trip exactly, or the store is unsafe",
            )
            .check(
                identical,
                if identical {
                    "big.bin matches"
                } else {
                    "MISMATCH"
                },
            ),
        );
        j.push(hyd);
    }

    // Cleanup: rm the ref (idempotent, requires --yes).
    let rm = r.step(
        "rm the ref (cleanup)",
        &["--json", "rm", "--name", &name, "--yes"],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(rm);

    let _ = std::fs::remove_dir_all(&workdir);
    Some(j)
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
