//! J2 — the core lifecycle a user lives in, end to end against the live backend.
//!
//! snapshot → status(clean) → status(dirty) → ls(one) → ls(all) → hydrate → run(miss→hit)
//! → rm. Every step is the real binary; the evidence is the JSON shape, the exit codes
//! (status 0=clean/1=dirty is a contract), byte-identity on hydrate, and — the subtle
//! one — memoization proven from OUTSIDE: a `run` whose child appends to a marker file
//! runs the child exactly ONCE across two invocations (the second is a replayed hit).

use std::path::Path;

use super::{pseudo_bytes, ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner};

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let ws = Workspace::new("j2");
    let pid = std::process::id();
    let name = format!("e2e-j2-{pid}");

    // Fixture: a small text file + a deterministic 3 MiB blob (multi-chunk).
    let big = pseudo_bytes(3 * 1024 * 1024);
    ws.write("src/small.txt", format!("hello-{pid}\n").as_bytes());
    ws.write("src/big.bin", &big);
    let src = ws.dir("src");
    let dst = ws.dir("dst");
    let src_s = src.to_string_lossy().into_owned();
    let dst_s = dst.to_string_lossy().into_owned();

    let r = Runner::new(
        bin,
        &creds.endpoint,
        "prod",
        &creds.tenant,
        creds.token.clone(),
        ws.cache(),
    );
    let mut j = Journey::new(
        "J2-lifecycle",
        "the core lifecycle: snapshot, status, ls, hydrate, run(memoize), rm",
        creds.target(),
    );

    // 1. snapshot — populate the content-addressed store; capture the root digest.
    let mut snap = r.step(
        "snapshot a fresh workspace",
        &["--json", "snapshot", &src_s, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    let root = snap
        .json
        .as_ref()
        .and_then(|v| v.get("root"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let uploaded = snap
        .json
        .as_ref()
        .and_then(|v| v.get("chunks_uploaded"))
        .and_then(serde_json::Value::as_u64);
    snap.observe(
        Observation::behavior(
            "a fresh snapshot uploads chunks and returns a root digest",
            "the first snapshot must populate the CAS and name a content-addressed root",
        )
        .check(
            uploaded.unwrap_or(0) > 0 && root.len() == 64,
            format!(
                "chunks_uploaded={uploaded:?}, root={}…",
                &root.chars().take(12).collect::<String>()
            ),
        ),
    );
    j.push(snap);

    // 2. status right after snapshot — the working tree matches the remote ref → clean,
    //    exit 0. (status exit code is a contract: 0=clean, 1=dirty, 2=error.)
    let mut st_clean = r.step(
        "status (clean — matches remote)",
        &["--json", "status", &src_s, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    let clean = st_clean
        .json
        .as_ref()
        .and_then(|v| v.get("clean"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    st_clean.observe(
        Observation::behavior(
            "an unmodified tree reports clean (exit 0, clean=true)",
            "status is how a user/CI decides whether a re-snapshot is needed",
        )
        .check(clean, format!("clean={clean}")),
    );
    j.push(st_clean);

    // 3. status after a local edit — dirty, exit 1, the changed file named.
    ws.write("src/small.txt", format!("hello-{pid}-EDITED\n").as_bytes());
    let mut st_dirty = r.step(
        "status (dirty — after a local edit)",
        &["--json", "status", &src_s, "--name", &name],
        &[],
        ExpectExit::Eq(1),
    );
    let changed = st_dirty
        .json
        .as_ref()
        .and_then(|v| v.get("changed"))
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .any(|p| p.as_str().map(|s| s.contains("small.txt")).unwrap_or(false))
        })
        .unwrap_or(false);
    st_dirty.observe(
        Observation::behavior(
            "a modified tree reports dirty (exit 1) and names the changed file",
            "the exit-1 dirty signal is the contract a CI gate branches on",
        )
        .check(changed, format!("changed lists small.txt: {changed}")),
    );
    j.push(st_dirty);

    // 4. ls the ref — the stored root must equal the snapshot's root (identity).
    let mut ls_one = r.step(
        "ls --name (the stored ref)",
        &["--json", "ls", "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    let ls_root = ls_one
        .json
        .as_ref()
        .and_then(|v| v.get("root"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    ls_one.observe(
        Observation::behavior(
            "ls returns the same root digest the snapshot reported",
            "a ref must resolve to exactly the content that was stored under it",
        )
        .check(
            !root.is_empty() && ls_root == root,
            format!("ls root == snapshot root: {}", ls_root == root),
        ),
    );
    j.push(ls_one);

    // 5. ls --all — the ref enumeration includes our root (read-only, tolerant).
    let mut ls_all = r.step(
        "ls --all (enumerate refs)",
        &["--json", "ls", "--all"],
        &[],
        ExpectExit::Eq(0),
    );
    ls_all.observe(
        Observation::behavior(
            "the tenant-wide ref listing includes the ref we just created",
            "enumeration must surface a freshly-written ref, not a stale snapshot",
        )
        .check(
            !root.is_empty() && ls_all.stdout.contains(&root),
            format!(
                "listing contains our root: {}",
                ls_all.stdout.contains(&root)
            ),
        ),
    );
    j.push(ls_all);

    // 6. hydrate into an empty dest — byte-identity of the large blob is the core
    //    content-addressing guarantee.
    let mut hyd = r.step(
        "hydrate into an empty dest",
        &["--json", "hydrate", &dst_s, "--name", &name],
        &[],
        ExpectExit::Eq(0),
    );
    let identical = std::fs::read(dst.join("big.bin"))
        .map(|b| b == big)
        .unwrap_or(false);
    hyd.observe(
        Observation::behavior(
            "the hydrated blob is byte-identical to the source",
            "content-addressing must round-trip exactly or the store is unsafe",
        )
        .check(
            identical,
            if identical {
                "big.bin matches".to_string()
            } else {
                "MISMATCH".to_string()
            },
        ),
    );
    j.push(hyd);

    // 7. run (miss) — a child that appends to a marker, so a later hit is provable from
    //    OUTSIDE: the marker gains a line ONLY when the child actually executes. The
    //    input content is pid-unique so this is a guaranteed first-time miss, not a hit
    //    inherited from a prior run on the shared tenant.
    // A per-run nonce in the key CONTENT makes run #1 an unconditional miss: the memo
    // key is content-derived, so without it a pid reused within the AC retention window
    // could inherit a prior HIT (a loud false-negative, but avoidable). Written ONCE, so
    // both invocations read the SAME keyfile → run #2 is still a genuine hit.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let keyfile = ws.write("run/key.txt", format!("key-{pid}-{nonce}").as_bytes());
    let marker = ws.root_join("run/marker.txt");
    let key_s = keyfile.to_string_lossy().into_owned();
    let marker_s = marker.to_string_lossy().into_owned();
    let script = format!("printf 'tick\\n' >> '{marker_s}'; printf hello");
    let run_args = ["run", "--input", &key_s, "--", "sh", "-c", &script];

    let mut run_miss = r.step(
        "run (first — cache miss, executes)",
        &run_args,
        &[],
        ExpectExit::Eq(0),
    );
    let miss_signal = run_miss.stderr.contains("cache miss");
    let miss_stdout_hello = run_miss.stdout.trim() == "hello";
    run_miss.observe(
        Observation::behavior(
            "first run executes the child and reports a cache miss",
            "memoization must actually run the command the first time",
        )
        .check(
            miss_signal && miss_stdout_hello,
            format!(
                "cache-miss on stderr: {miss_signal}; child stdout==hello: {miss_stdout_hello}"
            ),
        ),
    );
    j.push(run_miss);

    // 8. run (hit) — identical invocation. The child MUST NOT re-execute: stdout is
    //    replayed and the marker is unchanged. Proven from outside by the line count.
    let mut run_hit = r.step(
        "run (second — cache hit, replays)",
        &run_args,
        &[],
        ExpectExit::Eq(0),
    );
    let hit_signal = run_hit.stderr.contains("cache hit");
    let hit_stdout_hello = run_hit.stdout.trim() == "hello";
    let ticks = std::fs::read_to_string(&marker)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    run_hit.observe(
        Observation::behavior(
            "second run replays from cache WITHOUT re-executing the child",
            "a cache hit must return the memoized output and cause no new side effect",
        )
        .check(
            hit_signal && hit_stdout_hello && ticks == 1,
            format!("cache-hit on stderr: {hit_signal}; replayed stdout==hello: {hit_stdout_hello}; child side-effects (marker lines) = {ticks} (must be 1)"),
        ),
    );
    j.push(run_hit);

    // 9. rm --yes — delete the ref (consent satisfied non-interactively).
    let mut rm = r.step(
        "rm --name --yes (cleanup)",
        &["--json", "rm", "--name", &name, "--yes"],
        &[],
        ExpectExit::Eq(0),
    );
    let deleted = rm
        .json
        .as_ref()
        .and_then(|v| v.get("deleted"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    rm.observe(
        Observation::behavior(
            "rm --yes deletes the ref non-interactively",
            "cleanup / automation must be able to delete without a TTY prompt",
        )
        .check(deleted, format!("deleted={deleted}")),
    );
    j.push(rm);

    j
}
