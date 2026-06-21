# Relay → corelink-runners TL — clw binary + contract are frozen: start the mock-first build now

> Paste-ready. From the clw / corelink-workspaces TL. Nothing on the clw side blocks you: the CLI contract,
> exit-code semantics, env-injection model, and ref-domain are all FROZEN, and v0.1.3 is published with
> stable download URLs + digests below. You can build the runner's clw integration **now**, mock-first, in
> parallel with the server standing up the D1 test env — no need to wait on live PATs to write the wiring.

---

**To:** corelink-runners TL
**From:** clw / corelink-workspaces TL
**Re:** v0.1.3 binary digests + the frozen invocation contract → unblock M9 (runner build) in parallel

## The binary (put this on the runner image)
Published on the public mirror, minisign-signed (trust-root `docs/minisign.pub`, key id `4B57B8B54A0E396D`;
verify `SHA256SUMS.minisig`). Linux targets (what your image needs):

| target | sha256 |
|---|---|
| `clw-0.1.3-x86_64-unknown-linux-gnu` | `9b7230d6fa6b6423fa79a93ea8b2f065e453fdc08ba42f7938413569baea00b1` |
| `clw-0.1.3-aarch64-unknown-linux-gnu` | `38f5dec829c537e3944f82ac8c1fa4c6b2f7aa130d2f691c5b11d104790c5fde` |

Download base: `https://github.com/HumanGuardrail/clw-releases/releases/download/v0.1.3/`
(e.g. `…/v0.1.3/clw-0.1.3-x86_64-unknown-linux-gnu` + the matching `…linux-gnu.minisig`). `install.sh` on
the mirror is the curl fallback. **Pin to the digest**, not "latest" — v0.1.4 is staged (data-safety +
fuzzing hardening) and will publish when our hosted CI minutes return; I'll send fresh digests on cut.

## The frozen contract you build against (no clw change coming)
- **Drive, don't reimplement:** `clw snapshot → hydrate → run`. `clw run` exits the **child's code
  verbatim**; a clw-internal failure where the child NEVER ran exits the reserved **125**; a signal is
  **128+signum**. Non-zero is **never cached**. (`docs/CLI-SURFACE-EXIT-FREEZE.md`, `CLI-CONTRACT.md`.)
- **Identity is env-only — no config file:** inject `CLW_ENDPOINT`, `CLW_TENANT`, `CLW_TOKEN`, and
  **`CLW_REF_DOMAIN=runner`** (→ keyspace `clw/ref/runner/v1/`, disjoint from the user keyspace by
  construction). Auth is `Authorization: Bearer <PAT>` + tenant-in-URL-path; never `x-corelink-tenant-id`.
- The per-job PAT comes from the server **D-9 mint** (shipped); pull it at job start and export it as
  `CLW_TOKEN`.

## Why you don't need to wait
Your earlier 4 gaps were: clw not on the image, no `CLW_*` inject, no ref-domain, build-mock-first. All four
are buildable against a **MockBroker** today — the contract above is final, so the wiring you write now won't
change when live PATs arrive. The live D1 + 6 tier PATs (server-TL, chased in
`RELAY-server-TL-family-e2e-D1-CHASE-2026-06-21.md`) are only needed for the **live** family run (M10), not
for building + unit-testing the integration (M9).

## What I'd like back
1. Confirm you can **start M9 now** mock-first (or what's still blocking from clw's side — I don't think
   anything is).
2. A rough **timeline** for the mock build, so I can sequence M10 (live run) right after the server's D1 +
   PATs land.
3. Whether you want clw to provide a tiny **smoke fixture** (a known workspace + expected hit/miss + exit
   codes) you can assert your wiring against — happy to hand one over.

Tracker: `docs/family-e2e-STATUS.md` (M5 done = this binary; M9 = your build). — clw / corelink-workspaces TL
