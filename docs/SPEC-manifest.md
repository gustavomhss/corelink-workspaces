# CoreLink Workspaces — Manifest Format v1

**Specification status:** Normative  
**Version:** 1  
**Date:** 2026-06-04  
**Source of truth:** the frozen
[`crates/clw-types/src/lib.rs`](https://github.com/HuGR-Labs/corelink-workspaces/blob/2df125f0a2a040dc77f2fa277cfa4f914cdd3f06/crates/clw-types/src/lib.rs)
contract at immutable WP-40 baseline commit `2df125f0a2a040dc77f2fa277cfa4f914cdd3f06`.
This commit reference is intentionally used instead of a file-content hash that must be manually refreshed.

---

## 1. Notation

The key words MUST, MUST NOT, REQUIRED, SHALL, SHOULD, SHOULD NOT, RECOMMENDED,
MAY, and OPTIONAL in this document are to be interpreted as described in RFC 2119.

---

## 2. Digest

All content identifiers in this specification are BLAKE3-256 digests.

- A digest is the 32-byte output of BLAKE3 applied to the content bytes.
- On the wire (JSON values, URL path segments) a digest MUST be represented as 64
  lowercase hexadecimal characters.
- Implementations MUST reject any digest string that is not exactly 64 lowercase hex
  characters.

**JSON type:** string, 64 lowercase hex characters.

Example: `"af1349b9f5f9a1a6a0404dea36dcc949..."`

---

## 3. Manifest

### 3.1 Root object

A manifest is the root object of a workspace snapshot. Its canonical JSON shape is:

```json
{
  "version": 1,
  "total_size": <u64>,
  "entries": [ <Entry>, ... ]
}
```

Fields are serialized in declaration order: `version`, `total_size`, `entries`.

- `version` (integer): MUST equal `1`. Readers MUST reject manifests with any other
  value.
- `total_size` (u64): Sum in bytes of the uncompressed content of all `File` entries.
- `entries` (array): Zero or more `Entry` objects. See §3.2.

### 3.2 Entries

Each element of `entries` is a JSON object with a `"kind"` discriminant field
(`"file"`, `"symlink"`, or `"dir"`, snake_case). The `kind` field MUST appear in the
object. Readers MUST reject entries with unknown `kind` values.

#### 3.2.1 File entry

```json
{
  "kind": "file",
  "path": "<relative-path>",
  "mode": <u32>,
  "size": <u64>,
  "chunks": [ <ChunkRef>, ... ]
}
```

- `path`: relative path (see §3.3).
- `mode`: Unix permission bits (e.g., `420` for `0o644`, `493` for `0o755`).
  Windows targets are out of scope for v1.
- `size`: uncompressed file content size in bytes. MUST equal the sum of all
  `ChunkRef.size` values in `chunks`.
- `chunks`: ordered list of `ChunkRef` objects. The concatenation of the bytes
  identified by each `ChunkRef`, in list order, MUST equal the full file content.
  An empty file MAY have an empty `chunks` array.

#### 3.2.2 Symlink entry

```json
{
  "kind": "symlink",
  "path": "<relative-path>",
  "target": "<target-string>"
}
```

- `path`: relative path of the symlink itself (see §3.3).
- `target`: the symlink target string, stored verbatim. Implementations SHOULD
  validate that targets do not escape the workspace root on hydration.

#### 3.2.3 Dir entry

```json
{
  "kind": "dir",
  "path": "<relative-path>"
}
```

Dir entries MUST be recorded only for empty directories. Non-empty directories are
implied by the paths of their contained `File` and `Symlink` entries; recording them
explicitly is redundant and implementations MUST NOT do so.

### 3.3 Path rules

All `path` values across all entry kinds MUST conform to all of the following rules:

1. **Relative:** MUST NOT begin with `/`.
2. **Separator:** MUST use `/` as the component separator.
3. **No dot-components:** MUST NOT contain `.` or `..` as a path component.
4. **Encoding:** MUST be valid UTF-8.
5. **Non-empty:** MUST NOT be an empty string.

### 3.4 Chunk reference

```json
{
  "digest": "<64-char hex>",
  "size": <u64>
}
```

- `digest`: BLAKE3-256 digest of the chunk bytes (see §2).
- `size`: byte length of the chunk. MUST be greater than zero and MUST be less than
  or equal to `CAS_BLOB_CAP_BYTES` (5 MiB; see §6.2).

---

## 4. Canonical encoding and manifest digest

### 4.1 Canonical form

The canonical byte representation of a manifest (used for digest computation and
for storage in the CAS) is compact `serde_json` serialization with all of the
following properties:

- No extra whitespace (no indentation, no trailing newlines beyond what serde_json
  emits by default).
- Struct fields serialized in **declaration order**: for `Manifest` — `version`,
  `total_size`, `entries`; for `Entry` variants — `kind` then the remaining fields
  in declaration order; for `ChunkRef` — `digest`, `size`.
- The `entries` array MUST be sorted in **ascending byte order** of each entry's
  `path` field (lexicographic byte comparison of the UTF-8 encoding).
- All paths in `entries` MUST be **unique** (no two entries share the same `path`).

### 4.2 Manifest digest

The CAS digest of a manifest is:

```
manifest_digest = BLAKE3(canonical_bytes(manifest))
```

Implementations MUST compute the digest over the canonical bytes defined in §4.1.
Implementations MUST NOT recompute the canonical form from a parsed object unless
the field order and sort invariants are guaranteed to be preserved.

### 4.3 Invariant enforcement

Writers MUST ensure the entries array is sorted and deduplicated before calling
`canonical_bytes`. Readers SHOULD validate sort order and uniqueness on parse and
SHOULD return an error on violation.

---

## 5. Chunking

### 5.1 Algorithm

File content MUST be split into chunks using FastCDC with the following parameters
(frozen — see §5.3):

| Parameter    | Value      |
|--------------|------------|
| Minimum size | 256 KiB    |
| Average size | 1 MiB      |
| Maximum size | 4 MiB      |

These correspond to the constants `CHUNK_MIN_BYTES`, `CHUNK_AVG_BYTES`, and
`CHUNK_MAX_BYTES` in the contract.

### 5.2 Reconstruction invariant

The concatenation of chunks in the order listed in a `File` entry's `chunks` array
MUST exactly equal the file's original content. No padding, no framing, no
compression — raw bytes only.

### 5.3 Frozen parameters

The chunking parameters above are **frozen**. Changing them would change the chunk
boundaries of every file ever stored, destroying cross-snapshot deduplication and
making all existing CAS chunk digests unreachable from new manifests. Any change
to these parameters requires a lead decision record and a coordinated migration;
WP agents MUST NOT alter them.

### 5.4 CAS cap

Every chunk MUST be strictly smaller than `CAS_BLOB_CAP_BYTES` (5 MiB). The
FastCDC maximum of 4 MiB satisfies this constraint by construction, but
implementations MUST NOT upload a chunk of 5 MiB or more (the server will
reject it with HTTP 413). See §6.2.

---

## 6. Workspace refs

### 6.1 Ref key derivation

A workspace ref named `name` is stored in the AC under the key for its selected ref domain:

```
ref_key(user, name)   = BLAKE3("clw/ref/v1/" || name)
ref_key(runner, name) = BLAKE3("clw/ref/runner/v1/" || name)
```

where `||` denotes byte concatenation. `user` is the default; `runner` is selected by the frozen
`CLW_REF_DOMAIN=runner` / `--ref-domain runner` runners seam. The domain separator is part of the
BLAKE3 preimage, so the two keyspaces are disjoint by construction.

The domain prefix provides namespace separation: for either domain, `ref_key("x")` MUST NOT equal
`BLAKE3("x")`. The user and runner derivations are domain-separated and form the client contract's
disjoint keyspaces. Implementations MUST use these exact derivations.

### 6.2 RefRecord format

The value stored at a ref key in the AC is the compact JSON serialization of a
`RefRecord`:

```json
{
  "name": "<workspace name>",
  "root": "<64-char hex digest>",
  "manifest_size": <u64>,
  "created_at_unix": <u64>,
  "parent": "<64-char hex digest> | null",
  "tool_version": "<string>"
}
```

Fields serialized in declaration order: `name`, `root`, `manifest_size`,
`created_at_unix`, `parent`, `tool_version`.

- `name`: the human-readable workspace name.
- `root`: CAS digest of the root `Manifest` canonical bytes.
- `manifest_size`: byte size of the root manifest in the CAS.
- `created_at_unix`: Unix timestamp (seconds) at write time.
- `parent`: digest of the previous root manifest if this updates an existing ref,
  otherwise JSON `null`. Records lineage for history and debugging.
- `tool_version`: version string of the `clw` binary that wrote this record.

### 6.3 Concurrency model (v1)

The server contract is **atomic put-if-absent**: a `PUT` on a ref key returns 201 for a
brand-new key, 200 for an idempotent re-PUT of the identical value, and 409 when the key
already holds a **divergent** value. There is genuinely no compare-and-swap token — no
`If-Match`, no `ETag`, no generation number is sent on the AC PUT. The `parent` field is
lineage only, recorded for post-hoc inspection, and is never read on the write path.

*Replacing* an existing name is therefore delete-then-put: a client that wants to overwrite
a divergent occupant must delete the ref and PUT again, a non-atomic window in which two
concurrent `--force` writers can race. An identical concurrent write is not part of that
race — the server's idempotent 200 (and a client-side reconcile that treats a matching
occupant on 409 as a safe no-op) makes a duplicate write of the same value harmless; only a
genuinely divergent occupant errors. Future versions MAY extend this with richer optimistic
concurrency (e.g. an explicit CAS token).

