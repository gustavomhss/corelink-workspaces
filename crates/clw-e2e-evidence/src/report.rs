//! H2 — render captured [`Journey`] evidence into an analyzable report.
//!
//! Two forms: a Markdown report (diff-friendly, greppable) and a self-contained HTML
//! report (scannable, color-coded verdicts, per-step collapsible output). The report
//! is the deliverable — the owner reads it to analyze behavior + security per step.

use crate::{Journey, ObsKind, Observation, StepEvidence, Verdict};

/// Report-wide metadata shown in the header.
pub struct Meta {
    pub title: String,
    pub clw_version: String,
    /// e.g. "2026-07-18T…Z" — passed in (this crate does not read the clock).
    pub generated_at: String,
    pub os: String,
}

struct Totals {
    journeys_pass: usize,
    journeys_total: usize,
    steps_pass: usize,
    steps_total: usize,
    security_pass: usize,
    security_fail: usize,
    behavior_fail: usize,
}

fn totals(journeys: &[Journey]) -> Totals {
    let mut t = Totals {
        journeys_pass: 0,
        journeys_total: journeys.len(),
        steps_pass: 0,
        steps_total: 0,
        security_pass: 0,
        security_fail: 0,
        behavior_fail: 0,
    };
    for j in journeys {
        if j.passed() {
            t.journeys_pass += 1;
        }
        for s in &j.steps {
            t.steps_total += 1;
            if s.passed() {
                t.steps_pass += 1;
            }
            for o in &s.observations {
                match (o.kind, o.verdict) {
                    (ObsKind::Security, Verdict::Pass) => t.security_pass += 1,
                    (ObsKind::Security, Verdict::Fail) => t.security_fail += 1,
                    (ObsKind::Behavior, Verdict::Fail) => t.behavior_fail += 1,
                    _ => {}
                }
            }
        }
    }
    t
}

fn verdict_mark(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "✔",
        Verdict::Fail => "✗",
        Verdict::Info => "•",
    }
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

pub fn render_markdown(journeys: &[Journey], meta: &Meta) -> String {
    let t = totals(journeys);
    let mut o = String::new();
    o.push_str(&format!("# {}\n\n", meta.title));
    o.push_str(&format!(
        "- clw `{}` · {} · generated {}\n",
        meta.clw_version, meta.os, meta.generated_at
    ));
    o.push_str(&format!(
        "- **Journeys: {}/{} passed** · Steps: {}/{} passed\n",
        t.journeys_pass, t.journeys_total, t.steps_pass, t.steps_total
    ));
    o.push_str(&format!(
        "- **Security observations: {} pass / {} fail** · Behavior fails: {}\n\n",
        t.security_pass, t.security_fail, t.behavior_fail
    ));
    o.push_str("> Every step below is the REAL `clw` binary invoked as a user (argv+env). Secrets are redacted. The evidence is the observed behavior, not a code read.\n\n");

    for j in journeys {
        let (p, n) = j.counts();
        o.push_str(&format!("## {} — {}  ({p}/{n})\n\n", j.name, j.about));
        o.push_str(&format!("_target: {}_\n\n", j.target));
        for (i, s) in j.steps.iter().enumerate() {
            render_step_md(&mut o, i + 1, s);
        }
    }
    o
}

