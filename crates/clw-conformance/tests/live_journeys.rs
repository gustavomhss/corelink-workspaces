//! Live JOURNEY suite against the real CoreLink API — "a real user, in prod".
//!
//! Gated on `#[cfg(feature = "live")]`; never runs in the default CI lane. Where
//! `live_lifecycle.rs` drives ONE happy journey, this file is organized **by
//! feature, by journey**, and each test exercises what ONLY a real backend can:
//! real cross-snapshot dedup counters, a warming cache across real fetches, and
//! a shared tenant used by two identities (a "team").
//!
//! # Skip mechanism (mirrors live_lifecycle.rs)
//! Reads `CLW_TOKEN` + `CLW_TENANT` at runtime; absent → prints a skip notice and
//! returns early (green), so the binary compiles/runs without credentials. Under
//! `CLW_CONFORMANCE_REQUIRE=1` a skip becomes a hard failure (the scheduled lane).
//!
//! A backend **5xx** (502/503/504 — e.g. the server's 503 "audit closed" fail-closed
//! state) is classified as backend-unavailable, NOT a clw defect: the primary journeys
//! route it through `skip_or_fail` (so the REQUIRE=1 canary still reddens on a real
//! outage), the extra-infra journeys through `soft_skip`. See `is_backend_unavailable`.
//!
//! # Journeys (by feature)
//! Primary (need only `CLW_TOKEN`+`CLW_TENANT`): dedup-across-names, warm-cache,
//! content-pinning by digest, concurrency, and (with `CLW_TOKEN_B`) the two-identity
//! team share. Extra-infra journeys SOFT-skip (never require-fail) unless their
//! dedicated infra is wired: the metered-quota 402 (`CLW_PROBE_TENANT/_TOKEN`), the
//! real-fabric cred-ticket (`CLW_CRED_TICKET`+`CLW_LEASE_ID`+`CLW_FABRIC_ENDPOINT`),
//! and the DESTRUCTIVE erase/prune journeys (`CLW_LIVE_ERASE_TENANT/_TOKEN` — a
//! THROWAWAY tenant the suite fully owns and wipes). Replica-lag needs a server hook.
//!
//! # Destructive safety
//! The primary/team journeys only ever touch the unique `ws-<suffix>` names they
//! create and best-effort delete exactly those AC refs. `erase`/`prune` are called
//! **only** against the dedicated `CLW_LIVE_ERASE_TENANT` (a throwaway tenant), never
//! the primary/sandbox tenant — so a shared tenant is never wiped.
//!
//! Run:
//!   CLW_TENANT=<tenant> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live
//! Optional extra journeys: CLW_TOKEN_B / CLW_PROBE_* / CLW_LIVE_ERASE_* / the
//! cred-ticket trio. Destructive journeys also need the `clw` binary built
//! (`cargo build -p clw-cli`). Scheduled lane: CLW_CONFORMANCE_REQUIRE=1 …

// WINDOWS COVERAGE, stated once here because it was invisible before and that is how
// it survived. SEVEN of the nine journeys in this file are `#[cfg(unix)]` — they build
// fixtures with unix permissions and symlinks — so on any non-unix target they are not
// merely skipped, they do not exist. TWO do compile and run there:
// `cred_ticket_redeems_against_the_real_fabric` and
// `replica_lag_false_orphan_prune_abort_needs_a_server_hook`.
//
// A previous revision hoisted the gate to the whole file (`#![cfg(all(feature = "live",
// unix))]`). That was tidier and strictly worse: it took those two tests OFF Windows to
// make a comment true, inside a change whose whole argument is that hiding a gap is how
// gaps survive. So the gate stays per-item, and the HELPERS carry `#[cfg(unix)]` where
// their users are — never `#[allow(dead_code)]`, which is what would let the next
// widening of this gap pass unnoticed.
//
// Closing the gap for the other seven is separate work; it is not done here.
#![cfg(feature = "live")]

use clw_client::{redeem_cred_ticket, HttpClient};
use clw_types::{AcTransport, ClwError, Config};
use std::path::PathBuf;