---

## 7. Transport

### 7.1 Endpoints

| Operation        | Method | URL pattern                                   |
|------------------|--------|-----------------------------------------------|
| CAS blob read    | GET    | `/v1/cas/:tenant/:hash`                       |
| CAS blob write   | PUT    | `/v1/cas/:tenant/:hash`                       |
| AC value read    | GET    | `/v1/ac/:tenant/:key`                         |
| AC value write   | PUT    | `/v1/ac/:tenant/:key`                         |

- `:tenant` is the tenant identifier string.
- `:hash` and `:key` are 64-char lowercase hex BLAKE3 digests.

Base URL example: `https://corelink-api.humangr.com`

### 7.2 Authentication

All requests MUST carry a CoreLink Personal Access Token (PAT) in the HTTP
`Authorization` header:

```
Authorization: Bearer <token>
```

A missing or invalid token results in HTTP 401. Implementations MUST surface 401
as an authentication error and MUST NOT retry with the same token.

The static PAT MUST be sourced from `CLW_TOKEN` environment variable, `~/.clw/config.toml`,
or a CLI flag (in precedence order: flag > env > config). A static PAT MUST NEVER be logged or
included in error messages, and MAY be written only to the designated static-token config location.
When selected by the current command gate, the runner C2c path instead redeems its single-use credential
ticket for a scoped per-job PAT before CAS/AC use; it does not send the ticket as that bearer token.
A C2c ticket MUST NEVER be logged,
included in error messages, or written to `~/.clw/config.toml` or any other file. A redeemed PAT is
initially held in process memory; commands that persist a resolved identity can write that PAT to the
designated static-token config location. The current redemption gate is documented in
[`CLI-CONTRACT.md`](CLI-CONTRACT.md).

