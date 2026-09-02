# CoreLink Workspaces — Architecture

## Thesis

The workspace — source, dependencies, toolchain, and build state — is a
content-addressed object in CoreLink's CAS; compute is a disposable cursor over it.
Any machine (a CI runner, an AI agent's sandbox, a developer's laptop) hydrates a
workspace in seconds from a cache that grows warmer with every snapshot, then
attaches, works, writes back a delta as a new immutable manifest, and is discarded —
the durable object is the workspace, not the machine that held it.

---

## Crate graph

```
clw-types
    ^
    |
    +------ clw-chunk
    |
    +------ clw-client
    |
    +------ clw-manifest
    |
    +------ clw-cache
                ^
                |
                +------ clw-snapshot
                |
                +------ clw-hydrate
                |
                +------ clw-run
                            ^
                            |
                        clw-cli

integration-tests/clw-conformance  (reads clw-types; drives the live API)
```

`clw-types` is the frozen contract (types, traits, constants). All other crates
depend on it; none of them depend on each other except through the layers shown
above. `clw-cli` is the leaf that wires the commands together. `clw-conformance`
is a test-only crate that exercises the live CoreLink API surfaces directly.

---

## Dataflows

### Snapshot

```
Working directory
       |
       v
  [walk tree]        -- enumerate files, symlinks, empty dirs
       |
       v
  [FastCDC chunk]    -- split each file: 256 KiB min / 1 MiB avg / 4 MiB max
       |
       v
  [dedupe check]     -- CasTransport::exists(chunk_digest) per chunk
       |
       +-- already in CAS --> skip upload
       |
       +-- not in CAS    --> CasTransport::put(chunk_digest, bytes)
       |
       v
  [build Manifest]   -- collect Entry{File/Symlink/Dir}, sort by path (byte order),
                        compute total_size, version=1
       |
       v
  [canonical_bytes]  -- compact JSON, declaration field order
       |
       v
  [Manifest digest]  -- BLAKE3(canonical_bytes)
       |
       v
  CasTransport::put(manifest_digest, canonical_bytes)
       |
       v
  [write RefRecord]  -- {name, root=manifest_digest, manifest_size, created_at_unix,
                         parent=prev_root|null, tool_version}
       |
       v
  AcTransport::put(ref_key_in(active_ref_domain, name), RefRecord::to_bytes())
```

### Hydrate

```
  AcTransport::get(ref_key_in(active_ref_domain, name))
       |
       v
  [parse RefRecord]  -- extract root manifest digest
       |
       v
  CasTransport::get(root)     -- fetch + integrity-verify manifest bytes
       |
       v
  [parse Manifest]   -- deserialize entries
       |
       v
  [parallel fetch]   -- for each ChunkRef across all File entries:
                          cache-first: check clw-cache (local content cache)
                              hit  --> use cached bytes
                              miss --> CasTransport::get(chunk_digest)
                                          + integrity-verify
                                          + write to local cache
       |
       v
  [atomic materialize] -- reconstruct files by concatenating chunks in order,
                           restore symlinks, create empty dirs,
                           apply mode bits; write to destination atomically
```

### Run (memoized command)

```
  [derive run key]   -- BLAKE3 `clw/run/v3/` key over declared input digests
                         (+ Unix modes), declared env presence/values, command,
                         resolved tool identity, and canonical cwd
       |
       v
  AcTransport::get(run_key)
       |
       +-- HIT  --> [replay]  -- deserialize { exit_code, stdout, stderr },
       |                         fetch stdout/stderr blobs from CAS, replay streams
       |
       +-- MISS --> [execute] -- run the command in the current workspace
                       |
                       v
                   [capture streams] -- capture stdout and stderr (each capped)
                       |
                       v
                   [success only]    -- on exit 0, best-effort cache stdout/stderr
                       |                blobs and { exit_code, stdout, stderr } in AC
                       |
                       v
                   return actual exit code
```

The run cache records no output-file list and does not reconstruct output files on a hit. Non-zero,
signalled, oversized-output, corrupt, or unfetchable-cache cases are not successful cache records (a
corrupt or unfetchable record falls through to a miss). See
[`clw-run`](../crates/clw-run/src/lib.rs) for the key and record implementation.

---

## Security notes

- **PAT handling:** a static CoreLink PAT is read from `CLW_TOKEN`,
  `~/.clw/config.toml`, or a CLI flag (flag > env > config precedence), and may be
  persisted only in that owner-only config file. In the runner C2c path, the
  command-gated, single-use `CLW_CRED_TICKET` plus lease/fabric binding is redeemed
  at trusted boot for a scoped per-job PAT; the ticket is never the CAS/AC bearer
  token and is never written to config or another file. Redemption is fail-closed
  with no `CLW_TOKEN` fallback. The static or redeemed PAT is sent as
  `Authorization: Bearer <token>` on CAS/AC requests; neither PAT nor ticket is
  logged or included in error messages. See the current gate in
  [`main.rs`](../crates/clw-cli/src/main.rs) and credential resolution in
  [`config.rs`](../crates/clw-cli/src/config.rs).
- **Integrity verification:** every CAS GET response MUST be verified against its
  expected BLAKE3 digest before any bytes are used or cached. A mismatch raises
  `ClwError::Integrity` and the bytes are discarded.
- **Tenant isolation:** every client CAS and AC URL contains the configured tenant
  (`/v1/cas/{tenant}/…` and `/v1/ac/{tenant}/…`), so `clw` has no cross-tenant
  lookup, sharing, or deduplication path. The tenant is the remote-sharing boundary;
  whether a server deduplicates physical storage below that API is server behavior,
  not a client contract. See the matching [GUIDE tenant model](GUIDE.md#who-shares-what--you-your-team-ci).
- **Pure client:** this repo contains zero server code. It is a client of the live
  CoreLink API (`/v1/cas`, `/v1/ac` at `corelink-api.humangr.com`). Server behavior,
  auth enforcement, and tenant isolation are implemented in `corelink-server` and
  are outside the scope of this codebase.
