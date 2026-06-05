# HANDOFF — signup-worker hardcodes every tenant's region as `enam`

> **For:** the corelink-server session owning `apps/signup-worker/` (the GAP-5 /
> onboarding lane). **From:** the AC-500 incident investigation (2026-06-05).
> **Why a handoff, not a fix:** the file is in your active lane — touching it from
> another session risks an off-baseline collision. All evidence is below; the fix
> is small and self-contained.

---

## TL;DR

Every new tenant is written to D1 with `primary_region = 'enam'`, **hardcoded** —
the region derived from the user's Cloudflare colo is computed, passed along, and
then **silently discarded**. Two defects stack:

1. `createTenant` hardcodes the literal `'enam'` in the INSERT.
2. `configureTenant` (which receives the derived region) is a **no-op**.
3. (Adjacent) `regionFromColo` returns a **raw colo code** (e.g. `"iah"`, `"gru"`)
   or `"auto"` — neither is a valid macro-region nor a valid AC region.

**Impact today:** low — single-region traffic works because `enam` happens to map
through. **Impact at scale:** this is the data-residency correctness path. A
customer who contractually requires EU residency (GDPR) would be silently pinned
to North America. It is a **compliance bug wearing a cosmetic disguise**, and the
product *sells* regional residency. Fix before that promise is load-bearing.

> Note: this is a SEPARATE issue from the AC-500 prod outage (already fixed in
> corelink-server PR #139). The `enam` value itself is schema-valid and was NOT
> the cause of the 500s — that was an empty `R2_AC_BUCKET` env var. This handoff
> is purely the latent multi-region correctness gap surfaced during that dig.

---

## Evidence (`apps/signup-worker/src/webhooks/clerk.ts`)

**Defect 1 — hardcoded region in the INSERT** (line ~454-456):
```ts
"(tenant_id, primary_region, tenant_state, email_hash, clerk_user_id, " +
...
"VALUES (?1, 'enam', 'active', ?2, ?3, ?4, ?4, ?4, ?4)",
//             ^^^^^^ hardcoded — the derived region never reaches the column
```

**Defect 2 — `configureTenant` is a no-op** (line ~464-476):
```ts
// The region is embedded in the tenant row at INSERT time (primary_region
// defaults to 'enam'; per-colo override would update this column).
// For now this is a no-op ...
async configureTenant(_tenantId, _region, _plan): Promise<void> {
  // no-op: tenant row already written in createTenant.
},
```
The derived `region` is passed in at the call site (line ~264:
`await input.api.configureTenant(tenant.id, region, "free")`) and dropped on the floor.

**Defect 3 — `regionFromColo` yields invalid region literals** (line ~141-144):
```ts
export function regionFromColo(colo: string | undefined | null): string {
  if (!colo || typeof colo !== "string") return "auto";
  return colo.toLowerCase();   // e.g. "iah", "gru", "lhr" — a COLO, not a region
}
```
The canonical AC regions are exactly `sam | iad | lhr | nrt | syd`
(`crates/corelink-ac/src/schema/region.rs:35`), and the D1 `tenant.primary_region`
CHECK allows `wnam | enam | weur | sam | apac | afr`. A raw colo like `"iah"`
satisfies neither set — so even if the value were written, it would violate the
CHECK or fail region resolution.

---

## Recommended fix (small, self-contained)

1. **Add a colo→macro-region map** in `regionFromColo` (or a new helper):
   map each Cloudflare colo to one of the 6 valid `primary_region` literals,
   defaulting to `enam` ONLY as an explicit fallback for unknown colos (so the
   default is intentional, not a silent hardcode). Example shape:
   `IAH/EWR/... → enam`, `LHR/CDG/FRA/... → weur`, `GRU/... → sam`, etc.
2. **Use the derived region in the INSERT** — bind it as a parameter instead of
   the `'enam'` literal in `createTenant`. (Simplest single-point fix.)
3. **Either** make `configureTenant` do the `UPDATE primary_region` it documents,
   **or** delete it and fold region selection into `createTenant`. Pick one — a
   no-op that pretends to configure is a trap for the next reader.
4. **Test:** a unit test asserting that a non-NA colo (e.g. `LHR`) produces a
   non-`enam` `primary_region`, and that an unknown colo falls back to a valid
   literal (not a raw colo string).

## Verification after fix

```bash
# Fresh signup from an EU-colo request should land a EU primary_region:
#   SELECT primary_region FROM tenant WHERE clerk_user_id = '<new user>'
#   → expect 'weur' (not 'enam', not 'lhr')
```

## Cross-references

- Full incident forensics: `corelink-workspaces/docs/BUGREPORT-prod-ac-500.md`
- Valid AC regions: `crates/corelink-ac/src/schema/region.rs:35`
- D1 region CHECK: `migrations/002_auth_tables.sql` (tenant table)
- Call site that discards the region: `clerk.ts:~264`
