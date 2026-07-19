//! `clw-e2e-evidence` — user-simulating E2E for `clw`.
//!
//! The owner's non-negotiable: these tests **simulate a user** — they drive the
//! REAL `clw` binary (argv + env) and observe its behavior from the OUTSIDE. There
//! are NO library calls, NO unit paths, NO Rust-level mock-transport injection —
//! "não pode ser por outro caminho". Every step is a real process invocation.
//!
//! And every step captures **evidence**: the exact command (secrets redacted), the
//! exit code (expected vs actual), stdout, stderr, parsed `--json`, timing, and the
//! **behavior/security observations** it proves. The evidence is rendered into an
//! analyzable report ([`report`]) so the owner can analyze behavior and security
//! per step — the report IS the deliverable, not just a green/red.
//!
//! Backend targets are chosen purely by `CLW_ENDPOINT` (a [`Runner`] field): real
//! PROD for the core/error/security journeys, or a controllable hostile backend for
//! the adversarial journeys. Same runner, same evidence capture, both via the binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub mod hostile;
pub mod report;

/// A secret (PAT, ticket) that must never appear in captured output, the report, a
/// log, or a `Debug`. Its value is registered as a [`Runner`] redaction and scrubbed
/// from every captured string before it is stored.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }
    fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// What exit code a step expects — asserted as an observation, never a panic (a
/// mismatch is captured as a FAIL in the evidence so the report shows it).
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub enum ExpectExit {
    /// Exactly this code (e.g. 0 success, 2 fatal, 125 run-internal).
    Eq(i32),
    /// Any non-zero (a declined-consent / error path).
    NonZero,
    /// Don't assert the code (evidence still captured).
    Any,
}

impl ExpectExit {
    fn satisfied_by(&self, code: Option<i32>) -> bool {
        match (self, code) {
            (ExpectExit::Any, _) => true,
            (ExpectExit::Eq(want), Some(got)) => *want == got,
            (ExpectExit::NonZero, Some(got)) => got != 0,
            (_, None) => false, // killed by signal → no code
        }
    }
    fn describe(&self) -> String {
        match self {
            ExpectExit::Eq(c) => format!("exit == {c}"),
            ExpectExit::NonZero => "exit != 0".to_string(),
            ExpectExit::Any => "exit (any)".to_string(),
        }
    }
}

/// Whether an observation is about functional behavior or a security property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ObsKind {
    Behavior,
    Security,
}

/// The verdict of a single observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Verdict {
    Pass,
    Fail,
    /// Recorded as evidence for analysis, not asserted (e.g. a timing or a counter).
    Info,
}

/// One behavior/security claim asserted against a step's captured evidence.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Observation {
    pub kind: ObsKind,
    /// The claim, in the user's terms (e.g. "token never echoed", "exit 0", "0600").
    pub claim: String,
    /// Why it matters (so the report is analyzable without re-deriving intent).
    pub why: String,
    pub verdict: Verdict,
    /// The concrete evidence for the verdict (what was actually observed).
    pub detail: String,
}

impl Observation {
    pub fn behavior(claim: impl Into<String>, why: impl Into<String>) -> ObservationBuilder {
        ObservationBuilder::new(ObsKind::Behavior, claim, why)
    }
    pub fn security(claim: impl Into<String>, why: impl Into<String>) -> ObservationBuilder {
        ObservationBuilder::new(ObsKind::Security, claim, why)
    }
}

/// Builder that resolves to Pass/Fail with a detail string.
pub struct ObservationBuilder {
    kind: ObsKind,
    claim: String,
    why: String,
}