### 7.3 CAS integrity — clients MUST verify

On every CAS GET response, clients MUST:

1. Compute `BLAKE3(response_body)`.
2. Compare it to the expected digest from the URL path segment or from the
   `ChunkRef.digest` / `Manifest.root` field that triggered the request.
3. If the digests do not match, discard the bytes and return a
   `ClwError::Integrity` error. MUST NOT use or cache the corrupted bytes.

This rule is not optional. An implementation that skips integrity verification
is non-conformant.

### 7.4 CAS PUT size limit

A CAS PUT body MUST NOT exceed `CAS_BLOB_CAP_BYTES` (5,242,880 bytes, i.e., 5 MiB).
The server enforces this limit and returns HTTP 413 on violation. Implementations
MUST check the size before sending and return `ClwError::TooLarge` locally without
making a network request when the limit would be exceeded.

---

## 8. Summary of MUST/SHOULD/MAY requirements

| § | Requirement |
|---|-------------|
| 2 | Digest MUST be 64 lowercase hex chars on the wire. |
| 3.1 | `version` MUST equal `1`; readers MUST reject other values. |
| 3.2 | `kind` MUST be present; unknown kinds MUST be rejected. |
| 3.2.3 | Dir entries MUST NOT be recorded for non-empty directories. |
| 3.3 | Paths MUST be relative, `/`-separated, no `.`/`..`, valid UTF-8. |
| 4.1 | Canonical form: compact JSON, declaration field order, entries sorted ascending by path (byte order), unique paths. |
| 4.2 | Manifest digest MUST be `BLAKE3(canonical_bytes)`. |
| 5.1 | Chunks MUST use FastCDC with frozen parameters. |
| 5.3 | Chunking parameters MUST NOT be changed without a lead decision record. |
| 5.4 | Every chunk MUST be < 5 MiB. |
| 6.1 | Ref key MUST be `BLAKE3("clw/ref/v1/" || name)` in the user domain or `BLAKE3("clw/ref/runner/v1/" || name)` in the runner domain. |
| 6.3 | v1 concurrency is atomic put-if-absent (201 new / 200 idempotent / 409 divergent), not last-write-wins; future MAY add a CAS token. |
| 7.2 | All requests MUST use `Authorization: Bearer <token>`. PAT MUST NEVER be logged. |
| 7.3 | Clients MUST verify BLAKE3 digest on every CAS GET response. |
| 7.4 | CAS PUT body MUST NOT exceed 5 MiB. |
