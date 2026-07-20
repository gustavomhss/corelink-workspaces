//! STORY — "Dana hardens how her team uses clw." A security engineer's audit, driven
//! through the real binary. She confirms clw never leaks the PAT (even in verbose logs),
//! redacts credentials embedded in an endpoint, refuses to send the token over plaintext,
//! and won't destroy anything without explicit consent. Every property is OBSERVED through
//! the binary — not read off the code.
//!
//! Same gold-standard bar as the other stories: real data, a name she'd type, each beat
//! narrated + judged on what a security reviewer actually needs to see.

use std::path::Path;

use super::realistic;
use super::{ProdCreds, Workspace};
use crate::{ExpectExit, Journey, Observation, Persona, Runner, Secret, StepEvidence};

const WS: &str = "infra-config";

pub fn run(bin: &Path, creds: &ProdCreds) -> Journey {
    let persona = Persona::new(
        "dana",
        "security engineer reviewing how her team uses clw",
        "prove clw never leaks the PAT, redacts embedded credentials, refuses plaintext, and \
         never destroys anything without explicit consent",
    );
    let mut j = Journey::story(
        "dana-hardens-clw-usage",
        persona,
        "a security review, observed through the binary: no PAT in verbose logs → endpoint \
         credential redaction → plaintext refused before the token leaves → consent gates on \
         destroy and logout",
    );

    let ws = Workspace::new("story-security");
    let project = realistic::orders_api(ws.root_dir(), WS);
    let src = project.to_string_lossy().into_owned();
    let work = Runner::new(
        bin,
        &creds.endpoint,
        creds.target(),
        &creds.tenant,
        creds.token.clone(),
        ws.dir("cache"),
    );

    // ---- Beat 1: verbose audit — the PAT must NOT appear in the diagnostic logs ----
    {
        let mut s = work.step(
            "snapshot -vv (audit the verbose request log)",
            &["-vv", "--json", "snapshot", &src, "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate(
            "Dana turns on the most verbose logging clw has and snapshots a repo, then reads the \
             ENTIRE request-lifecycle log. If the PAT shows up ANYWHERE in that output, it would \
             end up in CI logs and support tickets — so it must never appear, even at -vv.",
        );
        s.expect_no_secret_leak(
            "the -vv channel logs the whole request lifecycle; a PAT there lands in CI logs / tickets",
        );
        j.push(s);
    }

    // ---- Beat 2: a credential embedded in the endpoint must be redacted -----------
    {
        let home = ws.dir("home-userinfo").to_string_lossy().into_owned();
        let mut r_userinfo = Runner::new(
            bin,
            "https://e2euser:e2esecret@corelink-api.humangr.com",
            "endpoint-with-userinfo",
            &creds.tenant,
            creds.token.clone(),
            ws.dir("cache-userinfo"),
        );
        r_userinfo.add_redaction(&Secret::new("e2esecret"));
        let mut doc = r_userinfo.step(
            "doctor with a user:pass@ endpoint → the password is redacted",
            &["--json", "doctor"],
            &[("HOME", &home, false)],
            ExpectExit::Eq(0),
        );
        doc.narrate(
            "A teammate pasted an endpoint with embedded credentials (`https://user:pass@host`) \
             into the config. Dana runs doctor — clw must REDACT the password when it echoes the \
             endpoint, never print it in the clear.",
        );
        doc.expect_no_secret_leak(
            "an endpoint's embedded password must be scrubbed from clw's own output, not echoed raw",
        );
        j.push(doc);
    }

    // ---- Beat 3: a plaintext http:// endpoint must be REFUSED before the PAT flies -
    {
        let r_plain = Runner::new(
            bin,
            "http://corelink-api.humangr.com",
            "plaintext endpoint",
            &creds.tenant,
            creds.token.clone(),
            ws.dir("cache-plain"),
        );
        let mut s = r_plain.step(
            "snapshot to an http:// non-loopback endpoint → refused",
            &["snapshot", &src, "--name", &format!("{WS}-plain")],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Dana checks the worst case: someone mis-configures a plaintext `http://` endpoint to \
             a real host. clw must REFUSE before a single byte of the PAT crosses the wire in the \
             clear — no 'best effort', a hard stop.",
        );
        s.observe(
            Observation::security(
                "clw refuses to send the PAT over plaintext http:// to a non-loopback host",
                "sending a bearer token over cleartext to a real host would expose it to any on-path observer",
            )
            .check(
                s.stderr.contains("plaintext") && s.stderr.contains("loopback"),
                "stderr refuses and names plaintext + the loopback exception",
            ),
        );
        j.push(s);
    }

    // ---- Beat 4: destroy needs consent — rm without --yes refuses, deletes nothing -
    {
        let mut s = work.step(
            "rm without --yes → refused, nothing deleted",
            &["--json", "rm", "--name", WS],
            &[],
            ExpectExit::NonZero,
        );
        s.narrate(
            "Dana makes sure a destructive command can't fire by accident in a script. `rm` \
             without `--yes` (no TTY to prompt) must DECLINE and delete nothing.",
        );
        let deleted = s
            .json
            .as_ref()
            .and_then(|v| v.get("deleted"))
            .and_then(|v| v.as_bool());
        s.observe(
            Observation::security(
                "a destructive rm without --yes is refused and deletes nothing (deleted=false)",
                "a destroy that fires without explicit consent in a non-interactive context loses data silently",
            )
            .check(
                deleted == Some(false) && s.stderr.contains("declined"),
                format!("deleted={deleted:?}, stderr mentions declined={}", s.stderr.contains("declined")),
            ),
        );
        j.push(s);

        // ...and the workspace really is still there (the refusal wasn't a lie).
        let mut still = work.step(
            "ls --name (the ref survived the refused rm)",
            &["--json", "ls", "--name", WS],
            &[],
            ExpectExit::Eq(0),
        );
        still.narrate(
            "She confirms infra-config still resolves — the refusal actually protected it.",
        );
        still.observe(
            Observation::security(
                "the refused rm left the workspace intact",
                "a consent gate must actually protect the data, not just print a refusal",
            )
            .check(
                json_str(&still, "name").as_deref() == Some(WS),
                "the ref still resolves",
            ),
        );
        j.push(still);
    }

    // ---- Beat 5: logout needs consent too — and must not silently wipe the config --
    {
        let home = ws.dir("home-logout");
        let home_s = home.to_string_lossy().into_owned();
        let cfg_file = home.join(".clw/config.toml");
        // Give her a config to log out FROM (sandboxed HOME — never touches the operator's ~/.clw).
        let _ = work.step(
            "init a sandbox config (to log out from)",
            &[
                "init",
                "--tenant",
                &creds.tenant,
                "--endpoint",
                &creds.endpoint,
            ],
            &[("HOME", &home_s, false)],
            ExpectExit::Any,
        );
        let mut logout = work.step(
            "auth logout --all without --yes → refused, config preserved",
            &["auth", "logout", "--all"],
            &[("HOME", &home_s, false)],
            ExpectExit::NonZero,
        );
        logout.narrate(
            "Finally Dana runs `auth logout --all` without confirming. It must REFUSE — and \
             crucially must NOT have already wiped her saved identity. She checks the config on \
             disk still carries the tenant + endpoint.",
        );
        // The config file holds the plaintext PAT, so we only check for KEY NAMES (never dump it).
        let cfg_body = std::fs::read_to_string(&cfg_file).unwrap_or_default();
        let intact = cfg_body.contains("tenant") && cfg_body.contains("endpoint");
        logout.observe(
            Observation::security(
                "logout --all without --yes is refused AND leaves the saved identity intact",
                "a logout that wipes credentials before confirming would lock the user out on a fat-finger",
            )
            .check(
                intact && (logout.exit_code.map(|c| c != 0).unwrap_or(false)),
                format!("refused (exit {:?}); config still has tenant+endpoint={intact}", logout.exit_code),
            ),
        );
        j.push(logout);
    }

    // ---- Beat 6: cleanup ----------------------------------------------------------
    {
        let mut s = work.step(
            "rm infra-config --yes (cleanup)",
            &["--json", "rm", "--name", WS, "--yes"],
            &[],
            ExpectExit::Eq(0),
        );
        s.narrate("Audit done — Dana removes the scratch workspace, this time WITH consent.");
        s.observe(
            Observation::behavior(
                "with --yes the delete goes through",
                "consent given, the destroy proceeds",
            )
            .check(json_bool(&s, "deleted") == Some(true), "deleted=true"),
        );
        j.push(s);
    }

    j
}

fn json_str(ev: &StepEvidence, key: &str) -> Option<String> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}
fn json_bool(ev: &StepEvidence, key: &str) -> Option<bool> {
    ev.json
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
}
