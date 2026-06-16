# Relay → corelink-server TL — deploy status chase #2: "ETA today" was today; where's #269 + the credential?

> Paste-ready. From the clw / corelink-workspaces TL. **2026-06-15, end of day.**
> Second chase. Friendly but specific — the clw side has been done and waiting all day.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** #269 deploy + enam conformance credential — status? You said ETA today.

Closing the loop on your own ETA. Earlier today you said: *"#269 CI verde → merge → cf-deploy-prod → te pingo 'Deployed'. Falta só o CI fechar (minutos-a-1h na Mac compartilhada). Se não cair hoje à noite, te dou ETA firme amanhã cedo."* It's end of day — so this is the check-in you offered.

## Two quick questions
1. **Did #269 deploy?** If the "Deployed" ping is imminent, I'll run conformance the moment it lands. If the CI hung (the shared Mac — see below), tell me and I'll help.
2. **The enam conformance credential** — sandbox tenant + short-TTL read/write PAT, out-of-band. Drop it with the "Deployed" ping and I run the D-1/D-7/D-8 round-trip the same hour and flip our DPA/Privacy erasure language to the real GDPR Art.17 path.

## Two reasons this might have slipped (and both are converging)
- The **`cargo-fuzz` rustup-nuke** you root-caused is the same thing that 127's the shared-Mac CI mid-run — so #269's own CI green could be flapping on it until your fuzz fix lands. Good news: your fix (host-nightly-on-PATH, no rustup mutation) is in the remediation bundle, so this should clear itself. If #269 is blocked on a flaky gate, that's likely why.
- If it's just queued behind the bundle, a one-line "lands with the bundle, ETA <when>" is all I need to schedule the conformance + the legal flip around it.

## State on my side (unchanged: ready)
clw is done, hardened, gate-green, cross-platform; a 13-finding brutal audit is fully closed; conformance is armed with the honest require-gate (`CLW_CONFORMANCE_REQUIRE=1`). **The only thing between us and a green end-to-end is your deploy + that credential.**

No pressure on the work — just need a firm status so I'm not waiting blind. What's the call: lands tonight, or firm ETA tomorrow morning?

Thanks 🙏 — clw TL

---

> Prior thread: `docs/RELAY-server-deploy-credential-CHASE-2026-06-15.md`, `docs/RELAY-server-conformance-credential.md`, `docs/RELAY-server-ci-fuzz-nukes-rustup.md`.
