//! `Math.random` and `seedRandom` (core 1.12, docs/GAPS.md G6): SplitMix64,
//! a small, fast generator with a 64-bit state. Not for security. The host
//! gives the seed in the `init` record `RANDOM_SEED` (fresh entropy for a
//! normal run, a fixed value in tests); without one, the first use seeds
//! from the clock.

use crate::global::GlobalCell;

static STATE: GlobalCell<Option<u64>> = GlobalCell::new(None);

/// Sets the state from a seed (the init record, or `seedRandom`).
pub fn seed(seed: u64) {
    STATE.set(Some(seed));
}

/// The next number in [0, 1).
pub fn next() -> f64 {
    let mut s = STATE.get().unwrap_or_else(clock_seed);
    s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    STATE.set(Some(s));
    let mut z = s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // The top 53 bits as a fraction: every value is exact in an f64.
    (z >> 11) as f64 / (1u64 << 53) as f64
}

fn clock_seed() -> u64 {
    (crate::host::now() as u64) ^ ((crate::host::monotonic_now() as u64) << 20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_the_same_numbers_in_range() {
        seed(42);
        let a: [f64; 4] = core::array::from_fn(|_| next());
        seed(42);
        let b: [f64; 4] = core::array::from_fn(|_| next());
        assert_eq!(a, b);
        assert!(a.iter().all(|v| (0.0..1.0).contains(v)), "{a:?}");
        assert_ne!(a[0], a[1]);
        seed(43);
        assert_ne!(next(), a[0], "another seed gives other numbers");
    }
}
