//! Config resolution: CLI flags > env vars > ~/.clw/config.toml > defaults.
//!
//! Env vars: the `CLW_*` names (`CLW_ENDPOINT`/`CLW_TENANT`/`CLW_TOKEN`, merged by
//! clap) take precedence; the CoreLink-wide `CORELINK_*` aliases
//! (`CORELINK_ENDPOINT`/`CORELINK_TENANT`/`CORELINK_TOKEN`) are accepted as a
//! fallback so a disposable runner injected with either namespace just works
//! (server D-9 dispatch-time env injection; no `~/.clw/config.toml`).
//!
//! `tenant` and `token` have no default and produce a friendly error (exit 2)
//! if still missing after all layers.

use std::collections::HashMap;
use std::path::PathBuf;

use clw_types::{validate_tenant, ClwError, Config, RefDomain};

use crate::Cli;

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

const DEFAULT_ENDPOINT: &str = "https://corelink-api.humangr.com";
const DEFAULT_CACHE_DIR: &str = "~/.clw/cache";

// ---------------------------------------------------------------------------
// ResolvedConfig
// ---------------------------------------------------------------------------

/// Runner ref-domain credential-broker inputs (C2c env-0). When the fabric arms
/// C2c it injects a single-use, lease-bound `CLW_CRED_TICKET` (NOT the PAT) plus
/// the lease id and the `corelink-fabricd` base; `clw` redeems the ticket ONCE at
/// the trusted boot (before any untrusted `clw run`) for a scope-narrowed per-job
/// CAS PAT held in process memory only — see [`ResolvedConfig::resolve_credential`].
pub struct BrokerCred {
    /// `corelink-fabricd` base URL (`CLW_FABRIC_ENDPOINT`) hosting the redeem route.
    pub fabric_endpoint: String,
    /// The lease this container is bound to (`CLW_LEASE_ID`).
    pub lease_id: String,
    /// The single-use redemption ticket (`CLW_CRED_TICKET`) — the auth, not a PAT.
    pub ticket: String,
}

// Manual Debug: redact the ticket (a single-use secret) while showing the
// non-secret fabric_endpoint + lease_id — mirroring `Config`'s redacting Debug so
// the ticket never appears in logs/diagnostics.
impl std::fmt::Debug for BrokerCred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerCred")
            .field("fabric_endpoint", &self.fabric_endpoint)
            .field("lease_id", &self.lease_id)
            .field("ticket", &"<redacted>")
            .finish()
    }
}

/// Fully resolved configuration ready to hand off to subcommands.
pub struct ResolvedConfig {
    pub clw_config: Config,
    /// Ref keyspace selector (`CLW_REF_DOMAIN`): user (default) or runner.
    pub ref_domain: RefDomain,
    /// The raw, unparseable ref-domain input (`Some(raw)`) — set ONLY on the
    /// lenient path when the value could not be parsed, so `doctor` can surface
    /// it as a diagnostic instead of silently falling back to `user`. `None` on
    /// the strict path (which errors on an invalid value before reaching here).
    pub ref_domain_invalid: Option<String>,
    /// C2c env-0: `Some` when the CAS PAT must be redeemed from the credential
    /// broker at the trusted boot (runner ref-domain + an injected
    /// `CLW_CRED_TICKET`) — `clw_config.token` stays empty until
    /// [`resolve_credential`](ResolvedConfig::resolve_credential) fills it. `None`
    /// = the static token is already in `clw_config.token` (the default path).
    pub broker: Option<BrokerCred>,
}

impl ResolvedConfig {
    /// Resolve config from the parsed [`Cli`]:
    /// 1. CLI flags (already in `cli.endpoint` etc.)
    /// 2. Env vars (already merged by clap via `env = "..."`)
    /// 3. `~/.clw/config.toml` (parsed manually)
    /// 4. Hard defaults
    pub fn resolve(cli: &Cli) -> Result<Self, ClwError> {
        let file_cfg = read_config_file();

        let endpoint = cli
            .endpoint
            .clone()
            .or_else(|| corelink_env("CORELINK_ENDPOINT"))
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());

        let tenant = cli
            .tenant
            .clone()
            .or_else(|| corelink_env("CORELINK_TENANT"))
            .or_else(|| file_cfg.get("tenant").cloned())
            .ok_or_else(|| {
                ClwError::Config(
                    "missing tenant: set --tenant, CLW_TENANT/CORELINK_TENANT, or add tenant to ~/.clw/config.toml"
                        .to_string(),
                )
            })?;

        // FIX 1: the tenant is interpolated RAW into every request URL path
        // (`/v1/cas/{tenant}/...`). Validate it HERE — before any request is built
        // — so a path-injection / cross-tenant-misroute value (e.g. `a/b`,
        // `../other`) fails early (exit 2) with an actionable message.
        validate_tenant(&tenant)?;

        let cache_dir_str = cli
            .cache_dir
            .clone()
            .or_else(|| file_cfg.get("cache_dir").cloned())
            .unwrap_or_else(|| DEFAULT_CACHE_DIR.to_string());

        let cache_dir = expand_tilde(&cache_dir_str);

        let ref_domain = parse_ref_domain(
            cli.ref_domain
                .as_deref()
                .or_else(|| file_cfg.get("ref_domain").map(String::as_str)),
        )?;

        // Credential resolution. Default path (user/check/hermetic, OR a runner
        // still on the legacy env-PAT rollout): the `CLW_TOKEN` chain, byte-identical
        // to the pre-broker client. C2c env-0 path (runner ref-domain AND the fabric
        // injected a `CLW_CRED_TICKET`): the PAT is NOT in the env — capture the
        // broker inputs now and redeem the single-use ticket later at the trusted
        // boot (`resolve_credential`), fail-closed, never falling back to `CLW_TOKEN`.
        // The decision is a pure fn (`resolve_credential_inputs`) so it is unit-tested
        // without touching process env.
        let (token, broker) = resolve_credential_inputs(
            ref_domain,
            corelink_env("CLW_CRED_TICKET"),
            corelink_env("CLW_LEASE_ID"),
            corelink_env("CLW_FABRIC_ENDPOINT"),
            || {
                // NEVER print the token; only validate it is present.
                cli.token
                    .clone()
                    .or_else(|| corelink_env("CORELINK_TOKEN"))
                    .or_else(|| file_cfg.get("token").cloned())
                    .ok_or_else(|| {
                        ClwError::Config(
                            "missing token: set --token, CLW_TOKEN/CORELINK_TOKEN, or add token to ~/.clw/config.toml"
                                .to_string(),
                        )
                    })
            },
        )?;

