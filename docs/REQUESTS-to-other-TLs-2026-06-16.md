# Cross-TL requests — index (2026-06-16)

> One relay-ready doc **per techlead** (replaces the single consolidated ask, and supersedes the
> 14-Jun `docs/REQUESTS-to-other-TLs.md`). Hand each TL their own file.

| Recipient | Doc | Headline open ask | Blocks |
|---|---|---|---|
| **corelink-server TL** (+ CoreLink-API / Clerk) | [`REQUEST-server-TL-2026-06-16.md`](REQUEST-server-TL-2026-06-16.md) → reply [`REPLY-to-server-TL-2026-06-16-D9-frozen.md`](REPLY-to-server-TL-2026-06-16-D9-frozen.md) | ✅ **D-9 FROZEN** — clw env-injection-ready; pending server route merge + env-name confirm | family e2e (M7.5) |
| **CoreLink runners TL** (M7.3, ownerless) | [`REQUEST-runners-TL-2026-06-16.md`](REQUEST-runners-TL-2026-06-16.md) | Freeze the runner ↔ clw contract (4 points) | family e2e (M7.5) |
| **hugit TL** | [`REQUEST-hugit-TL-2026-06-16.md`](REQUEST-hugit-TL-2026-06-16.md) | ✅ **RESOLVED** — seam 2 = hugit consumes `clw hydrate` (byte+mode identical); one setuid confirm-back open | nothing (early) |
| **githugr TL** | [`REQUEST-githugr-TL-2026-06-16.md`](REQUEST-githugr-TL-2026-06-16.md) | ✅ **RESOLVED** — githugr = read-only display; push→compute is the runner/engine lane (no githugr→clw, no PAT) | nothing (early) |

## What CLOSED since 2026-06-14 (recorded so nobody re-asks)

D-1 / D-7 / D-8 are **LIVE** (server #269 deployed; conformance 8/8 green; live binary e2e this
session); **CONFIRM-a** (PAT grants CAS+AC) ratified + proven; **CONFIRM-b** region routing resolved
to mechanism **(c)** (clw needs no change); **D-3** signup-region fix shipped in #269; **D-10**
superseded. The whole `rm`/`prune`/`ls --all`/**GDPR Art.17 erasure** milestone is live-verified.

## Not a TL — OWNER actions (tracked, not relayed here)
Signing-secret provisioning, CI billing (M2.4), and legal counsel sign-off are **owner** actions, not
cross-TL asks — they live in `docs/go-live/LAUNCH-CHECKLIST.md`. The `release.yml` signing/publish
**code** is now implemented (secret-gated); the owner only provisions the secrets.
