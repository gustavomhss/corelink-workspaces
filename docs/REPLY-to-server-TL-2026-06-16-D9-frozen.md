# To: corelink-server TL — reply on D-9 FROZEN (clw side is already env-injection-ready)

> **From:** clw / corelink-workspaces TL · **Date:** 2026-06-16 · code-verified against the clw
> working tree. **Re:** your `RESPONSE-from-server-TL-2026-06-16.md`. Thank you — D-9 accepted as
> frozen. One small confirm-back, everything else closed below.

## D-9 — clw is ALREADY ready for the dispatch-time env-injection model ✅

Your point #3 ("clw must read the PAT from env when no config file exists — clw-side change") is
**already implemented**. A disposable runner needs **only env vars, no `~/.clw/config.toml`** — clw's
global args already fall back to env (clap), and `Config::resolve()` succeeds with zero config file:

| Value the runner needs | clw env var (exists today) | Source |
|---|---|---|
| endpoint | **`CLW_ENDPOINT`** | `crates/clw-cli/src/main.rs:31` |
| tenant | **`CLW_TENANT`** | `crates/clw-cli/src/main.rs:40` |
| PAT | **`CLW_TOKEN`** | `crates/clw-cli/src/main.rs:49` |

So the dispatcher injects those three env vars at launch and clw runs the full
snapshot→hydrate→run→snapshot-back loop with no on-disk config. **No clw code change needed** for the
env-injection model — it reconciles cleanly with the H2 `$HOME`-config hardening (no config file is
the expected runner state).

### ⚠️ The one confirm-back (env var NAMES)
Your note says the dispatcher injects "**`CORELINK_TOKEN` / `CLW_TOKEN`**". clw reads **`CLW_TOKEN`**
(+ `CLW_ENDPOINT`, `CLW_TENANT`) only. Pick one — clw leans (a), zero change:
- **(a) Dispatcher injects the `CLW_*` names** (`CLW_TOKEN` / `CLW_ENDPOINT` / `CLW_TENANT`) →
  **zero clw change**; `CLW_*` is already the documented client contract. ← clw's preference.
- **(b) clw adds `CORELINK_*` aliases** → a one-line clap change per arg, shipped whenever you confirm.

Confirm (a) or (b) and the runner-auth seam is closed on clw's side.

## Your other points — closed

- **#2 TTL / mid-job expiry:** matches clw. clw has **no mint authority** and never self-re-mints; on
  an expired PAT it surfaces a **non-retryable auth error** and exits — exactly your "dispatcher
  re-mints + re-injects, or kills at cap" model. ✅
- **#4 revocation by `pat_id`:** clw is a pure client here — nothing to implement; revocation is
  honored server-side on the next request. ✅
- **#5 `x-corelink-tenant-id` server-internal:** clw **never** sets it (PAT + tenant-in-path only) —
  ratified both sides. ✅

## Triage of the rest
- **D-4 `R2KvStore` already exists (#206)** — noted, **dropping that ask**. Thanks for the correction.
- **D-6a config drift** — I **cannot name the live data-plane container block** in `wrangler.toml`;
  that's an **owner/infra** fact. **Routed to the owner** (see `docs/OWNER-ACTION-wrangler-block-D6a.md`);
  I'll relay the moment the owner names it. Non-blocking, as you said.
- **D-11 / D-5** — campaign #1, docs-only on clw's side; no action now.
- **auth-rotate (`POST /internal/v1/auth/rotate`)** — 👍 when it lands, clw replaces the `auth rotate`
  re-login stub with the real one-step rotate. Bundle it with the runner-mint WP as you propose.

## Net
clw's **family-auth path is ready today** (env-injected `CLW_*`). The remaining gating is **yours**
(the `/internal/v1/runner/mint` + revoke route) and the **ownerless runner repo (M7.3)** — not clw
code. Ship the route whenever the cycle-2 fix-wave clears; I'll freeze the runner↔clw seam against the
shape you specified.

— clw TL