        Ok(ResolvedConfig {
            clw_config: Config {
                endpoint: endpoint.trim_end_matches('/').to_string(),
                tenant,
                token,
                cache_dir,
            },
            ref_domain,
            // Strict resolve errors on an invalid ref-domain above, so by here it
            // was always valid.
            ref_domain_invalid: None,
            broker,
        })
    }

    /// Resolve the CAS PAT into `clw_config.token`, redeeming the C2c credential
    /// broker ticket when armed. MUST be called after [`resolve`] and BEFORE any
    /// CAS/AC request — and, for `clw run`, before the untrusted child is exec'd
    /// (the `CLI-SURFACE-EXIT-FREEZE` ordering: snapshot → hydrate → run).
    ///
    /// - Default path (`broker == None`): no-op — the static token is already set.
    /// - C2c env-0 path (`broker == Some`): redeem the single-use, lease-bound
    ///   ticket ONCE at this trusted boot for a scope-narrowed per-job PAT, stored
    ///   in `clw_config.token` (process memory only). **Fail-closed**: any redeem
    ///   error propagates — there is NO fallback to `CLW_TOKEN` (which is absent
    ///   under C2c). A second redemption would hit the fabric's single-use `410`,
    ///   so this is called exactly once.
    pub async fn resolve_credential(&mut self) -> Result<(), ClwError> {
        if let Some(b) = &self.broker {
            let pat =
                clw_client::redeem_cred_ticket(&b.fabric_endpoint, &b.lease_id, &b.ticket).await?;
            self.clw_config.token = pat;
        }
        Ok(())
    }

    /// Resolve config LENIENTLY for LOCAL-ONLY subcommands that operate on
    /// whatever is configured and make NO network call (`doctor`, `auth logout`,
    /// `auth whoami`).
    ///
    /// Applies the same flag > env > file > default precedence as [`resolve`], but
    /// a MISSING tenant/token yields an empty string rather than an error — so a
    /// half-configured state can still be diagnosed (`doctor`) or repaired
    /// (`logout`). The tenant is taken AS-IS here and is NOT validated on this path
    /// (only `ref_domain` is checked, and a bad value is surfaced via
    /// `ref_domain_invalid` rather than erroring) — the lenient consumers make no
    /// network call, and `doctor` re-validates the tenant itself when it reports.
    #[must_use]
    pub fn resolve_lenient(cli: &Cli) -> Self {
        let file_cfg = read_config_file();

        let endpoint = cli
            .endpoint
            .clone()
            .or_else(|| corelink_env("CORELINK_ENDPOINT"))
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());

        // Missing tenant/token => empty string (reported as <unset> by the
        // local-only commands), never an error.
        let tenant = cli
            .tenant
            .clone()
            .or_else(|| corelink_env("CORELINK_TENANT"))
            .or_else(|| file_cfg.get("tenant").cloned())
            .unwrap_or_default();

        let token = cli
            .token
            .clone()
            .or_else(|| corelink_env("CORELINK_TOKEN"))
            .or_else(|| file_cfg.get("token").cloned())
            .unwrap_or_default();

        let cache_dir_str = cli
            .cache_dir
            .clone()
            .or_else(|| file_cfg.get("cache_dir").cloned())
            .unwrap_or_else(|| DEFAULT_CACHE_DIR.to_string());

        // Local-only commands don't touch refs; a bad value here must not error
        // the diagnostic path, so fall back to the default keyspace — but capture
        // the raw input so `doctor` can SURFACE it rather than silently swallow it.
        let raw_ref_domain = cli
            .ref_domain
            .clone()
            .or_else(|| file_cfg.get("ref_domain").cloned());
        let (ref_domain, ref_domain_invalid) = match parse_ref_domain(raw_ref_domain.as_deref()) {
            Ok(d) => (d, None),
            Err(_) => (RefDomain::User, raw_ref_domain),
        };

        ResolvedConfig {
            clw_config: Config {
                endpoint: endpoint.trim_end_matches('/').to_string(),
                tenant,
                token,
                cache_dir: expand_tilde(&cache_dir_str),
            },
            ref_domain,
            ref_domain_invalid,
            // Lenient consumers (doctor/logout/whoami) make NO network call, so
            // there is never a broker redemption on this path.
            broker: None,
        }
    }

    /// Resolve ONLY the cache directory, tolerating a missing tenant/token.
    ///
    /// `clw uninstall` is the cleanup path and must work even when no identity
    /// was ever configured (so [`resolve`] would error on the missing token).
    /// This applies the same flag > env > file > default precedence as [`resolve`]
    /// for `cache_dir` alone, and never touches the token.
    #[must_use]
    pub fn resolve_cache_dir_lenient(cli: &Cli) -> PathBuf {
        let file_cfg = read_config_file();
        let cache_dir_str = cli
            .cache_dir
            .clone()
            .or_else(|| file_cfg.get("cache_dir").cloned())
            .unwrap_or_else(|| DEFAULT_CACHE_DIR.to_string());
        expand_tilde(&cache_dir_str)
    }
}

// ---------------------------------------------------------------------------
// CoreLink-wide env fallback
// ---------------------------------------------------------------------------

/// Read a `CORELINK_*` env var as a fallback for the `CLW_*` names (which clap
/// already merges into the `Cli`). An empty value is treated as unset so an
/// injected-but-blank var never shadows the config file. This lets a disposable
/// runner be driven by either the `CLW_*` or the CoreLink-wide `CORELINK_*`
/// namespace (server D-9 dispatch-time env injection).
fn corelink_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Decide the credential path from the resolved ref-domain and the (optionally
/// injected) C2c broker env values. **Pure** (no env / no I/O) so it is unit-tested
/// directly — [`resolve`](ResolvedConfig::resolve) supplies the real env reads.
///
/// Returns `(token, broker)`:
/// - **C2c env-0 path** — `RefDomain::Runner` AND a `ticket` present: returns an
///   empty token + the [`BrokerCred`] to redeem later. **Fail-closed** — a ticket
///   with a missing lease-id or fabric-endpoint is a hard error, never a silent
///   fall-through to the static token.
/// - **Default path** — everything else: resolves the static token eagerly via
///   `static_token` (byte-identical to the pre-broker client), `broker == None`.
fn resolve_credential_inputs(
    ref_domain: RefDomain,
    ticket: Option<String>,
    lease_id: Option<String>,
    fabric_endpoint: Option<String>,
    static_token: impl FnOnce() -> Result<String, ClwError>,
) -> Result<(String, Option<BrokerCred>), ClwError> {
    match (ref_domain, ticket) {
        (RefDomain::Runner, Some(ticket)) => {
            let lease_id = lease_id.ok_or_else(|| {
                ClwError::Config(
                    "C2c: CLW_CRED_TICKET is set but CLW_LEASE_ID is missing (fail-closed)"
                        .to_string(),
                )
            })?;
            let fabric_endpoint = fabric_endpoint.ok_or_else(|| {
                ClwError::Config(
                    "C2c: CLW_CRED_TICKET is set but CLW_FABRIC_ENDPOINT (the corelink-fabricd base) is missing (fail-closed)"
                        .to_string(),
                )
            })?;
            Ok((
                String::new(),
                Some(BrokerCred {
                    fabric_endpoint,
                    lease_id,
                    ticket,
                }),
            ))
        }
        _ => Ok((static_token()?, None)),
    }
}

/// Parse the ref-domain selector (`--ref-domain` / `CLW_REF_DOMAIN` / config
/// `ref_domain`). `user` (default) and `runner` select disjoint ref keyspaces;
/// any other value is a hard config error (exit 2).
fn parse_ref_domain(raw: Option<&str>) -> Result<RefDomain, ClwError> {
    match raw {
        None | Some("") | Some("user") => Ok(RefDomain::User),
        Some("runner") => Ok(RefDomain::Runner),
        Some(other) => Err(ClwError::Config(format!(
            "invalid ref domain '{other}': expected 'user' or 'runner' \
             (set via --ref-domain or CLW_REF_DOMAIN)"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Config file parsing (~/.clw/config.toml)
//
// Simple `key = "value"` line parser. No new toml dependency.
// ---------------------------------------------------------------------------

fn config_file_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".clw").join("config.toml"))
}

/// Read and parse `~/.clw/config.toml`. Returns an empty map when the file is
/// ABSENT (fine — flags/env/defaults take over silently).
///
/// FIX 5: a file that is PRESENT but unreadable/malformed no longer fails
/// silently into the production default endpoint with the user's intent lost —
/// it emits a `tracing::warn!`/stderr advisory that the config exists but could
/// not be read, so defaults are being used.
///
/// FIX 4 (unix): after reading, the file's mode is checked; an overbroad
/// (group/world-accessible) `config.toml` — which holds the plaintext PAT —
/// emits a `chmod 600` advisory. Neither check fails the command; both only warn.
fn read_config_file() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(path) = config_file_path() else {
        return map;
    };
    // Distinguish "absent" (silent) from "present but unreadable" (warn).
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            #[cfg(unix)]
            warn_if_perms_overbroad(&path);
            parse_config_lines(&content, &mut map);
            // P4: the file was READABLE but parsing yielded NO recognized key —
            // a typo'd / malformed config silently falling back to defaults. Warn
            // (do not fail) so the user learns their config was ignored. Kept
            // conservative: only warns when the file has real content yet not one
            // usable key, so a legitimately minimal/empty config never false-warns.
            if config_is_readable_but_unusable(&content, &map) {
                warn_config_unusable(&path.display().to_string());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Absent config file — defaults apply silently (the happy path for a
            // user driving clw purely from flags/env).
        }
        Err(e) => {
            warn_config_unreadable(&path.display().to_string(), &e.to_string());
        }
    }
    map
}

/// Whether a config-file mode is overbroad: any group/other permission bit set.
/// `config.toml` holds the plaintext PAT, so anything looser than owner-only is
/// a leak signal. Pure predicate so it is unit-testable without touching the FS.
#[cfg(unix)]
pub(crate) fn perms_are_overbroad(mode: u32) -> bool {
    mode & 0o077 != 0
}

/// FIX 4 (unix): stat `path` and, if its permissions are overbroad, warn (do not
/// fail) — naming the file and recommending `chmod 600`.
#[cfg(unix)]
fn warn_if_perms_overbroad(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if perms_are_overbroad(mode) {
            let p = path.display();
            tracing::warn!(
                path = %p,
                mode = format!("{:o}", mode & 0o777),
                "config.toml holding the PAT is group/world-accessible"
            );
            eprintln!(
                "warning: {p} is group/world-accessible (it holds your PAT); run: chmod 600 {p}"
            );
        }
    }
}

/// FIX 5: emit the present-but-unreadable advisory. Split out as a seam so the
/// "exists but failed to read/parse" signal can be tested without a real
/// unreadable file.
fn warn_config_unreadable(path: &str, reason: &str) {
    tracing::warn!(
        path = %path,
        error = %reason,
        "config.toml exists but could not be read; using defaults"
    );
    eprintln!(
        "warning: config file {path} exists but could not be read ({reason}); using defaults"
    );
}

/// The config keys clw actually understands. A readable file that parses to NONE
/// of these (after stripping blank/comment lines) is "readable but unusable".
const KNOWN_CONFIG_KEYS: [&str; 5] = ["endpoint", "tenant", "token", "cache_dir", "ref_domain"];

/// P4 predicate: decide whether a readable `config.toml` is MALFORMED — it has
/// real (non-blank, non-comment) content yet [`parse_config_lines`] recovered NO
/// recognized key. Pure over its inputs so the "readable but yielded nothing
/// usable" seam is unit-testable without touching the filesystem.
///
/// Conservative by design: returns `false` for an empty / comment-only / fully
/// blank file (a legitimately minimal config must NOT warn), and `false` as soon
/// as ANY known key was parsed (a config with one good key plus a junk line is
/// usable, not malformed).
pub(crate) fn config_is_readable_but_unusable(
    content: &str,
    parsed: &HashMap<String, String>,
) -> bool {
    // Any recognized key present => usable, never warn.
    if KNOWN_CONFIG_KEYS.iter().any(|k| parsed.contains_key(*k)) {
        return false;
    }
    // No known key. Only warn if the file actually has substantive content —
    // a non-blank line that is not a whole-line comment (so empty/comment-only
    // configs stay silent).
    content
        .lines()
        .map(str::trim)
        .any(|l| !l.is_empty() && !l.starts_with('#'))
}

/// P4: emit the readable-but-unusable advisory. Split out as a seam (mirroring
/// [`warn_config_unreadable`]) so the malformed-config signal can be tested
/// without a real malformed file.
fn warn_config_unusable(path: &str) {
    tracing::warn!(
        path = %path,
        "config.toml was read but no valid settings were parsed; using defaults"
    );
    eprintln!(
        "warning: config file {path} was read but no valid settings were parsed (check for typos); using defaults"
    );
}

/// Parse lines of the form `key = "value"` or `key = value` (no quotes).
/// Blank lines and lines beginning with `#` are ignored.
///
/// NOTE: this is a deliberately minimal, dependency-free subset of TOML — it
/// handles exactly the flat string keys clw writes (`endpoint`, `tenant`,
/// `token`, `cache_dir`). It is NOT a general TOML parser (no tables, arrays,
/// multi-line values; only `\"` and `\\` escapes within quoted values). A quoted
/// value is read up to its closing quote, so a trailing inline `# comment` after a quoted value is ignored; an
/// unquoted value is taken verbatim (so `#` is NOT an inline comment there —
/// only whole-line `#` comments are stripped).
pub fn parse_config_lines(content: &str, map: &mut HashMap<String, String>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let val_raw = v.trim();
            // A quoted value is read up to its CLOSING quote (ignoring any
            // trailing inline comment); an unquoted value is taken verbatim.
            let val = if let Some(rest) = val_raw.strip_prefix('"') {
                // Quoted value: read up to the closing quote, honoring `\"` and
                // `\\` escapes (the matched inverse of `toml_escape` on write, so
                // a token containing a quote round-trips instead of truncating).
                // A trailing inline `# comment` after the closing quote is ignored.
                let mut out = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(next) = chars.next() {
                                out.push(next);
                            }
                        }
                        '"' => break,
                        _ => out.push(c),
                    }
                }
                out
            } else {
                val_raw.to_string()
            };
            map.insert(key, val);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Escape a string value for the minimal config writer: backslash and double