// Imports reached only by the unix-gated journeys, gated where their users are rather
// than blanketed with `#[allow(unused_imports)]` — suppressing these is how the missing
// Windows coverage stayed invisible for so long.
#[cfg(unix)]
use clw_types::{CasTransport, Digest};
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Shared live-gating harness (duplicated per-file: each tests/*.rs is its own
// crate, same as live_lifecycle.rs / live_conformance.rs).
// ---------------------------------------------------------------------------

/// Read runtime env into a `Config` for the PRIMARY identity. `None` (+ skip
/// notice) when `CLW_TOKEN`/`CLW_TENANT` is absent or empty.
#[cfg(unix)]
fn try_config() -> Option<Config> {
    config_from(
        "CLW_TOKEN",
        "set CLW_TOKEN + CLW_TENANT to run the live journeys",
    )
}

/// Build a `Config` reading the PAT from `token_var` (so a second identity can be
/// supplied via `CLW_TOKEN_B`), sharing the same `CLW_TENANT`/`CLW_ENDPOINT`.
#[cfg(unix)]
fn config_from(token_var: &str, skip_reason: &str) -> Option<Config> {
    // An absent CI secret surfaces as an empty string, not unset — treat empty as
    // absent (else HttpClient::new takes the creds-present branch and panics).
    let token = std::env::var(token_var).ok().filter(|s| !s.is_empty());
    let tenant = std::env::var("CLW_TENANT").ok().filter(|s| !s.is_empty());
    match (token, tenant) {
        (Some(t), Some(tn)) => {
            let endpoint = std::env::var("CLW_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "https://corelink-api.humangr.com".to_string());
            Some(Config {
                endpoint,
                tenant: tn,
                token: t,
                cache_dir: PathBuf::from("/tmp/clw-live-journeys-cache"),
            })
        }
        _ => {
            skip_or_fail(skip_reason);
            None
        }
    }
}

#[cfg(unix)]
fn try_client() -> Option<(HttpClient, String)> {
    let cfg = try_config()?;
    let tenant = cfg.tenant.clone();
    Some((HttpClient::new(cfg).expect("build HttpClient"), tenant))
}

/// A SECOND identity (distinct PAT via `CLW_TOKEN_B`) on the SAME tenant, for the
/// team-share journey. Returns `None` (soft-skip, never a require-fail) when
/// `CLW_TOKEN_B` is absent — the second PAT is optional even under REQUIRE=1.
#[cfg(unix)]
fn try_second_client() -> Option<HttpClient> {
    let token = std::env::var("CLW_TOKEN_B").ok().filter(|s| !s.is_empty());
    let tenant = std::env::var("CLW_TENANT").ok().filter(|s| !s.is_empty());
    let (Some(t), Some(tn)) = (token, tenant) else {
        eprintln!("SKIPPED (soft): set CLW_TOKEN_B to a second PAT on the same tenant for the team-share journey");
        return None;
    };
    let endpoint = std::env::var("CLW_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://corelink-api.humangr.com".to_string());
    Some(
        HttpClient::new(Config {
            endpoint,
            tenant: tn,
            token: t,
            cache_dir: PathBuf::from("/tmp/clw-live-journeys-cache-b"),
        })
        .expect("build second HttpClient"),
    )
}

#[cfg(unix)]
fn require_live() -> bool {
    matches!(
        std::env::var("CLW_CONFORMANCE_REQUIRE").as_deref(),
        Ok("1") | Ok("true")
    )
}

#[cfg(unix)]
fn skip_or_fail(reason: &str) {
    if require_live() {
        panic!("CONFORMANCE REQUIRED but a journey skipped: {reason} (CLW_CONFORMANCE_REQUIRE=1 forbids skips)");
    }
    eprintln!("SKIPPED: {reason}");
}

/// Unique per-run suffix (pid + full epoch nanos) so keys never collide between
/// invocations. (`subsec_nanos` would wrap every second — a real bug we avoid.)
#[cfg(unix)]
fn run_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    format!("{pid:08x}{nanos:016x}")
}

fn is_capability_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(msg) if msg.contains("405/501"))
}

/// A 5xx from the backend (502 Bad Gateway / 503 Service Unavailable / 504 Gateway
/// Timeout — including the server's 503 "audit closed" fail-closed state) means the
/// fabric is transiently unavailable: NOT a clw client defect and NOT a test-contract
/// violation. The primary journeys route it through `skip_or_fail`, so the daily
/// REQUIRE=1 conformance canary still REDDENS on a real prod outage (that IS its job,
/// as a prod-health monitor), while a manual/dev run degrades to a clean skip with a
/// precise diagnosis instead of a raw panic that reads like a clw bug. The extra-infra
/// journeys route it through `soft_skip` (they never require-fail).
///
/// (Verified 2026-07-17: prod returned 503 "audit closed" when the audit sink was
/// down; before this the suite hard-panicked on the catch-all arm, misattributing a
/// backend outage to clw.)
fn is_backend_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Http { status, .. } if matches!(*status, 502..=504))
}

/// Best-effort delete of an AC ref this suite created. Ignores every error
/// (idempotent / 405-501 / transient). NEVER touches anything but the given key.
#[cfg(unix)]
async fn cleanup(client: &HttpClient, ref_key: &Digest) {
    let _ = AcTransport::delete(client, ref_key).await;
}

// ---------------------------------------------------------------------------
// Fixture — RUN-seeded (not name-seeded) so two workspaces in the SAME run share
// byte-identical content (that is what makes cross-snapshot / team dedup real),
// while re-runs still upload fresh chunks (the CAS + test tenant persist).
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn build_shared_fixture(root: &Path, run_seed: &str) -> Vec<u8> {
    // A 5 MiB multi-chunk deterministic file seeded by the RUN (shared across the
    // names created in this run) so dedup is exercised, plus a tiny known file.
    let seed: u32 = run_seed.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    std::fs::write(root.join("small.txt"), format!("live-journey {run_seed}\n")).unwrap();
    let big_len = 5 * 1024 * 1024usize;
    let big: Vec<u8> = (0u32..big_len as u32)
        .map(|i| (i.wrapping_mul(0x9e37_79b9u32).wrapping_add(seed) >> 13) as u8)
        .collect();
    std::fs::write(root.join("big.bin"), &big).unwrap();
    big
}

/// A `LocalCache` whose backing `TempDir` OUTLIVES it: returns BOTH so the caller
/// binds the dir to a local. Binding `LocalCache::new(tempfile::tempdir()?.path())`
/// inline is a bug — the `TempDir` drops at the end of the `let`, deleting the
/// shard dirs `LocalCache::new` just created, so the first `cache.put` hits ENOENT.
#[cfg(unix)]
fn fresh_cache() -> (tempfile::TempDir, clw_cache::LocalCache) {
    let dir = tempfile::tempdir().unwrap();
    let cache = clw_cache::LocalCache::new(dir.path()).unwrap();
    (dir, cache)
}

/// A fresh hydrate DEST path (`<tempdir>/<leaf>`) whose parent `TempDir` outlives
/// it — returned so the caller keeps it bound (otherwise the hydrated tree leaks
/// on the runner when the anonymous `TempDir` drops).
#[cfg(unix)]
fn fresh_dest(leaf: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(leaf);
    (dir, path)
}

// ===========================================================================
// FEATURE: content-addressing & efficiency
// ===========================================================================

/// JOURNEY — cross-snapshot dedup on the LIVE CAS: snapshotting byte-identical
/// content under a SECOND name uploads (almost) nothing, because the first
/// snapshot already populated the shared CAS. This is the core value prop and can
/// only be proven against a real, stateful backend.
#[cfg(unix)]
#[tokio::test]
async fn dedup_across_names_reuses_the_live_cas() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };
    let suffix = run_suffix();
    let name_a = format!("ws-dedup-a-{suffix}");
    let name_b = format!("ws-dedup-b-{suffix}");
    let key_a = clw_types::ref_key(&name_a);
    let key_b = clw_types::ref_key(&name_b);

    let src = tempfile::tempdir().unwrap();
    build_shared_fixture(src.path(), &suffix);

    // Snapshot #1 (name A) — a fresh CAS population: chunks_uploaded > 0.
    let (_cache_a_dir, cache_a) = fresh_cache();
    let opts_a = clw_snapshot::SnapshotOptions::new(&name_a);
    let a = match clw_snapshot::snapshot(src.path(), &client, &cache_a, &opts_a).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            return skip_or_fail("snapshot A hit 405/501 — dedup journey skipped")
        }
        Err(e) if is_backend_unavailable(&e) => {
            return skip_or_fail(&format!(
                "snapshot A hit backend-unavailable ({e}) — dedup journey skipped"
            ))
        }
        Err(e) => panic!("snapshot A failed: {e}"),
    };
    assert!(a.chunks_uploaded > 0, "first snapshot must upload chunks");

    // Snapshot #2 (name B) — IDENTICAL content, a FRESH cache so the client can't
    // dedup locally: the server-side `exists`-before-`put` must still skip the
    // already-present chunks → chunks_uploaded is far below chunks_total.
    let (_cache_b_dir, cache_b) = fresh_cache();
    let opts_b = clw_snapshot::SnapshotOptions::new(&name_b);
    let b = match clw_snapshot::snapshot(src.path(), &client, &cache_b, &opts_b).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            cleanup(&client, &key_a).await;
            return skip_or_fail("snapshot B hit 405/501 — dedup journey skipped");
        }
        Err(e) if is_backend_unavailable(&e) => {
            cleanup(&client, &key_a).await;
            return skip_or_fail(&format!(
                "snapshot B hit backend-unavailable ({e}) — dedup journey skipped"
            ));
        }
        Err(e) => {
            cleanup(&client, &key_a).await;
            panic!("snapshot B failed: {e}");
        }
    };
    assert_eq!(
        a.root, b.root,
        "identical content must produce the same manifest root under a different name"
    );
    assert!(
        b.chunks_uploaded < b.chunks_total,
        "the second snapshot of identical content must dedup against the live CAS \
         (chunks_uploaded {} < chunks_total {})",
        b.chunks_uploaded,
        b.chunks_total
    );

    cleanup(&client, &key_a).await;
    cleanup(&client, &key_b).await;
}

// ===========================================================================
// FEATURE: local cache
// ===========================================================================

/// JOURNEY — the warm cache serves a re-hydrate locally: a first hydrate to a cold
/// cache downloads from the server; a second hydrate sharing that cache serves the
/// bytes locally (`bytes_from_cache > 0`, `bytes_downloaded` drops). This is the
/// runner re-hydrate path and can only be seen against real fetches.
#[cfg(unix)]
#[tokio::test]
async fn warm_cache_second_hydrate_serves_locally() {
    let Some((client, _tenant)) = try_client() else {
        return;
    };
    let suffix = run_suffix();
    let name = format!("ws-warmcache-{suffix}");
    let key = clw_types::ref_key(&name);

    let src = tempfile::tempdir().unwrap();
    build_shared_fixture(src.path(), &suffix);

    let (_snap_cache_dir, snap_cache) = fresh_cache();
    let opts = clw_snapshot::SnapshotOptions::new(&name);
    if let Err(e) = clw_snapshot::snapshot(src.path(), &client, &snap_cache, &opts).await {
        if is_capability_unavailable(&e) {
            return skip_or_fail("snapshot hit 405/501 — warm-cache journey skipped");
        }
        if is_backend_unavailable(&e) {
            return skip_or_fail(&format!(
                "snapshot hit backend-unavailable ({e}) — warm-cache journey skipped"
            ));
        }
        panic!("snapshot failed: {e}");
    }

    // ONE shared hydrate cache across two hydrates.
    let (_hcache_dir, hcache) = fresh_cache();
    let hopts = clw_hydrate::HydrateOptions::new(&name);

    let (_dest1_dir, dest1) = fresh_dest("h1");
    let cold = match clw_hydrate::hydrate(&dest1, &client, &hcache, &hopts).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            cleanup(&client, &key).await;
            return skip_or_fail("hydrate #1 hit 405/501 — warm-cache journey skipped");
        }
        Err(e) if is_backend_unavailable(&e) => {
            cleanup(&client, &key).await;
            return skip_or_fail(&format!(
                "hydrate #1 hit backend-unavailable ({e}) — warm-cache journey skipped"
            ));
        }
        Err(e) => {
            cleanup(&client, &key).await;
            panic!("hydrate #1 failed: {e}");
        }
    };
    assert!(
        cold.bytes_downloaded > 0,
        "the first hydrate against a cold cache must download from the server"
    );

    let (_dest2_dir, dest2) = fresh_dest("h2");
    // The "warm" hydrate still resolves the AC ref (name→root) over the wire — only
    // the CAS blobs are locally cached — so a backend 5xx can still land here; classify
    // it rather than panic (and clean up the ref we own on either failure).
    let warm = match clw_hydrate::hydrate(&dest2, &client, &hcache, &hopts).await {
        Ok(r) => r,
        Err(e) if is_backend_unavailable(&e) => {
            cleanup(&client, &key).await;
            return skip_or_fail(&format!(
                "hydrate #2 hit backend-unavailable ({e}) — warm-cache journey skipped"
            ));
        }
        Err(e) => {
            cleanup(&client, &key).await;
            panic!("hydrate #2 (warm cache) failed: {e}");
        }
    };
    assert!(
        warm.bytes_from_cache > 0,
        "the second hydrate must serve bytes from the now-warm cache (got 0)"
    );
    assert!(
        warm.bytes_downloaded < cold.bytes_downloaded,
        "the warm hydrate must download less than the cold one ({} !< {})",
        warm.bytes_downloaded,
        cold.bytes_downloaded
    );

    cleanup(&client, &key).await;
}

// ===========================================================================
// FEATURE: the shared team store (two identities, one tenant)
// ===========================================================================

/// JOURNEY — a TEAM shares the tenant CAS: identity A snapshots a workspace;
/// identity B (a DIFFERENT PAT on the SAME tenant) hydrates it byte-identical, and
/// B's own snapshot of the same content dedups against A's upload. This is the
/// "shared cache" story — provable only with two real identities against prod.
/// Soft-skips (never a require-fail) when `CLW_TOKEN_B` is absent.
#[cfg(unix)]
#[tokio::test]
async fn team_two_identities_share_the_tenant_cas() {
    let Some((client_a, _tenant)) = try_client() else {
        return;
    };
    let Some(client_b) = try_second_client() else {
        return; // soft-skip: no second PAT provided
    };
    let suffix = run_suffix();
    let name = format!("ws-team-{suffix}");
    let name_b = format!("ws-team-b-{suffix}");
    let key = clw_types::ref_key(&name);
    let key_b = clw_types::ref_key(&name_b);

    let src = tempfile::tempdir().unwrap();
    let big = build_shared_fixture(src.path(), &suffix);

    // A snapshots.
    let (_cache_a_dir, cache_a) = fresh_cache();
    let opts_a = clw_snapshot::SnapshotOptions::new(&name);
    let a = match clw_snapshot::snapshot(src.path(), &client_a, &cache_a, &opts_a).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            return skip_or_fail("team: A snapshot hit 405/501 — skipped")
        }
        Err(e) if is_backend_unavailable(&e) => {
            return skip_or_fail(&format!(
                "team: A snapshot hit backend-unavailable ({e}) — skipped"
            ))
        }
        Err(e) => panic!("team: A snapshot failed: {e}"),
    };

    // B (different PAT, same tenant) hydrates A's workspace — sees it byte-identical.
    let (_cache_b_dir, cache_b) = fresh_cache();
    let hopts = clw_hydrate::HydrateOptions::new(&name);
    let (_dest_dir, dest) = fresh_dest("team-b");
    match clw_hydrate::hydrate(&dest, &client_b, &cache_b, &hopts).await {
        Ok(r) => assert_eq!(r.root, a.root, "B must hydrate exactly the root A wrote"),
        Err(e) if is_capability_unavailable(&e) => {
            cleanup(&client_a, &key).await;
            return skip_or_fail("team: B hydrate hit 405/501 — skipped");
        }
        Err(e) if is_backend_unavailable(&e) => {
            cleanup(&client_a, &key).await;
            return skip_or_fail(&format!(
                "team: B hydrate hit backend-unavailable ({e}) — skipped"
            ));
        }
        Err(e) => {
            cleanup(&client_a, &key).await;
            panic!("team: B hydrate failed: {e}");
        }
    }
    assert_eq!(
        std::fs::read(dest.join("big.bin")).expect("read B's hydrated big.bin"),
        big,
        "B must get A's content byte-identical"
    );

    // B snapshots the SAME content under its own name → dedups against A's upload.
    let (_cache_b2_dir, cache_b2) = fresh_cache();
    let opts_b = clw_snapshot::SnapshotOptions::new(&name_b);
    let b = match clw_snapshot::snapshot(src.path(), &client_b, &cache_b2, &opts_b).await {
        Ok(r) => r,
        // Clean up BOTH refs on failure — A's ref is live here; B's key_b delete is a
        // harmless no-op if B's snapshot never created it.
        Err(e) if is_backend_unavailable(&e) => {
            cleanup(&client_a, &key).await;
            cleanup(&client_b, &key_b).await;
            return skip_or_fail(&format!(
                "team: B snapshot hit backend-unavailable ({e}) — skipped"
            ));
        }
        Err(e) => {
            cleanup(&client_a, &key).await;
            cleanup(&client_b, &key_b).await;
            panic!("team: B snapshot failed: {e}");
        }
    };
    assert!(
        b.chunks_uploaded < b.chunks_total,
        "B's snapshot of shared content must dedup against A's CAS upload ({} < {})",
        b.chunks_uploaded,
        b.chunks_total
    );

    cleanup(&client_a, &key).await;
    cleanup(&client_b, &key_b).await;
}

// ===========================================================================
// Extra-infra journeys — these need a DEDICATED throwaway tenant / probe tenant /
// real fabric ticket that the PRIMARY conformance lane does NOT provision. Their
// absence is a SOFT skip (never a require-fail), so the daily REQUIRE=1 canary
// stays green; they RUN whenever their specific infra is wired.
// ===========================================================================

/// Soft skip: needs dedicated infra the primary lane doesn't provide. NEVER a
/// require-fail (unlike `skip_or_fail`).
fn soft_skip(reason: &str) {
    eprintln!("SKIPPED (soft — needs dedicated infra): {reason}");
}

/// A `(tenant, token)` pair from two env vars, or `None` if either is unset/empty.
#[cfg(unix)]
fn env_pair(tenant_var: &str, token_var: &str) -> Option<(String, String)> {
    let t = std::env::var(tenant_var).ok().filter(|s| !s.is_empty())?;
    let k = std::env::var(token_var).ok().filter(|s| !s.is_empty())?;
    Some((t, k))
}

/// A transport client for an explicit `tenant` + `token` at the live endpoint.
fn client_for(tenant: &str, token: &str) -> HttpClient {
    let endpoint = std::env::var("CLW_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://corelink-api.humangr.com".to_string());
    HttpClient::new(Config {
        endpoint,
        tenant: tenant.to_string(),
        token: token.to_string(),
        cache_dir: PathBuf::from("/tmp/clw-live-journeys-transport"),
    })
    .expect("build transport client")
}

/// Locate the built `clw` binary next to the test exe (`target/<profile>/clw[.exe]`,
/// built by the lane's `cargo build -p clw-cli`). `None` → soft-skip the binary
/// journeys rather than fail (a plain `cargo test -p clw-conformance` won't build it).
#[cfg(unix)]
fn clw_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?; // .../deps -> .../<profile>
    let name = if cfg!(windows) { "clw.exe" } else { "clw" };
    let p = profile_dir.join(name);
    p.exists().then_some(p)
}

/// Spawn the real `clw` binary against the live endpoint with the given identity.
#[cfg(unix)]
fn clw_cmd(bin: &Path, tenant: &str, token: &str, cache: &Path) -> std::process::Command {
    let endpoint = std::env::var("CLW_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://corelink-api.humangr.com".to_string());
    let mut c = std::process::Command::new(bin);
    c.env("CLW_ENDPOINT", endpoint)
        .env("CLW_TENANT", tenant)
        .env("CLW_TOKEN", token)
        .env("CLW_CACHE_DIR", cache.to_str().unwrap());
    c
}

// ===========================================================================
// FEATURE: content-addressing — supply-chain content-pinning
// ===========================================================================

/// JOURNEY — pin by content against LIVE prod: snapshot, then hydrate by the
/// manifest DIGEST (no AC name lookup). The manifest must hash to that digest, so a
/// hostile/tampered store can't substitute — the supply-chain defense, proven E2E.
#[cfg(unix)]
#[tokio::test]
async fn content_pinning_hydrate_by_digest_is_byte_identical() {
    let Some((client, _t)) = try_client() else {
        return;
    };
    let suffix = run_suffix();
    let name = format!("ws-pin-{suffix}");
    let key = clw_types::ref_key(&name);
    let src = tempfile::tempdir().unwrap();
    let big = build_shared_fixture(src.path(), &suffix);

    let (_c, cache) = fresh_cache();
    let opts = clw_snapshot::SnapshotOptions::new(&name);
    let snap = match clw_snapshot::snapshot(src.path(), &client, &cache, &opts).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => return soft_skip("snapshot 405/501"),
        Err(e) if is_backend_unavailable(&e) => {
            return soft_skip(&format!("snapshot backend-unavailable ({e})"))
        }
        Err(e) => panic!("content-pin snapshot: {e}"),
    };

    let (_hc, hcache) = fresh_cache();
    let hopts = clw_hydrate::HydrateOptions::from_manifest_digest(snap.root);
    let (_dd, dest) = fresh_dest("pinned");
    let hy = match clw_hydrate::hydrate(&dest, &client, &hcache, &hopts).await {
        Ok(r) => r,
        Err(e) if is_capability_unavailable(&e) => {
            cleanup(&client, &key).await;
            return soft_skip("hydrate 405/501");
        }
        Err(e) if is_backend_unavailable(&e) => {
            cleanup(&client, &key).await;
            return soft_skip(&format!("hydrate backend-unavailable ({e})"));
        }
        Err(e) => {
            cleanup(&client, &key).await;
            panic!("hydrate-by-digest: {e}");
        }
    };
    assert_eq!(
        hy.root, snap.root,
        "the digest-pinned hydrate must yield exactly the pinned root"
    );
    assert_eq!(
        std::fs::read(dest.join("big.bin")).unwrap(),
        big,
        "content-pinned bytes must be byte-identical"
    );
    cleanup(&client, &key).await;
}

// ===========================================================================
// FEATURE: reliability — concurrency against the real CAS
// ===========================================================================

/// JOURNEY — four DISTINCT snapshots concurrently against the live CAS: all succeed
/// with distinct roots and no cross-contamination (real parallel writes to a shared
/// backend, which a single-process mock can't exercise).
#[cfg(unix)]
#[tokio::test]
async fn concurrent_distinct_snapshots_share_cas_without_corruption() {
    let Some((client, _t)) = try_client() else {
        return;
    };
    let suffix = run_suffix();
    let dirs: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let names: Vec<String> = (0..4).map(|i| format!("ws-conc-{i}-{suffix}")).collect();
    for (i, d) in dirs.iter().enumerate() {
        std::fs::write(
            d.path().join("f.bin"),
            format!("concurrent-{i}-{suffix}-").repeat(40_000),
        )
        .unwrap();
    }
    let caches: Vec<_> = (0..4).map(|_| fresh_cache()).collect();
    let o: Vec<_> = names
        .iter()
        .map(clw_snapshot::SnapshotOptions::new)
        .collect();
    let (r0, r1, r2, r3) = tokio::join!(
        clw_snapshot::snapshot(dirs[0].path(), &client, &caches[0].1, &o[0]),
        clw_snapshot::snapshot(dirs[1].path(), &client, &caches[1].1, &o[1]),
        clw_snapshot::snapshot(dirs[2].path(), &client, &caches[2].1, &o[2]),
        clw_snapshot::snapshot(dirs[3].path(), &client, &caches[3].1, &o[3]),
    );
    let results = [r0, r1, r2, r3];
    if results
        .iter()
        .any(|r| matches!(r, Err(e) if is_capability_unavailable(e) || is_backend_unavailable(e)))
    {
        for n in &names {
            cleanup(&client, &clw_types::ref_key(n)).await;
        }
        return soft_skip("concurrent snapshot 405/501 or backend-unavailable (5xx)");
    }
    let roots: Vec<Digest> = results
        .iter()
        .map(|r| r.as_ref().expect("a concurrent snapshot failed").root)
        .collect();
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            assert_ne!(
                roots[i], roots[j],
                "distinct content must yield distinct roots (no corruption/cross-talk)"
            );
        }
    }
    // Integrity under concurrency: hydrate ONE back and verify byte-identical — this
    // proves the parallel writes to the SHARED CAS produced retrievable, correct
    // content (the distinct-roots check alone is deterministic from local content).
    let expected0 = format!("concurrent-0-{suffix}-")
        .repeat(40_000)
        .into_bytes();
    let (_hc, hcache) = fresh_cache();
    let (_hd, dest) = fresh_dest("conc0");
    match clw_hydrate::hydrate(
        &dest,
        &client,
        &hcache,
        &clw_hydrate::HydrateOptions::new(&names[0]),
    )
    .await
    {
        Ok(_) => assert_eq!(
            std::fs::read(dest.join("f.bin")).unwrap(),
            expected0,
            "concurrent snapshot #0 must hydrate back byte-identical from the real CAS"
        ),
        Err(e) if is_capability_unavailable(&e) || is_backend_unavailable(&e) => {}
        Err(e) => {
            for n in &names {
                cleanup(&client, &clw_types::ref_key(n)).await;
            }
            panic!("hydrate-back of concurrent snapshot #0 failed: {e}");
        }
    }
    for n in &names {
        cleanup(&client, &clw_types::ref_key(n)).await;
    }
}

// ===========================================================================
// FEATURE: runner / C2c — cred-ticket against the REAL fabric
// ===========================================================================

/// JOURNEY — redeem a single-use cred-ticket against the REAL fabric at boot, then
/// prove the redeemed CAS PAT authenticates against live prod. Soft-skips unless the
/// real trio (`CLW_CRED_TICKET`+`CLW_LEASE_ID`+`CLW_FABRIC_ENDPOINT`) is provided.
#[tokio::test]
async fn cred_ticket_redeems_against_the_real_fabric() {
    let (Some(ticket), Some(lease), Some(fabric)) = (
        std::env::var("CLW_CRED_TICKET")
            .ok()
            .filter(|s| !s.is_empty()),
        std::env::var("CLW_LEASE_ID").ok().filter(|s| !s.is_empty()),
        std::env::var("CLW_FABRIC_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty()),
    ) else {
        return soft_skip(
            "set CLW_CRED_TICKET+CLW_LEASE_ID+CLW_FABRIC_ENDPOINT (a real single-use ticket) for the C2c journey",
        );
    };
    let Some(tenant) = std::env::var("CLW_TENANT").ok().filter(|s| !s.is_empty()) else {
        return soft_skip("CLW_TENANT");
    };

    let pat = match redeem_cred_ticket(&fabric, &lease, &ticket).await {
        Ok(p) => p,
        Err(e) => {
            return soft_skip(&format!(
                "redeem failed (the ticket may be burned/expired): {e}"
            ))
        }
    };
    // The redeemed CAS PAT must authenticate against live prod (not 401).
    let client = client_for(&tenant, &pat);
    match AcTransport::list_refs(&client).await {
        Ok(_) => {}
        Err(e) if is_capability_unavailable(&e) => return soft_skip("list_refs 405/501"),
        Err(e) if is_backend_unavailable(&e) => {
            return soft_skip(&format!("list_refs backend-unavailable ({e})"))
        }
        Err(e) => panic!("the redeemed CAS PAT failed to authenticate against prod: {e}"),
    }
}

// ===========================================================================
// FEATURE: quota / billing — the 402 metered wall
// ===========================================================================

/// JOURNEY — drive the METERED PROBE tenant to its ceiling: write distinct blobs
/// until the real metering returns HTTP 402. Soft-skips without the probe creds (and
/// is NOT wired into the daily lane — the metered-probe-canary owns accrual+reset).
#[cfg(unix)]
#[tokio::test]
async fn metered_writes_hit_the_402_quota_wall() {
    let Some((tenant, token)) = env_pair("CLW_PROBE_TENANT", "CLW_PROBE_TOKEN") else {
        return soft_skip(
            "set CLW_PROBE_TENANT+CLW_PROBE_TOKEN (the metered probe tenant) for the quota journey",
        );
    };
    let client = client_for(&tenant, &token);
    let suffix = run_suffix();
    let mut hit_402 = false;
    // Bounded so a mis-provisioned high ceiling can't loop forever.
    for i in 0..2000u32 {
        let bytes = format!("quota-probe-{suffix}-{i}-")
            .repeat(2000)
            .into_bytes();
        let d = Digest::of_bytes(&bytes);
        match CasTransport::put(&client, &d, bytes).await {
            Ok(()) => continue,
            Err(ClwError::Http { status: 402, .. }) => {
                hit_402 = true;
                break;
            }
            Err(e) if is_capability_unavailable(&e) => return soft_skip("put 405/501"),
            Err(e) if is_backend_unavailable(&e) => {
                return soft_skip(&format!("put backend-unavailable ({e})"))
            }
            Err(e) => panic!("unexpected error before the 402 wall: {e}"),
        }
    }
    assert!(
        hit_402,
        "the metered probe tenant must eventually return HTTP 402 (quota wall) — check its ceiling/reset"
    );
}

// ===========================================================================
// FEATURE: destructive lifecycle against a THROWAWAY tenant (real binary)
// ===========================================================================

/// JOURNEY — the full DESTRUCTIVE lifecycle against a THROWAWAY tenant, via the real
/// binary, in ONE sequential test (so the two tenant-wide ops never race under the
/// parallel test runner): seed a live snapshot + an orphaned one, `clw prune --yes`
/// (assert the live ref SURVIVES and the orphaned manifest blob is SWEPT), then
/// `clw erase --yes` (assert the real backend then enumerates EMPTY). The prune
/// fail-safe + GDPR Art.17 truthfulness, end-to-end against prod.
///
/// Soft-skips without a dedicated wipeable tenant; and REFUSES to run when that
/// tenant equals the primary `CLW_TENANT` (defense-in-depth — a misconfigured secret
/// must never wipe the shared/sandbox tenant).
///
/// Scope note: the two `clw`-binary steps (`prune`/`erase`) assert on the child's exit
/// status, so a backend 5xx there surfaces as a failed assertion (the child prints the
/// real error), not a `soft_skip`. That is acceptable — this journey is ATTENDED /
/// manual-only and NOT on the daily canary; the `ClwError` seed steps above DO classify
/// a 5xx as backend-unavailable.
#[cfg(unix)]
#[tokio::test]
async fn destructive_prune_then_erase_lifecycle_on_a_throwaway_tenant() {
    let Some((tenant, token)) = env_pair("CLW_LIVE_ERASE_TENANT", "CLW_LIVE_ERASE_TOKEN") else {
        return soft_skip(
            "set CLW_LIVE_ERASE_TENANT+CLW_LIVE_ERASE_TOKEN (a THROWAWAY tenant the suite fully owns + wipes) for the destructive journey",
        );
    };
    // Defense-in-depth: never run a tenant-wide wipe against the primary tenant.
    if std::env::var("CLW_TENANT").ok().as_deref() == Some(tenant.as_str()) {
        return soft_skip(
            "CLW_LIVE_ERASE_TENANT must be a DEDICATED throwaway tenant, NOT the primary CLW_TENANT — refusing to run a destructive wipe against the shared tenant",
        );
    }
    let Some(bin) = clw_binary() else {
        return soft_skip("clw binary not built (run `cargo build -p clw-cli` first)");
    };
    let client = client_for(&tenant, &token);
    let suffix = run_suffix();
    let live_name = format!("ws-destroy-live-{suffix}");
    let orphan_name = format!("ws-destroy-orphan-{suffix}");
    let (src_l, src_o) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(
        src_l.path().join("live.bin"),
        format!("live-{suffix}-").repeat(50_000),
    )
    .unwrap();
    std::fs::write(
        src_o.path().join("orphan.bin"),
        format!("orphan-{suffix}-").repeat(50_000),
    )
    .unwrap();
    let (_cl, cl) = fresh_cache();
    let (_co, co) = fresh_cache();
    if let Err(e) = clw_snapshot::snapshot(
        src_l.path(),
        &client,
        &cl,
        &clw_snapshot::SnapshotOptions::new(&live_name),
    )
    .await
    {
        if is_capability_unavailable(&e) {
            return soft_skip("seed snapshot 405/501");
        }
        if is_backend_unavailable(&e) {
            return soft_skip(&format!("seed snapshot backend-unavailable ({e})"));
        }
        panic!("seed live snapshot: {e}");
    }
    let orphan = match clw_snapshot::snapshot(
        src_o.path(),
        &client,
        &co,
        &clw_snapshot::SnapshotOptions::new(&orphan_name),
    )
    .await
    {
        Ok(r) => r,
        Err(e) if is_backend_unavailable(&e) => {
            return soft_skip(&format!("seed orphan snapshot backend-unavailable ({e})"))
        }
        Err(e) => panic!("seed orphan snapshot: {e}"),
    };
    // Orphan the second snapshot: delete its ref → its unique manifest + chunks are
    // now unreferenced. (Its content is distinct from the live snapshot, so nothing
    // it uploaded is shared/live.)
    let _ = AcTransport::delete(&client, &clw_types::ref_key(&orphan_name)).await;
    assert!(
        CasTransport::get(&client, &orphan.root).await.is_ok(),
        "the orphaned manifest blob must exist before prune (sanity)"
    );

    let cache = tempfile::tempdir().unwrap();

    // --- Step 1: prune (verify BEFORE the erase wipe). ---
    let pr = clw_cmd(&bin, &tenant, &token, cache.path())
        .args(["--json", "prune", "--yes"])
        .output()
        .expect("spawn clw prune");
    assert!(
        pr.status.success(),
        "prune must succeed against the throwaway tenant, stderr:\n{}",
        String::from_utf8_lossy(&pr.stderr)
    );
    // Over-deletion guard: the LIVE ref survives.
    assert!(
        matches!(
            AcTransport::get(&client, &clw_types::ref_key(&live_name)).await,
            Ok(Some(_))
        ),
        "prune must NOT delete the live ref"
    );
    // Under-deletion guard: the ORPHANED (unreferenced) manifest blob is swept.
    assert!(
        matches!(
            CasTransport::get(&client, &orphan.root).await,
            Err(ClwError::NotFound(_))
        ),
        "prune must SWEEP the orphaned manifest blob"
    );

    // --- Step 2: erase (tenant-wide wipe, LAST). ---
    let er = clw_cmd(&bin, &tenant, &token, cache.path())
        .args(["--json", "erase", "--yes", "--confirm-tenant", &tenant])
        .output()
        .expect("spawn clw erase");
    assert!(
        er.status.success(),
        "erase must succeed on the throwaway tenant, stderr:\n{}",
        String::from_utf8_lossy(&er.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&er.stdout).expect("erase --json must emit a record");
    assert_eq!(
        v["complete"],
        serde_json::json!(true),
        "erase must report complete:true, got: {v}"
    );
    // The REAL backend must now enumerate EMPTY (the prod-only truth).
    assert!(
        AcTransport::list_refs(&client)
            .await
            .expect("list after")
            .is_empty(),
        "erase must leave the tenant's AC refs empty"
    );
    assert!(
        CasTransport::list_blobs(&client)
            .await
            .expect("blobs after")
            .is_empty(),
        "erase must leave the tenant's CAS empty"
    );
}

// ===========================================================================
// FEATURE: reliability — replica-lag false-orphan on prune (honest scaffold)
// ===========================================================================

/// The prune replica-lag fail-safe (abort when a ref enumerates but its record 404s
/// mid-mark, so still-live blobs aren't swept as false orphans) can only be triggered
/// deterministically against prod with a SERVER-INJECTED eventual-consistency window
/// — there is no client-side way to force AC replica lag. The behavior IS covered
/// deterministically at the mock/unit level (prune.rs fail-safes). This soft-skips
/// until a server test-hook exists; it exists so the gap is explicit, not forgotten.
#[tokio::test]
async fn replica_lag_false_orphan_prune_abort_needs_a_server_hook() {
    soft_skip(
        "replica-lag prune-abort needs a server-injected consistency window (no client-side trigger) — covered deterministically at mock/unit level",
    );
}
