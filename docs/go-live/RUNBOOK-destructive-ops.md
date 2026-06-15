# Runbook — destructive operations (`prune`, `erase`) & GDPR Art.17 erasure

> **Audience:** operators / support / on-call. **Scope:** the two commands that
> can irreversibly delete tenant data — `clw prune` (CAS garbage collection) and
> `clw erase` (full-tenant right-to-erasure). **Status:** the underlying server
> capabilities (D-7 ref-list, D-8 CAS delete/list) are **server-gated**; until
> `corelink-server` PR #269 deploys, both commands surface an actionable
> "not yet deployed" error and change nothing. This runbook is the procedure to
> follow **once they are live**.
>
> Serves exit-gate **#8** (a real right-to-erasure path exists) and **#10**
> (runbook published). Companion to the destructive-paths hardening (PR #34).

---

## 0. The one rule

**Always run the dry-run first, read it, and only then re-run with `--yes`.**
Both commands are dry-run-by-default for this reason. There is no undo.

---

## 1. `clw prune` — CAS garbage collection

### What it does
Mark-and-sweep over the tenant's content-addressed store. The **live set** is
every manifest root reachable from a ref plus every chunk those manifests
reference; any CAS blob **outside** that set is an orphan and is deleted.

### Safety properties (built in — do not bypass)
- **Dry-run by default.** Without `--yes` it prints what it *would* delete and
  changes nothing.
- **List-before-mark ordering.** Blob candidates are frozen (T0) *before* the
  ref/live-set read (T1 > T0), so a blob that becomes live in the gap is never
  swept. (See "Residual window" below for the one case this does not cover.)
- **Empty-live-set safety floor.** If the live set is empty but CAS is non-empty,
  `prune --yes` **refuses** to run — an empty ref index is far more likely a
  transient/stale D-7 page than a tenant where every blob is truly orphaned.
- **Partial-progress reporting.** On a mid-sweep delete failure it reports how
  many blobs were deleted (resumable/auditable) then exits non-zero.

### Procedure
1. **Pre-check** the tenant is intended and reachable:
   ```
   clw ls --all --tenant <TENANT> --json | jq '.count'
   ```
2. **Dry-run** and read the orphan list:
   ```
   clw prune --tenant <TENANT>
   ```
   - `nothing to prune` → done, nothing to do.
   - A list of orphans → eyeball the count; an unexpectedly large fraction of
     CAS is a red flag (see Troubleshooting).
3. **Prefer a low-write window.** Snapshots in flight during the prune are the
   only thing the residual window can affect (below).
4. **Execute:**
   ```
   clw prune --yes --tenant <TENANT>
   ```
5. **Verify:** re-run the dry-run; it should report `nothing to prune`.

### Residual window (documented, accept-and-mitigate)
A push whose blobs were already in CAS before T0 but whose **ref write lands
after** the live-set read can still have those blobs swept. The fully-correct
fix is a **server-side GC lease/epoch** (an open `corelink-server` ask — relay
on file). Until that exists, mitigate by: (a) running `prune` in a low-write
window, (b) always reviewing the dry-run, (c) never scripting `prune --yes`
unattended on a busy tenant. **Do not** attempt a client-only "fix" — it cannot
close the window without the server primitive.

---

## 2. `clw erase` — full-tenant erasure (GDPR Art.17)

### What it does
Irreversibly deletes **every ref and every CAS blob** for one tenant. This is
the right-to-erasure path: after it completes, the tenant has no recoverable
data in the store.

### The three rails (all required to actually delete)
1. **Dry-run by default** — reports the blast radius and stops.
2. **`--yes`** — explicit intent.
3. **Typed-tenant confirmation** — the operator must reproduce the exact tenant
   id, interactively or via `--confirm-tenant <TENANT>` for non-interactive use.
   A mismatch, or a non-TTY without the flag, **aborts** (non-zero exit).

Additionally: immediately before deleting, `erase` **echoes the tenant + blast
radius to stderr**, so even a wrong-but-confirmed tenant in CI is auditable in
the job log. Refs are deleted **before** blobs, so a partial failure leaves only
unreferenced orphans (reclaimable), never a live ref pointing at deleted content.

### Procedure (for a verified Art.17 request)
1. **Authorize.** Confirm the erasure request is validated per the Privacy
   Policy / DPA process (identity of requester, the tenant mapping). This is a
   compliance gate, not a technical one — do not skip it.
2. **Record** the request (ticket id, requester, tenant, timestamp) for the
   data-processing log.
3. **Dry-run** to capture the blast radius:
   ```
   clw erase --tenant <TENANT>
   ```
   Records `N ref(s) + M blob(s)` — note these for the completion record.
4. **Execute** with both rails:
   ```
   clw erase --yes --confirm-tenant <TENANT> --tenant <TENANT>
   ```
   (Interactive alternative: omit `--confirm-tenant` and type the tenant id at
   the prompt.)
5. **Verify erasure:**
   ```
   clw ls --all --tenant <TENANT> --json | jq '.count'   # expect 0
   clw prune --tenant <TENANT>                            # expect nothing/empty
   ```
6. **Close** the compliance record with the deleted counts and timestamp.

---

## 3. Exit codes (for automation)

Both commands follow the same contract so a script can never mistake a no-op for
success:

| Outcome | Exit | Notes |
|---|---|---|
| Dry-run completed | 0 | reported only; nothing deleted |
| Delete completed | 0 | `--yes` (+ confirmation for erase) |
| Endpoint not deployed (D-7/D-8) | **non-zero** | actionable "not yet deployed" error in **both** human and `--json` mode |
| `erase` confirmation mismatch/refused | **non-zero** | distinguishes a refused erase from a completed one |
| `prune` empty-live-set floor tripped | **non-zero** | refuses the full wipe |
| Partial delete then failure | **non-zero** | reports count deleted before failing; re-run to resume |

`--json` mode prints a structured note to stdout **and** still exits non-zero on
the failure cases above — parse the exit code, not just stdout.

---

## 4. Troubleshooting

- **"requires corelink-server capabilities not yet deployed (D-7/D-8)"** — the
  server endpoints aren't live yet. This is expected pre-#269-deploy and is a
  hard error by design, not a tenant-empty signal. Do not retry as a workaround.
- **`prune` reports nearly all of CAS as orphans** — stop. This usually means the
  ref index returned empty/stale (replica lag, reindex), not that the tenant is
  empty. Verify with `clw ls --all`; the safety floor will already have refused
  a `--yes` run if the live set was fully empty.
- **`erase` aborts with "tenant confirmation did not match"** — the typed/`--confirm-tenant`
  value did not equal the resolved tenant. Re-check `--tenant` and the confirmation
  string; this rail is working as intended.
- **Partial `prune`/`erase` failure** — the command reports how far it got and
  exits non-zero. Both operations are idempotent on re-run (deleting an
  already-absent key is a no-op), so simply re-run to resume.

---

## 5. Related

- Command sources: `crates/clw-cli/src/subcmds/{prune,erase,ls}.rs`
- Hardening record: PR #34 (9 adversarial-review findings closed)
- Compliance framing: `docs/PRIVACY-POLICY.md`, `docs/DPA-subprocessors.md` (Art.17)
- Server dependency: D-7 / D-8 in the dependency ledger (`docs/ROADMAP-go-live.md`)
- Residual GC-lease ask: relayed to the `corelink-server` TL
