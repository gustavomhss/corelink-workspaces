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

/// Domain separator for RUNNER-produced workspace ref keys. Disjoint from
/// [`REF_KEY_DOMAIN`] **by construction**: [`ref_key_in`] folds the domain into
/// the BLAKE3 pre-image, so no name in the user keyspace can ever collide with a
/// name in the runner keyspace. A client without `CLW_REF_DOMAIN=runner` can
/// never produce a runner-domain key, for any name. Frozen contract with the
/// corelink-runners TL (2026-06-16).
pub const REF_KEY_DOMAIN_RUNNER: &str = "clw/ref/runner/v1/";

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
            if let Entry::Symlink { path, target } = entry {
                // Consumer-side validation MUST match the producer (clw-manifest):
                // allow `..` that stays inside the workspace, reject only escapes.
                // Otherwise a manifest the producer accepts would fail to hydrate.
                validate_symlink_target_contained(path, target)?;
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

/// Validate a symlink target while ALLOWING `..` traversal that stays INSIDE the
/// workspace. `link_rel` is the link's own path relative to the workspace root
/// (e.g. `bin/main-alias.rs`); `t` is the (already `/`-normalized) target. The
/// target is resolved LEXICALLY (no filesystem access) against the link's parent
/// directory and rejected only if it would ESCAPE the workspace root, is absolute,
/// or uses characters no client can represent (backslash, colon/NTFS-ADS).
///
/// This is the producer-side check the v0.1.1 power-user test motivated: the strict
/// [`validate_symlink_target`] rejects ANY `..`, which breaks real projects whose
/// relative symlinks (e.g. `bin/x -> ../src/x`) resolve harmlessly within the tree.
///
/// PRECONDITION: `link_rel` MUST already be a clean workspace-relative path (no `..`,
/// `.`, empty, or absolute components). Every caller runs [`validate_rel_path`] on the
/// link path first, so this holds at all real call sites; the `debug_assert!` below
/// pins it. (The seed is the link's parent dir taken from a clean `link_rel`; it does
/// NOT lexically resolve a `..` inside `link_rel`, so an unclean `link_rel` would be
/// modeled wrong — hence the precondition rather than silent handling.)
pub fn validate_symlink_target_contained(link_rel: &str, t: &str) -> Result<()> {
    debug_assert!(
        !link_rel.split('/').any(|c| c == ".." || c == "."),
        "validate_symlink_target_contained requires a validate_rel_path-clean link_rel, got: {link_rel}"
    );
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
    // Seed the resolution stack with the link's PARENT directory components (the
    // dir the relative target is interpreted from). `link_rel` is workspace-root-
    // relative and already validated by `validate_rel_path` (precondition above),
    // so its only droppable components are the trailing-slash/`.` artifacts.
    let mut depth: Vec<&str> = link_rel.split('/').collect();
    depth.pop(); // drop the link's own filename → its parent dir
    depth.retain(|c| !c.is_empty() && *c != ".");
    for comp in t.split('/') {
        if comp.contains(':') {
            return Err(ClwError::InvalidManifest(format!(
                "colon not allowed in symlink target (Windows drive-letter / NTFS ADS): {t}"
            )));
        }
        match comp {
            // Reject empty ('//', trailing '/') components — same as the strict check.
            "" => {
                return Err(ClwError::InvalidManifest(format!(
                    "empty component in symlink target: {t}"
                )));
            }
            "." => { /* current dir — a harmless no-op in the middle of a path */ }
            ".." => {
                if depth.pop().is_none() {
                    // Popped above the workspace root → escapes.
                    return Err(ClwError::InvalidManifest(format!(
                        "symlink target escapes workspace: {t}"
                    )));
                }
            }
            normal => depth.push(normal),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Workspace refs (the mutable namespace over the immutable CAS)
// ---------------------------------------------------------------------------

/// The mutable pointer stored in the AC: workspace name → root manifest.
/// Concurrency model v1: atomic put-if-absent (201 new / 200 idempotent on an
/// identical value / 409 on a divergent value) — not last-write-wins, and there
/// is no CAS token on the wire; `parent` records lineage git-style for
/// history/debugging and is never read on the write path.
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

/// Which ref keyspace a workspace name resolves into. `User` (the default) and
/// `Runner` are **disjoint** keyspaces by construction — see
/// [`REF_KEY_DOMAIN_RUNNER`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefDomain {
    /// User-facing refs — the default for every interactive `clw` invocation.
    #[default]
    User,
    /// Runner-produced refs — selected by `CLW_REF_DOMAIN=runner`.
    Runner,
}

impl RefDomain {
    /// The BLAKE3 domain separator string for this keyspace.
    #[must_use]
    pub fn separator(self) -> &'static str {
        match self {
            RefDomain::User => REF_KEY_DOMAIN,
            RefDomain::Runner => REF_KEY_DOMAIN_RUNNER,
        }
    }
}

/// AC key for a workspace name in the default (user) domain:
/// BLAKE3(REF_KEY_DOMAIN || name).
pub fn ref_key(name: &str) -> Digest {
    ref_key_in(RefDomain::User, name)
}

/// AC key for a workspace name in an explicit ref [`RefDomain`]:
/// BLAKE3(domain.separator() || name). Folding the domain into the pre-image is
/// what makes the user and runner keyspaces disjoint by construction.
pub fn ref_key_in(domain: RefDomain, name: &str) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.separator().as_bytes());
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
    /// A pre-formatted, actionable, user-facing message rendered VERBATIM (no
    /// prefix). For application-level conditions a subcommand has already turned
    /// into a clear message — auth hints, "workspace not found", "destination not
    /// empty", usage errors — that are NOT transport failures and must not be
    /// mislabeled `transport error:` (the v0.1.1 power-user test caught that).
    #[error("{0}")]
    Message(String),
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
    fn ref_domains_are_disjoint_keyspaces() {
        // Same name, different domain => different key: collision-proof by
        // construction (a user can never reach the runner keyspace, any name).
        assert_ne!(
            ref_key_in(RefDomain::User, "ws"),
            ref_key_in(RefDomain::Runner, "ws")
        );
        // `ref_key(name)` is exactly the user domain (back-compat).
        assert_eq!(ref_key("ws"), ref_key_in(RefDomain::User, "ws"));
        // Stable + injective within the runner domain.
        assert_eq!(
            ref_key_in(RefDomain::Runner, "a"),
            ref_key_in(RefDomain::Runner, "a")
        );
        assert_ne!(
            ref_key_in(RefDomain::Runner, "a"),
            ref_key_in(RefDomain::Runner, "b")
        );
        // The frozen contract strings + default.
        assert_eq!(RefDomain::Runner.separator(), "clw/ref/runner/v1/");
        assert_eq!(RefDomain::User.separator(), "clw/ref/v1/");
        assert_eq!(RefDomain::default(), RefDomain::User);
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
        // `../../x` from `link` (at the root) escapes the workspace → rejected.
        let bytes = br#"{"version":1,"total_size":0,"entries":[{"kind":"symlink","path":"link","target":"../../x"}]}"#;
        let err = Manifest::from_bytes(bytes).unwrap_err();
        assert!(
            matches!(err, ClwError::InvalidManifest(_)),
            "expected InvalidManifest, got: {err}"
        );
    }

    #[test]
    fn from_bytes_accepts_intra_workspace_dotdot_symlink() {
        // `bin/main-alias.rs -> ../src/main.rs` resolves to `src/main.rs` INSIDE the
        // tree — the consumer must accept it (matches the producer; v0.1.1 finding).
        let bytes = br#"{"version":1,"total_size":0,"entries":[{"kind":"symlink","path":"bin/main-alias.rs","target":"../src/main.rs"}]}"#;
        assert!(
            Manifest::from_bytes(bytes).is_ok(),
            "intra-workspace `..` symlink must hydrate"
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

    // -----------------------------------------------------------------------
    // PART A — proptest fuzz-robustness (runs in the ordinary gate).
    //
    // The invariant for every parser: on ARBITRARY input it returns Ok or Err —
    // it NEVER panics, hangs, or OOMs. A panic = test failure (non-vacuous by
    // construction). Modest case counts keep the gate fast.
    // -----------------------------------------------------------------------
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        // Case count for the in-crate fuzzers. The deep run (50k) is OPT-IN via
        // the repo's existing fuzz lane signal (CLW_FUZZ_GATE=1, see
        // scripts/gates.sh); the default keeps the per-PR gate fast while still
        // exercising a meaningful 1k cases.
        fn fuzz_cases() -> u32 {
            if std::env::var("CLW_FUZZ_GATE").is_ok() {
                50_000
            } else {
                1_000
            }
        }

        // A JSON value that LOOKS like a manifest (right keys, plausible shapes)
        // so the fuzzer reaches serde's structural decode + validate_canonical,
        // not just the first-byte JSON rejection. Mixes valid-ish and hostile
        // fields (bad versions, traversal paths, escaping symlink targets).
        fn manifestish_json() -> impl Strategy<Value = Vec<u8>> {
            // A hostile-ish path/target value: mixes clean-looking strings with
            // traversal/absolute/ADS payloads so validate_canonical is exercised.
            let pathish = prop_oneof![
                "[a-zA-Z0-9_/.-]{0,12}",
                Just("../escape".to_string()),
                Just("/abs".to_string()),
                Just("a/../b".to_string()),
                Just("c:\\ads".to_string()),
            ];
            // One arbitrary entry as a serde_json::Value (kind chosen at random,
            // hostile path/target). prop_flat_map picks the kind AFTER the fields
            // are drawn so each kind is reachable.
            let entry = (pathish.clone(), any::<u32>(), any::<u64>(), pathish, 0u8..3u8)
                .prop_map(|(p, mode, size, target, kind)| match kind {
                    0 => serde_json::json!({"kind":"file","path":p,"mode":mode,"size":size,"chunks":[]}),
                    1 => serde_json::json!({"kind":"symlink","path":p,"target":target}),
                    _ => serde_json::json!({"kind":"dir","path":p}),
                });
            // Bias `version` toward the ONLY accepted value (MANIFEST_VERSION):
            // `from_bytes` rejects any other version at the version-gate BEFORE
            // `validate_canonical` runs, so an unbiased `any::<u32>()` would make
            // the parses-Ok branch (and its canonical invariant) effectively
            // unreachable. Keep arbitrary versions in the mix to still fuzz the
            // version-reject path.
            let verish = prop_oneof![
                9 => Just(MANIFEST_VERSION),
                1 => any::<u32>(),
            ];
            (verish, any::<u64>(), proptest::collection::vec(entry, 0..6)).prop_map(
                |(ver, total, entries)| {
                    serde_json::to_vec(&serde_json::json!({
                        "version": ver, "total_size": total, "entries": entries,
                    }))
                    .unwrap()
                },
            )
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]

            /// `Manifest::from_bytes` on ARBITRARY bytes never panics — only
            /// `Ok`/`Err`. Covers the serde_json + validate_canonical parse path
            /// against hostile/garbage blobs.
            #[test]
            fn manifest_from_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
                let _ = Manifest::from_bytes(&bytes);
            }

            /// `Manifest::from_bytes` on STRUCTURED manifest-shaped JSON (right
            /// keys, hostile path/target values) never panics — reaches serde's
            /// field decode + validate_canonical, not just the JSON-syntax reject.
            /// INVARIANT: any manifest that parses Ok has strictly-ascending paths
            /// and only safe/contained entries (validate_canonical held).
            #[test]
            fn manifest_from_bytes_structured_never_panics(bytes in manifestish_json()) {
                if let Ok(m) = Manifest::from_bytes(&bytes) {
                    prop_assert!(m.validate_canonical().is_ok());
                    for w in m.entries.windows(2) {
                        prop_assert!(w[0].path() < w[1].path());
                    }
                }
            }

            /// `RefRecord::from_bytes` (clw-types:393) on ARBITRARY bytes never
            /// panics — only `Ok`/`Err`.
            #[test]
            fn refrecord_from_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
                let _ = RefRecord::from_bytes(&bytes);
            }

            /// `Digest::from_hex` on an ARBITRARY string never panics — only
            /// `Ok`/`Err`. INVARIANT: a successful parse round-trips back to the
            /// 64-char lowercase-hex canonical form.
            #[test]
            fn digest_from_hex_never_panics(s in ".*") {
                if let Ok(d) = Digest::from_hex(&s) {
                    prop_assert_eq!(d.to_hex().len(), 64);
                    prop_assert_eq!(Digest::from_hex(&d.to_hex()).unwrap(), d);
                }
            }

            /// `Digest::from_hex` on hex-ALPHABET strings of every length never
            /// panics; INVARIANT: Ok IFF the input is exactly 64 hex chars.
            #[test]
            fn digest_from_hex_len_invariant(s in "[0-9a-fA-F]{0,80}") {
                let ok = Digest::from_hex(&s).is_ok();
                prop_assert_eq!(ok, s.len() == 64);
            }

            /// `validate_rel_path` on ARBITRARY strings never panics — only
            /// Ok/Err. INVARIANT: an accepted path has no '.'/'..'/empty/colon
            /// component, no leading '/', and no backslash.
            #[test]
            fn validate_rel_path_never_panics(p in ".*") {
                if validate_rel_path(&p).is_ok() {
                    prop_assert!(!p.is_empty());
                    prop_assert!(!p.starts_with('/'));
                    prop_assert!(!p.contains('\\'));
                    for c in p.split('/') {
                        prop_assert!(!c.is_empty() && c != "." && c != ".." && !c.contains(':'));
                    }
                }
            }

            /// `validate_symlink_target` (strict) on ARBITRARY strings never
            /// panics. INVARIANT: an accepted target is relative, has no '..',
            /// no backslash, no empty/'.'/colon component.
            #[test]
            fn validate_symlink_target_never_panics(t in ".*") {
                if validate_symlink_target(&t).is_ok() {
                    prop_assert!(!t.is_empty());
                    prop_assert!(!t.starts_with('/'));
                    prop_assert!(!t.contains('\\'));
                    for c in t.split('/') {
                        prop_assert!(!c.is_empty() && c != "." && c != ".." && !c.contains(':'));
                    }
                }
            }

            /// `validate_symlink_target_contained` on a CLEAN link_rel (precondition)
            /// + ARBITRARY target never panics. INVARIANT: an accepted target,
            /// resolved lexically against the link's parent dir, never escapes the
            /// workspace root (the pop-below-root depth check held).
            #[test]
            fn validate_symlink_target_contained_never_panics(
                link in safe_rel_path(),
                t in ".*",
            ) {
                if validate_symlink_target_contained(&link, &t).is_ok() {
                    // Re-derive the lexical resolution and assert non-escape.
                    let mut depth: i64 = link.split('/').filter(|c| !c.is_empty() && *c != ".").count() as i64 - 1;
                    let mut escaped = false;
                    for c in t.split('/') {
                        match c {
                            "" | "." => {}
                            ".." => { depth -= 1; if depth < 0 { escaped = true; } }
                            _ => depth += 1,
                        }
                    }
                    prop_assert!(!escaped);
                }
            }

            /// `validate_tenant` on ARBITRARY strings never panics. INVARIANT: an
            /// accepted tenant is non-empty, contains only [A-Za-z0-9._-], is not
            /// '.'/'..', and does not start or end with '.' — i.e. it cannot carry
            /// a '/' path-injection payload into the request URL.
            #[test]
            fn validate_tenant_never_panics(t in ".*") {
                if validate_tenant(&t).is_ok() {
                    prop_assert!(!t.is_empty());
                    prop_assert!(t != "." && t != "..");
                    prop_assert!(!t.starts_with('.') && !t.ends_with('.'));
                    prop_assert!(!t.contains('/') && !t.contains('\\'));
                    for c in t.chars() {
                        prop_assert!(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
                    }
                }
            }

            /// Path-injection corpus: NO tenant containing a separator or traversal
            /// form is ever accepted (the cross-tenant-misroute defense).
            #[test]
            fn validate_tenant_rejects_injection(
                t in prop_oneof![
                    "[A-Za-z0-9._/-]{0,16}",
                    Just("../other".to_string()),
                    Just("a/b".to_string()),
                    Just("..".to_string()),
                    Just(".".to_string()),
                    Just("a\\b".to_string()),
                ]
            ) {
                if t.contains('/') || t.contains('\\') || t == "." || t == ".." {
                    prop_assert!(validate_tenant(&t).is_err());
                }
            }
        }

        // Strategy: a SAFE relative path (the canonical-manifest invariant) — one
        // or more '/'-joined components drawn from a closed alphabet that excludes
        // '/', '.', ':', '\\' and empties, so validate_rel_path always accepts it.
        // This keeps the round-trip property NON-VACUOUS: canonical_bytes succeeds
        // for these, so the property actually exercises the serialize→parse path.
        fn safe_component() -> impl Strategy<Value = String> {
            "[a-zA-Z0-9_-]{1,8}"
        }
        fn safe_rel_path() -> impl Strategy<Value = String> {
            proptest::collection::vec(safe_component(), 1..4).prop_map(|c| c.join("/"))
        }

        // Strategy: an arbitrary Entry with a safe path/target (so the canonical
        // round-trip is exercised, not short-circuited by a validation Err).
        fn arb_entry() -> impl Strategy<Value = Entry> {
            prop_oneof![
                (safe_rel_path(), any::<u32>(), any::<u64>()).prop_map(|(path, mode, size)| {
                    Entry::File {
                        path,
                        mode,
                        size,
                        chunks: vec![],
                    }
                }),
                (safe_rel_path(), safe_rel_path())
                    .prop_map(|(path, target)| Entry::Symlink { path, target }),
                safe_rel_path().prop_map(|path| Entry::Dir { path }),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]

            /// Canonical round-trip: build an ARBITRARY `Manifest`, sort+dedup its
            /// entries by path (the canonical-form precondition), then assert
            /// `from_bytes(canonical_bytes())` round-trips to an EQUAL manifest.
            /// Proves canonical_bytes is the one legal serialization the parser
            /// accepts and reproduces exactly.
            #[test]
            fn manifest_canonical_roundtrip(
                mut entries in proptest::collection::vec(arb_entry(), 0..16),
                total_size in any::<u64>(),
            ) {
                // Canonical precondition: entries STRICTLY ascending by path. Sort,
                // then dedup duplicate paths (canonical_bytes rejects both unsorted
                // AND duplicate paths), so canonical_bytes does not error first.
                entries.sort_by(|a, b| a.path().cmp(b.path()));
                entries.dedup_by(|a, b| a.path() == b.path());

                let manifest = Manifest {
                    version: MANIFEST_VERSION,
                    total_size,
                    entries,
                };
                // With safe paths + sorted/deduped entries, canonical_bytes succeeds.
                let bytes = manifest
                    .canonical_bytes()
                    .expect("sorted/deduped safe-path manifest must canonicalize");
                let parsed = Manifest::from_bytes(&bytes)
                    .expect("canonical bytes must round-trip back through from_bytes");
                prop_assert_eq!(parsed, manifest);
            }
        }
    }

    #[test]
    fn message_variant_renders_verbatim_without_transport_prefix() {
        // v0.1.1 power-user finding: auth / usage / not-found errors were rendered
        // as "transport error: ..." (mislabel). The Message variant renders VERBATIM.
        let e = ClwError::Message("specify exactly one of --name <NAME> or --all".to_string());
        assert_eq!(
            e.to_string(),
            "specify exactly one of --name <NAME> or --all"
        );
        assert!(
            !e.to_string().starts_with("transport error"),
            "Message must not be labeled 'transport error', got: {e}"
        );
        // Transport keeps its prefix (genuine transport failures are still labeled).
        assert!(ClwError::Transport("blip".into())
            .to_string()
            .starts_with("transport error"));
    }

    #[test]
    fn validate_symlink_target_contained_allows_intra_workspace_dotdot() {
        // v0.1.1 power-user finding: `bin/x -> ../src/x` resolves INSIDE the tree
        // and must be ACCEPTED (the strict validator rejected all `..`).
        assert!(validate_symlink_target_contained("bin/main-alias.rs", "../src/main.rs").is_ok());
        assert!(validate_symlink_target_contained("a/b/link", "../../c").is_ok()); // a/b -> a -> root -> c
        assert!(validate_symlink_target_contained("src/alias.rs", "main.rs").is_ok()); // same dir
        assert!(validate_symlink_target_contained("deep/x", "./sib").is_ok());
        // ESCAPES the workspace root → still rejected.
        assert!(validate_symlink_target_contained("link", "../x").is_err()); // root -> escape
        assert!(validate_symlink_target_contained("bin/link", "../../../etc/passwd").is_err());
        // Absolute / backslash / colon / empty → still rejected.
        assert!(validate_symlink_target_contained("bin/link", "/etc/passwd").is_err());
        assert!(validate_symlink_target_contained("bin/link", "..\\x").is_err());
        assert!(validate_symlink_target_contained("bin/link", "a//b").is_err());
        assert!(validate_symlink_target_contained("bin/link", "").is_err());
        // A target may climb to the root and back down, but not one level past it.
        assert!(validate_symlink_target_contained("a/b/link", "../../sib/x").is_ok()); // a/b -> root -> sib/x
        assert!(validate_symlink_target_contained("a/b/link", "../../../x").is_err());
        // one past root
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
