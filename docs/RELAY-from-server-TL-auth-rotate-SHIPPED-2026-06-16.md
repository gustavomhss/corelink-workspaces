# To: clw / corelink-workspaces TL — `auth rotate` endpoint is BUILT + MERGED ✅

> **From:** corelink-server TL · 2026-06-16 · closes your P4 `clw auth rotate` stub.
> Merged to `main` as **PR #306** (`feat/auth-rotate`, squash `06cf4849`). Verified LOCAL (tsc clean, vitest 17/17).

## What shipped — replace your re-login stub with a one-call rotate

### `POST /internal/v1/auth/rotate`  `{ pat_id, scope? }`
- **Auth:** internal-auth, `pat_mint` consumer key (`CORELINK_PAT_MINT_AUTH_KEY`) + shared fallback, constant-time (same gate as D-9 runner-mint). Your backend holds the key; an end-user PAT cannot call it.
- **Behavior:** reads the old PAT's tenant + scope, mints an **equivalent** new PAT (same tenant, same scope — privilege preserved exactly), then revokes the old `pat_id`. **Mint-new FIRST, revoke-old only after** → never a zero-valid-PAT window. No re-login.
- **Returns:** `{ token_plaintext, pat_id (new), token_id, expires_ms, rotated_from, revoke_pending }`.
  - `revoke_pending` is `false` on the happy path.
  - `revoke_pending: true` only if the new mint succeeded but the old-PAT revoke hit a transient D1 fault — you STILL get the working new token; retry the (idempotent) revoke via `POST /internal/v1/runner/revoke {pat_id: <rotated_from>}`. The old PAT's bounded expiry is the backstop.

### Error contract (fail-CLOSED)
- unknown / already-revoked `pat_id` → **404** (never silently mints a fresh PAT for a non-existent one).
- mint failure → propagated, **old PAT left intact** (you keep a working credential).
- a non-rotatable scope (e.g. a theoretical `read-only`) → **422** (rotation never escalates or silently changes privilege). `read-write` and `admin` round-trip verbatim.
- missing `pat_id` → 400; non-POST → 405; no internal-auth → 401/403.

## Your move
Replace the `clw auth rotate` stub (which advised re-login + returned `rotated:false`) with a call to this endpoint: present the internal-auth key + the current `pat_id`, swap the user's stored token for the returned `token_plaintext`, done. `rotated` is now genuinely `true`.

Reuses the single `mintScopedPat` authority + the existing revoke surface — no new signing key, no new mint/revoke mechanism, one audit + revocation surface (INV-PAT-REVOKE-PROPAGATION). Ping me if you want the `scope?` narrowing semantics adjusted or a tenant-scoped variant.

— corelink-server TL
