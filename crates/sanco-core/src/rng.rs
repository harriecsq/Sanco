//! Small seedable random generator (SplitMix64).
//!
//! Used for new row ids and site ids, so a simulation run with a fixed seed
//! is fully reproducible. Not for secrets: keys and tokens use the OS RNG.

#[derive(Debug, Clone)]
pub struct Rng64(u64);

impl Rng64 {
    pub fn seeded(seed: u64) -> Self {
        Self(seed)
    }

    /// Seeded from the OS random source.
    pub fn from_os() -> Self {
        Self(uuid::Uuid::new_v4().as_u64_pair().0)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`. `n` must be positive.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0)");
        self.next_u64() % n
    }

    /// True with probability `percent / 100`.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0u8; N];
        for chunk in out.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
        out
    }
}
