//! J5 — destructive: `prune` (GC unreferenced blobs) and `erase` (irreversible tenant
//! wipe), proven end-to-end against a DEDICATED THROWAWAY tenant.
//!
//! SAFETY (defense in depth): this journey only ever runs with [`EraseCreds`], which the
//! loader returns ONLY for a `CLW_E2E_ERASE_TENANT` that is set AND not equal to the
//! primary tenant. J5 additionally proves the `--confirm-tenant` mismatch-abort (a wrong
//! confirmation deletes NOTHING) before the real erase. Both `prune` and `erase` are
//! dry-run by default; the destructive form requires `--yes`.
//!
//! Flow: seed a LIVE ref + an ORPHAN → prune dry-run (reports orphans) → prune --yes
//! (sweeps the orphan, LIVE survives) → erase dry-run (reports blast radius) → erase with
//! a WRONG --confirm-tenant (aborts, LIVE survives) → erase --yes --confirm-tenant <right>
//! (wipes) → verify the tenant is empty.

use std::path::Path;

use super::{pseudo_bytes_seeded, EraseCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Runner, StepEvidence};

/// A syntactically-valid tenant that is deliberately NOT the throwaway — used to prove the
/// erase `--confirm-tenant` mismatch aborts without deleting anything.
const WRONG_CONFIRM: &str = "00000000-0000-4000-8000-0000deadbeef";

