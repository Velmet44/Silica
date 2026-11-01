//! The single deterministic PRNG: xoshiro128** (SPEC.md §8 D4).

/// The world's one PRNG stream: xoshiro128\*\*, seeded from the 64-bit
/// world seed.
///
/// Seeding expands the seed with SplitMix64 (Vigna's recommended seeding
/// for the xoshiro family): `state[i]` is the low 32 bits of the i-th
/// successive SplitMix64 output, in stream order. The expansion cannot
/// produce the all-zero state, xoshiro's one degenerate fixed point.
///
/// Determinism (SPEC.md §8 D4): there is exactly one stream per world; it
/// it is drawn from only at the documented consumption points. The
/// first live one is `aux_init` at spawn; reaction attempts, gas
/// jitter, and crumble jitter join when their §5/§6 passes land. RNG
/// state is part of world state: hashed, saved, replayed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Rng {
    s: [u32; 4],
}

impl Rng {
    /// Seeds the stream from a 64-bit world seed via SplitMix64.
    pub(crate) fn new(seed: u64) -> Rng {
        let mut x = seed;
        let mut s = [0u32; 4];
        for slot in &mut s {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *slot = (z ^ (z >> 31)) as u32;
        }
        debug_assert!(
            s != [0, 0, 0, 0],
            "degenerate all-zero RNG state from splitmix64 seeding"
        );
        Rng { s }
    }

    /// Draws the next 32 bits from the stream (xoshiro128\*\*).
    pub(crate) fn next_u32(&mut self) -> u32 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 9;

        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];

        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(11);

        result
    }

    /// The current state words, in stream order.
    pub(crate) fn state(&self) -> [u32; 4] {
        self.s
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn seed_42_matches_the_reference_vectors() {
        // Computed with an independent re-implementation of Vigna's
        // SplitMix64 + xoshiro128** (JS, limb-wise 64-bit math). These
        // values are the D4 anchor: any change to the seeding or the
        // generator changes them.
        let mut rng = Rng::new(42);
        assert_eq!(
            rng.state(),
            [0x2FEB_6E95, 0xB266_F103, 0x130F_9F52, 0x0E4A_E394]
        );
        assert_eq!(rng.next_u32(), 0x0C2E_C5AE);
        assert_eq!(rng.next_u32(), 0x6D11_3D84);
        assert_eq!(rng.next_u32(), 0xEFB7_993E);
        assert_eq!(rng.next_u32(), 0xBB7E_773A);
    }

    #[test]
    fn seed_0xc0ffee_matches_the_reference_vectors() {
        // The seed of the future `random_soup` golden world (TESTING.md).
        let mut rng = Rng::new(0xC0FFEE);
        assert_eq!(
            rng.state(),
            [0x9058_D0FA, 0xCE87_0479, 0xA16A_73CB, 0x57A5_0D44]
        );
        assert_eq!(rng.next_u32(), 0xDDE4_A2A4);
        assert_eq!(rng.next_u32(), 0x7733_D877);
        assert_eq!(rng.next_u32(), 0x0CB2_3738);
        assert_eq!(rng.next_u32(), 0x0B01_BC0E);
    }

    #[test]
    fn same_seed_reproduces_the_stream() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.state(), b.state());
        assert_ne!(a.next_u32(), b.next_u32());
    }

    #[test]
    fn zero_seed_yields_a_non_degenerate_state() {
        // The all-zero state is a fixed point of xoshiro; the SplitMix64
        // expansion must never produce it.
        assert_ne!(Rng::new(0).state(), [0, 0, 0, 0]);
    }
}
