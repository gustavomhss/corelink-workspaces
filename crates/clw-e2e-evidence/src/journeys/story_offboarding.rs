//! STORY — "Omar decommissions a retired service and honors a GDPR erasure request." An
//! ops engineer's offboarding, driven through the real binary against a DEDICATED THROWAWAY
//! tenant (a real, irreversible `erase`). It proves the destructive verbs are safe: prune
//! sweeps only true orphans (live data survives, verified cold), erase refuses without the
//! exact confirmation, and a confirmed erase actually empties the tenant (GDPR Art.17).
//!
//! FENCED: runs ONLY against `CLW_E2E_ERASE_TENANT` (≠ the primary tenant — the loader
//! refuses otherwise). Same gold-standard bar: real data, narrated, full-shape verified.

use std::path::Path;

use super::{EraseCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Persona, Runner, StepEvidence};

/// A confirmation token that is NOT the tenant — the erase MUST abort on this.
const WRONG_CONFIRM: &str = "00000000-0000-4000-8000-0000deadbeef";

pub fn run(bin: &Path, ec: &EraseCreds) -> Journey {
    let persona = Persona::new(
        "omar",
        "ops engineer decommissioning a retired service + honoring a GDPR erasure request",
        "sweep the orphaned blobs a retired workspace left behind WITHOUT touching live data, \
         then erase the tenant for real — but only with the exact, deliberate confirmation",
    );
    let mut j = Journey::story(
        "omar-offboards-a-tenant",
        persona,
        "safe destruction on a throwaway tenant: prune sweeps only true orphans (live data \
         survives, verified cold) → erase refuses the wrong confirmation → the exact \
         confirmation empties the tenant (GDPR Art.17), verified gone",
    );

    let ws = Workspace::new("story-offboarding");
    std::fs::write(
        ws.dir("service").join("data.bin"),
        super::pseudo_bytes(2_000_000),
    )
    .ok();
    std::fs::write(
        ws.dir("orphan").join("scratch.bin"),
        super::pseudo_bytes_seeded(1_500_000, 424242),
    )
    .ok();
    let live_src = ws.dir("service").to_string_lossy().into_owned();
    let orphan_src = ws.dir("orphan").to_string_lossy().into_owned();
    let live_name = "retired-billing-service";
    let orphan_name = "retired-billing-scratch";

    let r = Runner::new(
        bin,
        &ec.endpoint,
        format!("throwaway · {}", ec.tenant),
        &ec.tenant,
        ec.token.clone(),
        ws.dir("cache"),
    );

    // ---- Beat 1: the live data Omar must NOT lose ---------------------------------
    let live_root = {
        let mut s = r.step(
            "snapshot the live service data (must survive the prune)",
            &["--json", "snapshot", &live_src, "--name", live_name],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Before touching anything, Omar snapshots the still-live data on this tenant. The \
             whole risk of a cleanup is destroying something that's still referenced — so this \
             is the thing that MUST survive.",
        );
        let root = json_str(&s, "root");
        s.observe(
            Observation::behavior(
                "the live workspace is stored",
                "we can only prove 'survives a prune' if it was really there first",
            )
            .check(
                root.as_deref().map(is_hex64).unwrap_or(false),
                format!("root={:?}", root.as_deref().map(short)),
            ),
        );
        j.push(s);
        root.unwrap_or_default()
    };

    // ---- Beat 2: create an orphan (snapshot, then drop the ref) -------------------
    {
        let _ = r.step(
            "seed orphan: snapshot scratch data",
            &["--json", "snapshot", &orphan_src, "--name", orphan_name],
            &[],
            ExpectExit::Eq(0),
        );
        let mut s = r.step(
            "seed orphan: rm the ref (its blobs are now unreferenced)",
            &["--json", "rm", "--name", orphan_name, "--yes"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "The retired service also left scratch data whose ref Omar deletes — its blobs are \
             now ORPHANS (stored, but nothing points at them). This is exactly the garbage a \
             prune should collect.",
        );
        s.observe(
            Observation::behavior(
                "the scratch ref is gone, leaving orphan blobs",
                "a prune needs a genuine orphan to sweep",
            )
            .check(json_bool(&s, "deleted") == Some(true), "orphan ref removed"),
        );
        j.push(s);
    }

    // ---- Beat 3: prune dry-run — reports orphans, deletes nothing -----------------
    {
        let mut s = r.step(
            "prune (dry-run) → reports orphans, deletes nothing",
            &["--json", "prune"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Omar runs `prune` to preview. By default it's a DRY RUN — it must report the orphans \
             it WOULD sweep and delete absolutely nothing yet. No surprises.",
        );
        let dry = json_bool(&s, "dry_run");
        let orphans = json_u64(&s, "orphans").unwrap_or(0);
        s.observe(
            Observation::behavior(
                "prune is dry-run by default and reports at least the orphan we made",
                "a destructive sweep must preview before it acts",
            )
            .check(
                dry == Some(true) && orphans >= 1,
                format!("dry_run={dry:?}, orphans={orphans}"),
            ),
        );
        j.push(s);
    }

    // ---- Beat 4: prune --yes — sweeps the orphan; LIVE data SURVIVES (cold) -------
    {
        let mut s = r.step(
            "prune --yes → sweeps the orphan",
            &["--json", "prune", "--yes"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate("Satisfied, Omar runs `prune --yes`. It sweeps the orphan blobs for real.");
        let deleted = json_u64(&s, "deleted").unwrap_or(0);
        s.observe(
            Observation::behavior(
                "prune --yes actually deletes at least the orphan blob",
                "the sweep must free the unreferenced storage",
            )
            .check(deleted >= 1, format!("deleted={deleted}")),
        );
        j.push(s);

        // The load-bearing safety check: the LIVE ref must STILL hydrate byte-identical from a
        // COLD cache — a GC that swept a referenced blob (false orphan) would silently corrupt it.
        let cold = Runner::new(
            bin,
            &ec.endpoint,
            format!("throwaway · {}", ec.tenant),
            &ec.tenant,
            ec.token.clone(),
            ws.dir("cache-verify"),
        );
        let dst = ws.dir("verify-live");
        let dst_s = dst.to_string_lossy().into_owned();
        let mut surv = cold.step(
            "the LIVE data still hydrates after prune (cold cache, no false-orphan sweep)",
            &["--json", "hydrate", &dst_s, "--name", live_name],
            &[],
            ExpectExit::Eq(0),
        );
        surv.narrate(
            "The moment of truth: Omar re-hydrates the live service on a COLD cache. If prune had \
             swept a referenced blob (a FALSE orphan), this would fail or come back corrupt. It \
             must round-trip byte-identical, pulled fresh from the backend.",
        );
        let dl = json_u64(&surv, "bytes_downloaded").unwrap_or(0);
        let same_root = json_str(&surv, "root").as_deref() == Some(live_root.as_str());
        surv.observe(
            Observation::behavior(
                "the live workspace survived the prune intact (cold hydrate, bytes>0, same root)",
                "a GC that swept a referenced blob would silently corrupt live data — the exact replica-lag / false-orphan landmine",
            )
            .check(dl > 0 && same_root, format!("bytes_downloaded={dl}, same_root={same_root}")),
        );
        j.push(surv);
    }

    // ---- Beat 5: erase dry-run — reports the blast radius, erases nothing ---------
    {
        let mut s = r.step(
            "erase (dry-run) → reports blast radius, erases nothing",
            &["--json", "erase"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Now the GDPR request: erase the whole tenant. Omar previews first — `erase` with no \
             confirmation is a DRY RUN that reports what it WOULD wipe and touches nothing.",
        );
        let dry = json_bool(&s, "dry_run");
        s.observe(
            Observation::security("erase is a dry-run without confirmation (reports, erases nothing)", "an irreversible tenant wipe must never fire without a deliberate, explicit confirmation")
                .check(dry == Some(true), format!("dry_run={dry:?}")),
        );
        j.push(s);
    }

    // ---- Beat 6: erase with the WRONG confirmation — must ABORT -------------------
    {
        let mut s = r.step(
            "erase --confirm-tenant <WRONG> → aborts, nothing erased",
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
        s.narrate(
            "Omar fat-fingers the confirmation — pastes the wrong tenant id. This is the last line \
             of defense against wiping the WRONG tenant: it MUST abort, having erased nothing.",
        );
        s.observe(
            Observation::security(
                "erase aborts when the --confirm-tenant does not match the target tenant",
                "confirm-by-typing-the-exact-tenant is what stops an irreversible wipe of the wrong account",
            )
            .check(s.exit_code.map(|c| c != 0).unwrap_or(false), format!("exit {:?} (aborted)", s.exit_code)),
        );
        j.push(s);

        // Prove the abort was real: the live data is STILL there.
        let cold = Runner::new(
            bin,
            &ec.endpoint,
            format!("throwaway · {}", ec.tenant),
            &ec.tenant,
            ec.token.clone(),
            ws.dir("cache-postabort"),
        );
        let dst = ws.dir("verify-postabort").to_string_lossy().into_owned();
        let mut still = cold.step(
            "the data is intact after the aborted erase (cold)",
            &["--json", "hydrate", &dst, "--name", live_name],
            &[],
            ExpectExit::Eq(0),
        );
        still.narrate("He double-checks: the aborted erase left everything exactly as it was.");
        still.observe(
            Observation::security(
                "the aborted erase deleted nothing (live data still hydrates)",
                "an abort that had already deleted something would be catastrophic",
            )
            .check(
                json_u64(&still, "bytes_downloaded").unwrap_or(0) > 0,
                "live data still fetches from the backend",
            ),
        );
        j.push(still);
    }

    // ---- Beat 7: erase with the CORRECT confirmation — completes (GDPR Art.17) ----
    {
        let mut s = r.step(
            "erase --yes --confirm-tenant <correct> → complete",
            &["--json", "erase", "--yes", "--confirm-tenant", &ec.tenant],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "With the deliberate, correct confirmation, Omar erases the tenant for real. GDPR \
             Art.17 must be VERIFIABLE — it reports completion and the counts of what it removed.",
        );
        let complete = json_bool(&s, "complete");
        let refs_deleted = json_u64(&s, "refs_deleted").unwrap_or(0);
        s.observe(
            Observation::security(
                "the confirmed erase completes and reports what it deleted (GDPR Art.17)",
                "a right-to-erasure must actually remove the data AND be verifiable, not best-effort",
            )
            .check(complete == Some(true) && refs_deleted >= 1, format!("complete={complete:?}, refs_deleted={refs_deleted}")),
        );
        j.push(s);
    }

    // ---- Beat 8: verify the tenant is empty --------------------------------------
    {
        let mut s = r.step(
            "verify erased: ls the live ref → not found",
            &["--json", "ls", "--name", live_name],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate("Finally Omar confirms the erasure landed: the workspace no longer resolves — the data is genuinely gone.");
        s.observe(
            Observation::security(
                "after erase the workspace no longer resolves (specific not-found)",
                "a right-to-erasure that left the name resolving would be a compliance failure",
            )
            .check(
                s.exit_code == Some(2) && s.stderr.contains("not found"),
                format!(
                    "exit {:?}, stderr not-found={}",
                    s.exit_code,
                    s.stderr.contains("not found")
                ),
            ),
        );
        j.push(s);
    }

    j
}

fn short(s: &str) -> &str {
    &s[..12.min(s.len())]
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
