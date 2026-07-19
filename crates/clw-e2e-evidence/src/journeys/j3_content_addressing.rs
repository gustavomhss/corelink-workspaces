//! J3 — content-addressing: dedup and pin-by-digest, proven against the live CAS.
//!
//! snapshot X under name-A (uploads all chunks) → snapshot the SAME X under name-B with
//! a COLD cache (uploads ZERO — the server already has every chunk: server-side dedup,
//! not a local-cache artifact) → hydrate by `--manifest-digest` (content-addressed pin,
//! no name lookup) and prove byte-identity. The content is nonce-seeded so it is globally
//! fresh (name-A is a real first upload) yet identical between A and B (so the roots match
//! and the second snapshot is a pure dedup).

use std::path::Path;

use super::{pseudo_bytes_seeded, ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner};

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let ws = Workspace::new("j3");
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Ref names include the nonce (not just pid): a run that crashes before cleanup must
    // not leave `e2e-j3a-<pid>` so a later PID-reused run collides create-only (409 → a
    // false RED). The nonce makes each run's ref names globally unique.
    let name_a = format!("e2e-j3a-{pid}-{nonce}");
    let name_b = format!("e2e-j3b-{pid}-{nonce}");

    // A multi-chunk blob SEEDED by the per-run nonce, so EVERY byte (thus every chunk) is
    // globally unique — name-A is a genuine fresh upload of all chunks, not a partial dedup
    // against a prior run's deterministic tail. A and B read the same file → identical root.
    let content = pseudo_bytes_seeded(6 * 1024 * 1024, pid as u64 ^ nonce as u64);
    ws.write("src/big.bin", &content);
    let small_content = format!("j3-{pid}-{nonce}\n").into_bytes();
    ws.write("src/small.txt", &small_content);
    let src = ws.dir("src");
    let dst = ws.dir("dst");
    let src_s = src.to_string_lossy().into_owned();
    let dst_s = dst.to_string_lossy().into_owned();

    // Two runners, SAME identity, DIFFERENT caches: the B snapshot runs cold so a zero
    // upload can only mean the server already has the content (server-side dedup).
    let r_a = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-a"),
    );
    let r_b = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-b"),
    );

    let mut j = Journey::new(
        "J3-content-addressing",
        "dedup across a cold cache + pin-by-manifest-digest, against the live CAS",
        creds.target(),
    );

    // 1. First snapshot — a genuine fresh upload of every chunk. Capture the root digest.
    let mut a = r_a.step(
        "snapshot X under name-A (fresh upload)",
        &["--json", "snapshot", &src_s, "--name", &name_a],
        &[],
        ExpectExit::Eq(0),
    );
    let root_a = json_str(&a, "root");
    let total_a = json_u64(&a, "chunks_total");
    let uploaded_a = json_u64(&a, "chunks_uploaded");
    a.observe(
        Observation::behavior(
            "the first snapshot uploads every chunk (nothing pre-existed)",
            "a globally-fresh workspace must be a real upload — the baseline for the dedup proof",
        )
        .check(
            total_a.unwrap_or(0) > 1 && uploaded_a == total_a,
            format!("chunks_uploaded={uploaded_a:?} of chunks_total={total_a:?}"),
        ),
    );
    j.push(a);

    // 2. Second snapshot of the SAME content, different name, COLD cache → zero upload.
    let mut b = r_b.step(
        "snapshot the SAME X under name-B (cold cache → dedup)",
        &["--json", "snapshot", &src_s, "--name", &name_b],
        &[],
        ExpectExit::Eq(0),
    );
    let root_b = json_str(&b, "root");
    let total_b = json_u64(&b, "chunks_total");
    let uploaded_b = json_u64(&b, "chunks_uploaded");
    b.observe(
        Observation::behavior(
            "identical content re-snapshotted cold uploads ZERO chunks (server-side dedup)",
            "content-addressing must not re-store bytes the CAS already holds — this is the storage-cost guarantee",
        )
        .check(
            uploaded_b == Some(0) && total_b == total_a,
            format!("chunks_uploaded={uploaded_b:?} of chunks_total={total_b:?} (A uploaded {uploaded_a:?})"),
        ),
    );
    b.observe(
        Observation::behavior(
            "identical content yields an identical root digest under a different name",
            "the root is a pure function of content — determinism is what makes dedup and pinning safe",
        )
        .check(
            !root_a.is_empty() && root_b == root_a,
            format!("root_B == root_A: {}", !root_a.is_empty() && root_b == root_a),
        ),
    );
    j.push(b);

    // 3. Hydrate by MANIFEST DIGEST (no name) on a THIRD, COLD cache — so the manifest
    //    and every chunk are pulled from the LIVE CAS by digest, not replayed from a warm
    //    local cache (r_b's cache was just populated by snapshot B). bytes_downloaded > 0
    //    is the proof the round-trip actually reached the server.
    let r_c = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-c"),
    );
    let mut h = r_c.step(
        "hydrate --manifest-digest <root-A> on a COLD cache (pin by digest, from live CAS)",
        &["--json", "hydrate", &dst_s, "--manifest-digest", &root_a],
        &[],
        ExpectExit::Eq(0),
    );
    let big_ok = std::fs::read(dst.join("big.bin"))
        .map(|got| got == content)
        .unwrap_or(false);
    let small_ok = std::fs::read(dst.join("small.txt"))
        .map(|got| got == small_content)
        .unwrap_or(false);
    let downloaded = json_u64(&h, "bytes_downloaded");
    h.observe(
        Observation::behavior(
            "a cold content-addressed hydrate pulls from the live CAS and round-trips byte-identically",
            "pinning to an immutable digest must reproduce the exact workspace FROM THE SERVER, not a warm local cache",
        )
        .check(
            big_ok && small_ok && downloaded.unwrap_or(0) > 0,
            format!(
                "big.bin match={big_ok}, small.txt match={small_ok}, bytes_downloaded={downloaded:?} (must be > 0)"
            ),
        ),
    );
    j.push(h);

    // 4. Cleanup — delete both refs (the CAS blobs stay; prune is J5's concern).
    for name in [&name_a, &name_b] {
        let rm = r_a.step(
            "rm the ref (cleanup)",
            &["--json", "rm", "--name", name, "--yes"],
            &[],
            ExpectExit::Eq(0),
        );
        j.push(rm);
    }

    j
}

fn json_str(ev: &crate::StepEvidence, key: &str) -> String {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default()
}

fn json_u64(ev: &crate::StepEvidence, key: &str) -> Option<u64> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(serde_json::Value::as_u64)
}
