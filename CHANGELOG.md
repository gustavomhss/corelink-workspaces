# Changelog

## [Unreleased] — phase-1 complete (2026-06-05)

### Added
- **The workspace primitive, client-side** — `clw` CLI: `init`, `snapshot`, `hydrate`, `status`, `run`, `ls` (#13, #17).
- `clw-types`: frozen contract — BLAKE3 digests, manifest v1 (canonical JSON, Merkle-style), RefRecord namespace over the AC, CasTransport/AcTransport traits (contract hash fe251941).
- `clw-chunk`: FastCDC chunking, frozen params 256K/1M/4M (#7).
- `clw-client`: HTTP CAS/AC transports with content-integrity verification, retry, PAT auth (#5).
- `clw-manifest`: ignore-aware tree walk + manifest diff (#3).
- `clw-cache`: atomic local content store, self-healing reads, LRU gc (#4).
- `clw-snapshot`: walk→chunk→dedupe→upload→ref pipeline; second snapshot of an unchanged tree uploads zero chunks (#10).
- `clw-hydrate`: ref→manifest→cache-first parallel materialization, atomic writes, mode bits (#11).
- `clw-run`: AC-backed command memoization — deterministic commands cached, replayed byte-exact, success-only (#9).
- Integration suite over real HTTP (wiremock stateful CoreLink) (#12) and binary-level e2e: snapshot→wipe→hydrate bit-identical roundtrip (#16).
- Live-API conformance suite, feature-gated (`--features live`, env-skip without PAT) (#14).
- Docs: SPEC-manifest v1 + ARCHITECTURE (#2), real README + usage recipes (#15).

### CI
- DCO gate, PR template (#1); change-scoped gates — PR lane scoped to the diff, main always full-lane; persistent CI target dir (#8).

### Fixed
- Compile-time const-assert for chunk params (clippy clean) (#6).