fn render_step_md(o: &mut String, n: usize, s: &StepEvidence) {
    let head = if s.passed() { "✔" } else { "✗ FAIL" };
    o.push_str(&format!("### {n}. {head} {}\n\n", s.label));
    o.push_str(&format!("```console\n$ {}\n```\n", s.cmd));
    if !s.env_shown.is_empty() {
        o.push_str(&format!("env: `{}`\n\n", s.env_shown.join("` `")));
    }
    o.push_str(&format!(
        "exit: **{}** · {} ms\n\n",
        s.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into()),
        s.duration_ms
    ));
    // Observations table.
    o.push_str("| | observation | why it matters | evidence |\n|---|---|---|---|\n");
    for obs in &s.observations {
        let kind = match obs.kind {
            ObsKind::Security => "🔒",
            ObsKind::Behavior => "⚙",
        };
        o.push_str(&format!(
            "| {} {} | {} | {} | {} |\n",
            verdict_mark(obs.verdict),
            kind,
            md_cell(&obs.claim),
            md_cell(&obs.why),
            md_cell(&obs.detail),
        ));
    }
    o.push('\n');
    // Captured output (collapsed).
    push_details_md(o, "stdout", &s.stdout);
    push_details_md(o, "stderr", &s.stderr);
    o.push('\n');
}

fn push_details_md(o: &mut String, label: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    // A fence must be longer than the longest backtick run in the body, else captured
    // output containing ``` closes the block early and corrupts the report.
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    o.push_str(&format!(
        "<details><summary>{label}</summary>\n\n{fence}\n{body}\n{fence}\n</details>\n"
    ));
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

// ---------------------------------------------------------------------------
// HTML (self-contained, theme-aware, scannable)
// ---------------------------------------------------------------------------

pub fn render_html(journeys: &[Journey], meta: &Meta) -> String {
    let t = totals(journeys);
    let mut o = String::new();
    o.push_str("<!doctype html><html><head><meta charset=\"utf-8\">");
    o.push_str(&format!("<title>{}</title>", esc(&meta.title)));
    o.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    o.push_str(STYLE);
    o.push_str("</head><body>");
    o.push_str(&format!("<h1>{}</h1>", esc(&meta.title)));
    o.push_str(&format!(
        "<p class=\"meta\">clw <code>{}</code> · {} · generated {}</p>",
        esc(&meta.clw_version),
        esc(&meta.os),
        esc(&meta.generated_at)
    ));
    let sec_cls = if t.security_fail == 0 { "ok" } else { "bad" };
    o.push_str(&format!(
        "<div class=\"summary\"><span class=\"pill\">Journeys {}/{}</span>\
         <span class=\"pill\">Steps {}/{}</span>\
         <span class=\"pill {}\">Security {} pass / {} fail</span>\
         <span class=\"pill\">Behavior fails {}</span></div>",
        t.journeys_pass,
        t.journeys_total,
        t.steps_pass,
        t.steps_total,
        sec_cls,
        t.security_pass,
        t.security_fail,
        t.behavior_fail
    ));
    o.push_str("<p class=\"note\">Every step is the REAL <code>clw</code> binary invoked as a user (argv+env). Secrets redacted. Evidence is observed behavior, not a code read.</p>");

    for j in journeys {
        let (p, n) = j.counts();
        let cls = if j.passed() { "ok" } else { "bad" };
        o.push_str(&format!(
            "<section><h2 class=\"{}\">{} <small>{}</small> <span class=\"badge\">{}/{}</span></h2>",
            cls,
            esc(&j.name),
            esc(&j.about),
            p,
            n
        ));
        o.push_str(&format!(
            "<p class=\"target\">target: {}</p>",
            esc(&j.target)
        ));
        for (i, s) in j.steps.iter().enumerate() {
            render_step_html(&mut o, i + 1, s);
        }
        o.push_str("</section>");
    }
    o.push_str("</body></html>");
    o
}

fn render_step_html(o: &mut String, n: usize, s: &StepEvidence) {
    let cls = if s.passed() { "ok" } else { "bad" };
    o.push_str(&format!("<div class=\"step {}\">", cls));
    o.push_str(&format!(
        "<div class=\"step-head\"><span class=\"mark\">{}</span> <b>{}. {}</b> \
         <span class=\"tag\">exit {} · {} ms</span></div>",
        if s.passed() { "✔" } else { "✗" },
        n,
        esc(&s.label),
        s.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into()),
        s.duration_ms
    ));
    o.push_str(&format!("<pre class=\"cmd\">$ {}</pre>", esc(&s.cmd)));
    if !s.env_shown.is_empty() {
        o.push_str(&format!(
            "<pre class=\"env\">{}</pre>",
            esc(&s.env_shown.join("  "))
        ));
    }
    o.push_str("<table class=\"obs\"><thead><tr><th></th><th>observation</th><th>why it matters</th><th>evidence</th></tr></thead><tbody>");
    for obs in &s.observations {
        render_obs_html(o, obs);
    }
    o.push_str("</tbody></table>");
    push_details_html(o, "stdout", &s.stdout);
    push_details_html(o, "stderr", &s.stderr);
    o.push_str("</div>");
}

