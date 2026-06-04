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
        /// Unix permission bits (e.g. 0o644, 0o755). Windows is out of scope for v1.
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
        debug_assert!(
            self.entries.windows(2).all(|w| w[0].path() < w[1].path()),
            "manifest entries must be sorted by path with no duplicates"
        );
        Ok(serde_json::to_vec(self)?)
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
        Ok(m)
    }
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

/// Resolved client configuration. Sources (precedence): CLI flags > env
/// (`CLW_ENDPOINT`, `CLW_TENANT`, `CLW_TOKEN`, `CLW_CACHE_DIR`) > `~/.clw/config.toml`.
#[derive(Clone, Debug)]
pub struct Config {
    /// e.g. "https://corelink-api.humangr.com" (no trailing slash).
    pub endpoint: String,
    pub tenant: String,
    /// CoreLink PAT, sent as `Authorization: Bearer <token>`. NEVER logged.
    pub token: String,
    /// Local content cache root (default `~/.clw/cache`).
    pub cache_dir: PathBuf,
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
}

/// Mutable key→value transport (CoreLink `/v1/ac`). Values are opaque bytes.
#[async_trait::async_trait]
pub trait AcTransport: Send + Sync {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>>;
    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()>;
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
    fn chunk_params_fit_under_cas_cap() {
        assert!(CHUNK_MAX_BYTES < CAS_BLOB_CAP_BYTES);
    }
}
