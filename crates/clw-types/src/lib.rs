//! clw-types — THE FROZEN CONTRACT (wave-0, 2026-06-04).
//!
//! Every other crate in this workspace builds against the surfaces in this file.
//! WP agents TRANSCRIBE against this contract; they never redesign it. Changes to
//! this file require the tech lead (orchestrator) — never a WP agent.
//!
//! Server reality this contract is frozen against (verified live 2026-06-04):
//! - `GET/PUT https://corelink-api.humangr.com/v1/cas/:tenant/:hash` (5 MiB blob cap)
//! - `GET/PUT https://corelink-api.humangr.com/v1/ac/:tenant/:key`
//! - Auth: CoreLink PAT via `Authorization: Bearer <token>`; 401 = bad/missing PAT.
//! - Hashing: BLAKE3 everywhere (matches CoreLink CAS digests).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Frozen constants
// ---------------------------------------------------------------------------

/// Manifest schema version emitted by this tool generation.
pub const MANIFEST_VERSION: u32 = 1;

/// FastCDC chunking parameters (frozen: changing them re-chunks the world and
/// destroys cross-snapshot dedupe — do not touch without a lead decision record).
pub const CHUNK_MIN_BYTES: usize = 256 * 1024;
pub const CHUNK_AVG_BYTES: usize = 1024 * 1024;
pub const CHUNK_MAX_BYTES: usize = 4 * 1024 * 1024;

/// CoreLink CAS single-blob cap (server-enforced; PUTs above this 413).
pub const CAS_BLOB_CAP_BYTES: usize = 5 * 1024 * 1024;

// Compile-time guarantee: every chunk fits a single CAS blob.
const _: () = assert!(CHUNK_MAX_BYTES < CAS_BLOB_CAP_BYTES);

/// Domain separator for workspace ref keys in the AC namespace.
pub const REF_KEY_DOMAIN: &str = "clw/ref/v1/";

// ---------------------------------------------------------------------------
// Digest (BLAKE3-256)
// ---------------------------------------------------------------------------

/// A BLAKE3-256 content digest. Hex-encoded (64 lowercase chars) on the wire,
/// in manifests, and in URLs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    /// Hash raw bytes with BLAKE3.
    pub fn of_bytes(data: &[u8]) -> Self {
        Digest(*blake3::hash(data).as_bytes())
    }

    /// Parse a 64-char lowercase hex digest.
    pub fn from_hex(s: &str) -> Result<Self> {
        let raw = hex::decode(s).map_err(|_| ClwError::InvalidDigest(s.to_string()))?;
        let arr: [u8; 32] = raw
            .try_into()
            .map_err(|_| ClwError::InvalidDigest(s.to_string()))?;
        Ok(Digest(arr))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({})", self.to_hex())
    }
}

impl Serialize for Digest {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Digest::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Manifest (the workspace-as-an-object)
// ---------------------------------------------------------------------------

/// One stored chunk of a file's content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub digest: Digest,
    pub size: u64,
}

/// A manifest entry. `path` is always relative to the workspace root, uses `/`
/// as separator, and never contains `.` / `..` components.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    File {
        path: String,
        /// Unix permission bits (e.g. 0o644, 0o755), masked to 0o777 at walk time
        /// (setuid/setgid/sticky stripped; hydrate strips them too). On Windows,
        /// which IS supported (M2.3), a constant 0o644 is recorded and the mode is
        /// applied best-effort on hydrate.
        mode: u32,
        size: u64,
        /// In path order; concatenation of chunks == file content.
        chunks: Vec<ChunkRef>,
    },
    Symlink {
        path: String,
        target: String,
    },
    /// Recorded only for EMPTY directories (others are implied by file paths).
    Dir {
        path: String,
    },
}

impl Entry {
    pub fn path(&self) -> &str {
        match self {
            Entry::File { path, .. } | Entry::Symlink { path, .. } | Entry::Dir { path } => path,
        }
    }
}

/// The root object of a snapshot. INVARIANT: `entries` sorted ascending by
/// `path()` (byte order), no duplicate paths. `canonical_bytes` is the ONLY
/// legal serialization for digest purposes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub total_size: u64,
    pub entries: Vec<Entry>,
}