fn render_obs_html(o: &mut String, obs: &Observation) {
    let (vcls, vmark) = match obs.verdict {
        Verdict::Pass => ("v-pass", "✔"),
        Verdict::Fail => ("v-fail", "✗"),
        Verdict::Info => ("v-info", "•"),
    };
    let kind = match obs.kind {
        ObsKind::Security => "🔒",
        ObsKind::Behavior => "⚙",
    };
    o.push_str(&format!(
        "<tr class=\"{}\"><td class=\"vmark\">{} {}</td><td>{}</td><td class=\"why\">{}</td><td class=\"detail\">{}</td></tr>",
        vcls,
        vmark,
        kind,
        esc(&obs.claim),
        esc(&obs.why),
        esc(&obs.detail)
    ));
}

fn push_details_html(o: &mut String, label: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    o.push_str(&format!(
        "<details><summary>{label}</summary><pre class=\"out\">{}</pre></details>",
        esc(body)
    ));
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const STYLE: &str = r#"<style>
:root{--bg:#fff;--fg:#1a1c1e;--mut:#5b6570;--line:#e3e6ea;--ok:#137a3f;--bad:#b3261e;--info:#5b6570;--card:#f7f8fa;--accent:#2b5bb0;}
@media(prefers-color-scheme:dark){:root{--bg:#0f1113;--fg:#e6e8ea;--mut:#9aa4af;--line:#262b30;--ok:#4ccf83;--bad:#ff6b5e;--info:#9aa4af;--card:#161a1e;--accent:#6fa0ff;}}
*{box-sizing:border-box}body{margin:0 auto;max-width:1000px;padding:2rem 1.25rem;background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
h1{font-size:1.5rem;margin:0 0 .25rem}h2{font-size:1.15rem;margin:2rem 0 .25rem;border-top:1px solid var(--line);padding-top:1.25rem}h2 small{color:var(--mut);font-weight:400;font-size:.85rem}
h2.ok{color:var(--fg)}h2.bad{color:var(--bad)}.badge{float:right;color:var(--mut);font-size:.8rem;font-weight:400}
.meta,.note,.target{color:var(--mut);font-size:.85rem;margin:.25rem 0}.note{margin-bottom:1rem}
.summary{display:flex;flex-wrap:wrap;gap:.5rem;margin:.75rem 0}
.pill{background:var(--card);border:1px solid var(--line);border-radius:999px;padding:.2rem .7rem;font-size:.82rem}
.pill.ok{color:var(--ok);border-color:var(--ok)}.pill.bad{color:var(--bad);border-color:var(--bad)}
.step{border:1px solid var(--line);border-left:3px solid var(--line);border-radius:8px;padding:.75rem .9rem;margin:.6rem 0;background:var(--card)}
.step.ok{border-left-color:var(--ok)}.step.bad{border-left-color:var(--bad)}
.step-head{display:flex;align-items:center;gap:.5rem;flex-wrap:wrap}.step-head .mark{font-weight:700}.step.ok .mark{color:var(--ok)}.step.bad .mark{color:var(--bad)}
.tag{color:var(--mut);font-size:.8rem;margin-left:auto}
pre{overflow-x:auto;background:var(--bg);border:1px solid var(--line);border-radius:6px;padding:.5rem .6rem;font:12.5px/1.45 ui-monospace,SFMono-Regular,Menlo,monospace;margin:.5rem 0}
pre.cmd{color:var(--accent)}pre.env{color:var(--mut);font-size:11.5px}
table.obs{width:100%;border-collapse:collapse;margin:.5rem 0;font-size:.85rem}
table.obs th{text-align:left;color:var(--mut);font-weight:600;border-bottom:1px solid var(--line);padding:.3rem .4rem}
table.obs td{border-bottom:1px solid var(--line);padding:.3rem .4rem;vertical-align:top}
.vmark{white-space:nowrap;font-weight:700}.v-pass .vmark{color:var(--ok)}.v-fail .vmark{color:var(--bad)}.v-info .vmark{color:var(--info)}
.v-fail{background:color-mix(in srgb,var(--bad) 8%,transparent)}
.why{color:var(--mut)}.detail{font-family:ui-monospace,monospace;font-size:.8rem}
details{margin:.35rem 0}summary{cursor:pointer;color:var(--mut);font-size:.82rem}
pre.out{max-height:22rem;overflow:auto}
</style>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExpectExit;

    fn sample() -> Vec<Journey> {
        let mut j = Journey::new("J1 demo", "prove a thing", "prod");
        let mut s = StepEvidence {
            label: "do a snapshot".into(),
            cmd: "clw --json snapshot . --name x".into(),
            env_shown: vec!["CLW_TENANT=t".into(), "CLW_TOKEN=<redacted>".into()],
            expected_exit: ExpectExit::Eq(0),
            exit_code: Some(0),
            stdout: "{\"name\":\"x\"}".into(),
            stderr: String::new(),
            json: Some(serde_json::json!({"name":"x"})),
            duration_ms: 12,
            observations: vec![],
        };
        s.observe(
            Observation::security("token never echoed", "a leaked PAT is full account access")
                .check(true, "no token substring in output"),
        );
        j.push(s);
        vec![j]
    }

    fn meta() -> Meta {
        Meta {
            title: "E2E evidence".into(),
            clw_version: "0.1.11".into(),
            generated_at: "2026-07-18T00:00:00Z".into(),
            os: "macos".into(),
        }
    }

    #[test]
    fn markdown_has_command_and_observation() {
        let md = render_markdown(&sample(), &meta());
        assert!(md.contains("$ clw --json snapshot . --name x"));
        assert!(md.contains("token never echoed"));
        assert!(md.contains("Security observations: 1 pass"));
    }

    #[test]
    fn html_is_self_contained_and_escaped() {
        let html = render_html(&sample(), &meta());
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<style>"));
        assert!(!html.contains("<script"));
        assert!(html.contains("token never echoed"));
    }

    #[test]
    fn markdown_fence_survives_backticks_in_output() {
        // L2: captured output containing ``` must not close the fence early.
        let mut o = String::new();
        push_details_md(&mut o, "stdout", "before ``` after");
        // The opening fence must be longer than the 3-backtick run in the body.
        assert!(o.contains("````\nbefore ``` after\n````"));
    }

    #[test]
    fn html_escapes_angle_brackets_in_output() {
        let mut j = Journey::new("j", "a", "prod");
        j.push(StepEvidence {
            label: "x".into(),
            cmd: "clw doctor".into(),
            env_shown: vec![],
            expected_exit: ExpectExit::Any,
            exit_code: Some(0),
            stdout: "token=<set>".into(),
            stderr: String::new(),
            json: None,
            duration_ms: 1,
            observations: vec![Observation::behavior("c", "w").info("d")],
        });
        let html = render_html(&j_slice(&j), &meta());
        assert!(html.contains("token=&lt;set&gt;"));
    }

    fn j_slice(j: &Journey) -> Vec<Journey> {
        vec![j.clone()]
    }
}
