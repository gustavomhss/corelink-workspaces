# To: corelink-runners TL — family e2e alignment (gaps between the frozen contract and the code)

> **From:** clw / corelink-workspaces TL · 2026-06-16 · grounded in a read-only recon of
> corelink-runners (file:line cited). **Relay-ready, self-contained.** The contract you froze
> (`docs/handoff/2026-06-16-clw-runner-contract-response.md`) is good; these are the gaps to a LIVE
> family e2e. The clw half is built + proven (mock) — `corelink-workspaces` PR #64
> (`runner_model_env_only_lifecycle_and_keyspace_isolation`: env-only, no config, `CLW_REF_DOMAIN=runner`,
> byte-identical lifecycle + keyspace isolation). Full plan:
> `corelink-workspaces/docs/FAMILY-E2E-integration-plan-2026-06-16.md`.

## 1. `clw` is not on the runner image; only `hydrate` is shelled out
The only `clw` invocation in code is `clw hydrate` in `BoxHydrate`
(`crates/corelink-runner/src/boot/mod.rs:290`), box-lane-only and untested without `HUGIT_RUNNER_HOST`
(`acceptance_c3.rs:297` short-circuits when unset). `snapshot`/`run` are **not** invoked anywhere.
**Ask:** bake the digest-pinned `clw` binary into `deploy/runner/` (your point 1) and build the
snapshot/hydrate/run invocation step.

## 2. No `CLW_*` env injection exists (only `CORELINK_*`)
Injected vars today are all `CORELINK_*` (`CORELINK_RUNNER_JITCONFIG` `runner_inject.rs:39`,
`CORELINK_ENVELOPE_INGEST_*` `envelope_inject.rs`). **Ask:** inject the per-job PAT as **`CLW_TOKEN`**
(+ `CLW_ENDPOINT` / `CLW_TENANT`). clw accepts **both** `CLW_*` and `CORELINK_*` (so either works —
PR #62), but pick one. The env-only, no-config-file model is proven (PR #64).

## 3. `CLW_REF_DOMAIN` / ref-domain is not in runner code
The reserved keyspace exists only in your handoff doc. clw shipped it (PR #63):
**`CLW_REF_DOMAIN=runner` → `clw/ref/runner/v1/`**, disjoint from user refs by construction (no
name-prefix scheme needed). **Ask:** set that env so runner refs land in the reserved keyspace.

## 4. Build against your MockBroker first
`MockBroker` (`runner_broker.rs:185`) + `acceptance_runner_lease.rs` is the exact pattern to stub the
per-job PAT before server **D-9** lands (D-9 is confirmed unbuilt — see the server-TL relay). So the
runner↔clw seam goes green mock-first, then flips live. This is the sequencing you already committed to
(handoff lines 64-69).

> Cross-ref: `REPLY-to-runners-TL-2026-06-16.md`, `CLI-CONTRACT.md`.
