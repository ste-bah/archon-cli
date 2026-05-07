/// Seeded PRNG matching the xoshiro128** variant used in generate-gnn-fixtures.cjs.
///
/// The JS implementation uses a non-standard output function: `Math.imul(s[1] * 5, 0x7FFFFFFF)`
/// instead of the canonical `rotl(s[1] * 5, 7) * 9`. This struct reproduces the JS
/// behaviour exactly so weight initialization matches fixture expectations.
pub struct Xoshiro128StarStar {
    s: [u32; 4],
}

impl Xoshiro128StarStar {
    #[allow(clippy::needless_range_loop)]
    /// Seed via SplitMix64 (matching the JS SeededRng constructor).
    pub fn new(seed: u64) -> Self {
        let mut s = [0u32; 4];
        let mut sm = seed as u32;
        for i in 0..4 {
            sm = sm.wrapping_add(0x9e3779b9);
            let z = sm;
            let z = (z ^ (z >> 16)) as i32;
            let z = z.wrapping_mul(0x85ebca6b_u32 as i32) as u32;
            let z = (z ^ (z >> 13)) as i32;
            let z = z.wrapping_mul(0xc2b2ae35_u32 as i32) as u32;
            s[i] = z ^ (z >> 16);
        }
        Self { s }
    }

    /// Return a float in `[-0.5, 0.5]` - matches JS `nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        let x = self.s[1].wrapping_mul(5) as i32;
        let result = x.wrapping_mul(0x7FFFFFFF_i32) as u32;

        let t = self.s[1] << 9;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(11);

        (result as f64 / 0xFFFF_FFFF_u32 as f64) as f32 - 0.5
    }
}
