//! STORY — "Priya and Sam share the billing service." Two engineers on ONE tenant, two
//! distinct PATs, collaborating through clw. The story proves the multi-identity properties
//! a solo persona can't: one teammate reads another's snapshot byte-identical, the CAS
//! dedups ACROSS identities, create-only stops a teammate from clobbering your work, and
//! two people saving the same content at the same time converge on one root without
//! corruption.
//!
//! Same gold-standard bar as the Marina story: real project data, names a team would type,
//! every beat narrated (whose action it is) + judged on the FULL shape of what clw returns.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::realistic;
use super::{ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Persona, Runner, Secret, StepEvidence};

/// The shared workspace name the team collaborates on — distinct from any other persona's,
/// so two personas in one suite run never collide on a name.
const WS: &str = "billing-service";

pub fn run(bin: &Path, creds: &ProdCreds, sam_token: &Secret) -> Journey {
    let persona = Persona::new(
        "priya + sam",
        "two backend engineers sharing the billing-service workspace on one tenant",
        "collaborate through clw — read each other's snapshots byte-identically, share \
         storage via dedup, and never silently clobber each other's work",
    );
    let mut j = Journey::story(
        "team-shares-billing-service",
        persona,
        "two engineers, one tenant, two identities: cross-identity read → cross-identity \
         dedup → create-only stops a teammate clobbering your work → two concurrent saves of \
         the same content converge on one root",
    );

    let ws = Workspace::new("story-team");
    let project = realistic::orders_api(ws.root_dir(), WS);
    let project_s = project.to_string_lossy().into_owned();

    // Priya = the primary identity; Sam = the second PAT on the SAME tenant.
    let priya = Runner::new(
        bin,
        &creds.endpoint,
        creds.target(),
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache-priya"),
    );
    let sam = Runner::new(
        bin,
        &creds.endpoint,
        creds.target(),
        &creds.tenant,
        sam_token.clone(),
        ws.dir("cache-sam"),
    );

    // ---- Beat 1: Priya creates the shared workspace -------------------------------
    let root_v1 = {
        let mut s = priya.step(
            "Priya snapshots billing-service",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Priya kicks off the billing-service workspace and snapshots it so Sam can pick it \
             up. She's the first writer on this name.",
        );
        let root = json_str(&s, "root");
        s.observe(
            Observation::behavior(
                "Priya's snapshot succeeds and names a content-addressed root",
                "the shared workspace must be persisted before a teammate can read it",
            )
            .check(
                root.as_deref().map(is_hex64).unwrap_or(false),
                format!("root={:?}", root.as_deref().map(short)),
            ),
        );
        j.push(s);
        root.unwrap_or_default()
    };

    // ---- Beat 2: Sam (a DIFFERENT identity) reads Priya's work byte-identical ------
    {
        let dest = ws.dir("sam-checkout");
        let dest_s = dest.to_string_lossy().into_owned();
        let mut s = sam.step(
            "Sam hydrates Priya's snapshot (cross-identity, cold)",
            &["--json", "hydrate", &dest_s, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Sam — a SEPARATE identity on the same tenant — pulls billing-service down on his \
             own cold cache. A teammate's snapshot must come back BYTE-FOR-BYTE, fetched from \
             the backend, or the team can't trust shared state.",
        );
        let downloaded = json_u64(&s, "bytes_downloaded");
        let identical = dirs_identical(&project, &dest);
        s.observe(
            Observation::behavior(
                "a second identity reads the first's snapshot byte-identically from the backend",
                "cross-identity read is the foundation of team collaboration — it must round-trip exactly on a cold cache",
            )
            .check(
                identical && downloaded.map(|b| b > 0).unwrap_or(false) && json_str(&s, "root").as_deref() == Some(root_v1.as_str()),
                format!("tree_identical={identical} bytes_downloaded={downloaded:?} same_root={}", json_str(&s, "root").as_deref() == Some(root_v1.as_str())),
            ),
        );
        j.push(s);
    }

    // ---- Beat 3: Sam re-snapshots the SAME content — dedup ACROSS identities -------
    {
        let mut s = sam.step(
            "Sam snapshots the same content under a second name (cross-identity dedup)",
            &[
                "--json",
                "snapshot",
                &project_s,
                "--name",
                &format!("{WS}-review"),
            ],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Sam snapshots the identical tree under a review name. Because Priya already \
             uploaded every chunk, Sam's save must upload ZERO — the CAS dedups ACROSS \
             identities (that's the shared-storage guarantee), and land on the identical root.",
        );
        let uploaded = json_u64(&s, "chunks_uploaded");
        let same_root = json_str(&s, "root").as_deref() == Some(root_v1.as_str());
        s.observe(
            Observation::behavior(
                "identical content from a DIFFERENT identity uploads 0 chunks and shares the root",
                "cross-identity dedup is what makes a shared tenant storage-efficient; the content-address is identity-independent",
            )
            .check(
                uploaded == Some(0) && same_root,
                format!("chunks_uploaded={uploaded:?} (want 0), same_root_as_Priya={same_root}"),
            ),
        );
        j.push(s);
        // Sam cleans up his review name (best-effort) so only the shared name remains.
        let _ = sam.step(
            "cleanup review name",
            &["rm", "--name", &format!("{WS}-review"), "--yes"],
            &[],
            ExpectExit::Any,
        );
    }

    // ---- Beat 4: Sam tries to clobber Priya's name with HIS changes — 409 ----------
    {
        // Save the real v1 file so we can restore it exactly (no hand-duplicated literal).
        let pricing_path = project.join("src/pricing.rs");
        let pricing_v1 = std::fs::read(&pricing_path).unwrap_or_default();
        realistic::add_free_shipping_rule(&project); // Sam's local edit
        let mut s = sam.step(
            "Sam tries to overwrite billing-service with his edits (no --force)",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Sam changed the pricing module and tries to save over the shared name. He must be \
             STOPPED — create-only protects Priya's snapshot from being silently overwritten. \
             This is the guardrail from the teammate's side: the conflict is a feature.",
        );
        s.observe(
            Observation::behavior(
                "create-only stops a teammate overwriting the shared name (409 + actionable hint)",
                "if Sam could clobber Priya's snapshot silently, a team would lose work; the refusal is the protection",
            )
            .check(
                s.stderr.contains("already exists with different content") && s.stderr.contains("--force"),
                "stderr refuses the overwrite and points at --force / rm",
            ),
        );
        j.push(s);
        // Restore the exact v1 tree so the concurrent beat writes IDENTICAL content from both.
        std::fs::write(&pricing_path, &pricing_v1).ok();
    }

    // ---- Beat 5: Priya and Sam save the SAME content at the SAME time — converge ---
    {
        let concurrent_name = format!("{WS}-release");
        let uploaded_total = AtomicU64::new(0);
        // Two identities snapshot IDENTICAL content concurrently under the same name. The
        // create-only store + content-addressing must let them CONVERGE on one root (an
        // identical write is a safe no-op), never corrupt or split-brain.
        let roots: Vec<Option<String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = [(&priya, "Priya"), (&sam, "Sam")]
                .into_iter()
                .map(|(who, _label)| {
                    let name = concurrent_name.clone();
                    let project_s = project_s.clone();
                    let uploaded_total = &uploaded_total;
                    scope.spawn(move || {
                        let ev = who.step(
                            "concurrent save (identical content)",
                            &["--json", "snapshot", &project_s, "--name", &name],
                            &[],
                            ExpectExit::Any,
                        );
                        if let Some(u) = ev
                            .json
                            .as_ref()
                            .and_then(|v| v.get("chunks_uploaded"))
                            .and_then(|v| v.as_u64())
                        {
                            uploaded_total.fetch_add(u, Ordering::Relaxed);
                        }
                        (
                            ev.json
                                .as_ref()
                                .and_then(|v| v.get("root"))
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            ev,
                        )
                    })
                })
                .collect();
            let mut out = Vec::new();
            let mut evs = Vec::new();
            for h in handles {
                let (root, ev) = h.join().expect("thread");
                out.push(root);
                evs.push(ev);
            }
            // Attach both concurrent invocations as evidence, narrated.
            for (i, mut ev) in evs.into_iter().enumerate() {
                ev.narrate(format!(
                    "{} saves the release at the same moment as the other — an identical write \
                     must be a safe no-op, not a corruption.",
                    if i == 0 { "Priya" } else { "Sam" }
                ));
                j.push(ev);
            }
            out
        });

        // Whichever ordering the server saw, BOTH must have succeeded (or no-op'd) and the
        // two roots must be IDENTICAL — a single converged workspace, no split-brain.
        let both_present = roots
            .iter()
            .all(|r| r.as_deref().map(is_hex64).unwrap_or(false));
        let converged = matches!((&roots[0], &roots[1]), (Some(a), Some(b)) if a == b);
        let verdict = j.push(StepEvidence {
            label: "concurrent-write convergence".to_string(),
            cmd: String::new(),
            env_shown: Vec::new(),
            expected_exit: ExpectExit::Any,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            json: None,
            duration_ms: 0,
            leaked_secret: false,
            narration: Some(
                "The verdict on the race: two identities wrote the same content at once — \
                 content-addressing means they must land on ONE identical root, with zero \
                 wasted uploads beyond the first, and never a corrupt or split workspace."
                    .to_string(),
            ),
            observations: Vec::new(),
        });
        verdict.observe(
            Observation::behavior(
                "two concurrent identical saves converge on ONE root (no corruption, no split-brain)",
                "the write-write race on a shared name must be resolved by content-addressing, not left to chance",
            )
            .check(
                both_present && converged,
                format!("roots: {:?} / {:?} → converged={converged}", roots[0].as_deref().map(short), roots[1].as_deref().map(short)),
            ),
        );
        verdict.observe(
            Observation::behavior(
                "the shared content was uploaded at most once across the two racers",
                "content-addressing means the second racer dedups against the first — a race must not double-store",
            )
            .info(format!("total chunks uploaded across both racers = {}", uploaded_total.load(Ordering::Relaxed))),
        );

        // Cleanup the release name (best-effort).
        let _ = priya.step(
            "cleanup release name",
            &["rm", "--name", &concurrent_name, "--yes"],
            &[],
            ExpectExit::Any,
        );
    }

    // ---- Beat 6: teardown — Priya removes the shared workspace --------------------
    {
        let mut s = priya.step(
            "Priya removes billing-service",
            &["--json", "rm", "--name", WS, "--yes"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate("The release is out. Priya tidies up the shared workspace.");
        s.observe(
            Observation::behavior(
                "the shared workspace is removed cleanly",
                "team cleanup must work the same as solo cleanup",
            )
            .check(json_bool(&s, "deleted") == Some(true), "deleted=true"),
        );
        j.push(s);
    }

    j
}

// --- helpers -----------------------------------------------------------------------

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
    fa.iter().all(|rel| matches!((std::fs::read(a.join(rel)), std::fs::read(b.join(rel))), (Ok(x), Ok(y)) if x == y))
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