impl ObservationBuilder {
    fn new(kind: ObsKind, claim: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            kind,
            claim: claim.into(),
            why: why.into(),
        }
    }
    /// Pass/Fail from a boolean predicate + the observed detail.
    pub fn check(self, pass: bool, detail: impl Into<String>) -> Observation {
        Observation {
            kind: self.kind,
            claim: self.claim,
            why: self.why,
            verdict: if pass { Verdict::Pass } else { Verdict::Fail },
            detail: detail.into(),
        }
    }
    /// Recorded as evidence only (not asserted).
    pub fn info(self, detail: impl Into<String>) -> Observation {
        Observation {
            kind: self.kind,
            claim: self.claim,
            why: self.why,
            verdict: Verdict::Info,
            detail: detail.into(),
        }
    }
}

/// The full evidence of ONE user step: a real `clw` invocation and everything it
/// produced, plus the observations proven from it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StepEvidence {
    /// Human description of what this step does ("snapshot a fresh workspace").
    pub label: String,
    /// The command line as a user would type it, SECRETS REDACTED.
    pub cmd: String,
    /// Extra env vars set for this step (secret values shown as `<redacted>`).
    pub env_shown: Vec<String>,
    pub expected_exit: ExpectExit,
    pub exit_code: Option<i32>,
    /// Captured stdout, secrets scrubbed.
    pub stdout: String,
    /// Captured stderr, secrets scrubbed.
    pub stderr: String,
    /// Parsed stdout when it is valid JSON (a `--json` step), else `None`.
    pub json: Option<serde_json::Value>,
    pub duration_ms: u128,
    pub observations: Vec<Observation>,
}

impl StepEvidence {
    /// True when the exit expectation held AND no observation FAILed.
    pub fn passed(&self) -> bool {
        self.expected_exit.satisfied_by(self.exit_code)
            && !self.observations.iter().any(|o| o.verdict == Verdict::Fail)
    }
    /// Add an observation derived from this step's captured evidence.
    pub fn observe(&mut self, obs: Observation) -> &mut Self {
        self.observations.push(obs);
        self
    }
    /// Convenience: assert stdout/stderr contains a needle (a behavior observation).
    pub fn expect_output_contains(&mut self, needle: &str, why: &str) -> &mut Self {
        let hit = self.stdout.contains(needle) || self.stderr.contains(needle);
        self.observe(
            Observation::behavior(format!("output contains {needle:?}"), why).check(
                hit,
                if hit {
                    "found in output".to_string()
                } else {
                    "NOT found in output".to_string()
                },
            ),
        )
    }
}

/// A named sequence of user steps against ONE backend target, accumulating evidence.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Journey {
    pub name: String,
    /// One-line description of what the journey proves.
    pub about: String,
    /// The backend the steps ran against (redacted host label, e.g. "prod" or "hostile").
    pub target: String,
    pub steps: Vec<StepEvidence>,
}

impl Journey {
    pub fn new(
        name: impl Into<String>,
        about: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Journey {
            name: name.into(),
            about: about.into(),
            target: target.into(),
            steps: Vec::new(),
        }
    }
    pub fn push(&mut self, step: StepEvidence) -> &mut StepEvidence {
        self.steps.push(step);
        self.steps.last_mut().expect("just pushed")
    }
    pub fn passed(&self) -> bool {
        self.steps.iter().all(StepEvidence::passed)
    }
    pub fn counts(&self) -> (usize, usize) {
        let pass = self.steps.iter().filter(|s| s.passed()).count();
        (pass, self.steps.len())
    }
}

/// Drives the real `clw` binary as a user against ONE backend target.
///
/// The token is a [`Secret`]: it is passed to the child via the `CLW_TOKEN` env
/// (never on argv, never `ps`-visible) and its value — plus any other registered
/// secret — is scrubbed from every captured string BEFORE it is stored, so nothing
/// secret can ever reach the report.
pub struct Runner {
    bin: PathBuf,
    endpoint: String,
    /// Redacted label for the endpoint host shown in the report (e.g. "prod").
    target_label: String,
    tenant: String,
    token: Secret,
    cache_dir: PathBuf,
    redactions: Vec<String>,
}

