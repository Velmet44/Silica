//! FNV-1a 64 and the canonical state-hash machinery (SPEC.md §8 D7,
//! appendix A).

/// The FNV-1a 64-bit running hash state.
///
/// Streamed: callers push byte slices in hash order and finalize with
/// [`Fnv1a64::finish`]. No intermediate buffer is ever allocated, so
/// [`World::state_hash`](crate::world::World::state_hash) can stream the
/// whole world through it (`SPEC.md` §5, zero-allocation tick).
///
/// Public because the canonical hashes of sibling crates must reuse the
/// same primitive (the DSL's D11 pack hash, the formats crate's save
/// encoding) — one FNV-1a 64 implementation in the engine, nowhere
/// else.
///
/// The primitive is pinned unit-test-first against the published FNV
/// test vectors (draft-eastlake-fnv's `test_fnv.c` harness): FNV over
/// the draft's four test strings plus their `"\0"`-terminated block
/// variants. Any deviation would shift every golden hash, so this is
/// test-first material, not an implementation detail.
pub struct Fnv1a64 {
    state: u64,
}

impl Fnv1a64 {
    /// The FNV-1a 64 offset basis.
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    /// The FNV-1a 64 prime (ungrouped — grouping it in nibbles would
    /// produce an uneven byte-aligned pattern).
    const PRIME: u64 = 0x100000001b3;

    /// Starts a hash at the FNV-1a 64 offset basis.
    pub fn new() -> Fnv1a64 {
        Fnv1a64 {
            state: Self::OFFSET_BASIS,
        }
    }

    /// Mixes a byte slice into the running state.
    ///
    /// Determinism: XOR-then-multiply per byte, exactly as FNV-1a; the
    /// multiply uses `wrapping_mul` because it can overflow (§8 D2) —
    /// the mod-2^64 reduction *is* the specification.
    pub fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    /// Finalizes and returns the 64-bit hash.
    pub fn finish(self) -> u64 {
        self.state
    }
}

impl Default for Fnv1a64 {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot convenience hash for tests and diagnostics.
#[cfg(test)]
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = Fnv1a64::new();
    h.update(bytes);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::{Fnv1a64, fnv1a64};

    /// Published FNV-1a 64 vectors — draft-eastlake-fnv's `test_fnv.c`
    /// `FNV64svalues` (FNV over each test string `""`, `"a"`, `"foobar"`,
    /// `"Hello!\x01\xFF\xED"`) followed by `FNV64bvalues` (the same
    /// strings with a trailing NUL byte). The `"\0"` variants matter
    /// because the canonical world-hash buffer is dense with zero bytes.
    /// `b"b"` is not in the draft's four-string set; it was recomputed
    /// with an independent JS reference before being locked here.
    #[test]
    fn matches_the_published_draft_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"\0"), 0xaf63_bd4c_8601_b7df);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"a\0"), 0x089b_e207_b544_f1e4);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a64(b"foobar\0"), 0x3453_1ca7_168b_8f38);
        assert_eq!(fnv1a64(b"Hello!\x01\xFF\xED"), 0xbd51_ea70_94ee_6fa1);
        assert_eq!(fnv1a64(b"Hello!\x01\xFF\xED\0"), 0xa0a0_fe4d_1127_ae93);
        assert_eq!(fnv1a64(b"b"), 0xaf63_df4c_8601_f1a5);
    }

    #[test]
    fn streaming_in_chunks_matches_one_shot() {
        let expected = fnv1a64(b"Hello!\x01\xFF\xED\0");
        let mut streamed = Fnv1a64::new();
        streamed.update(b"Hello");
        streamed.update(b"!\x01");
        streamed.update(&[]);
        streamed.update(b"\xFF\xED\0");
        assert_eq!(streamed.finish(), expected);
    }

    #[test]
    fn empty_input_is_the_offset_basis() {
        assert_eq!(fnv1a64(&[]), 0xcbf2_9ce4_8422_2325);
    }
}
