//! The user-simulating journeys — each drives the REAL `clw` binary against a target
//! and returns a [`Journey`](crate::Journey) of per-step evidence.
//!
//! STORY-DRIVEN: each journey is a PERSONA (a real-looking customer) living a realistic
//! narrative, so the system can't tell it's a test and the evidence is judged against the
//! user's INTENT — not a step-list with baked asserts. The cast (`story_*`): Marina (a solo
//! backend engineer's real week), Priya + Sam (two identities on one tenant), Dana (a
//! security engineer's audit), Ravi (clw inside CI), Omar (a destructive decommission + GDPR
//! erase on a throwaway), and Nadia (a metered tenant hitting its 402 spend wall). Each
//! degrades gracefully to a "skipped (no creds)" step when its identity is absent, so an
//! offline run still produces a report (never a panic).
//!
//! Nothing here calls a clw library function: the ONLY entry point is the binary via
//! [`Runner::step`](crate::Runner). "Não pode ser por outro caminho."

use std::path::PathBuf;

use crate::{Journey, Observation, Secret};

pub mod realistic;
pub mod story_automation;
pub mod story_offboarding;
pub mod story_quota;
pub mod story_security;
pub mod story_ship_service;
pub mod story_team;

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

/// The DEDICATED throwaway tenant + PAT for the DESTRUCTIVE journey (J5). J5 runs a real,
/// irreversible `erase`, so this is fenced hard: `None` (→ J5 is skipped) when unset, AND
/// `None` if the erase tenant EQUALS `primary` — a misconfigured env must never let an
/// erase touch the shared/primary tenant.
pub struct EraseCreds {
    pub endpoint: String,
    pub tenant: String,
    pub token: Secret,
}

pub fn erase_creds(primary_tenant: &str) -> Option<EraseCreds> {
    let tenant = std::env::var("CLW_E2E_ERASE_TENANT")
        .ok()
        .filter(|s| !s.is_empty())?;
    let token = std::env::var("CLW_E2E_ERASE_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())?;
    // THE fence: never erase the primary tenant, whatever the env says.
    if tenant == primary_tenant {
        eprintln!(
            "REFUSING J5: CLW_E2E_ERASE_TENANT == the primary CLW_TENANT ({primary_tenant}) — \
             an erase must target a dedicated throwaway tenant, never the working one"
        );
        return None;
    }
    let endpoint = std::env::var("CLW_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
    Some(EraseCreds {
        endpoint,
        tenant,
        token: Secret::new(token),
    })
}

/// A DEDICATED low-quota tenant + PAT for the quota persona (its monthly $-ceiling is set
/// tiny out-of-band so a snapshot trips the real 402). Fenced like the erase creds: `None`
/// unless set AND `!= primary` — a $1 ceiling must never land on the working tenant.
pub type QuotaCreds = EraseCreds;

pub fn quota_creds(primary_tenant: &str) -> Option<QuotaCreds> {
    let tenant = std::env::var("CLW_E2E_QUOTA_TENANT")
        .ok()
        .filter(|s| !s.is_empty())?;
    let token = std::env::var("CLW_E2E_QUOTA_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())?;
    if tenant == primary_tenant {
        eprintln!("REFUSING quota persona: CLW_E2E_QUOTA_TENANT == primary — a $-ceiling must target a dedicated tenant");
        return None;
    }
    let endpoint = std::env::var("CLW_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
    Some(EraseCreds {
        endpoint,
        tenant,
        token: Secret::new(token),
    })
}

/// A SECOND identity's PAT (`CLW_TOKEN_B`) on the same tenant, for the team journey (J4).
/// `None` → J4 is skipped. Kept separate from [`ProdCreds`] because most journeys are
/// single-identity; only the team journey needs two.
pub fn second_token() -> Option<Secret> {
    std::env::var("CLW_TOKEN_B")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Secret::new)
}

/// A single "skipped — needs live creds" step, so a journey run with no identity still
/// renders a truthful report entry (Info, never a false Pass or a panic). This is a
/// REQUIRED skip: under `CLW_E2E_REQUIRE_PROD=1` it is a hard failure (the primary-cred
/// journeys must always run in CI, or the lane is canary-blind).
pub fn skipped_journey(name: &str, about: &str, need: &str) -> Journey {
    build_skip(name, about, need, false)
}

/// Like [`skipped_journey`] but the skip is EXPECTED-optional: the journey needs extra
/// infra beyond the primary PAT (J4's second identity, J5's throwaway erase tenant). It
/// is reported truthfully but does NOT trip `CLW_E2E_REQUIRE_PROD` — so the CI lane is
/// green with just the primary credential (mirrors the conformance suite's `soft_skip`).
pub fn skipped_optional_journey(name: &str, about: &str, need: &str) -> Journey {
    build_skip(name, about, need, true)
}

fn build_skip(name: &str, about: &str, need: &str, optional: bool) -> Journey {
    let mut j = Journey::new(name, about, "skipped (no creds)");
    j.skipped = true;
    j.optional = optional;
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
        narration: None,
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

    /// The workspace root (for generators that write a whole tree under it).
    pub fn root_dir(&self) -> &std::path::Path {
        &self.root
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

/// Deterministic bytes SEEDED so EVERY byte depends on `seed` (a splitmix64 hash of
/// `i + seed`). Two blobs with the same seed are byte-identical (so their content-address
/// / dedup match), while a fresh seed makes the WHOLE blob globally unique — a genuine
/// first upload where no chunk pre-exists in the CAS (unlike [`pseudo_bytes`], whose tail
/// chunks are constant across runs and would dedup). Still RNG-free and reproducible.
pub fn pseudo_bytes_seeded(len: usize, seed: u64) -> Vec<u8> {
    (0..len as u64)
        .map(|i| {
            let mut z = i.wrapping_add(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z >> 24) as u8
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_bytes_are_deterministic_per_seed_but_differ_across_seeds() {
        // Same seed → byte-identical (so J3's A and B share a root and dedup).
        assert_eq!(pseudo_bytes_seeded(4096, 42), pseudo_bytes_seeded(4096, 42));
        // Different seed → different content (so a fresh run is a genuine first upload);
        // require a large fraction of bytes to differ, not just one.
        let a = pseudo_bytes_seeded(4096, 1);
        let b = pseudo_bytes_seeded(4096, 2);
        let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert!(
            differing > 4096 * 3 / 4,
            "seed must perturb the whole blob, only {differing}/4096 bytes differed"
        );
    }

    #[test]
    fn seeded_bytes_have_the_requested_length() {
        assert_eq!(pseudo_bytes_seeded(0, 7).len(), 0);
        assert_eq!(pseudo_bytes_seeded(1_000_003, 7).len(), 1_000_003);
    }
}
