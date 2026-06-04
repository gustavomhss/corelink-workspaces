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
  AcTransport::put(ref_key(name), RefRecord::to_bytes())
```

### Hydrate

```
  AcTransport::get(ref_key(name))
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
  [hash inputs]      -- deterministic digest of: command + args + relevant env +
                         workspace manifest digest
       |
       v
  AcTransport::get(input_digest)
       |
       +-- HIT  --> [replay]  -- deserialize cached outputs/exit-code,
       |                         reconstruct output files from CAS chunks,
       |                         return cached exit code
       |
       +-- MISS --> [execute] -- run the command in the current workspace
                       |
                       v
                   [collect outputs] -- capture stdout, stderr, output files
                       |
                       v
                   [store outputs]   -- chunk + upload output files to CAS
                       |
                       v
                   AcTransport::put(input_digest, serialized_result)
                       |
                       v
                   return actual exit code
```

---

## Security notes

- **PAT handling:** the CoreLink PAT is read from `CLW_TOKEN` environment variable,
  `~/.clw/config.toml`, or a CLI flag (flag > env > config precedence). It is sent
  as `Authorization: Bearer <token>` on every request. The PAT is NEVER logged,
  never included in error messages, and never written anywhere outside the
  designated config file.
- **Integrity verification:** every CAS GET response MUST be verified against its
  expected BLAKE3 digest before any bytes are used or cached. A mismatch raises
  `ClwError::Integrity` and the bytes are discarded.
- **Workspace privacy:** workspaces are private by default. Only content with
  identical digests (public, deterministic dependencies) is shared across tenants
  via the content-addressed layer; private workspace content is isolated by tenant
  prefix. No private content crosses tenant boundaries.
- **Pure client:** this repo contains zero server code. It is a client of the live
  CoreLink API (`/v1/cas`, `/v1/ac` at `corelink-api.humangr.com`). Server behavior,
  auth enforcement, and tenant isolation are implemented in `corelink-server` and
  are outside the scope of this codebase.
