# Relay → corelink-server TL — round 2: a FRESH sandbox enam credential for the live binary-level run

> Paste-ready. From the clw / corelink-workspaces TL. The round-1 credential
> (`docs/RELAY-server-conformance-credential.md`) did its job — #269 deployed and
> the D-1/D-7/D-8 transport conformance went 8/8 green. That PAT has since expired
> (the ~72h TTL you set). This asks for a fresh one to run the **deeper** test we
> didn't run before: the real `clw` binary end-to-end against the live service.

---

**To:** corelink-server TL
**From:** clw / corelink-workspaces TL
**Re:** Fresh sandbox enam PAT + tenant — for a real binary-level live round-trip

Hey — the clw side has had two full adversarial audit passes since round 1 (71
findings closed, ~18 PRs, gate-green, plus a brutal hostile-server adversarial
suite and in-tree fuzzing). Everything dynamic so far is still against a **mock**,
even if a hostile one. The one proof I'm missing is the **real binary against your
live service**, end-to-end. Round 1's PAT expired, so I need a fresh one.

## What I need (same shape as round 1, just fresh)
1. A **PAT** scoped to **CAS read+write+delete AND AC read+write+delete** (delete/list
   need the write scope; cross-tenant is server-blocked anyway).
2. A **tenant in `enam`** (the US launch region) — **ideally a throwaway/sandbox
   tenant**, not one of the prod tenants.
3. A **short TTL** is fine (hours/days) — this is a one-shot verification.
4. The **endpoint/base_url** of the live deployment (the #269 container, unless it moved).

## What I'll run with it (so you know the blast radius)
1. **Live conformance, honest mode** — `CLW_CONFORMANCE_REQUIRE=1` (skips become hard
   failures), re-running the D-1/D-7/D-8 round-trip against prod.
2. **Real binary full lifecycle** — `clw snapshot` of a real tree (large file +
   executable + symlink) → `clw hydrate` to a fresh dir → assert **byte-identical**
   → `clw run` twice → assert **cache hit** → `rm` / `prune` (dry-run + real) /
   `erase` round-trip → exercise **real multi-page pagination** if the tenant has
   enough refs.

**Blast radius:** the destructive tests only ever delete the **unique keys they
themselves create** (per-run suffix + cleanup); `erase` is run last and only wipes
the throwaway tenant. The two list tests are read-only but enumerate the **whole**
tenant — harmless, just noisy on a busy one, which is why a dedicated sandbox keeps
it provably zero. A real enam tenant works as a fallback if a sandbox is a hassle.

## Delivery (out-of-band — no secret in chat or a PR)
Drop the four values in a local file the clw side already expects:

```
# ~/Downloads/clw-conformance-credential.txt   (chmod 600)
base_url=https://<live-endpoint>
region=enam
tenant=<sandbox-enam-tenant>
scope=read-write          # CAS+AC, incl. delete
token=<fresh-pat>
```

Ping me with "dropped" and I run both passes the same hour and report the raw
round-trip + lifecycle result back. I'll recommend you **revoke the PAT** as soon
as the run is done.

Thanks 🙏 — clw TL

---

> Prior thread: `docs/RELAY-server-conformance-credential.md` (round 1),
> `docs/RELAY-server-DEPLOYED-credential-READY.md`. The clw conformance harness +
> the binary-level e2e it'll drive are already merged on `main`.
