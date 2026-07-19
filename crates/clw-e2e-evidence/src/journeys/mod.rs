//! The user-simulating journeys — each drives the REAL `clw` binary against a target
//! and returns a [`Journey`](crate::Journey) of per-step evidence.
//!
//! Onda 2 = the prod journeys (J1–J7): a real user's paths against the live backend.
//! Every journey is runnable — it drives the binary and emits analyzable evidence — and
//! degrades gracefully to a single "skipped (no creds)" info step when the prod identity
//! is absent, so an offline run still produces a report (never a panic).
//!
//! Nothing here calls a clw library function: the ONLY entry point is the binary via
//! [`Runner::step`](crate::Runner). "Não pode ser por outro caminho."

use std::path::PathBuf;

use crate::{Journey, Observation, Secret};

pub mod j1_onboarding;
pub mod j2_lifecycle;

/// The default prod endpoint (same host the live conformance suite dials).
pub const DEFAULT_ENDPOINT: &str = "https://corelink-api.humangr.com";

/// The live prod identity, loaded from the environment — the same source the live
/// conformance suite uses (`CLW_ENDPOINT`/`CLW_TENANT`/`CLW_TOKEN`). The token is a
/// [`Secret`], so it never reaches the report.
pub struct ProdCreds {
    pub endpoint: String,
    pub tenant: String,
    pub token: Secret,
}

impl ProdCreds {
    /// Load from the environment. `None` (→ the journey emits a skip step, no panic)
    /// when the tenant or token is absent — an offline run still yields a report.
    pub fn from_env() -> Option<ProdCreds> {
        let token = std::env::var("CLW_TOKEN").ok().filter(|s| !s.is_empty())?;
        let tenant = std::env::var("CLW_TENANT").ok().filter(|s| !s.is_empty())?;
        let endpoint = std::env::var("CLW_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        Some(ProdCreds {
            endpoint,
            tenant,
            token: Secret::new(token),
        })
    }

    /// A short, redacted label for the report target (`prod · <tenant>`).
    pub fn target(&self) -> String {
        format!("prod · {}", self.tenant)
    }
}

/// A single "skipped — needs live creds" step, so a journey run with no identity still
/// renders a truthful report entry (Info, never a false Pass or a panic).
pub fn skipped_journey(name: &str, about: &str, need: &str) -> Journey {
    let mut j = Journey::new(name, about, "skipped (no creds)");
    j.skipped = true;
    let mut ev = crate::StepEvidence {
        label: format!("skipped — {need}"),
        cmd: String::new(),
        env_shown: Vec::new(),
        expected_exit: crate::ExpectExit::Any,
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
            "journey skipped without live credentials",
            "the prod journeys require a real identity; absence is reported, not passed",
        )
        .info(format!("set {need} to run this journey against prod")),
    );
    j.push(ev);
    j
}

/// A deterministic, self-cleaning workspace fixture under a per-journey temp root.
///
/// The root is removed on drop so a run leaves no litter. Content is deterministic
/// (a fixed seed) so byte-identity and dedup observations are reproducible.
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// A fresh root at `<tmp>/clw-e2e-<tag>-<pid>` (wiped first so a crashed prior run
    /// can't poison it). `tag` disambiguates concurrent journeys in one process.
    pub fn new(tag: &str) -> Workspace {
        let root = std::env::temp_dir().join(format!("clw-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create workspace root");
        Workspace { root }
    }

    /// A named subdirectory (created).
    pub fn dir(&self, name: &str) -> PathBuf {
        let p = self.root.join(name);
        std::fs::create_dir_all(&p).expect("create workspace subdir");
        p
    }

    /// Write `bytes` to `rel` under the root (parents created); returns the full path.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = self.root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&p, bytes).expect("write fixture file");
        p
    }

    /// A path under the root (NOT created) — for files a child process will create
    /// itself (e.g. a `run` side-effect marker). The parent dir must already exist.
    pub fn root_join(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// A dedicated cache dir under the root (so a journey never shares clw's real cache).
    pub fn cache(&self) -> PathBuf {
        self.dir("cache")
    }

    /// A sandbox `$HOME` under the root, for journeys that write `~/.clw/config.toml`
    /// (init/auth) — so they NEVER touch the operator's real config.
    pub fn home(&self) -> PathBuf {
        self.dir("home")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Deterministic pseudo-random bytes (a cheap LCG-ish hash of the index). Used for the
/// "large file" fixtures so content-addressing / byte-identity is reproducible without
/// an RNG dependency (and `Math.random`-style nondeterminism).
pub fn pseudo_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

/// Read a file's unix permission bits (`mode & 0o777`), or `None` off-unix.
#[cfg(unix)]
pub fn unix_mode(path: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
pub fn unix_mode(_path: &std::path::Path) -> Option<u32> {
    None
}