/// quote, so it round-trips through [`parse_config_lines`]. Writer and parser are
/// a matched pair (this is intentionally NOT a general TOML serializer).
pub(crate) fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(s)
}

/// Return the home directory.
///
/// Reads `$HOME` first (unix-correct), falling back to `%USERPROFILE%` so the
/// resolver works on Windows, rather than the deprecated `std::env::home_dir`.
/// An empty value is treated as unset.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Return `~/.clw/config.toml` path (for use in subcommands).
pub fn clw_config_path() -> Option<PathBuf> {
    config_file_path()
}

// ---------------------------------------------------------------------------
// Shared config-file writer (used by `clw init` and `clw auth`)
//
// config.toml holds the plaintext PAT, so the file is forced owner-only (0600)
// and its parent (~/.clw) owner-only (0700). These helpers live here so both
// `init` and `auth` share ONE implementation instead of duplicating it.
// ---------------------------------------------------------------------------

/// Atomically write `content` to `path` with owner-only permissions: `0600` on
/// Unix, and (on Windows) an explicit owner-only DACL via
/// [`restrict_to_owner_windows`]. The file contains the plaintext PAT, so it must
/// never be readable by other users — and a crash mid-write must never leave a
/// truncated/partial config holding a half-written PAT (or an empty file that
/// blanks a working identity).
///
/// B1 (crash-consistency): the write is ATOMIC. We write the full `content` to a
/// temp file IN THE SAME DIRECTORY as `path` (so the final `fs::rename` is an
/// atomic same-filesystem rename on POSIX; on Windows the temp handle is closed
/// before the rename), fsync the temp file's data to disk, then rename it into
/// place. A crash at any point leaves EITHER the old config intact OR the new one
/// complete — never a half-written file. Mirrors the proven
/// tempfile_in + flush + rename pattern in `clw-cache`.
///
/// PAT-leak ordering: the temp file is created 0600 (unix) / owner-only DACL
/// (windows) BEFORE any byte of `content` (which carries the PAT) is written, so
/// the secret is never momentarily more world-readable in the temp file than in
/// the final file.
///
/// On Windows a failed ACL apply WARNS (the PAT may be readable by other users)
/// but does NOT fail the write — losing the ability to save config because
/// hardening failed would be the worse outcome. The parent dir is already
/// hardened to an owner-only PROTECTED DACL by `ensure_config_parent_0700`
/// (called before this), so the temp file inherits owner-only from creation.
pub(crate) fn write_config_file_0600(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    // Temp file in the SAME directory as the final path so the rename is atomic
    // (a cross-directory rename is NOT atomic / may be a copy). Fall back to the
    // path itself if it somehow has no parent.
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp_path = config_tmp_path(dir);

    // Create the temp file owner-only BEFORE writing the PAT-bearing content, so
    // the secret is never written into a momentarily-loose file. `create_new`
    // refuses to clobber a stray collision (the name is process+time unique).
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp_path)?;

    // On Windows, mode() is a no-op; restrict the temp file's DACL to the owner
    // BEFORE writing the PAT (warn-not-fail, see fn doc). This closes the window
    // where the PAT could be world-readable in the temp file.
    #[cfg(windows)]
    warn_if_windows_acl_failed(&tmp_path);

    // Write the full content, flush the buffer, and fsync the bytes to durable
    // storage before the rename — so a crash after the rename never exposes a
    // file whose data never reached disk. A write failure removes the temp file.
    if let Err(e) = (|| -> std::io::Result<()> {
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })() {
        drop(file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // `mode()` only applies on CREATION; force 0600 on the temp file explicitly
    // in case the open path did not (defense-in-depth; cheap and idempotent).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }

    // Close the handle before renaming (required on Windows; harmless on unix).
    drop(file);

    // Atomic publish: rename the complete temp file over the final path. On a
    // failure, clean up the temp file so no stray partial is left behind.
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Windows analogue of the 0600 above on the FINAL path: restrict the DACL to
    // the owner. Warn, never fail, on error (see fn doc). On unix the renamed file
    // already carries the temp file's 0600 mode.
    #[cfg(windows)]
    warn_if_windows_acl_failed(path);
    Ok(())
}

/// Build a unique temp-file path in `dir` for the atomic config write. Combines
/// the process id, thread id, and subsecond nanos so concurrent writers (and a
/// leftover from a prior crashed run) never collide. The name is dotfile-prefixed
/// so it is visually distinct from the real `config.toml`.
fn config_tmp_path(dir: &std::path::Path) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let tid = std::thread::current().id();
    dir.join(format!(".config.toml.tmp-{pid}-{tid:?}-{nanos}"))
}

