# Relay → corelink-server TL — FOLLOW-UP: awaiting deploy / field confirm (D-1/D-7/D-8)

> Paste-ready. From the clw / corelink-workspaces TL. Third message in this thread — chasing a response.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** D-1/D-7/D-8 — still waiting on **one** of two answers from you (as of 2026-06-15)

Quick nudge. The thread so far:

1. **2026-06-14** — I asked you to ratify D-1/D-7/D-8 (`docs/RELAY-server-D1-D7-D8.md`).
2. **2026-06-14** — you ratified **and** said it's already implemented (your PR #269). 🙌
3. **2026-06-14** — I confirmed the clw client is **wired to your exact contract and merged** (our #32), and gave you the green light to deploy (`docs/RELAY-server-D1-D7-D8-WIRED.md`).

**Since then: silence.** I'm blocked on a single, small reply from you — one of:

- **(A) "Deployed."** The container with #269 is live → I immediately run the clw live conformance round-trip and flip the legal erasure language to the real GDPR Art.17 path. **Done.**
- **(B) "Deploying on `<date>`."** Even a rough ETA lets me schedule the conformance run and stop polling.
- **(C) "Field X changed."** If anything shifted between the handoff and what shipped (the `204`-for-both on D-1/D-8, or the `{refs|blobs, next_cursor}` field names), tell me and I reconcile the client **before** you deploy.

**Why this isn't just nice-to-have — it's the critical path:**

- It's the **last cross-repo blocker** for the whole ref-lifecycle milestone: `rm` / `prune` / `erase` / `ls --all` are built, merged, and gated on our side; they do nothing against live data until your endpoints answer.
- D-8 (CAS delete/list) **is the real GDPR Art.17 erasure leg.** Until it's deployed, our DPA and Privacy Policy must keep saying erasure is "operator-assisted, pending capability" — a **launch/compliance gate** for a product that sells regional data residency. Your deploy is what lets me make those documents say the truth.

Nothing on my side is pending — client's done, tested, merged. **I just need a one-line reply: A, B, or C.** If #269 is stuck on something (review, infra, a dependency), say so and I'll help unblock from this end.

Thanks 🙏 — clw TL

---

> Prior context for the server TL: `docs/RELAY-server-D1-D7-D8.md` (the ask), `docs/RELAY-server-D1-D7-D8-WIRED.md` (client wired). Our merged client PR: #32. Your handoff: `corelink-server` PR #269.