pub fn run(bin: &Path, ec: &EraseCreds) -> Journey {
    let ws = Workspace::new("j5");
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seed = pid as u64 ^ nonce;
    let name_live = format!("e2e-j5-live-{pid}-{nonce}");
    let name_orphan = format!("e2e-j5-orphan-{pid}-{nonce}");

    ws.write("live/data.bin", &pseudo_bytes_seeded(256 * 1024, seed));
    ws.write(
        "orphan/data.bin",
        &pseudo_bytes_seeded(256 * 1024, seed.wrapping_add(7)),
    );
    let live_src = ws.dir("live").to_string_lossy().into_owned();
    let orphan_src = ws.dir("orphan").to_string_lossy().into_owned();
    let live_content = pseudo_bytes_seeded(256 * 1024, seed);

    let r = Runner::new(
        bin,
        &ec.endpoint,
        "prod·THROWAWAY",
        &ec.tenant,
        ec.token.clone(),
        ws.dir("cache"),
    );

    let mut j = Journey::new(
        "J5-destructive",
        "prune + erase end-to-end on a DEDICATED throwaway tenant (never the primary)",
        format!("prod · THROWAWAY {}", ec.tenant),
    );

    // 0. The safety precondition, surfaced in the report: the target is NOT the primary
    //    tenant (the loader already enforced this; we record it as evidence).
    let mut guard = mk_note("safety: target is a dedicated throwaway tenant");
    guard.observe(
        Observation::security(
            "J5 runs ONLY against a dedicated throwaway tenant, never the primary",
            "erase is irreversible — the fence is that the erase tenant must differ from the working tenant",
        )
        .info(format!("erase target = {} (loader refuses if it equals the primary)", ec.tenant)),
    );
    j.push(guard);

    // 1. Seed a LIVE ref (must survive prune, and be the thing erase wipes).
    let live = r.step(
        "seed a LIVE ref",
        &["--json", "snapshot", &live_src, "--name", &name_live],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(live);

    // 2. Seed an ORPHAN: snapshot then rm the ref, leaving its unique blobs unreferenced.
    let orphan = r.step(
        "seed an orphan: snapshot",
        &["--json", "snapshot", &orphan_src, "--name", &name_orphan],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(orphan);
    let rm_orphan = r.step(
        "seed an orphan: rm the ref (blobs now unreferenced)",
        &["--json", "rm", "--name", &name_orphan, "--yes"],
        &[],
        ExpectExit::Eq(0),
    );
    j.push(rm_orphan);

    // 3. prune DRY-RUN (default): reports orphans, deletes NOTHING.
    let mut prune_dry = r.step(
        "prune (dry-run) → reports orphans, deletes nothing",
        &["--json", "prune"],
        &[],
        ExpectExit::Eq(0),
    );
    let dry = json_bool(&prune_dry, "dry_run");
    let orphans = json_u64(&prune_dry, "orphans").unwrap_or(0);
    prune_dry.observe(
        Observation::behavior(
            "prune is dry-run by default and reports at least the orphan we created",
            "a GC that deletes without an explicit --yes is a footgun — dry-run-by-default is the guard",
        )
        .check(dry == Some(true) && orphans >= 1, format!("dry_run={dry:?}, orphans={orphans}")),
    );
    j.push(prune_dry);

    // 4. prune --yes: sweeps the orphan. Then the LIVE ref MUST still hydrate (not a
    //    false-orphan sweep).
    let mut prune_yes = r.step(
        "prune --yes → sweeps the orphan",
        &["--json", "prune", "--yes"],
        &[],
        ExpectExit::Eq(0),
    );
    let deleted = json_u64(&prune_yes, "deleted").unwrap_or(0);
    prune_yes.observe(
        Observation::behavior(
            "prune --yes actually deletes at least the orphan blob",
            "GC must reclaim unreferenced storage once consent is given",
        )
        .check(deleted >= 1, format!("deleted={deleted}")),
    );
    j.push(prune_yes);
    // COLD cache (fresh Runner) so the survival hydrate is served FROM THE LIVE CAS, not
    // the warm cache the seed populated. Otherwise a prune that wrongly swept LIVE's blobs
    // from the backend would still hydrate from local cache → a vacuous "safe" green.
    let r_cold1 = cold_runner(bin, ec, ws.dir("cache-verify-prune"));
    let dst1 = ws.dir("dst-after-prune");
    let mut surv = r_cold1.step(
        "LIVE ref still hydrates after prune, COLD cache (no false-orphan sweep)",
        &[
            "--json",
            "hydrate",
            &dst1.to_string_lossy(),
            "--name",
            &name_live,
        ],
        &[],
        ExpectExit::Eq(0),
    );
    let survived = std::fs::read(dst1.join("data.bin"))
        .map(|b| b == live_content)
        .unwrap_or(false);
    let surv_dl = json_u64(&surv, "bytes_downloaded").unwrap_or(0);
    surv.observe(
        Observation::security(
            "prune did NOT sweep the LIVE ref — it hydrates byte-identically FROM THE CAS after the GC",
            "a GC that swept a referenced blob (a false orphan) would silently corrupt live data",
        )
        .check(
            survived && surv_dl > 0,
            format!("byte-identical from live CAS: {survived}, bytes_downloaded={surv_dl}"),
        ),
    );
    j.push(surv);

    // 5. erase DRY-RUN (default): reports the blast radius, deletes NOTHING.
    let mut erase_dry = r.step(
        "erase (dry-run) → reports blast radius, deletes nothing",
        &["--json", "erase"],
        &[],
        ExpectExit::Eq(0),
    );
    let edry = json_bool(&erase_dry, "dry_run");
    let refs = json_u64(&erase_dry, "refs").unwrap_or(0);
    erase_dry.observe(
        Observation::security(
            "erase is dry-run by default and reports the LIVE ref in the blast radius",
            "an irreversible tenant wipe must NEVER act without --yes — dry-run-by-default is mandatory",
        )
        .check(edry == Some(true) && refs >= 1, format!("dry_run={edry:?}, refs={refs}")),
    );
    j.push(erase_dry);

    // 6. erase --yes with a WRONG --confirm-tenant → ABORTS, deletes nothing. Then the LIVE
    //    ref MUST still hydrate (proof the mismatch guard prevented the wipe).
    let mut wrong = r.step(
        "erase --yes --confirm-tenant <WRONG> → aborted (typed-confirmation guard)",
        &[
            "--json",
            "erase",
            "--yes",
            "--confirm-tenant",
            WRONG_CONFIRM,
        ],
        &[],
        ExpectExit::NonZero,
    );
    wrong.observe(
        Observation::security(
            "a mismatched --confirm-tenant aborts the erase (exit non-zero) and deletes nothing",
            "the typed-tenant confirmation is the last rail before an irreversible wipe of the WRONG tenant",
        )
        .check(
            wrong.exit_code.map(|c| c != 0).unwrap_or(false),
            format!("exit {:?} (aborted)", wrong.exit_code),
        ),
    );
    j.push(wrong);
    // COLD cache again: prove the LIVE blobs are still IN THE CAS after the aborted erase,
    // not merely resolvable from a warm local cache.
    let r_cold2 = cold_runner(bin, ec, ws.dir("cache-verify-abort"));
    let dst2 = ws.dir("dst-after-abort");
    let mut still = r_cold2.step(
        "LIVE ref still hydrates after the aborted erase, COLD cache",
        &[
            "--json",
            "hydrate",
            &dst2.to_string_lossy(),
            "--name",
            &name_live,
        ],
        &[],
        ExpectExit::Eq(0),
    );
    let still_ok = std::fs::read(dst2.join("data.bin"))
        .map(|b| b == live_content)
        .unwrap_or(false);
    let still_dl = json_u64(&still, "bytes_downloaded").unwrap_or(0);
    still.observe(
        Observation::security(
            "the aborted erase left the LIVE ref fully intact IN THE CAS",
            "a confirmation-failed erase must be a total no-op, not a partial deletion",
        )
        .check(
            still_ok && still_dl > 0,
            format!("byte-identical from live CAS: {still_ok}, bytes_downloaded={still_dl}"),
        ),
    );
    j.push(still);

    // 7. erase --yes --confirm-tenant <CORRECT> → the real, verified wipe.
    let mut erase_go = r.step(
        "erase --yes --confirm-tenant <throwaway> → IRREVERSIBLE wipe (verified)",
        &["--json", "erase", "--yes", "--confirm-tenant", &ec.tenant],
        &[],
        ExpectExit::Eq(0),
    );
    let complete = json_bool(&erase_go, "complete");
    let refs_deleted = json_u64(&erase_go, "refs_deleted").unwrap_or(0);
    erase_go.observe(
        Observation::security(
            "erase with the correct confirmation completes and reports what it deleted",
            "GDPR Art.17 erasure must be verifiable — it reports complete=true and the deleted counts",
        )
        .check(
            complete == Some(true) && refs_deleted >= 1,
            format!("complete={complete:?}, refs_deleted={refs_deleted}"),
        ),
    );
    j.push(erase_go);

    // 8. Verify the tenant is EMPTY: the LIVE ref no longer exists (ls --name → not found).
    let mut gone = r.step(
        "verify wiped: ls the LIVE ref → not found",
        &["--json", "ls", "--name", &name_live],
        &[],
        ExpectExit::NonZero,
    );
    // Tightened: require the SPECIFIC not-found signal (exit 2 + "not found" in stderr), so
    // a transient error can't read as "wiped". The real proof is step 7 (complete=true);
    // this confirms the data is actually gone.
    let gone_ok = gone.exit_code == Some(2) && gone.stderr.contains("not found");
    gone.observe(
        Observation::security(
            "after erase the LIVE ref is gone — ls reports not-found (exit 2)",
            "the erasure must actually remove the data, not just report success",
        )
        .check(
            gone_ok,
            format!(
                "ls exit {:?}, stderr says not-found={}",
                gone.exit_code,
                gone.stderr.contains("not found")
            ),
        ),
    );
    j.push(gone);

    j
}

/// A fresh Runner on the throwaway identity with a COLD `cache` dir — so a hydrate it
/// runs must fetch from the live CAS rather than replaying a warm local cache.
fn cold_runner(bin: &Path, ec: &EraseCreds, cache: std::path::PathBuf) -> Runner {
    Runner::new(
        bin,
        &ec.endpoint,
        "prod·THROWAWAY",
        &ec.tenant,
        ec.token.clone(),
        cache,
    )
}

fn mk_note(label: &str) -> StepEvidence {
    StepEvidence {
        label: label.to_string(),
        cmd: String::new(),
        env_shown: Vec::new(),
        expected_exit: ExpectExit::Any,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        json: None,
        duration_ms: 0,
        leaked_secret: false,
        observations: Vec::new(),
    }
}

fn json_bool(ev: &StepEvidence, key: &str) -> Option<bool> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(serde_json::Value::as_bool)
}

fn json_u64(ev: &StepEvidence, key: &str) -> Option<u64> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(serde_json::Value::as_u64)
}