impl Manifest {
    /// Canonical wire form: compact JSON (serde_json default, no extra
    /// whitespace), struct fields in declaration order, entries pre-sorted.
    /// The manifest's CAS digest is `Digest::of_bytes(canonical_bytes)`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        // Enforce canonical form in RELEASE (was a `debug_assert!`, stripped in
        // release): a producer that built a non-canonical manifest must fail HERE,
        // before it publishes an unhydratable digest, rather than silently ship it.
        self.validate_canonical()?;
        Ok(serde_json::to_vec(self)?)
    }

    /// Enforce the canonical-form invariant that makes `canonical_bytes` the ONE
    /// legal serialization: entries STRICTLY ascending by path — a single check
    /// that yields both deterministic ordering AND uniqueness — plus every entry
    /// path a safe relative path and every symlink target in-workspace.
    ///
    /// Called by both [`from_bytes`](Self::from_bytes) (reject untrusted /
    /// non-canonical input — this is what defeats the duplicate-path hydrate race,
    /// where two entries sharing a path drove concurrent racing renames) and
    /// [`canonical_bytes`](Self::canonical_bytes) (catch a producer bug before a
    /// bad digest is published).
    fn validate_canonical(&self) -> Result<()> {
        for w in self.entries.windows(2) {
            if w[0].path() >= w[1].path() {
                return Err(ClwError::InvalidManifest(format!(
                    "manifest entries must be strictly ascending by path (unsorted or \
                     duplicate path at '{}')",
                    w[1].path()
                )));
            }
        }
        for entry in &self.entries {
            validate_rel_path(entry.path())?;
            if let Entry::Symlink { target, .. } = entry {
                validate_symlink_target(target)?;
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<Digest> {
        Ok(Digest::of_bytes(&self.canonical_bytes()?))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let m: Manifest = serde_json::from_slice(bytes)?;
        if m.version != MANIFEST_VERSION {
            return Err(ClwError::InvalidManifest(format!(
                "unsupported manifest version {}",
                m.version
            )));
        }
        // Enforce the full canonical invariant before the manifest is trusted by
        // hydrate: STRICTLY ascending unique paths (defeats the duplicate-path
        // racing-rename bug), safe relative paths, and in-workspace symlink targets
        // (defense against traversal payloads).
        m.validate_canonical()?;
        Ok(m)
    }
}

/// Validate that a manifest entry path is a safe RELATIVE path (the documented
/// invariant). Rejects absolute paths, backslashes, any component containing a
/// colon, and any '.'/'..'/empty component — on every platform, for portable
/// hydration.
pub fn validate_rel_path(p: &str) -> Result<()> {
    if p.is_empty() {
        return Err(ClwError::InvalidManifest("empty path".into()));
    }
    if p.starts_with('/') {
        return Err(ClwError::InvalidManifest(format!(
            "absolute path not allowed: {p}"
        )));
    }
    if p.contains('\\') {
        return Err(ClwError::InvalidManifest(format!(
            "backslash not allowed: {p}"
        )));
    }
    for comp in p.split('/') {
        if comp.is_empty() || comp == "." || comp == ".." {
            return Err(ClwError::InvalidManifest(format!(
                "unsafe path component in: {p}"
            )));
        }
        // Reject ANY colon in a component, not just a leading drive-letter (`C:`):
        // on Windows a mid-component colon (`secret.txt:hidden`) names an NTFS
        // Alternate Data Stream, so `File::create` would write into a hidden
        // stream and clobber the visible sibling. `:` is illegal in Windows
        // filenames anyway, so this rule keeps manifests portable on every host.
        if comp.contains(':') {
            return Err(ClwError::InvalidManifest(format!(
                "colon not allowed in path component (Windows drive-letter / NTFS ADS): {p}"
            )));
        }
    }
    Ok(())
}

/// Validate a symlink target stays within the workspace: relative, no '..', no
/// backslash, no Windows drive-letter (rejected on every platform so a tampered
/// manifest cannot escape `dest` on a Windows host).
pub fn validate_symlink_target(t: &str) -> Result<()> {
    if t.is_empty() {
        return Err(ClwError::InvalidManifest("empty symlink target".into()));
    }
    if t.starts_with('/') {
        return Err(ClwError::InvalidManifest(format!(
            "absolute symlink target not allowed: {t}"
        )));
    }
    if t.contains('\\') {
        return Err(ClwError::InvalidManifest(format!(
            "backslash not allowed in symlink target: {t}"
        )));
    }
    for comp in t.split('/') {
        if comp == ".." {
            return Err(ClwError::InvalidManifest(format!(
                "symlink target escapes workspace: {t}"
            )));
        }
        // Reject '.' and empty ('//', trailing '/') components too: a '.' target
        // (or one that normalizes to the parent) makes the Windows copy-fallback
        // recursively copy the link's own parent directory.
        if comp.is_empty() || comp == "." {
            return Err(ClwError::InvalidManifest(format!(
                "unsafe '.'/empty component in symlink target: {t}"
            )));
        }
        if comp.contains(':') {
            return Err(ClwError::InvalidManifest(format!(
                "colon not allowed in symlink target (Windows drive-letter / NTFS ADS): {t}"
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Workspace refs (the mutable namespace over the immutable CAS)
// ---------------------------------------------------------------------------

/// The mutable pointer stored in the AC: workspace name → root manifest.
/// Concurrency model v1: last-write-wins (single-operator tool); `parent`
/// records lineage git-style for history/debugging.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefRecord {
    pub name: String,
    /// CAS digest of the root `Manifest` (canonical bytes).
    pub root: Digest,
    pub manifest_size: u64,
    pub created_at_unix: u64,
    /// Previous root manifest digest, if this updates an existing ref.
    pub parent: Option<Digest>,
    pub tool_version: String,
}

impl RefRecord {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// AC key for a workspace name: BLAKE3(REF_KEY_DOMAIN || name).
pub fn ref_key(name: &str) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(REF_KEY_DOMAIN.as_bytes());
    hasher.update(name.as_bytes());
    Digest(*hasher.finalize().as_bytes())
}

// ---------------------------------------------------------------------------
// Client configuration
// ---------------------------------------------------------------------------

/// Validate a tenant id before it is interpolated RAW into a request URL path
/// (`/v1/cas/{tenant}/...`, `/v1/ac/{tenant}/...`). The tenant is never encoded,
/// so an unvalidated value containing `/`, `..`, whitespace, or control chars
/// enables path injection / cross-tenant misroute (e.g. `../othertenant`).
///
/// Allowed set (closed to path-injection, permissive enough for realistic
/// slug/UUID tenant ids): a NON-EMPTY string of ASCII alphanumerics plus `-`,
/// `_`, and `.`. REJECTS: empty; any `/` or `\`; any whitespace or control
/// char; any other non-allowed byte; a leading or trailing `.`; and the
/// traversal forms `.` and `..`.
pub fn validate_tenant(t: &str) -> Result<()> {
    if t.is_empty() {
        return Err(ClwError::InvalidTenant("tenant must not be empty".into()));
    }
    // `.` and `..` are path-traversal forms; a leading/trailing `.` is rejected
    // by the boundary checks below, but call these out explicitly for a clear msg.
    if t == "." || t == ".." {
        return Err(ClwError::InvalidTenant(format!(
            "tenant '{t}' is a path-traversal form"
        )));
    }
    if t.starts_with('.') || t.ends_with('.') {
        return Err(ClwError::InvalidTenant(format!(
            "tenant '{t}' must not start or end with '.'"
        )));
    }
    for c in t.chars() {
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
            return Err(ClwError::InvalidTenant(format!(
                "tenant '{t}' contains an illegal character {c:?}; allowed: ASCII \
                 alphanumerics and '-', '_', '.'"
            )));
        }
    }
    Ok(())
}

/// Resolved client configuration. Sources (precedence): CLI flags > env
/// (`CLW_ENDPOINT`, `CLW_TENANT`, `CLW_TOKEN`, `CLW_CACHE_DIR`) > `~/.clw/config.toml`.
///
/// `Debug` is hand-written (NOT derived) to redact `token`: the PAT must never
/// appear in logs/diagnostics, mirroring `HttpClient`'s redacting `Debug`.
#[derive(Clone)]
pub struct Config {
    /// e.g. "https://corelink-api.humangr.com" (no trailing slash).
    pub endpoint: String,
    pub tenant: String,
    /// CoreLink PAT, sent as `Authorization: Bearer <token>`. NEVER logged.
    pub token: String,
    /// Local content cache root (default `~/.clw/cache`).
    pub cache_dir: PathBuf,
}

// Manual Debug impl: redact the token (the PAT) while showing the other fields
// normally. The derived Debug would print the plaintext token, contradicting the
// field's own "NEVER logged" guarantee — mirror HttpClient's redacting Debug.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("endpoint", &self.endpoint)
            .field("tenant", &self.tenant)
            .field("token", &"<redacted>")
            .field("cache_dir", &self.cache_dir)
            .finish()
    }
}

impl Config {
    pub fn cas_url(&self, digest: &Digest) -> String {
        format!(
            "{}/v1/cas/{}/{}",
            self.endpoint.trim_end_matches('/'),
            self.tenant,
            digest
        )
    }

    pub fn ac_url(&self, key: &Digest) -> String {
        format!(
            "{}/v1/ac/{}/{}",
            self.endpoint.trim_end_matches('/'),
            self.tenant,
            key
        )
    }
}

// ---------------------------------------------------------------------------
// Transport traits (frozen seams between client / snapshot / hydrate / run)
// ---------------------------------------------------------------------------

/// Content-addressed blob transport (CoreLink `/v1/cas`).
/// Implementations MUST verify on `get` that the returned bytes hash to
/// `digest` and return `ClwError::Integrity` otherwise.
#[async_trait::async_trait]
pub trait CasTransport: Send + Sync {
    async fn exists(&self, digest: &Digest) -> Result<bool>;
    async fn get(&self, digest: &Digest) -> Result<Vec<u8>>;
    /// `bytes.len()` MUST be ≤ `CAS_BLOB_CAP_BYTES` (else `ClwError::TooLarge`).
    async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()>;
    /// Delete a blob by its hash. Idempotent: deleting an absent blob returns
    /// Ok(()). Unblocks `clw prune` (CAS mark-and-sweep GC) and the GDPR Art.17
    /// erasure path. (corelink-server DELETE /v1/cas/{tenant}/{hash} is LIVE —
    /// D-8, conformance-green; see docs/RELAY-server-D1-D7-D8.md. 405|501 surfaces
    /// a clear server-regression error.)
    async fn delete(&self, digest: &Digest) -> Result<()>;
    /// List the tenant's CAS blob hashes (the live blob set for prune's
    /// mark-and-sweep). Implementations paginate internally and return a flat
    /// `Vec`. (corelink-server GET /v1/cas/{tenant} is LIVE — D-8, conformance-
    /// green; see docs/RELAY-server-D1-D7-D8.md. 405|501 surfaces a clear
    /// server-regression error.)
    async fn list_blobs(&self) -> Result<Vec<Digest>>;
}

/// Mutable key→value transport (CoreLink `/v1/ac`). Values are opaque bytes.
#[async_trait::async_trait]
pub trait AcTransport: Send + Sync {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>>;
    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()>;
    /// Delete a ref by its key. Idempotent: deleting an absent key returns Ok(()).
    /// (corelink-server DELETE /v1/ac/{tenant}/{key} is LIVE — D-1, conformance-
    /// green; see docs/RELAY-server-D1-D7-D8.md. 405|501 surfaces a clear
    /// server-regression error.)
    async fn delete(&self, key: &Digest) -> Result<()>;
    /// List the tenant's ref keys. `ref_key = BLAKE3(domain‖name)` is one-way and
    /// the client keeps no name→key index, so enumeration must come from the
    /// server. Implementations paginate internally and return a flat `Vec`.
    /// Unblocks `clw ls --all` + the live-root-set for prune + GDPR erasure.
    /// (corelink-server GET /v1/ac/{tenant} is LIVE — D-7, conformance-green; see
    /// docs/RELAY-server-D1-D7-D8.md. 405|501 surfaces a clear server-regression
    /// error.)
    async fn list_refs(&self) -> Result<Vec<Digest>>;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(thiserror::Error, Debug)]
pub enum ClwError {
    #[error("authentication failed (401) — check CLW_TOKEN / config PAT")]
    Auth,
    #[error("not found in CAS: {0}")]
    NotFound(Digest),
    #[error("blob of {size} bytes exceeds the CAS cap of {cap} bytes")]
    TooLarge { size: u64, cap: u64 },
    #[error("HTTP {status} from {context}")]
    Http { status: u16, context: String },
    #[error("integrity violation: expected {expected}, got {actual}")]
    Integrity { expected: Digest, actual: Digest },
    #[error("invalid digest: {0}")]
    InvalidDigest(String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid tenant: {0}")]
    InvalidTenant(String),
    /// A CONFIGURATION problem (e.g. a missing tenant/token) — distinct from a
    /// network/transport failure. Carries an actionable message naming the
    /// flag/env/config key the user should set.
    #[error("configuration error: {0}")]
    Config(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ClwError>;

// ---------------------------------------------------------------------------
// Contract self-tests (the seams the WPs rely on)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_hex_roundtrip() {
        let d = Digest::of_bytes(b"corelink");
        assert_eq!(Digest::from_hex(&d.to_hex()).unwrap(), d);
        assert_eq!(d.to_hex().len(), 64);
    }

    #[test]
    fn manifest_canonical_digest_is_stable() {
        let m = Manifest {
            version: MANIFEST_VERSION,
            total_size: 3,
            entries: vec![Entry::File {
                path: "a.txt".into(),
                mode: 0o644,
                size: 3,
                chunks: vec![ChunkRef {
                    digest: Digest::of_bytes(b"abc"),
                    size: 3,
                }],
            }],
        };
        let d1 = m.digest().unwrap();
        let d2 = Manifest::from_bytes(&m.canonical_bytes().unwrap())
            .unwrap()
            .digest()
            .unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn ref_key_is_domain_separated() {
        assert_ne!(ref_key("x"), Digest::of_bytes(b"x"));
        assert_ne!(ref_key("a"), ref_key("b"));
        assert_eq!(ref_key("a"), ref_key("a"));
    }

    #[test]
    fn from_bytes_rejects_parent_traversal_path() {
        // Hand-written JSON: a file entry whose path escapes via "..".
        let bytes = br#"{"version":1,"total_size":0,"entries":[{"kind":"file","path":"../escape.txt","mode":420,"size":0,"chunks":[]}]}"#;
        let err = Manifest::from_bytes(bytes).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(_)),
            "expected InvalidManifest, got: {err}"
        );
    }

    #[test]
    fn from_bytes_rejects_absolute_path() {
        let bytes = br#"{"version":1,"total_size":0,"entries":[{"kind":"file","path":"/etc/passwd","mode":420,"size":0,"chunks":[]}]}"#;
        let err = Manifest::from_bytes(bytes).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(_)),
            "expected InvalidManifest, got: {err}"
        );
    }

    #[test]
    fn from_bytes_rejects_absolute_symlink_target() {
        let bytes = br#"{"version":1,"total_size":0,"entries":[{"kind":"symlink","path":"link","target":"/etc/passwd"}]}"#;
        let err = Manifest::from_bytes(bytes).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(_)),
            "expected InvalidManifest, got: {err}"
        );
    }

    #[test]
    fn from_bytes_rejects_traversal_symlink_target() {
        let bytes = br#"{"version":1,"total_size":0,"entries":[{"kind":"symlink","path":"link","target":"../../x"}]}"#;
        let err = Manifest::from_bytes(bytes).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(_)),
            "expected InvalidManifest, got: {err}"
        );
    }

    #[test]
    fn from_bytes_accepts_normal_manifest() {
        // A clean manifest (sorted by path) must round-trip through from_bytes.
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0,
            entries: vec![
                Entry::File {
                    path: "a.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
                Entry::File {
                    path: "dir/b.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
                Entry::Symlink {
                    path: "link.txt".into(),
                    target: "a.txt".into(),
                },
            ],
        };
        let bytes = manifest.canonical_bytes().unwrap();
        let parsed = Manifest::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, manifest);
    }

    // A duplicate-path manifest must be REJECTED by from_bytes (HIGH): two entries
    // sharing a path drove concurrent racing renames in hydrate (nondeterministic
    // output). Serialize a hand-built duplicate directly (bypassing canonical_bytes,
    // which would also reject it) to prove the PARSE side is guarded.
    #[test]
    fn from_bytes_rejects_duplicate_path() {
        let dup = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0,
            entries: vec![
                Entry::File {
                    path: "a.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
                Entry::File {
                    path: "a.txt".into(),
                    mode: 0o644,
                    size: 1,
                    chunks: vec![],
                },
            ],
        };
        let bytes = serde_json::to_vec(&dup).unwrap();
        assert!(
            Manifest::from_bytes(&bytes).is_err(),
            "a manifest with a duplicate path must be rejected"
        );
    }

    // An unsorted manifest must be REJECTED by from_bytes (MED): canonical_bytes is
    // the ONE legal serialization, so non-canonical input is untrusted.
    #[test]
    fn from_bytes_rejects_unsorted_entries() {
        let unsorted = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0,
            entries: vec![
                Entry::File {
                    path: "b.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
                Entry::File {
                    path: "a.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
            ],
        };
        let bytes = serde_json::to_vec(&unsorted).unwrap();
        assert!(
            Manifest::from_bytes(&bytes).is_err(),
            "unsorted entries must be rejected"
        );
    }

    // canonical_bytes now ENFORCES the invariant in release (was a debug_assert):
    // a producer that built a non-canonical manifest fails HERE, before publishing
    // an unhydratable digest.
    #[test]
    fn canonical_bytes_rejects_noncanonical_producer_manifest() {
        let bad = Manifest {
            version: MANIFEST_VERSION,
            total_size: 0,
            entries: vec![
                Entry::File {
                    path: "z.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
                Entry::File {
                    path: "a.txt".into(),
                    mode: 0o644,
                    size: 0,
                    chunks: vec![],
                },
            ],
        };
        assert!(
            bad.canonical_bytes().is_err(),
            "canonical_bytes must reject a non-canonical (unsorted) manifest in release"
        );
    }

    #[test]
    fn validate_rel_path_rejects_unsafe_and_accepts_clean() {
        assert!(validate_rel_path("a.txt").is_ok());
        assert!(validate_rel_path("dir/b.txt").is_ok());
        assert!(validate_rel_path("").is_err());
        assert!(validate_rel_path("/abs").is_err());
        assert!(validate_rel_path("../up").is_err());
        assert!(validate_rel_path("a/../b").is_err());
        assert!(validate_rel_path("a/./b").is_err());
        assert!(validate_rel_path("a//b").is_err());
        assert!(validate_rel_path("a\\b").is_err());
        // Windows drive-letter escapes (rejected on every platform).
        assert!(validate_rel_path("C:foo").is_err());
        assert!(validate_rel_path("C:/evil").is_err());
        assert!(validate_rel_path("dir/D:bar").is_err());
        // ANY colon is rejected: a mid-component colon names an NTFS Alternate
        // Data Stream on Windows (`secret.txt:hidden` → hidden stream + sibling
        // clobber), so it is refused on every platform for portable hydration.
        assert!(validate_rel_path("foo:bar").is_err());
        assert!(validate_rel_path("secret.txt:hidden").is_err());
        assert!(validate_rel_path("dir/file.txt:$DATA").is_err());
    }

    #[test]
    fn validate_tenant_accepts_safe_and_rejects_injection() {
        // Realistic slug / UUID tenant ids are accepted.
        assert!(validate_tenant("acme").is_ok());
        assert!(validate_tenant("acme-prod").is_ok());
        assert!(validate_tenant("t_1").is_ok());
        assert!(validate_tenant("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_tenant("a.b").is_ok());
        // Path-injection / misroute vectors are rejected.
        assert!(validate_tenant("").is_err());
        assert!(validate_tenant("a/b").is_err());
        assert!(validate_tenant("../x").is_err());
        assert!(validate_tenant("a\\b").is_err());
        assert!(validate_tenant("a b").is_err());
        assert!(validate_tenant(".").is_err());
        assert!(validate_tenant("..").is_err());
        assert!(validate_tenant(".hidden").is_err());
        assert!(validate_tenant("trail.").is_err());
    }

    #[test]
    fn config_debug_redacts_token() {
        let cfg = Config {
            endpoint: "https://corelink-api.humangr.com".into(),
            tenant: "acme".into(),
            token: "super-secret-pat-value".into(),
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let dbg = format!("{cfg:?}");
        assert!(
            !dbg.contains("super-secret-pat-value"),
            "Config Debug must NOT contain the token value, got: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "Config Debug must contain '<redacted>', got: {dbg}"
        );
        // The other fields are still shown normally.
        assert!(dbg.contains("acme"));
        assert!(dbg.contains("corelink-api.humangr.com"));
    }

    #[test]
    fn validate_symlink_target_rejects_escape_and_accepts_clean() {
        assert!(validate_symlink_target("a.txt").is_ok());
        assert!(validate_symlink_target("dir/a.txt").is_ok());
        assert!(validate_symlink_target("").is_err());
        assert!(validate_symlink_target("/etc/passwd").is_err());
        assert!(validate_symlink_target("../../x").is_err());
        // Windows-syntax escapes (rejected on every platform).
        assert!(validate_symlink_target("..\\..\\x").is_err());
        assert!(validate_symlink_target("C:\\evil").is_err());
        assert!(validate_symlink_target("C:/evil").is_err());
        // NTFS ADS colon rejected on every platform.
        assert!(validate_symlink_target("a.txt:hidden").is_err());
        // '.' and empty/'//' components rejected (LOW): a '.' target makes the
        // Windows copy-fallback recursively copy the link's own parent dir.
        assert!(validate_symlink_target(".").is_err());
        assert!(validate_symlink_target("a/./b").is_err());
        assert!(validate_symlink_target("a//b").is_err());
        assert!(validate_symlink_target("dir/").is_err());
    }
}
