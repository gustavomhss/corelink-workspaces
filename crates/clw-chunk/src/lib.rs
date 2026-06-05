//! clw-chunk — content-defined chunking via FastCDC (v2020).
//!
//! Chunking parameters (`CHUNK_MIN_BYTES`, `CHUNK_AVG_BYTES`, `CHUNK_MAX_BYTES`)
//! are frozen in `clw-types`. Changing them re-chunks the world and destroys
//! cross-snapshot deduplication — never alter without a lead decision record.

use clw_types::{ChunkRef, Digest, CHUNK_AVG_BYTES, CHUNK_MAX_BYTES, CHUNK_MIN_BYTES};
use fastcdc::v2020::FastCDC;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single content-defined chunk produced by [`chunk_bytes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub digest: Digest,
    pub offset: u64,
    pub len: u64,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Split `data` into content-defined chunks using FastCDC (v2020) with the
/// frozen chunking parameters from `clw-types`.
///
/// - Empty input returns an empty `Vec`.
/// - Every chunk satisfies `len <= CHUNK_MAX_BYTES < CAS_BLOB_CAP_BYTES`.
/// - The chunks tile `data` exactly (contiguous, no gaps, no overlaps).
pub fn chunk_bytes(data: &[u8]) -> Vec<Chunk> {
    if data.is_empty() {
        return Vec::new();
    }

    let chunker = FastCDC::new(
        data,
        CHUNK_MIN_BYTES as u32,
        CHUNK_AVG_BYTES as u32,
        CHUNK_MAX_BYTES as u32,
    );

    chunker
        .map(|entry| {
            let offset = entry.offset as u64;
            let len = entry.length as u64;
            let slice = &data[entry.offset..entry.offset + entry.length];
            let digest = Digest::of_bytes(slice);
            Chunk {
                digest,
                offset,
                len,
            }
        })
        .collect()
}

/// Convert a slice of [`Chunk`]s to the `ChunkRef` form used in manifests.
pub fn to_chunk_refs(chunks: &[Chunk]) -> Vec<ChunkRef> {
    chunks
        .iter()
        .map(|c| ChunkRef {
            digest: c.digest,
            size: c.len,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{CAS_BLOB_CAP_BYTES, CHUNK_MAX_BYTES};
    use proptest::prelude::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Simple xorshift64 PRNG (deterministic, seeded).
    fn xorshift64(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    /// Fill a buffer of `n` bytes deterministically using xorshift64.
    fn det_buf(n: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        let mut buf = Vec::with_capacity(n);
        while buf.len() < n {
            let word = xorshift64(&mut state).to_le_bytes();
            let needed = n - buf.len();
            buf.extend_from_slice(&word[..needed.min(8)]);
        }
        buf
    }

    // -----------------------------------------------------------------------
    // Structural invariants
    // -----------------------------------------------------------------------

    fn assert_tiling(data: &[u8], chunks: &[Chunk]) {
        let mut expected_offset: u64 = 0;
        for c in chunks {
            assert_eq!(
                c.offset, expected_offset,
                "chunk at offset {} but expected {}",
                c.offset, expected_offset
            );
            assert!(c.len > 0, "chunk length must be > 0");
            expected_offset += c.len;
        }
        assert_eq!(
            expected_offset,
            data.len() as u64,
            "sum of chunk lengths ({}) != data.len() ({})",
            expected_offset,
            data.len()
        );
    }

    fn assert_bounds(chunks: &[Chunk]) {
        for c in chunks {
            assert!(
                c.len <= CHUNK_MAX_BYTES as u64,
                "chunk len {} exceeds CHUNK_MAX_BYTES {}",
                c.len,
                CHUNK_MAX_BYTES
            );
            assert!(
                c.len < CAS_BLOB_CAP_BYTES as u64,
                "chunk len {} >= CAS_BLOB_CAP_BYTES {}",
                c.len,
                CAS_BLOB_CAP_BYTES
            );
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input_yields_empty_vec() {
        assert_eq!(chunk_bytes(&[]), Vec::new());
    }

    #[test]
    fn single_byte_yields_one_chunk() {
        let chunks = chunk_bytes(&[0x42]);
        assert_eq!(chunks.len(), 1, "expected exactly 1 chunk for 1-byte input");
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].len, 1);
        assert_eq!(chunks[0].digest, Digest::of_bytes(&[0x42]));
    }

    #[test]
    fn tiling_small_input() {
        let data = b"hello world, this is a small test payload";
        let chunks = chunk_bytes(data);
        assert_tiling(data, &chunks);
        assert_bounds(&chunks);
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let data = det_buf(2 * 1024 * 1024, 0xdeadbeef_cafebabe);
        let chunks1 = chunk_bytes(&data);
        let chunks2 = chunk_bytes(&data);
        assert_eq!(chunks1, chunks2, "chunking must be deterministic");
    }

    #[test]
    fn nine_mib_buffer_multi_chunk_within_bounds() {
        let nine_mib = 9 * 1024 * 1024;
        let data = det_buf(nine_mib, 0x0123456789abcdef);
        let chunks = chunk_bytes(&data);

        // 9 MiB with avg=1 MiB must produce more than 1 chunk.
        assert!(
            chunks.len() > 1,
            "9 MiB buffer produced only {} chunk(s); expected >1",
            chunks.len()
        );

        assert_tiling(&data, &chunks);
        assert_bounds(&chunks);

        // Digests must match the actual slice content.
        for c in &chunks {
            let slice = &data[c.offset as usize..c.offset as usize + c.len as usize];
            assert_eq!(c.digest, Digest::of_bytes(slice));
        }
    }

    #[test]
    fn to_chunk_refs_maps_correctly() {
        let data = det_buf(3 * 1024 * 1024, 0xfeedfacecafe1234);
        let chunks = chunk_bytes(&data);
        let refs = to_chunk_refs(&chunks);

        assert_eq!(refs.len(), chunks.len());
        for (chunk, chunk_ref) in chunks.iter().zip(refs.iter()) {
            assert_eq!(chunk_ref.digest, chunk.digest);
            assert_eq!(chunk_ref.size, chunk.len);
        }
    }

    // -----------------------------------------------------------------------
    // Proptest
    // -----------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        /// Tiling and bounds invariants hold for random inputs up to ~3 MiB.
        #[test]
        fn prop_tiling_and_bounds(data in proptest::collection::vec(any::<u8>(), 0..=3 * 1024 * 1024)) {
            let chunks = chunk_bytes(&data);

            // Empty → empty.
            if data.is_empty() {
                prop_assert!(chunks.is_empty());
                return Ok(());
            }

            // Tiling: contiguous from 0, sum == data.len().
            let mut expected_offset: u64 = 0;
            for c in &chunks {
                prop_assert_eq!(c.offset, expected_offset);
                prop_assert!(c.len > 0);
                expected_offset += c.len;
            }
            prop_assert_eq!(expected_offset, data.len() as u64);

            // Bounds.
            for c in &chunks {
                prop_assert!(c.len <= CHUNK_MAX_BYTES as u64);
                prop_assert!(c.len < CAS_BLOB_CAP_BYTES as u64);
            }
        }

        /// Chunking is deterministic: same input → identical chunk list.
        #[test]
        fn prop_deterministic(data in proptest::collection::vec(any::<u8>(), 0..=1024 * 1024)) {
            let chunks1 = chunk_bytes(&data);
            let chunks2 = chunk_bytes(&data);
            prop_assert_eq!(chunks1, chunks2);
        }
    }
}