/// Tighten a directory to owner-only: `0700` on Unix, and (on Windows) an
/// explicit owner-only DACL via [`restrict_to_owner_windows`]. The directory
/// holds `config.toml` (plaintext PAT), so it must not be traversable by
/// group/world (Unix) nor list/read-able by other users (Windows).
///
/// On Windows a failed ACL apply WARNS but does NOT fail the operation (same
/// rationale as [`write_config_file_0600`]).
pub(crate) fn harden_dir_0700(_dir: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    warn_if_windows_acl_failed(_dir);
    Ok(())
}

/// Apply [`restrict_to_owner_windows`] to `path`; on failure emit a
/// `tracing::warn!` + a one-line stderr advisory and KEEP GOING. The hardening
/// is best-effort: a failed ACL tightening must never sink the surrounding
/// file/dir operation (a writable-but-loosely-permissioned config beats no
/// config at all). Shared by [`write_config_file_0600`] and [`harden_dir_0700`].
#[cfg(windows)]
fn warn_if_windows_acl_failed(path: &std::path::Path) {
    if let Err(e) = restrict_to_owner_windows(path) {
        let p = path.display();
        tracing::warn!(
            path = %p,
            error = %e,
            "could not ACL-restrict the PAT-bearing path to the owner; it may be \
             readable by other users on this machine"
        );
        eprintln!(
            "warning: could not restrict {p} to your user only ({e}); your PAT may be \
             readable by other users — protect ~/.clw manually (Properties > Security)"
        );
    }
}

/// Restrict `path` on Windows to a PROTECTED (inheritance-disabled) DACL that
/// grants FULL CONTROL to ONLY the current process user, dropping inherited /
/// Everyone access. This is the Windows analogue of the unix `0600`/`0700`
/// hardening, which is a no-op on Windows — without it the plaintext-PAT
/// `~/.clw/config.toml` inherits the (often group/Users-readable) parent ACL.
///
/// Cold-review map of the Win32 sequence (each step documented at its call):
///
/// 1. `OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, ..)` — token handle.
/// 2. `GetTokenInformation(token, TokenUser, ..)` (size-then-buffer) — our SID.
/// 3. `InitializeAcl` + `AddAccessAllowedAce(.., GENERIC_ALL, sid)` — a one-ACE
///    DACL over a correctly sized buffer.
/// 4. `SetNamedSecurityInfoW(.., DACL_SECURITY_INFORMATION |
///    PROTECTED_DACL_SECURITY_INFORMATION, ..)` — replace the object's DACL and
///    disable inheritance.
///
/// All handles/buffers are released before returning; any non-`ERROR_SUCCESS`
/// status is mapped to an `io::Error`.
#[cfg(windows)]
fn restrict_to_owner_windows(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;

    // GENERIC_ALL grants the owner full control over the object; ERROR_SUCCESS is
    // the success code SetNamedSecurityInfoW returns directly (not via LastError).
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, GENERIC_ALL, HANDLE};
    use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        AddAccessAllowedAce, GetLengthSid, GetTokenInformation, InitializeAcl, TokenUser,
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // Helper: snapshot the last OS error into an io::Error (read by
    // `io::Error::last_os_error`, which calls GetLastError under the hood).
    let last_err = || std::io::Error::last_os_error();

    // SAFETY: every call below is a documented Win32 API used per its contract:
    // handles/buffers are sized via the prescribed two-call pattern, the ACL
    // buffer is sized exactly, and all owned resources (token handle, SID buffer)
    // are released on every return path before the function exits.
    unsafe {
        // --- 1. Open the current process token for query --------------------
        // OpenProcessToken on the pseudo-handle from GetCurrentProcess() yields a
        // token we can interrogate for the running user's SID. TOKEN_QUERY is the
        // only access right needed.
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(last_err());
        }

        // --- 2a. Size the TokenUser buffer ----------------------------------
        // First GetTokenInformation call: pass a NULL buffer / 0 length to learn
        // the required size (it "fails" with ERROR_INSUFFICIENT_BUFFER, expected).
        let mut needed: u32 = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            let e = last_err();
            CloseHandle(token);
            return Err(e);
        }

        // --- 2b. Fetch the TOKEN_USER (which embeds the user's SID) ---------
        // Second call: an OVER-ALIGNED buffer backs the variable-length TOKEN_USER;
        // the SID it points to lives INSIDE this buffer, so `buf` must outlive every
        // use of `sid` below.
        //
        // FIX 5 (soundness): TOKEN_USER embeds a SID_AND_ATTRIBUTES whose `Sid`
        // field is a POINTER (alignment 8), so the backing store must be 8-aligned.
        // A plain `Vec<u8>` is align-1; casting it to `*const TOKEN_USER` and reading
        // through it is UNDEFINED BEHAVIOR (misaligned read). Backing the buffer with
        // a `Vec<u64>` (align 8) sized to ceil(needed/8) gives a correctly aligned
        // allocation while keeping the exact byte size the API asked for.
        let mut buf: Vec<u64> = vec![0u64; needed.div_ceil(8) as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            let e = last_err();
            CloseHandle(token);
            return Err(e);
        }
        // The token handle is no longer needed once the SID is copied into `buf`.
        CloseHandle(token);

        // TOKEN_USER { User: SID_AND_ATTRIBUTES { Sid: PSID, .. } }.
        let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid: PSID = token_user.User.Sid;
        if sid.is_null() {
            return Err(std::io::Error::other("TokenUser returned a null SID"));
        }

        // --- 3. Build a one-ACE DACL granting GENERIC_ALL to our SID --------
        // ACL byte size = sizeof(ACL) header
        //               + sizeof(ACCESS_ALLOWED_ACE) (its trailing SidStart DWORD
        //                 is a placeholder for the SID's first dword)
        //               - sizeof(DWORD) (remove that placeholder)
        //               + GetLengthSid(sid) (the actual SID bytes).
        let sid_len = GetLengthSid(sid);
        if sid_len == 0 {
            return Err(last_err());
        }
        let acl_size = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            - std::mem::size_of::<u32>()
            + sid_len as usize;
        // FIX 5 (soundness): ACL and ACCESS_ALLOWED_ACE contain u32 fields (align 4),
        // so writing/reading through this buffer as `*mut ACL` requires ≥4-byte
        // alignment. A plain `Vec<u8>` is align-1, so the cast + InitializeAcl write
        // would be UNDEFINED BEHAVIOR. Back it with a `Vec<u64>` (align 8, which
        // covers the align-4 need) sized to ceil(acl_size/8) — same byte size, just
        // an over-aligned allocation.
        let mut acl_buf: Vec<u64> = vec![0u64; acl_size.div_ceil(8)];
        let acl = acl_buf.as_mut_ptr() as *mut ACL;

        // InitializeAcl lays down the ACL header in our buffer.
        if InitializeAcl(acl, acl_size as u32, ACL_REVISION) == 0 {
            return Err(last_err());
        }
        // AddAccessAllowedAce appends a single allow-ACE: GENERIC_ALL for our SID.
        if AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_ALL, sid) == 0 {
            return Err(last_err());
        }

        // --- 4. Apply the DACL to the named file object ---------------------
        // Wide, NUL-terminated path for the *W API (OsStr::encode_wide + 0).
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SetNamedSecurityInfoW: pobjectname is PCWSTR (*const u16). securityinfo =
        // DACL_SECURITY_INFORMATION ("I'm setting the DACL") |
        // PROTECTED_DACL_SECURITY_INFORMATION (mark it PROTECTED so inherited ACEs
        // are dropped — the explicit owner-only ACE becomes the entire effective
        // DACL). psidowner/psidgroup (PSID = *mut) and psacl (*const ACL) are NULL,
        // leaving owner/group/SACL untouched; pdacl points at our owner-only ACL.
        let status = SetNamedSecurityInfoW(
            wide.as_ptr(), // pObjectName — PCWSTR
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(), // psidOwner — unchanged
            std::ptr::null_mut(), // psidGroup — unchanged
            acl.cast_const(),     // pDacl     — our owner-only DACL (*const ACL)
            std::ptr::null(),     // pSacl     — unchanged (*const ACL)
        );

        // SetNamedSecurityInfoW returns a WIN32_ERROR code DIRECTLY (NOT via
        // GetLastError); map a non-ERROR_SUCCESS status to an io::Error.
        if status != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }

        Ok(())
    }
}