impl Runner {
    pub fn new(
        bin: impl Into<PathBuf>,
        endpoint: impl Into<String>,
        target_label: impl Into<String>,
        tenant: impl Into<String>,
        token: Secret,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        let endpoint = endpoint.into();
        let mut redactions = Vec::new();
        if !token.is_empty() {
            redactions.push(token.expose().to_string());
        }
        // L1: if the endpoint carries userinfo (user:pass@host), scrub that credential
        // from captured output too — not just the display — in case clw echoes the URL.
        if let Some(userinfo) = endpoint_userinfo(&endpoint) {
            redactions.push(userinfo);
        }
        Runner {
            bin: bin.into(),
            endpoint,
            target_label: target_label.into(),
            tenant: tenant.into(),
            token,
            cache_dir: cache_dir.into(),
            redactions,
        }
    }

    pub fn target_label(&self) -> &str {
        &self.target_label
    }

    /// Register an extra secret (e.g. a second identity's PAT, a cred-ticket) to be
    /// scrubbed from all captured output.
    pub fn add_redaction(&mut self, secret: &Secret) {
        if !secret.is_empty() {
            self.redactions.push(secret.expose().to_string());
        }
    }

    /// Scrub the registered redactions PLUS any ad-hoc `extra` secrets (e.g. a secret
    /// passed via `extra_env` for this one step) from `s` before it is ever stored.
    fn redact_with(&self, s: &str, extra: &[&str]) -> String {
        let mut out = s.to_string();
        for secret in self
            .redactions
            .iter()
            .map(String::as_str)
            .chain(extra.iter().copied())
        {
            if !secret.is_empty() {
                out = out.replace(secret, "<redacted>");
            }
        }
        out
    }

