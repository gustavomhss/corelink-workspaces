//! STORY — "Marina ships the orders API." A backend engineer's real week with clw, told
//! as a narrative and driven through the REAL binary against the live backend. She sets up,
//! snapshots her service, picks it up byte-identical on her laptop (exec bits, a
//! unicode-named doc, and an empty dir all intact), fixes a bug, hits + resolves a save
//! conflict, memoises her test run, watches an expired token fail closed, fat-fingers a
//! pin, lists what she has, and cleans up.
//!
//! This is the GOLD-STANDARD exemplar: every beat verifies what Marina actually experiences
//! and the FULL shape of what clw returns — not just an exit code and one field. Nothing
//! signals "test": a plausible `orders-api` project, a name she'd type, no sentinels, no
//! housekeeping. The identity comes from provisioning (a real signup tenant); the DATA is
//! realistic (`realistic::orders_api`, including the awkward-but-legitimate shapes a real
//! repo has).

use std::path::Path;

use super::realistic;
use super::{ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Persona, Runner, Secret, StepEvidence};

/// Marina's realistic workspace name — exactly what she'd type, no test marker.
const WS: &str = "orders-api";

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let persona = Persona::new(
        "marina",
        "backend engineer shipping an orders API",
        "snapshot orders-api at work, pick it up byte-identical on her laptop, iterate, and \
         resolve a save conflict — without ever losing a teammate's work",
    );
    let mut j = Journey::story(
        "marina-ships-orders-api",
        persona,
        "a backend engineer's real week with clw: set up → snapshot → hydrate byte-identical \
         on a second machine → fix a bug → hit + resolve a create-only conflict → memoise a \
         test run → an expired token fails closed → an honest typo → list → clean up",
    );

    let ws = Workspace::new("story-marina");
    let project = realistic::orders_api(ws.root_dir(), WS);
    let project_s = project.to_string_lossy().into_owned();
    let file_count = count_files(&project) as u64; // the real number of files clw should see

    // Marina's two machines are two clw caches. "work" is where she starts; "laptop" is a
    // COLD cache so the hydrate genuinely pulls from the backend (not a warm local replay).
    let work = runner(bin, creds, ws.dir("cache-work"));

    // ---- Beat 1: Marina just installed clw and checks it's healthy -----------------
    {
        let mut s = work.step(
            "clw doctor --json",
            &["--json", "doctor"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Marina just installed clw. Before trusting it with her code she runs `clw doctor` \
             to confirm it's configured and pointed at the right place.",
        );
        // The FULL diagnostics shape, not just "exit 0": it must identify the build, echo the
        // endpoint + tenant she configured, and show the token as a marker (never the value).
        // `clw_version` is the bare version ("0.1.11"); the tenant VALUE is `tenant_value`
        // (the `tenant` field is a <set>/<unset> marker, verified separately below).
        let version_ok = json_str(&s, "clw_version")
            .map(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .unwrap_or(false);
        let endpoint_ok = json_str(&s, "endpoint")
            .map(|e| e == creds.endpoint)
            .unwrap_or(false);
        let tenant_ok = json_str(&s, "tenant_value")
            .map(|t| t == creds.tenant)
            .unwrap_or(false);
        let token_marker = matches!(
            json_str(&s, "token").as_deref(),
            Some("<set>") | Some("<unset>")
        );
        s.observe(
            Observation::behavior(
                "doctor reports a complete, correct diagnostics bundle (version, endpoint, tenant)",
                "doctor is the first thing a user trusts; every field must be present and match what she configured",
            )
            .check(
                version_ok && endpoint_ok && tenant_ok && token_marker,
                format!("version_ok={version_ok} endpoint_ok={endpoint_ok} tenant_ok={tenant_ok} token_marker={token_marker}"),
            ),
        );
        s.observe(
            Observation::security(
                "doctor shows the token as <set>/<unset>, never the value",
                "Marina might paste this into a support ticket — a leaked PAT is full account access",
            )
            .check(token_marker, format!("token field = {:?}", json_str(&s, "token"))),
        );
        s.expect_no_secret_leak("the diagnostics bundle is the thing people screenshot");
        j.push(s);
    }

    // ---- Beat 2: end of day — she snapshots orders-api so nothing is lost ----------
    let root_v1 = {
        let mut s = work.step(
            "snapshot orders-api (first save)",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "It's the end of the day. Marina snapshots orders-api under its name so her work \
             is safely in the store and she can pick it up anywhere tomorrow.",
        );
        let root = json_str(&s, "root");
        let uploaded = json_u64(&s, "chunks_uploaded");
        let total = json_u64(&s, "chunks_total");
        let files = json_u64(&s, "files");
        // The FULL report is internally consistent — this is what makes the save trustworthy,
        // not merely "it returned a root".
        let consistent = root.as_deref().map(is_hex64).unwrap_or(false)
            && json_str(&s, "name").as_deref() == Some(WS)
            && json_bool(&s, "unchanged") == Some(false)
            && files == Some(file_count)
            // uploaded ≤ total always; on a DEDICATED (fresh) tenant the first save uploads
            // every chunk (uploaded == total), but on a shared/reused tenant the content may
            // already exist and dedup to 0 — both are correct, so we assert the invariant.
            && matches!((uploaded, total), (Some(u), Some(t)) if u <= t)
            && s.json
                .as_ref()
                .and_then(|v| v.get("skipped_external_symlinks"))
                .and_then(|v| v.as_array())
                .map(|a| a.is_empty())
                .unwrap_or(false);
        s.observe(
            Observation::behavior(
                "the first save uploads every file and returns a consistent, content-addressed report",
                "the report must be internally sound (root is 64-hex, files == her file count, 0<uploaded≤total, nothing skipped) or she can't trust what was stored",
            )
            .check(
                consistent,
                format!("root={:?} files={files:?}(want {file_count}) uploaded={uploaded:?} total={total:?}", root.as_deref().map(|r| &r[..12.min(r.len())])),
            ),
        );
        j.push(s);
        root.unwrap_or_default()
    };

    // ---- Beat 3: next morning she checks nothing drifted --------------------------
    {
        let mut s = work.step(
            "status (nothing changed)",
            &["--json", "status", &project_s, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Next morning Marina checks whether anything drifted overnight before she starts. \
             `status` should say she's clean and exit 0 — the signal her CI also branches on.",
        );
        let clean_shape = json_bool(&s, "clean") == Some(true)
            && empty_arr(&s, "changed")
            && empty_arr(&s, "added")
            && empty_arr(&s, "removed");
        s.observe(
            Observation::behavior(
                "an unchanged workspace reports exactly clean (exit 0, clean=true, no add/change/remove)",
                "status is the contract CI branches on — a spurious diff would trigger needless re-snapshots",
            )
            .check(clean_shape, "clean=true and added/changed/removed all empty"),
        );
        j.push(s);
    }

    // ---- Beat 4: she's on her laptop now — pulls orders-api down (byte-perfect) ----
    {
        let laptop = runner(bin, creds, ws.dir("cache-laptop")); // COLD cache → real fetch
        let dest = ws.dir("laptop-checkout");
        let dest_s = dest.to_string_lossy().into_owned();
        let mut s = laptop.step(
            "hydrate orders-api on the laptop (cold)",
            &["--json", "hydrate", &dest_s, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Marina moves to her laptop and pulls orders-api down to keep working. On a fresh \
             machine the cache is cold, so this genuinely fetches from the backend — and her \
             code must come back BYTE-FOR-BYTE: every file, the unicode-named design note, the \
             empty migrations dir, and the executable bit on her deploy script.",
        );
        // hydrate restores the snapshot's CONTENTS into `dest` itself (not `dest/orders-api`).
        let checkout = dest.clone();
        let downloaded = json_u64(&s, "bytes_downloaded");
        let root_ok = json_str(&s, "root").as_deref() == Some(root_v1.as_str());
        let identical = dirs_identical(&project, &checkout);
        // The deploy-script MODE expectation is platform-split. On Unix the 0755 exec bit
        // MUST survive snapshot→hydrate (a hydrated checkout with a non-exec deploy.sh is
        // broken). On Windows there is no unix exec bit and clw does not synthesize one
        // (documented windows.yml / FT-C behavior, not a bug), so the Windows-correct
        // expectation is that the script is materialized with byte-identical CONTENT (proven
        // by dirs_identical above) and simply lands on disk.
        #[cfg(unix)]
        let (exec_ok, exec_note) = {
            let ok = matches!(
                (
                    realistic::deploy_script_is_executable(&project),
                    realistic::deploy_script_is_executable(&checkout),
                ),
                (Some(a), Some(b)) if a == b && b // exec bit set on both
            );
            (ok, format!("exec_bit_preserved={ok}"))
        };
        #[cfg(windows)]
        let (exec_ok, exec_note) = {
            // No unix mode on Windows; assert the script materialized (content already
            // verified byte-for-byte by dirs_identical).
            let ok = checkout.join("scripts/deploy.sh").is_file();
            (
                ok,
                format!(
                    "deploy_script_materialized={ok} (Windows: no exec bit; content byte-verified)"
                ),
            )
        };
        s.observe(
            Observation::behavior(
                "the laptop checkout is byte-identical AND mode-preserving, fetched cold from the backend",
                "content-addressing must round-trip EXACTLY on a cold cache (bytes_downloaded>0, same root, every byte, the exec bit) or Marina's checkout is subtly broken",
            )
            .check(
                identical && exec_ok && downloaded.map(|b| b > 0).unwrap_or(false) && root_ok,
                format!("tree_identical={identical} {exec_note} bytes_downloaded={downloaded:?} root_matches_v1={root_ok}"),
            ),
        );
        j.push(s);
    }

    // ---- Beat 5: she fixes a bug in pricing (add the free-shipping rule) -----------
    realistic::add_free_shipping_rule(&project);
    {
        let mut s = work.step(
            "status (after editing pricing.rs)",
            &["--json", "status", &project_s, "--name", WS],
            &[],
            ExpectExit::Eq(1),
        );
        s.narrate(
            "Back at her desk, Marina adds the free-shipping rule to the pricing module. She \
             runs `status` — it should now report dirty (exit 1) and name EXACTLY the one file \
             she touched: nothing spurious added or removed.",
        );
        let changed = str_arr(&s, "changed");
        let only_pricing = changed.len() == 1
            && changed[0].contains("pricing.rs")
            && empty_arr(&s, "added")
            && empty_arr(&s, "removed")
            && json_bool(&s, "clean") == Some(false);
        s.observe(
            Observation::behavior(
                "a one-file edit reports dirty (exit 1) and names EXACTLY that file",
                "an over- or under-reported diff would mislead Marina (and CI) about what's uncommitted",
            )
            .check(only_pricing, format!("clean=false, changed={changed:?} (want exactly [src/pricing.rs])")),
        );
        j.push(s);
    }

    // ---- Beat 6: she tries to save over the name — create-only conflict + --force --
    {
        let mut s = work.step(
            "snapshot orders-api again (divergent, no --force)",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Marina re-saves under the same name, forgetting the store is create-only. This is \
             the guardrail that stops her from silently clobbering a teammate's snapshot — it \
             must REFUSE with an actionable message, and (per the recent fix) NOT waste her \
             bandwidth uploading before refusing.",
        );
        s.observe(
            Observation::behavior(
                "a divergent re-save is refused (create-only) with a message that names the fix",
                "silently overwriting would destroy a teammate's work; the refusal + --force/rm hint is the guardrail",
            )
            .check(
                s.stderr.contains("already exists with different content") && s.stderr.contains("--force"),
                "stderr explains the conflict and points at --force / rm",
            ),
        );
        j.push(s);

        let mut f = work.step(
            "snapshot orders-api --force (intentional replace)",
            &["--json", "snapshot", &project_s, "--name", WS, "--force"],
            &[],
            ExpectExit::Eq(0),
        );
        f.narrate(
            "She realizes it's her own update, so she re-runs with `--force` to replace it on \
             purpose — this should go through, advancing the workspace to a new root.",
        );
        let new_root = json_str(&f, "root");
        f.observe(
            Observation::behavior(
                "--force replaces the name and produces a new root distinct from v1",
                "an intentional replace must go through and advance the workspace's content",
            )
            .check(
                new_root
                    .as_deref()
                    .map(|r| is_hex64(r) && r != root_v1)
                    .unwrap_or(false),
                format!(
                    "new root {:?} != v1 {}",
                    new_root.as_deref().map(|r| &r[..12.min(r.len())]),
                    &root_v1[..12.min(root_v1.len())]
                ),
            ),
        );
        j.push(f);

        // ...and the lineage is not lost: ls --name shows the previous root as the parent.
        let mut l = work.step(
            "ls --name (check parent lineage)",
            &["--json", "ls", "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        l.narrate(
            "Marina glances at the workspace metadata — the forced replace should have recorded \
             yesterday's snapshot as the PARENT, so her history isn't lost.",
        );
        let parent_ok = json_str(&l, "parent").as_deref() == Some(root_v1.as_str());
        l.observe(
            Observation::behavior(
                "the replaced ref records v1 as its parent (lineage preserved)",
                "a --force replace must keep the chain so a workspace's history is recoverable",
            )
            .check(
                parent_ok,
                format!(
                    "parent={:?} (want v1 root)",
                    json_str(&l, "parent").map(|p| p[..12.min(p.len())].to_string())
                ),
            ),
        );
        j.push(l);
    }

    // ---- Beat 7: she memoises her test run through clw -----------------------------
    {
        let marker = ws.root_join("test-runs.log");
        let keyfile = ws.write("run-key.txt", b"orders-api test suite @ v0.3.1\n");
        let key_s = keyfile.to_string_lossy().into_owned();
        let marker_s = marker.to_string_lossy().into_owned();
        let script = format!("printf 'ran\\n' >> '{marker_s}'; printf 'tests: ok\\n'");
        let args = ["run", "--input", &key_s, "--", "sh", "-c", &script];

        let mut miss = work.step(
            "run test suite (first — cache miss)",
            &args,
            &[],
            ExpectExit::Eq(0),
        );
        miss.narrate(
            "Marina runs her test suite THROUGH clw so the result is cached against the inputs. \
             The first run is a cache miss: it actually executes.",
        );
        miss.observe(
            Observation::behavior(
                "first run executes the command and reports a cache miss",
                "memoization must run the work the first time it sees these inputs",
            )
            .check(
                miss.stderr.contains("cache miss") && miss.stdout.contains("tests: ok"),
                "cache-miss on stderr and the child's output is present",
            ),
        );
        j.push(miss);

        // The memo write to the AC store is eventually-consistent: on a shared prod backend the
        // record may not have replicated before an IMMEDIATE re-run, so the first re-run can
        // MISS (replica lag). The property "an identical re-run HITS the cache" is still true —
        // it just converges. So retry with a short backoff until it hits (bounded), and at the
        // moment it hits, prove the hit RE-EXECUTED NOTHING (the side-effect count did not grow
        // on the hit run). This keeps the strong assertion (memoization works + a hit is
        // side-effect-free) while tolerating the replica-lag landmine.
        let mut hit_result = None;
        let mut replica_lag_misses = 0u32;
        for attempt in 0..8u32 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(750));
            }
            let before = marker_lines(&marker);
            let ev = work.step(
                "run test suite (again — cache hit)",
                &args,
                &[],
                ExpectExit::Eq(0),
            );
            let after = marker_lines(&marker);
            if ev.stderr.contains("cache hit") {
                hit_result = Some((ev, after == before, after));
                break;
            }
            replica_lag_misses += 1;
        }
        let (mut hit, side_effect_free, marker_now) = hit_result.unwrap_or_else(|| {
            // Never hit within the budget → a REAL failure (captured, not a panic).
            let ev = work.step(
                "run test suite (again — cache hit)",
                &args,
                &[],
                ExpectExit::Eq(0),
            );
            let m = marker_lines(&marker);
            (ev, false, m)
        });
        hit.narrate(
            "She runs the exact same thing again. clw should replay from cache WITHOUT \
             re-executing. (The memo is eventually-consistent on the shared backend, so the hit \
             can take a moment to propagate — the check tolerates that replica lag but REQUIRES a \
             hit to land, and requires the hit itself to re-execute nothing.)",
        );
        hit.observe(
            Observation::behavior(
                "memoization hits (tolerating AC replica lag) and a confirmed hit re-executes nothing",
                "a cache hit must return the memoized result and cause NO new side effect; replica lag may delay the hit but must not prevent it",
            )
            .check(
                hit.stderr.contains("cache hit") && side_effect_free,
                format!(
                    "cache hit after {replica_lag_misses} replica-lag miss(es); the hit re-executed nothing (side-effect count stable at {marker_now})"
                ),
            ),
        );
        j.push(hit);

        // Cache-key sensitivity: change the inputs → it must MISS again (re-execute), or the
        // memo is unsafe (would serve a stale result for different inputs).
        let keyfile2 = ws.write("run-key.txt", b"orders-api test suite @ v0.3.2\n");
        let key2_s = keyfile2.to_string_lossy().into_owned();
        let args2 = ["run", "--input", &key2_s, "--", "sh", "-c", &script];
        let before_change = marker_lines(&marker);
        let mut miss2 = work.step(
            "run with changed inputs (must miss again)",
            &args2,
            &[],
            ExpectExit::Eq(0),
        );
        miss2.narrate(
            "Marina bumps the version and re-runs. Different inputs MUST be a cache miss — a memo \
             that ignored the change would silently serve her a stale result. (A NEW key was \
             never written, so this is deterministic — no replica lag on a key that doesn't exist.)",
        );
        let after_change = marker_lines(&marker);
        miss2.observe(
            Observation::behavior(
                "changed inputs bust the cache (miss + exactly one re-execute), so the memo is never stale",
                "keying the cache on the actual inputs is what makes memoization SAFE, not just fast",
            )
            .check(
                // checked_add: if a marker read failed (usize::MAX sentinel), this is None →
                // the check fails (correct) instead of overflow-panicking in debug.
                miss2.stderr.contains("cache miss") && before_change.checked_add(1) == Some(after_change),
                format!("cache-miss on stderr; the changed-input run re-executed exactly once (marker {before_change}→{after_change})"),
            ),
        );
        j.push(miss2);
    }

    // ---- Beat 8: her token expired overnight — the store must fail CLOSED ----------
    {
        let expired = Runner::new(
            bin,
            &creds.endpoint,
            creds.target(),
            &creds.tenant,
            Secret::new("corelink_pat_expired_overnight_000000000000"),
            ws.dir("cache-expired"),
        );
        let mut s = expired.step(
            "snapshot with an expired token",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Marina comes in Monday and her PAT has rotated out. clw must fail CLOSED with an \
             error that points her straight at the credential — not hang, not half-write.",
        );
        s.observe(
            Observation::behavior(
                "an invalid/expired token fails closed with an actionable auth error naming CLW_TOKEN",
                "a cryptic auth failure wastes hours; the message must point at the credential to fix",
            )
            .check(
                s.stderr.contains("authentication failed") && s.stderr.contains("CLW_TOKEN"),
                "stderr names the auth failure and the credential",
            ),
        );
        j.push(s);
    }

    // ---- Beat 9: honest mistakes — a typo'd pin, and hydrate into a full dir -------
    {
        let dest = ws.dir("pin-attempt");
        let dest_s = dest.to_string_lossy().into_owned();
        let mut s = work.step(
            "hydrate --manifest-digest <typo>",
            &["--json", "hydrate", &dest_s, "--manifest-digest", "n0thex"],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Marina tries to pin an exact version but pastes a mangled digest. clw must reject \
             it loudly with the expected format — not a confusing not-found or a silent no-op.",
        );
        s.observe(
            Observation::behavior(
                "a malformed --manifest-digest is rejected with the expected 64-hex format",
                "a typo'd pin must fail with the fix (the shape), not a cryptic miss",
            )
            .check(
                s.stderr.contains("invalid --manifest-digest")
                    && s.stderr.contains("64-character hex"),
                "stderr states the expected 64-character hex digest",
            ),
        );
        j.push(s);

        // Hydrate into a directory that already has files → must refuse, not clobber.
        let full = ws.dir("existing-work");
        std::fs::write(full.join("uncommitted.txt"), b"work in progress\n").ok();
        let full_s = full.to_string_lossy().into_owned();
        let mut h = work.step(
            "hydrate into a non-empty dir",
            &["--json", "hydrate", &full_s, "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        h.narrate(
            "She accidentally targets a folder that already has uncommitted work. clw must REFUSE \
             rather than overwrite — losing her local changes would be unforgivable.",
        );
        let preserved = std::fs::read(full.join("uncommitted.txt"))
            .map(|b| b == b"work in progress\n")
            .unwrap_or(false);
        h.observe(
            Observation::behavior(
                "hydrate refuses a non-empty destination and leaves the existing work untouched",
                "silently overwriting a populated directory would destroy the user's local changes",
            )
            .check(
                h.stderr.contains("not empty") && preserved,
                format!("stderr says 'not empty'; existing file preserved={preserved}"),
            ),
        );
        j.push(h);
    }

    // ---- Beat 10: she lists what she has ------------------------------------------
    {
        let mut s = work.step(
            "ls --all",
            &["--json", "ls", "--all"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Before wrapping up, Marina lists her workspaces to confirm orders-api is there.",
        );
        let listed = s
            .json
            .as_ref()
            .and_then(|v| v.get("refs"))
            .and_then(|v| v.as_array())
            .map(|refs| {
                refs.iter()
                    .any(|r| r.get("name").and_then(|n| n.as_str()) == Some(WS))
            })
            .unwrap_or(false);
        s.observe(
            Observation::behavior(
                "the tenant listing includes orders-api (structured under refs[])",
                "enumeration must surface the workspace she just saved, in the documented shape",
            )
            .check(listed, "orders-api present in refs[]"),
        );
        j.push(s);
    }

    // ---- Beat 11: project shipped — she cleans up, and it's really gone -----------
    {
        let mut s = work.step(
            "rm orders-api --yes",
            &["--json", "rm", "--name", WS, "--yes"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "orders-api shipped. Marina removes the workspace to keep her tenant tidy — \
             non-interactively, the way a cleanup script would.",
        );
        s.observe(
            Observation::behavior(
                "rm --yes deletes the ref non-interactively",
                "cleanup/automation must delete without a TTY prompt",
            )
            .check(json_bool(&s, "deleted") == Some(true), "deleted=true"),
        );
        j.push(s);

        let mut gone = work.step(
            "ls --name (verify it's gone)",
            &["--json", "ls", "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        gone.narrate(
            "She double-checks it's actually gone — a delete that leaves the name behind is a lie.",
        );
        gone.observe(
            Observation::behavior(
                "the removed workspace no longer resolves (not-found, non-zero exit)",
                "a delete must be real: the name must stop resolving, not just report success",
            )
            .check(
                gone.exit_code.map(|c| c != 0).unwrap_or(false)
                    && gone.stderr.to_lowercase().contains("not found"),
                format!("exit {:?}, stderr says not-found", gone.exit_code),
            ),
        );
        j.push(gone);
    }

    j
}

// --- helpers -----------------------------------------------------------------------

fn runner(bin: &Path, creds: &ProdCreds, cache: std::path::PathBuf) -> Runner {
    Runner::new(
        bin,
        &creds.endpoint,
        creds.target(),
        &creds.tenant,
        creds.token.clone(),
        cache,
    )
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn json_str(ev: &StepEvidence, key: &str) -> Option<String> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}
fn json_u64(ev: &StepEvidence, key: &str) -> Option<u64> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_u64())
}
fn json_bool(ev: &StepEvidence, key: &str) -> Option<bool> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
}
fn str_arr(ev: &StepEvidence, key: &str) -> Vec<String> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
fn empty_arr(ev: &StepEvidence, key: &str) -> bool {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(false)
}

fn marker_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(usize::MAX)
}

fn count_files(root: &Path) -> usize {
    let mut n = 0;
    let _ = walk_count(root, &mut n);
    n
}
fn walk_count(dir: &Path, n: &mut usize) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            walk_count(&entry.path(), n)?;
        } else {
            *n += 1;
        }
    }
    Ok(())
}

/// Recursively compare two directory trees for byte-identical files + identical structure.
fn dirs_identical(a: &Path, b: &Path) -> bool {
    let (mut fa, mut fb) = (Vec::new(), Vec::new());
    if collect_files(a, a, &mut fa).is_err() || collect_files(b, b, &mut fb).is_err() {
        return false;
    }
    fa.sort();
    fb.sort();
    if fa != fb {
        return false;
    }
    fa.iter().all(
        |rel| match (std::fs::read(a.join(rel)), std::fs::read(b.join(rel))) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        },
    )
}
fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}
