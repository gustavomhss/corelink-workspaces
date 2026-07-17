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
//! # Destructive safety
//! Only ever touches the sandbox/test tenant and the unique `ws-<suffix>` names
//! THIS suite creates, and best-effort deletes exactly those AC refs at the end.
//! It NEVER calls a tenant-wide `erase`/`prune`. (The destructive-verb journeys —
//! erase/prune against a throwaway tenant — are the one place a dedicated
//! `CLW_LIVE_ERASE_TENANT` is required; see the `erase`/`prune` section TODO.)
//!
//! Run:
//!   CLW_TENANT=<tenant> CLW_TOKEN=<pat> cargo test -p clw-conformance --features live
//! Two-identity (team) journey additionally needs a SECOND PAT on the SAME tenant:
//!   CLW_TOKEN_B=<pat2> …
//! Scheduled lane (skips become failures): CLW_CONFORMANCE_REQUIRE=1 …

#![cfg(feature = "live")]

use clw_client::HttpClient;
use clw_types::{AcTransport, ClwError, Config, Digest};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Shared live-gating harness (duplicated per-file: each tests/*.rs is its own
// crate, same as live_lifecycle.rs / live_conformance.rs).
// ---------------------------------------------------------------------------

/// Read runtime env into a `Config` for the PRIMARY identity. `None` (+ skip
/// notice) when `CLW_TOKEN`/`CLW_TENANT` is absent or empty.
fn try_config() -> Option<Config> {
    config_from(
        "CLW_TOKEN",
        "set CLW_TOKEN + CLW_TENANT to run the live journeys",
    )
}

/// Build a `Config` reading the PAT from `token_var` (so a second identity can be
/// supplied via `CLW_TOKEN_B`), sharing the same `CLW_TENANT`/`CLW_ENDPOINT`.
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

fn try_client() -> Option<(HttpClient, String)> {
    let cfg = try_config()?;
    let tenant = cfg.tenant.clone();
    Some((HttpClient::new(cfg).expect("build HttpClient"), tenant))
}

/// A SECOND identity (distinct PAT via `CLW_TOKEN_B`) on the SAME tenant, for the
/// team-share journey. Returns `None` (soft-skip, never a require-fail) when
/// `CLW_TOKEN_B` is absent — the second PAT is optional even under REQUIRE=1.
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

fn require_live() -> bool {
    matches!(
        std::env::var("CLW_CONFORMANCE_REQUIRE").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn skip_or_fail(reason: &str) {
    if require_live() {
        panic!("CONFORMANCE REQUIRED but a journey skipped: {reason} (CLW_CONFORMANCE_REQUIRE=1 forbids skips)");
    }
    eprintln!("SKIPPED: {reason}");
}

/// Unique per-run suffix (pid + full epoch nanos) so keys never collide between
/// invocations. (`subsec_nanos` would wrap every second — a real bug we avoid.)
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

/// Best-effort delete of an AC ref this suite created. Ignores every error
/// (idempotent / 405-501 / transient). NEVER touches anything but the given key.
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
    let warm = clw_hydrate::hydrate(&dest2, &client, &hcache, &hopts)
        .await
        .expect("hydrate #2 (warm cache) failed");
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
    let b = clw_snapshot::snapshot(src.path(), &client_b, &cache_b2, &opts_b)
        .await
        .expect("team: B snapshot failed");
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
// TODO — journeys that need a DEDICATED throwaway tenant / extra wiring (see
// docs/TEST-JOURNEY-MAP.md). Scaffolded here so the structure is explicit:
//   * erase/prune destructive journeys — require `CLW_LIVE_ERASE_TENANT` (a
//     tenant the suite fully owns and wipes) so we never touch shared data.
//   * quota 402 / storage 429 — drive the metered-probe tenant to its ceiling.
//   * cred-ticket (C2c) — redeem against the real fabric at boot.
//   * replica-lag false-orphan on prune — needs a controllable eventual-
//     consistency window (backend cooperation).
// ===========================================================================
