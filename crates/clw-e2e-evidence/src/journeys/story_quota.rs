//! STORY — "Nadia hits her plan's spend ceiling." A solo founder on a metered plan whose
//! tenant has reached its monthly $-budget. Her next snapshot must be REFUSED with a real,
//! actionable 402 — not a cryptic error, not a silent partial write. Driven through the real
//! binary against a DEDICATED tenant whose ceiling is set tiny out-of-band (never the primary
//! — the loader fences it), so the 402 is genuine, not simulated.

use std::path::Path;

use super::realistic;
use super::{QuotaCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Persona, Runner};

const WS: &str = "side-project";

pub fn run(bin: &Path, qc: &QuotaCreds) -> Journey {
    let persona = Persona::new(
        "nadia",
        "solo founder on a metered plan, near her monthly budget",
        "understand exactly why her save was refused when she hits her spend ceiling — an \
         actionable 402, not a mystery error",
    );
    let mut j = Journey::story(
        "nadia-hits-her-plan-ceiling",
        persona,
        "a metered tenant at its monthly $-budget: the next snapshot is refused with a real, \
         actionable 402 (plan/quota), never a cryptic failure or a partial write",
    );

    let ws = Workspace::new("story-quota");
    let project = realistic::orders_api(ws.root_dir(), WS);
    let src = project.to_string_lossy().into_owned();
    let r = Runner::new(
        bin,
        &qc.endpoint,
        format!("quota-capped · {}", qc.tenant),
        &qc.tenant,
        qc.token.clone(),
        ws.dir("cache"),
    );

    // ---- Beat 1: her save hits the monthly spend wall — a real 402 ----------------
    {
        let mut s = r.step(
            "snapshot after the tenant hit its monthly budget → 402",
            &["--json", "snapshot", &src, "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Nadia has been snapshotting all month and her tenant has reached its plan's spend \
             ceiling. Her next save must be REFUSED with a clear 402 that tells her WHY (the plan \
             / quota limit) and what to do — not a generic 500 or a half-written workspace.",
        );
        let msg = s.stderr.to_lowercase();
        let actionable = msg.contains("quota")
            || msg.contains("plan")
            || msg.contains("payment")
            || msg.contains("402")
            || msg.contains("limit");
        s.observe(
            Observation::behavior(
                "a tenant over its monthly budget is refused with an actionable plan/quota (402) message",
                "a paying user who hits their ceiling must understand it's a billing limit (upgrade/wait), not a bug to debug",
            )
            .check(
                s.exit_code.map(|c| c != 0).unwrap_or(false) && actionable,
                format!("exit {:?}; stderr names the plan/quota limit = {actionable}", s.exit_code),
            ),
        );
        j.push(s);
    }

    j
}
