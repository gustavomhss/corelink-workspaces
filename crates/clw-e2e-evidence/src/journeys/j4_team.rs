//! J4 — team + concurrency: two identities on ONE tenant, plus real concurrent writes.
//!
//! Identity A snapshots a workspace; identity B (a SECOND PAT, same tenant, cold cache)
//! hydrates it by name FROM the live CAS (cross-identity read) and re-snapshots the same
//! content to ZERO uploads (cross-identity dedup — B sees A's chunks). Then four GENUINELY
//! concurrent snapshots (real OS processes via scoped threads) prove the backend serializes
//! independent writes without corruption: four distinct roots, all exit 0.
//!
//! Needs `CLW_TOKEN_B` (identity B). Absent → the journey is skipped.
//!
//! Known minor gap: if a step PANICS mid-journey (an FS `expect` or a spawned-thread
//! panic), `guarded()` preserves the report but the cleanup `rm`s are skipped, so this
//! run's pid+nonce-unique refs are orphaned on the tenant. Names never collide, so this is
//! harmless litter swept by a periodic name-prefix cleanup, not a correctness issue.

use std::path::Path;

use super::{pseudo_bytes_seeded, ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner, Secret, StepEvidence};

const CONCURRENCY: usize = 4;

pub fn run(bin: &Path, creds: &ProdCreds, token_b: &Secret) -> Journey {
    let ws = Workspace::new("j4");
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seed = pid as u64 ^ nonce;
    let name_shared = format!("e2e-j4-shared-{pid}-{nonce}");
    let name_b = format!("e2e-j4-bdedup-{pid}-{nonce}");

    // A shared workspace identity A will create and identity B will read.
    let content = pseudo_bytes_seeded(4 * 1024 * 1024, seed);
    ws.write("src/data.bin", &content);
    let src = ws.dir("src");
    let dst = ws.dir("dst");
    let src_s = src.to_string_lossy().into_owned();
    let dst_s = dst.to_string_lossy().into_owned();

    let r_a = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-a"),
    );
    // Identity B: the SECOND PAT, same tenant. Its token is registered as a redaction so
    // it can never reach the report either.
    let r_b = Runner::new(
        bin,
        &creds.endpoint,
        "prod·B",
        &creds.tenant,
        token_b.clone(),
        ws.dir("cache-b"),
    );

    let mut j = Journey::new(
        "J4-team-concurrency",
        "two identities on one tenant (cross-identity read + dedup) and concurrent writes",
        format!("{} · 2 identities", creds.target()),
    );

    // 1. Identity A snapshots the shared workspace.
    let mut a = r_a.step(
        "identity A: snapshot the shared workspace",
        &["--json", "snapshot", &src_s, "--name", &name_shared],
        &[],
        ExpectExit::Eq(0),
    );
    let root_a = json_str(&a, "root");
    a.observe(
        Observation::behavior(
            "identity A creates the shared ref",
            "a team workflow starts with one member publishing a workspace",
        )
        .check(root_a.len() == 64, format!("root={}…", head12(&root_a))),
    );
    j.push(a);

    // 2. Identity B hydrates A's ref by name, on a COLD cache → served from the live CAS.
    let mut hb = r_b.step(
        "identity B: hydrate A's ref by name (cold cache)",
        &["--json", "hydrate", &dst_s, "--name", &name_shared],
        &[],
        ExpectExit::Eq(0),
    );
    let identical = std::fs::read(dst.join("data.bin"))
        .map(|got| got == content)
        .unwrap_or(false);
    let downloaded = json_u64(&hb, "bytes_downloaded").unwrap_or(0);
    hb.observe(
        Observation::security(
            "a second identity on the same tenant is GRANTED read access to the first's workspace",
            "same-tenant sharing across PATs must work; cross-TENANT DENIAL is proven separately in J7",
        )
        .check(
            identical && downloaded > 0,
            format!("byte-identical={identical}, bytes_downloaded={downloaded} (from live CAS)"),
        ),
    );
    j.push(hb);

    // 3. Identity B re-snapshots the SAME content (cold cache) → zero uploads: B sees the
    //    chunks A already stored (cross-identity, tenant-shared CAS dedup).
    let mut sb = r_b.step(
        "identity B: snapshot the same content (cross-identity dedup)",
        &["--json", "snapshot", &src_s, "--name", &name_b],
        &[],
        ExpectExit::Eq(0),
    );
    let uploaded_b = json_u64(&sb, "chunks_uploaded");
    let root_b = json_str(&sb, "root");
    sb.observe(
        Observation::behavior(
            "identity B uploads ZERO chunks and derives the same root as A",
            "content-addressing is tenant-wide: a teammate never re-stores bytes already in the CAS",
        )
        .check(
            uploaded_b == Some(0) && !root_a.is_empty() && root_b == root_a,
            format!("chunks_uploaded={uploaded_b:?}, root_B==root_A: {}", root_b == root_a),
        ),
    );
    j.push(sb);

    // 4. Four GENUINELY concurrent snapshots (real processes, scoped threads). The first
    //    SHARED_WRITERS threads write IDENTICAL content — they RACE to PUT the same chunks
    //    (the concurrent-dedup write-write path) and must CONVERGE on one root; the rest
    //    write distinct content and must stay distinct. "Distinct roots" alone is true by
    //    construction (the root is a client-side content hash) and proves nothing about the
    //    backend — the load-bearing signals are (a) the identical writers converging with no
    //    lost/clobbered write and (b) step 5 hydrating a concurrently-written ref back from
    //    the CAS byte-identically. Prep fixtures BEFORE the scope so threads only spawn clw.
    const SHARED_WRITERS: usize = 2;
    let shared = pseudo_bytes_seeded(1024 * 1024, seed.wrapping_add(100));
    let mut jobs: Vec<(String, String, std::path::PathBuf)> = Vec::with_capacity(CONCURRENCY);
    for i in 0..CONCURRENCY {
        let name = format!("e2e-j4-cc{i}-{pid}-{nonce}");
        let data = if i < SHARED_WRITERS {
            shared.clone()
        } else {
            pseudo_bytes_seeded(1024 * 1024, seed.wrapping_add(200 + i as u64))
        };
        let path = ws.write(&format!("cc{i}/data.bin"), &data);
        let dir = path.parent().unwrap().to_string_lossy().into_owned();
        jobs.push((name, dir, ws.dir(&format!("cache-cc{i}"))));
    }
    let results: Vec<StepEvidence> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .enumerate()
            .map(|(i, (name, dir, cache))| {
                scope.spawn(move || {
                    let r = Runner::new(
                        bin,
                        &creds.endpoint,
                        "prod",
                        &creds.tenant,
                        creds.token.clone(),
                        cache,
                    );
                    r.step(
                        format!("concurrent snapshot #{i}"),
                        &["--json", "snapshot", dir, "--name", name],
                        &[],
                        ExpectExit::Eq(0),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("snapshot thread"))
            .collect()
    });

    let roots: Vec<String> = results.iter().map(|s| json_str(s, "root")).collect();
    let all_ok = results.iter().all(StepEvidence::passed);
    // The identical-content writers must converge on ONE non-empty root (the write-write
    // dedup race resolved cleanly — no write lost or clobbered).
    let shared_root = roots.first().cloned().unwrap_or_default();
    let converged =
        !shared_root.is_empty() && roots[..SHARED_WRITERS].iter().all(|r| *r == shared_root);
    // The shared group plus each distinct writer must be mutually distinct root-groups.
    let mut groups: std::collections::HashSet<&String> = std::collections::HashSet::new();
    groups.insert(&roots[0]);
    for r in &roots[SHARED_WRITERS..] {
        groups.insert(r);
    }
    let expected_groups = 1 + (CONCURRENCY - SHARED_WRITERS);
    let distinct_ok = groups.iter().all(|r| !r.is_empty()) && groups.len() == expected_groups;
    for (i, ev) in results.into_iter().enumerate() {
        let pushed = j.push(ev);
        if i == CONCURRENCY - 1 {
            pushed.observe(
                Observation::behavior(
                    "concurrent writes are lossless: identical-content writers converge on one root, distinct writers stay distinct",
                    "parallel writes to one tenant must not interleave, clobber, or lose a write — the dedup write-write race must resolve cleanly",
                )
                .check(
                    all_ok && converged && distinct_ok,
                    format!(
                        "all exit0={all_ok}; {SHARED_WRITERS} identical writers converged={converged}; {} distinct root-groups (want {expected_groups})",
                        groups.len()
                    ),
                ),
            );
        }
    }

    // 5. Hydrate a CONCURRENTLY-WRITTEN ref back on a COLD cache — the definitive integrity
    //    check: a ref stored under the write-write race must reproduce byte-identically FROM
    //    the live CAS. (cc0 is a shared-writer, so its content is `shared`.)
    let dst2 = ws.dir("dst-verify");
    let dst2_s = dst2.to_string_lossy().into_owned();
    let r_v = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-verify"),
    );
    let mut hv = r_v.step(
        "hydrate a concurrently-written ref back (cold cache)",
        &["--json", "hydrate", &dst2_s, "--name", &jobs[0].0],
        &[],
        ExpectExit::Eq(0),
    );
    let verify_ok = std::fs::read(dst2.join("data.bin"))
        .map(|got| got == shared)
        .unwrap_or(false);
    let verify_dl = json_u64(&hv, "bytes_downloaded").unwrap_or(0);
    hv.observe(
        Observation::behavior(
            "a ref written under concurrency hydrates back byte-identically from the live CAS",
            "the definitive integrity check — concurrent writes must not corrupt what the CAS stores",
        )
        .check(
            verify_ok && verify_dl > 0,
            format!("byte-identical={verify_ok}, bytes_downloaded={verify_dl} (from live CAS)"),
        ),
    );
    j.push(hv);

    // 6. Cleanup — delete every ref this journey created (identity A owns the tenant view).
    let mut cleanup_names = vec![name_shared, name_b];
    for (name, _, _) in &jobs {
        cleanup_names.push(name.clone());
    }
    for name in &cleanup_names {
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

fn json_str(ev: &StepEvidence, key: &str) -> String {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default()
}

fn json_u64(ev: &StepEvidence, key: &str) -> Option<u64> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(serde_json::Value::as_u64)
}

fn head12(s: &str) -> String {
    s.chars().take(12).collect()
}
