//! STORY — "Ravi wires clw into the release pipeline." An automation/CI engineer's usage,
//! driven through the real binary. CI cares about two things a human rarely notices: the
//! EXIT CODE contract (a build step's real exit code must flow through clw untouched, and a
//! clw-internal failure that never ran the step must be distinguishable), and the RUNNER
//! ref-domain (CI names live in a keyspace disjoint from developers', so a pipeline can't
//! collide with a person's workspace).
//!
//! Same gold-standard bar: real data, names a pipeline would use, each beat narrated +
//! judged on the exact contract CI depends on.

use std::path::Path;

use super::realistic;
use super::{ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Persona, Runner, StepEvidence};

const WS: &str = "release-artifact";

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let persona = Persona::new(
        "ravi (release pipeline)",
        "platform engineer running clw non-interactively inside CI",
        "trust clw in automation: a step's real exit code flows through untouched, a \
         clw-internal failure is distinguishable, and CI names never collide with a developer's",
    );
    let mut j = Journey::story(
        "ravi-wires-clw-into-ci",
        persona,
        "clw in automation: exit-code passthrough (success → 0, failure → the child's code \
         verbatim) → a clw-internal failure never runs the step → the runner ref-domain is a \
         keyspace disjoint from developers'",
    );

    let ws = Workspace::new("story-automation");
    let project = realistic::orders_api(ws.root_dir(), WS);
    let project_s = project.to_string_lossy().into_owned();
    let run = Runner::new(
        bin,
        &creds.endpoint,
        creds.target(),
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache"),
    );

    // ---- Beat 1: a successful step exits 0 (the pipeline's green signal) -----------
    {
        let key = ws.write("k-ok.txt", b"build step: compile @ sha-aaaa\n");
        let key_s = key.to_string_lossy().into_owned();
        let mut s = run.step(
            "run a step that succeeds → exit 0",
            &[
                "run",
                "--input",
                &key_s,
                "--",
                "sh",
                "-c",
                "echo built; exit 0",
            ],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Ravi's pipeline runs a build step through clw. A success must exit 0 — that's the \
             green light the next CI stage waits on.",
        );
        s.observe(
            Observation::behavior(
                "a successful step exits 0 and its output is present",
                "CI branches on the exit code; a success must be an unambiguous 0",
            )
            .check(
                s.exit_code == Some(0) && s.stdout.contains("built"),
                format!("exit {:?}, output present", s.exit_code),
            ),
        );
        j.push(s);
    }

    // ---- Beat 2: a failing step's exit code flows through VERBATIM ----------------
    {
        let key = ws.write("k-fail.txt", b"build step: test @ sha-bbbb\n");
        let key_s = key.to_string_lossy().into_owned();
        let mut s = run.step(
            "run a step that exits 3 → clw exits 3 (verbatim)",
            &[
                "run",
                "--input",
                &key_s,
                "--",
                "sh",
                "-c",
                "echo tests failed; exit 3",
            ],
            &[],
            ExpectExit::Eq(3),
        );
        s.narrate(
            "A test step fails with exit 3. clw MUST surface exactly 3 — not mask it as a generic \
             error, not swallow it to 0. CI reads that exact code to decide retry vs abort. This \
             is the frozen contract: clw run exits the child's code verbatim.",
        );
        s.observe(
            Observation::behavior(
                "clw run passes the child's non-zero exit code through UNCHANGED (3 → 3)",
                "if clw masked or normalized a build step's exit code, CI would mis-route failures",
            )
            .check(
                s.exit_code == Some(3),
                format!("child exited 3 → clw exited {:?} (must be 3)", s.exit_code),
            ),
        );
        j.push(s);
    }

    // ---- Beat 3: a clw-internal failure NEVER runs the step -----------------------
    {
        let mut s = run.step(
            "run with a missing --input → clw-internal failure, step never runs",
            &[
                "run",
                "--input",
                "/nonexistent/pipeline/inputs.lock",
                "--",
                "sh",
                "-c",
                "echo THIS_MUST_NOT_RUN; exit 0",
            ],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Ravi points clw at an input file that isn't there — a config error in the pipeline. \
             clw must fail BEFORE running the step (the step's side effects must NOT happen), and \
             signal a clw-internal failure distinct from the step's own success. The reserved \
             exit 125 is exactly this 'the child never ran' case.",
        );
        let child_ran = s.stdout.contains("THIS_MUST_NOT_RUN");
        s.observe(
            Observation::behavior(
                "a clw-internal setup failure fails WITHOUT running the step (reserved exit 125)",
                "CI must be able to tell 'clw couldn't start my step' from 'my step ran and failed' — else it retries the wrong thing",
            )
            .check(
                s.exit_code == Some(125) && !child_ran,
                format!("exit {:?} (reserved 125 = child never ran); step_ran={child_ran}", s.exit_code),
            ),
        );
        j.push(s);
    }

    // ---- Beat 4: the runner ref-domain is a DISJOINT keyspace ---------------------
    {
        // The pipeline snapshots under the USER domain first (a developer's name)...
        let mut user_snap = run.step(
            "snapshot release-artifact in the USER domain",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        user_snap.narrate(
            "A developer already has a workspace named `release-artifact`. Ravi's pipeline is \
             about to use the SAME name — but in the RUNNER ref-domain, which must be a separate \
             keyspace so CI and the developer never step on each other.",
        );
        let user_root = json_str(&user_snap, "root");
        j.push(user_snap);

        // ...then, with DIFFERENT content, snapshots the SAME name in the RUNNER domain.
        realistic::add_free_shipping_rule(&project); // the pipeline's build differs from the dev's
        let mut runner_snap = run.step(
            "snapshot the SAME name with DIFFERENT content in the RUNNER domain",
            &["--json", "snapshot", &project_s, "--name", WS],
            &[("CLW_REF_DOMAIN", "runner", false)],
            ExpectExit::Eq(0),
        );
        runner_snap.narrate(
            "Same name, different content, but `CLW_REF_DOMAIN=runner`. If the two domains shared \
             a keyspace this would 409 (create-only divergent). It must SUCCEED — proving the \
             runner keyspace (`clw/ref/runner/v1/`) is disjoint from the user's.",
        );
        let runner_root = json_str(&runner_snap, "root");
        runner_snap.observe(
            Observation::behavior(
                "the same name resolves to DIFFERENT content in the runner vs user domain (disjoint keyspaces)",
                "a pipeline must not collide with — or clobber — a developer's identically-named workspace",
            )
            .check(
                matches!((&user_root, &runner_root), (Some(a), Some(b)) if is_hex64(a) && is_hex64(b) && a != b),
                format!("user_root={:?} runner_root={:?} (must both exist and DIFFER)", user_root.as_deref().map(short), runner_root.as_deref().map(short)),
            ),
        );
        j.push(runner_snap);

        // And each domain resolves ITS OWN root — the disjointness is real, not luck.
        let mut ls_user = run.step(
            "ls --name in the USER domain",
            &["--json", "ls", "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        ls_user.narrate(
            "Ravi confirms the developer's name still resolves to the developer's content.",
        );
        ls_user.observe(
            Observation::behavior(
                "the user domain still resolves to the user's root (untouched by the runner write)",
                "CI writing in the runner domain must leave the developer's workspace exactly as it was",
            )
            .check(json_str(&ls_user, "root") == user_root, "user-domain root unchanged"),
        );
        j.push(ls_user);

        let mut ls_runner = run.step(
            "ls --name in the RUNNER domain",
            &["--json", "ls", "--name", WS],
            &[("CLW_REF_DOMAIN", "runner", false)],
            ExpectExit::Eq(0),
        );
        ls_runner.narrate("...and the runner domain resolves to the pipeline's content.");
        ls_runner.observe(
            Observation::behavior(
                "the runner domain resolves to the pipeline's root",
                "the two keyspaces are independently addressable",
            )
            .check(
                json_str(&ls_runner, "root") == runner_root,
                "runner-domain root is the pipeline's",
            ),
        );
        j.push(ls_runner);

        // Cleanup BOTH domains (non-interactive, the way a pipeline's teardown would).
        let _ = run.step(
            "cleanup user domain",
            &["rm", "--name", WS, "--yes"],
            &[],
            ExpectExit::Any,
        );
        let _ = run.step(
            "cleanup runner domain",
            &["rm", "--name", WS, "--yes"],
            &[("CLW_REF_DOMAIN", "runner", false)],
            ExpectExit::Any,
        );
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