    /// Run one user step: spawn `clw <args>` with the identity env, capture
    /// everything, redact secrets, parse JSON, and return the evidence. `extra_env`
    /// is `(KEY, VALUE, is_secret)` — secret values are shown as `<redacted>` in the
    /// report and scrubbed from output.
    pub fn step(
        &self,
        label: impl Into<String>,
        args: &[&str],
        extra_env: &[(&str, &str, bool)],
        expected_exit: ExpectExit,
    ) -> StepEvidence {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .env("CLW_ENDPOINT", &self.endpoint)
            .env("CLW_TENANT", &self.tenant)
            .env("CLW_TOKEN", self.token.expose())
            .env("CLW_CACHE_DIR", &self.cache_dir)
            // Deterministic, non-interactive: no TTY, no color, no ambient RUST_LOG.
            .env("NO_COLOR", "1")
            .env_remove("RUST_LOG");

        let mut env_shown = vec![
            format!(
                "CLW_ENDPOINT={} ({})",
                redact_endpoint(&self.endpoint),
                self.target_label
            ),
            format!("CLW_TENANT={}", self.tenant),
            "CLW_TOKEN=<redacted>".to_string(),
        ];
        // M1: a secret extra-env value must be scrubbed from captured OUTPUT, not just
        // hidden in the display — else it could leak verbatim if clw echoes it.
        let mut extra_secrets: Vec<&str> = Vec::new();
        for (k, v, secret) in extra_env {
            cmd.env(k, v);
            if *secret {
                env_shown.push(format!("{k}=<redacted>"));
                if !v.is_empty() {
                    extra_secrets.push(v);
                }
            } else {
                env_shown.push(format!("{k}={v}"));
            }
        }

        let cmd_str = format!("clw {}", args.join(" "));
        let started = Instant::now();
        let output = cmd.output();
        let duration_ms = started.elapsed().as_millis();

        let (exit_code, stdout, stderr) = match output {
            Ok(o) => (
                o.status.code(),
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
            ),
            Err(e) => (None, String::new(), format!("<spawn failed: {e}>")),
        };
        let stdout = self.redact_with(&stdout, &extra_secrets);
        let stderr = self.redact_with(&stderr, &extra_secrets);
        let json = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok();

        let mut ev = StepEvidence {
            label: label.into(),
            cmd: self.redact_with(&cmd_str, &extra_secrets),
            env_shown,
            expected_exit,
            exit_code,
            stdout,
            stderr,
            json,
            duration_ms,
            observations: Vec::new(),
        };
        // The exit-code expectation is itself the first behavior observation.
        let ok = expected_exit.satisfied_by(exit_code);
        ev.observe(
            Observation::behavior(
                expected_exit.describe(),
                "the process exit code is the contract a script/runner depends on",
            )
            .check(
                ok,
                match exit_code {
                    Some(c) => format!("observed exit {c}"),
                    None => "killed by signal (no exit code)".to_string(),
                },
            ),
        );
        ev
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
    pub fn tenant(&self) -> &str {
        &self.tenant
    }
}

/// Redact userinfo from an endpoint URL for display (mirrors clw's own posture) so a
/// `user:pass@host` endpoint never leaks into the report.
fn redact_endpoint(ep: &str) -> String {
    match ep.split_once("://") {
        Some((scheme, rest)) => match rest.rsplit_once('@') {
            Some((_creds, host)) => format!("{scheme}://<redacted>@{host}"),
            None => ep.to_string(),
        },
        None => ep.to_string(),
    }
}

/// The `user:pass` credential of an endpoint URL, if present — registered as a
/// redaction so it is scrubbed from captured OUTPUT, not just the display.
fn endpoint_userinfo(ep: &str) -> Option<String> {
    let rest = ep.split_once("://")?.1;
    // Authority ends at the first '/', '?' or '#'.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let creds = authority.rsplit_once('@')?.0;
    (!creds.is_empty()).then(|| creds.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new("core-abc123");
        assert_eq!(format!("{s:?}"), "<redacted>");
    }

    #[test]
    fn redaction_scrubs_registered_secrets() {
        let r = Runner::new(
            "/bin/true",
            "https://x",
            "test",
            "t",
            Secret::new("core-SECRET"),
            "/tmp/c",
        );
        assert_eq!(
            r.redact_with("token=core-SECRET here", &[]),
            "token=<redacted> here"
        );
    }

    #[test]
    fn redact_with_scrubs_ad_hoc_extra_secrets() {
        // M1: a secret passed via extra_env must be scrubbed from captured output.
        let r = Runner::new(
            "/bin/true",
            "https://x",
            "test",
            "t",
            Secret::new("core-A"),
            "/tmp/c",
        );
        assert_eq!(
            r.redact_with("a=core-A ticket=tkt-123", &["tkt-123"]),
            "a=<redacted> ticket=<redacted>"
        );
    }

    #[test]
    fn endpoint_userinfo_is_registered_and_scrubbed() {
        // L1: an endpoint credential is scrubbed from captured output, not just display.
        assert_eq!(
            endpoint_userinfo("https://user:pw@host.example/v1"),
            Some("user:pw".to_string())
        );
        assert_eq!(endpoint_userinfo("https://host.example"), None);
        let r = Runner::new(
            "/bin/true",
            "https://user:pw@host.example",
            "test",
            "t",
            Secret::new(""),
            "/tmp/c",
        );
        assert_eq!(
            r.redact_with("dialed https://user:pw@host.example", &[]),
            "dialed https://<redacted>@host.example"
        );
    }

    #[test]
    fn endpoint_userinfo_redacted() {
        assert_eq!(
            redact_endpoint("https://user:pw@host.example"),
            "https://<redacted>@host.example"
        );
        assert_eq!(
            redact_endpoint("https://host.example"),
            "https://host.example"
        );
    }

    #[test]
    fn expect_exit_semantics() {
        assert!(ExpectExit::Eq(0).satisfied_by(Some(0)));
        assert!(!ExpectExit::Eq(0).satisfied_by(Some(2)));
        assert!(ExpectExit::NonZero.satisfied_by(Some(2)));
        assert!(!ExpectExit::NonZero.satisfied_by(Some(0)));
        assert!(!ExpectExit::Eq(0).satisfied_by(None));
        assert!(ExpectExit::Any.satisfied_by(None));
    }
}