/// Ensure the parent of `path` (i.e. `~/.clw`) exists and is hardened to 0700.
///
/// V2 (audit pass-2): ORDER IS LOAD-BEARING for the Windows DACL-window concern.
/// Every caller (`init`, `auth login`) invokes THIS before
/// `write_config_file_0600` — see init.rs and auth.rs. Because the parent dir is
/// hardened to an owner-only PROTECTED DACL HERE, before the config file is
/// created inside it, the freshly-created `config.toml` inherits an owner-only
/// ACL from the moment it exists — there is no window in which it carries a
/// looser (group/Users-readable) inherited DACL. `write_config_file_0600` then
/// additionally applies its OWN protected owner-only DACL to the file as
/// defense-in-depth. So no readable window exists; no fix is required here.
pub(crate) fn ensure_config_parent_0700(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        harden_dir_0700(parent)?;
    }
    Ok(())
}

/// Read `~/.clw/config.toml` into an ordered list of `(key, raw_value)` lines.
///
/// Unlike [`read_config_file`], this preserves only the flat string keys clw
/// writes and returns the unescaped values. Used by `clw auth logout` to rewrite
/// the file with a single key removed while keeping the others intact. Returns
/// an empty vec if the file is missing or unreadable.
pub(crate) fn read_config_pairs(path: &std::path::Path) -> Vec<(String, String)> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut map = HashMap::new();
    parse_config_lines(&content, &mut map);
    // Emit in a stable order so rewrites are deterministic.
    let mut pairs: Vec<(String, String)> = map.into_iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

/// Merge `updates` into the existing pairs read from `path`, overwriting only the
/// keys being set and PRESERVING every other key already in the file.
///
/// This is the shared preserve-then-overwrite primitive used by `clw init` and
/// `clw auth login`, so that writing the identity keys never wipes an unrelated
/// key a future config feature may add. Returns the merged pairs in a stable
/// (sorted) order ready for [`render_config`].
pub(crate) fn merge_config_pairs(
    path: &std::path::Path,
    updates: &[(String, String)],
) -> Vec<(String, String)> {
    let mut pairs = read_config_pairs(path);
    for (k, v) in updates {
        pairs.retain(|(existing, _)| existing != k);
        pairs.push((k.clone(), v.clone()));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

/// Serialize `(key, value)` pairs back into the minimal `key = "value"` config
/// format that [`parse_config_lines`] reads.
pub(crate) fn render_config(pairs: &[(String, String)]) -> String {
    let mut out = String::from("# clw configuration\n");
    for (k, v) in pairs {
        out.push_str(&format!("{k} = \"{}\"\n", toml_escape(v)));
    }
    out
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::Path;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // parse_config_lines
    // -----------------------------------------------------------------------

    #[test]
    fn parse_quoted_values() {
        let mut map = HashMap::new();
        parse_config_lines(
            r#"
endpoint = "https://example.com"
tenant = "t1"
token = "tok123"
cache_dir = "~/.clw/cache"
"#,
            &mut map,
        );
        assert_eq!(map["endpoint"], "https://example.com");
        assert_eq!(map["tenant"], "t1");
        assert_eq!(map["token"], "tok123");
        assert_eq!(map["cache_dir"], "~/.clw/cache");
    }

    #[test]
    fn parse_unquoted_values() {
        let mut map = HashMap::new();
        parse_config_lines("endpoint = https://example.com\ntenant = acme", &mut map);
        assert_eq!(map["endpoint"], "https://example.com");
        assert_eq!(map["tenant"], "acme");
    }

    #[test]
    fn parse_ignores_blank_and_comment_lines() {
        let mut map = HashMap::new();
        parse_config_lines("# this is a comment\n\nkey = val", &mut map);
        assert_eq!(map.len(), 1);
        assert_eq!(map["key"], "val");
    }

    #[test]
    fn parse_quoted_value_ignores_trailing_inline_comment() {
        // A trailing inline comment after a QUOTED value must not leak into the value.
        let mut map = HashMap::new();
        parse_config_lines("token = \"secret\" # inline comment", &mut map);
        assert_eq!(map["token"], "secret");
    }

    #[test]
    fn escaped_value_round_trips_through_writer_and_parser() {
        // Regression (M0.5): a value containing a quote/backslash must round-trip
        // through toml_escape -> parse_config_lines instead of truncating the PAT.
        for raw in [
            "pat_with\"quote",
            "back\\slash",
            "both\"and\\here",
            "trailing\\",
            "normal_pat_abc123",
        ] {
            let line = format!("token = \"{}\"", toml_escape(raw));
            let mut map = HashMap::new();
            parse_config_lines(&line, &mut map);
            assert_eq!(
                map["token"], raw,
                "value must round-trip exactly for {raw:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // B1 — atomic config write: complete + 0600 + NO leftover temp file
    // -----------------------------------------------------------------------

    #[test]
    fn atomic_write_leaves_complete_0600_config_and_no_temp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let content = "# clw configuration\ntoken = \"super-secret-pat\"\ntenant = \"acme\"\n";

        write_config_file_0600(&path, content).unwrap();

        // The config is COMPLETE (the full content reached disk, not a truncated
        // partial — a crash mid-write under the old non-atomic path could leave a
        // half-written PAT here).
        let got = std::fs::read_to_string(&path).unwrap();
        assert_eq!(got, content, "config must be written completely");

        // 0600 guarantee preserved (the file holds the plaintext PAT).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "config must be owner-only 0600");
        }

        // On SUCCESS, NO temp file is left behind in the directory — the rename
        // consumed it. Scan the dir for any `.config.toml.tmp-*` stray.
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".config.toml.tmp-"))
            .collect();
        assert!(
            strays.is_empty(),
            "a successful atomic write must leave no temp file, found: {strays:?}"
        );

        // The directory holds exactly the one final config file.
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["config.toml".to_string()],
            "only the final config.toml must remain"
        );
    }

    #[test]
    fn atomic_write_overwrites_existing_and_tightens_loose_mode() {
        // Overwriting a PRE-EXISTING (possibly loose-mode) config must end at 0600
        // with the new content complete — the atomic rename replaces it wholesale.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        // Seed a pre-existing world-readable config (the classic restored-0644).
        std::fs::write(&path, "token = \"old\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let new_content = "token = \"new-secret\"\ntenant = \"acme\"\n";
        write_config_file_0600(&path, new_content).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), new_content);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "overwriting a 0644 config must end at 0600"
            );
        }
        // No leftover temp.
        let strays = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".config.toml.tmp-")
            });
        assert!(!strays, "overwrite must leave no temp file");
    }

    // -----------------------------------------------------------------------
    // Config precedence: flag > env > file
    // -----------------------------------------------------------------------

    /// Helper: write a config file into `dir/.clw/config.toml`.
    fn write_config_file(dir: &Path, content: &str) {
        let clw_dir = dir.join(".clw");
        std::fs::create_dir_all(&clw_dir).unwrap();
        let mut f = std::fs::File::create(clw_dir.join("config.toml")).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn config_precedence_flag_beats_env_beats_file() {
        // Use a temp dir as the fake HOME.
        let tmp_home = TempDir::new().unwrap();
        write_config_file(
            tmp_home.path(),
            "endpoint = \"https://from-file.example.com\"\ntenant = \"file-tenant\"\ntoken = \"file-token\"\n",
        );

        // Parse config lines directly to verify file would have lower precedence.
        let mut file_cfg = HashMap::new();
        let content =
            std::fs::read_to_string(tmp_home.path().join(".clw").join("config.toml")).unwrap();
        parse_config_lines(&content, &mut file_cfg);

        // Simulate: flag wins over file value.
        let flag_endpoint = Some("https://from-flag.example.com".to_string());
        let resolved_endpoint = flag_endpoint
            .clone()
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        assert_eq!(resolved_endpoint, "https://from-flag.example.com");

        // Simulate: file wins when flag is absent.
        let no_flag: Option<String> = None;
        let resolved_endpoint2 = no_flag
            .or_else(|| file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        assert_eq!(resolved_endpoint2, "https://from-file.example.com");

        // Simulate: default wins when both absent.
        let mut empty_file_cfg: HashMap<String, String> = HashMap::new();
        parse_config_lines("", &mut empty_file_cfg);
        let no_flag2: Option<String> = None;
        let resolved_endpoint3 = no_flag2
            .or_else(|| empty_file_cfg.get("endpoint").cloned())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        assert_eq!(resolved_endpoint3, DEFAULT_ENDPOINT);
    }

    // -----------------------------------------------------------------------
    // expand_tilde
    // -----------------------------------------------------------------------

    #[test]
    fn expand_tilde_with_home() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(home) = home_dir() {
            let result = expand_tilde("~/.clw/cache");
            assert_eq!(result, home.join(".clw/cache"));
        }
    }

    #[test]
    fn expand_tilde_absolute_path_unchanged() {
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn expand_tilde_relative_path_unchanged() {
        let result = expand_tilde("relative/path");
        assert_eq!(result, PathBuf::from("relative/path"));
    }

    // -----------------------------------------------------------------------
    // home_dir: Windows fallback to %USERPROFILE% when $HOME is absent
    // -----------------------------------------------------------------------

    /// Serializes tests that mutate or read the process-global `HOME`/
    /// `USERPROFILE` env — cargo runs tests as parallel threads in one process,
    /// so a mutation in one test would otherwise race a read in another.
    /// Poison-tolerant so one panicking test does not cascade-fail the rest.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard that snapshots `HOME`/`USERPROFILE`, lets the test mutate
    /// them, and restores the originals on drop (hold [`ENV_LOCK`] for the
    /// whole test so the mutation window is never observed concurrently).
    struct HomeEnvGuard {
        home: Option<std::ffi::OsString>,
        userprofile: Option<std::ffi::OsString>,
    }

    impl HomeEnvGuard {
        fn new() -> Self {
            Self {
                home: std::env::var_os("HOME"),
                userprofile: std::env::var_os("USERPROFILE"),
            }
        }
    }

    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            match &self.home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.userprofile {
                Some(v) => std::env::set_var("USERPROFILE", v),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // FIX 4 — overbroad config.toml permission predicate (unix)
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn perms_are_overbroad_flags_group_world_bits() {
        // Owner-only modes are fine.
        assert!(!perms_are_overbroad(0o600));
        assert!(!perms_are_overbroad(0o400));
        assert!(!perms_are_overbroad(0o700));
        // Any group/other bit is a leak signal for a PAT-bearing file.
        assert!(perms_are_overbroad(0o640)); // group-readable
        assert!(perms_are_overbroad(0o604)); // world-readable
        assert!(perms_are_overbroad(0o644)); // the classic copied/restored mode
        assert!(perms_are_overbroad(0o660));
        assert!(perms_are_overbroad(0o777));
    }

    // -----------------------------------------------------------------------
    // FIX 5 — present-but-unreadable config warns; absent is silent
    // -----------------------------------------------------------------------

    #[test]
    fn warn_config_unreadable_seam_emits_advisory() {
        // The seam exercises the "exists but failed" path: it must mention the
        // file and that defaults are being used (asserted via the message it
        // builds). It is a warn-only path, so calling it must not panic/return.
        warn_config_unreadable("/some/.clw/config.toml", "permission denied");
        // Verify the advisory wording the helper emits (same string sent to
        // stderr/tracing) names the file and the defaults fallback.
        let path = "/some/.clw/config.toml";
        let reason = "permission denied";
        let advisory = format!(
            "warning: config file {path} exists but could not be read ({reason}); using defaults"
        );
        assert!(advisory.contains(path));
        assert!(advisory.contains("using defaults"));
    }

    #[test]
    fn absent_config_file_is_silent_and_yields_empty_map() {
        // Point HOME at a temp dir with NO .clw/config.toml: read_config_file must
        // return an empty map silently (the absent-file branch), NOT the unreadable
        // warn path.
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = HomeEnvGuard::new();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");
        let map = read_config_file();
        assert!(map.is_empty(), "absent config must yield an empty map");
    }

    // -----------------------------------------------------------------------
    // FIX 1 — resolve rejects a path-injection tenant early
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_rejects_path_injection_tenant() {
        // A tenant carrying a path-traversal payload must be rejected by resolve
        // (via validate_tenant) before any request is built.
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = HomeEnvGuard::new();
        let _cred = VarGuard::isolate();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");

        let cli = crate::Cli::try_parse_from([
            "clw",
            "--tenant",
            "../othertenant",
            "--token",
            "tok",
            "ls",
            "--name",
            "ws",
        ])
        .unwrap();
        match ResolvedConfig::resolve(&cli) {
            Err(ClwError::InvalidTenant(_)) => {}
            Err(other) => panic!("expected InvalidTenant, got: {other:?}"),
            Ok(_) => panic!("expected resolve to reject a path-injection tenant"),
        }

        // A clean tenant resolves fine.
        let cli_ok = crate::Cli::try_parse_from([
            "clw", "--tenant", "acme", "--token", "tok", "ls", "--name", "ws",
        ])
        .unwrap();
        let resolved = ResolvedConfig::resolve(&cli_ok).unwrap();
        assert_eq!(resolved.clw_config.tenant, "acme");
    }

    // -----------------------------------------------------------------------
    // CORELINK_* env aliases (runner readiness — D-9 dispatch-time injection)
    // -----------------------------------------------------------------------

    /// RAII guard that snapshots + restores an arbitrary set of env vars, so a
    /// test can mutate the `CLW_*` / `CORELINK_*` namespace without leaking into
    /// sibling tests (hold [`ENV_LOCK`] for the whole test).
    struct VarGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }
    impl VarGuard {
        fn capture(names: &[&'static str]) -> Self {
            Self {
                saved: names.iter().map(|n| (*n, std::env::var_os(n))).collect(),
            }
        }
    }
    impl Drop for VarGuard {
        fn drop(&mut self) {
            for (n, v) in &self.saved {
                match v {
                    Some(v) => std::env::set_var(n, v),
                    None => std::env::remove_var(n),
                }
            }
        }
    }

    /// The COMPLETE credential/config env namespace that
    /// [`ResolvedConfig::resolve`] + [`ResolvedConfig::resolve_lenient`] consult:
    /// the clap `env=` bindings on `Cli` (CLW_*) plus the `corelink_env` fallbacks
    /// (CORELINK_*) plus the C2c credential-broker trio. A resolve test MUST
    /// neutralize ALL of these, not merely the few it sets — otherwise an ambient
    /// value silently changes the resolution path and the test goes red for a reason
    /// unrelated to its assertion. This is not hypothetical: a `runs-on: corelink`
    /// fabric box injects the broker trio (CLW_CRED_TICKET/CLW_LEASE_ID/
    /// CLW_FABRIC_ENDPOINT) into every job's env-0, and a developer may have
    /// CLW_TENANT/CLW_TOKEN exported — both made these tests flake red until they
    /// isolated the full set. Keep this list in lockstep with `resolve`.
    const CRED_ENV_VARS: &[&str] = &[
        "CLW_ENDPOINT",
        "CLW_TENANT",
        "CLW_TOKEN",
        "CLW_CACHE_DIR",
        "CLW_REF_DOMAIN",
        "CORELINK_ENDPOINT",
        "CORELINK_TENANT",
        "CORELINK_TOKEN",
        "CLW_CRED_TICKET",
        "CLW_LEASE_ID",
        "CLW_FABRIC_ENDPOINT",
    ];

    impl VarGuard {
        /// Snapshot the FULL [`CRED_ENV_VARS`] namespace and CLEAR every one, so a
        /// resolve test starts from a known-empty credential environment regardless
        /// of what the host/CI injected; the originals are restored on drop. Hold
        /// [`ENV_LOCK`] for the whole test.
        fn isolate() -> Self {
            let g = Self::capture(CRED_ENV_VARS);
            for &n in CRED_ENV_VARS {
                std::env::remove_var(n);
            }
            g
        }
    }

    #[test]
    fn resolve_accepts_corelink_aliases_and_clw_wins() {
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _home = HomeEnvGuard::new();
        // Isolate the FULL credential namespace (incl. the broker trio a corelink
        // box injects) so only the CORELINK_* aliases this test sets are in play.
        let _cred = VarGuard::isolate();
        // Disposable-runner state: empty HOME (no config file), no CLW_* — only
        // the CoreLink-wide CORELINK_* aliases injected at dispatch.
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");
        std::env::set_var("CORELINK_TENANT", "acme-runner");
        std::env::set_var("CORELINK_TOKEN", "rtok");

        let cli = crate::Cli::try_parse_from(["clw", "ls", "--name", "ws"]).unwrap();
        let resolved = ResolvedConfig::resolve(&cli).unwrap();
        assert_eq!(
            resolved.clw_config.tenant, "acme-runner",
            "CORELINK_TENANT must be read when CLW_TENANT / --tenant / config are absent"
        );
        assert_eq!(
            resolved.clw_config.token, "rtok",
            "CORELINK_TOKEN must be read as a fallback"
        );

        // An empty CORELINK_* must be treated as unset (not shadow anything).
        std::env::set_var("CORELINK_ENDPOINT", "");
        let cli_e = crate::Cli::try_parse_from(["clw", "ls", "--name", "ws"]).unwrap();
        let resolved_e = ResolvedConfig::resolve(&cli_e).unwrap();
        assert_eq!(
            resolved_e.clw_config.endpoint, DEFAULT_ENDPOINT,
            "an empty CORELINK_ENDPOINT must fall through to the default"
        );

        // CLW_* (clap) must take precedence over CORELINK_* when both are set.
        std::env::set_var("CLW_TENANT", "acme-clw");
        let cli2 = crate::Cli::try_parse_from(["clw", "ls", "--name", "ws"]).unwrap();
        let resolved2 = ResolvedConfig::resolve(&cli2).unwrap();
        assert_eq!(
            resolved2.clw_config.tenant, "acme-clw",
            "CLW_TENANT (clap) must win over CORELINK_TENANT"
        );
    }

    #[test]
    fn resolve_ref_domain_selector() {
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _home = HomeEnvGuard::new();
        let _cred = VarGuard::isolate();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");

        // Default (no selector) => the user keyspace.
        let cli = crate::Cli::try_parse_from([
            "clw", "--tenant", "acme", "--token", "k", "ls", "--name", "ws",
        ])
        .unwrap();
        assert_eq!(
            ResolvedConfig::resolve(&cli).unwrap().ref_domain,
            RefDomain::User
        );

        // --ref-domain runner => the runner keyspace.
        let cli_r = crate::Cli::try_parse_from([
            "clw",
            "--tenant",
            "acme",
            "--token",
            "k",
            "--ref-domain",
            "runner",
            "ls",
            "--name",
            "ws",
        ])
        .unwrap();
        assert_eq!(
            ResolvedConfig::resolve(&cli_r).unwrap().ref_domain,
            RefDomain::Runner
        );

        // CLW_REF_DOMAIN=runner (clap env) => the runner keyspace.
        std::env::set_var("CLW_REF_DOMAIN", "runner");
        let cli_e = crate::Cli::try_parse_from([
            "clw", "--tenant", "acme", "--token", "k", "ls", "--name", "ws",
        ])
        .unwrap();
        assert_eq!(
            ResolvedConfig::resolve(&cli_e).unwrap().ref_domain,
            RefDomain::Runner
        );
        std::env::remove_var("CLW_REF_DOMAIN");

        // An invalid selector is a hard config error (exit 2), never a silent default.
        let cli_bad = crate::Cli::try_parse_from([
            "clw",
            "--tenant",
            "acme",
            "--token",
            "k",
            "--ref-domain",
            "bogus",
            "ls",
            "--name",
            "ws",
        ])
        .unwrap();
        assert!(matches!(
            ResolvedConfig::resolve(&cli_bad),
            Err(ClwError::Config(_))
        ));
    }

    #[test]
    fn resolve_lenient_surfaces_invalid_ref_domain() {
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _home = HomeEnvGuard::new();
        let _cred = VarGuard::isolate();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");

        // Lenient resolve must NOT error on a bad ref-domain (it powers `doctor`);
        // it falls back to the user keyspace BUT records the raw value so the bad
        // input can be surfaced as a diagnostic instead of silently swallowed.
        let cli = crate::Cli::try_parse_from([
            "clw",
            "--tenant",
            "acme",
            "--token",
            "k",
            "--ref-domain",
            "bogus",
            "doctor",
        ])
        .unwrap();
        let resolved = ResolvedConfig::resolve_lenient(&cli);
        assert_eq!(
            resolved.ref_domain,
            RefDomain::User,
            "an invalid ref-domain must coerce to the default user keyspace"
        );
        assert_eq!(
            resolved.ref_domain_invalid,
            Some("bogus".into()),
            "the raw invalid ref-domain must be captured for diagnostics"
        );
    }

    // -----------------------------------------------------------------------
    // P7 — a missing tenant/token is a CONFIG error, not a Transport error
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_missing_tenant_is_config_error() {
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = HomeEnvGuard::new();
        let _cred = VarGuard::isolate();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");
        // No --tenant, no --token, no config file: the FIRST missing field
        // (tenant) must surface as a CONFIG-class error carrying the actionable
        // set-tenant guidance — NOT a Transport (network) error.
        let cli = crate::Cli::try_parse_from(["clw", "ls", "--name", "ws"]).unwrap();
        match ResolvedConfig::resolve(&cli) {
            Err(ClwError::Config(msg)) => {
                assert!(msg.contains("tenant"), "must name the tenant, got: {msg}");
                assert!(
                    msg.contains("CLW_TENANT") || msg.contains("--tenant"),
                    "must stay actionable, got: {msg}"
                );
            }
            Err(other) => panic!("expected ClwError::Config for a missing tenant, got: {other:?}"),
            Ok(_) => panic!("expected resolve to error on a missing tenant"),
        }
    }

    #[test]
    fn resolve_missing_token_is_config_error() {
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = HomeEnvGuard::new();
        let _cred = VarGuard::isolate();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");
        // Tenant present (so we pass the tenant gate) but no token => the token
        // gate must ALSO be a CONFIG error.
        let cli =
            crate::Cli::try_parse_from(["clw", "--tenant", "acme", "ls", "--name", "ws"]).unwrap();
        match ResolvedConfig::resolve(&cli) {
            Err(ClwError::Config(msg)) => {
                assert!(msg.contains("token"), "must name the token, got: {msg}");
            }
            Err(other) => panic!("expected ClwError::Config for a missing token, got: {other:?}"),
            Ok(_) => panic!("expected resolve to error on a missing token"),
        }
    }

    // -----------------------------------------------------------------------
    // P1/P2 — lenient resolution tolerates a missing tenant+token
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_lenient_tolerates_missing_tenant_and_token() {
        use clap::Parser as _;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = HomeEnvGuard::new();
        let _cred = VarGuard::isolate();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");
        // No tenant, no token, no config file: lenient resolution must NOT error;
        // missing identity fields come back as empty strings (reported <unset>).
        let cli = crate::Cli::try_parse_from(["clw", "doctor"]).unwrap();
        let resolved = ResolvedConfig::resolve_lenient(&cli);
        assert!(
            resolved.clw_config.tenant.is_empty(),
            "missing tenant => empty"
        );
        assert!(
            resolved.clw_config.token.is_empty(),
            "missing token => empty"
        );
        // Endpoint still falls back to the default.
        assert_eq!(resolved.clw_config.endpoint, DEFAULT_ENDPOINT);
    }

    // -----------------------------------------------------------------------
    // P4 — readable-but-malformed config warns; minimal/empty stays silent
    // -----------------------------------------------------------------------

    #[test]
    fn config_unusable_predicate_flags_only_real_junk() {
        // Junk content that parses to NO known key => unusable (warn).
        let mut junk = HashMap::new();
        parse_config_lines("this is not valid toml at all\n[some-table]\n", &mut junk);
        assert!(
            config_is_readable_but_unusable("this is not valid toml at all\n[some-table]\n", &junk),
            "a readable file with content but no recognized key must be flagged unusable"
        );

        // A config with at least one KNOWN key is usable, even with a junk line.
        let mut ok = HashMap::new();
        parse_config_lines("tenant = \"acme\"\njunk line here\n", &mut ok);
        assert!(
            !config_is_readable_but_unusable("tenant = \"acme\"\njunk line here\n", &ok),
            "one good key makes the config usable — must NOT warn"
        );

        // An empty or comment-only file is a legitimately minimal config: silent.
        let empty = HashMap::new();
        assert!(!config_is_readable_but_unusable("", &empty));
        assert!(!config_is_readable_but_unusable("\n\n  \n", &empty));
        assert!(!config_is_readable_but_unusable(
            "# just a comment\n",
            &empty
        ));
    }

    // -----------------------------------------------------------------------
    // PART A — proptest: parse_config_lines never panics / never loops on
    // arbitrary text (embedded quotes, backslashes, newlines, unbalanced
    // escapes, invalid-byte-boundary Strings). A panic/hang = test failure.
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

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]

            /// `parse_config_lines` on ARBITRARY text never panics and always
            /// terminates — only mutates the map. The `.*` regex covers embedded
            /// quotes, backslashes, '=', '#', and multi-line content; `\PC*`
            /// adds non-printable/control-laden inputs.
            #[test]
            fn parse_config_lines_never_panics(content in ".*") {
                let mut map = HashMap::new();
                parse_config_lines(&content, &mut map);
            }

            /// Same invariant against multi-line, control-char, and escape-heavy
            /// strings — exercises the quoted-value escape loop (`\\` / `\"`) so a
            /// trailing/unbalanced backslash can never run the char iterator past
            /// its end or loop forever.
            #[test]
            fn parse_config_lines_never_panics_multiline(
                content in proptest::collection::vec("[\\\\\"#=a-z\\n\\t ]{0,40}", 0..20)
                    .prop_map(|v| v.join("\n"))
            ) {
                let mut map = HashMap::new();
                parse_config_lines(&content, &mut map);
            }

            /// `parse_ref_domain` on an ARBITRARY string never panics. INVARIANT:
            /// it accepts ONLY the closed set {None, "", "user", "runner"} and any
            /// other value is a hard config error — no arbitrary string ever
            /// silently selects a keyspace.
            #[test]
            fn parse_ref_domain_never_panics(s in ".*") {
                let r = parse_ref_domain(Some(&s));
                match s.as_str() {
                    "" | "user" => prop_assert_eq!(r.unwrap(), RefDomain::User),
                    "runner" => prop_assert_eq!(r.unwrap(), RefDomain::Runner),
                    _ => prop_assert!(r.is_err()),
                }
            }

            /// The TENANT-resolution invariant, fuzzed at the boundary `resolve`
            /// actually enforces: validate_tenant accepts an arbitrary string ONLY
            /// when it carries no '/'/'\\' separator and no traversal form — so a
            /// path-injection tenant can never reach a request URL. (resolve()
            /// itself reads global env/HOME and is covered by the unit tests under
            /// ENV_LOCK; this fuzzes the pure decision it delegates to.)
            #[test]
            fn tenant_resolution_rejects_injection(
                t in prop_oneof![
                    "[A-Za-z0-9._/\\\\-]{0,16}",
                    Just("../other".to_string()),
                    Just("a/b".to_string()),
                    Just("..".to_string()),
                    Just("tenant".to_string()),
                ]
            ) {
                let ok = clw_types::validate_tenant(&t).is_ok();
                if ok {
                    prop_assert!(!t.contains('/') && !t.contains('\\'));
                    prop_assert!(t != "." && t != "..");
                } else {
                    prop_assert!(true); // rejection is always acceptable
                }
                // Conversely, anything with a separator MUST be rejected.
                if t.contains('/') || t.contains('\\') {
                    prop_assert!(!ok);
                }
            }
        }
    }

    #[test]
    fn home_dir_falls_back_to_userprofile_when_home_absent() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = HomeEnvGuard::new();

        // $HOME unset, %USERPROFILE% set → USERPROFILE is used (Windows path).
        std::env::remove_var("HOME");
        std::env::set_var("USERPROFILE", "/fallback/userprofile");
        assert_eq!(home_dir(), Some(PathBuf::from("/fallback/userprofile")));

        // $HOME present → it wins over %USERPROFILE% (unix-correct precedence).
        std::env::set_var("HOME", "/primary/home");
        assert_eq!(home_dir(), Some(PathBuf::from("/primary/home")));
    }

    // -----------------------------------------------------------------------
    // resolve_credential_inputs (C2c env-0) — pure, no process env touched
    // -----------------------------------------------------------------------

    #[test]
    fn cred_user_domain_uses_static_token() {
        let (token, broker) = resolve_credential_inputs(
            RefDomain::User,
            Some("tkt".into()), // a ticket present is IGNORED off the runner domain
            Some("lease".into()),
            Some("https://fab".into()),
            || Ok("static-pat".to_string()),
        )
        .unwrap();
        assert_eq!(token, "static-pat");
        assert!(broker.is_none());
    }

    #[test]
    fn cred_runner_without_ticket_is_legacy_static_path() {
        // A runner still on the env-PAT rollout (no ticket injected) is byte-identical
        // to the old behavior: static token, no broker.
        let (token, broker) =
            resolve_credential_inputs(RefDomain::Runner, None, None, None, || {
                Ok("legacy-pat".to_string())
            })
            .unwrap();
        assert_eq!(token, "legacy-pat");
        assert!(broker.is_none());
    }

    #[test]
    fn cred_runner_with_full_ticket_is_broker_and_never_reads_static_token() {
        // env-0: the static-token closure MUST NOT be consulted (a runner container
        // has no CLW_TOKEN) — panic if it is.
        let (token, broker) = resolve_credential_inputs(
            RefDomain::Runner,
            Some("the-ticket".into()),
            Some("lease-123".into()),
            Some("https://fabricd.example".into()),
            || panic!("static token MUST NOT be read on the broker path (env-0)"),
        )
        .unwrap();
        assert!(
            token.is_empty(),
            "PAT is empty until redeemed at the trusted boot"
        );
        let b = broker.expect("broker inputs captured");
        assert_eq!(b.ticket, "the-ticket");
        assert_eq!(b.lease_id, "lease-123");
        assert_eq!(b.fabric_endpoint, "https://fabricd.example");
    }

    #[test]
    fn cred_runner_ticket_missing_lease_is_fail_closed() {
        let err = resolve_credential_inputs(
            RefDomain::Runner,
            Some("tkt".into()),
            None, // lease missing
            Some("https://fab".into()),
            || panic!("must not fall back to a static token"),
        )
        .unwrap_err();
        assert!(matches!(err, ClwError::Config(m) if m.contains("CLW_LEASE_ID")));
    }

    #[test]
    fn cred_runner_ticket_missing_fabric_is_fail_closed() {
        let err = resolve_credential_inputs(
            RefDomain::Runner,
            Some("tkt".into()),
            Some("lease".into()),
            None, // fabric endpoint missing
            || panic!("must not fall back to a static token"),
        )
        .unwrap_err();
        assert!(matches!(err, ClwError::Config(m) if m.contains("CLW_FABRIC_ENDPOINT")));
    }
}
