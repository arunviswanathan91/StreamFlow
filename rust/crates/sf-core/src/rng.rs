//! Seeded pseudo-random numbers (xoshiro256** seeded through SplitMix64).
//!
//! The project invariant is that anything feeding a reported number is seeded and reproducible:
//! the same seed gives the same stream on every machine and every run. Not cryptographic.

#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut st = seed;
        Rng { s: [splitmix64(&mut st), splitmix64(&mut st), splitmix64(&mut st), splitmix64(&mut st)] }
    }

    pub fn next_u64(&mut self) -> u64 {
        let r = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    /// Uniform in `[0, 1)` with 53 bits of resolution.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in `[0, n)` without modulo bias (rejection sampling). `n` must be > 0.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0)");
        let n = n as u64;
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < zone {
                return (v % n) as usize;
            }
        }
    }

    /// `count` distinct indices from `0..n`, in ascending order (a seeded subsample).
    pub fn choose_sorted(&mut self, n: usize, count: usize) -> Vec<usize> {
        let count = count.min(n);
        let mut idx: Vec<usize> = (0..n).collect();
        for i in 0..count {
            let j = i + self.below(n - i);
            idx.swap(i, j);
        }
        let mut out = idx[..count].to_vec();
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_different_seed_different_stream() {
        let (mut a, mut b, mut c) = (Rng::new(42), Rng::new(42), Rng::new(43));
        let (sa, sb, sc): (Vec<u64>, Vec<u64>, Vec<u64>) =
            ((0..8).map(|_| a.next_u64()).collect(), (0..8).map(|_| b.next_u64()).collect(), (0..8).map(|_| c.next_u64()).collect());
        assert_eq!(sa, sb);
        assert_ne!(sa, sc);
    }

    #[test]
    fn floats_are_in_unit_interval_with_a_sane_mean() {
        let mut r = Rng::new(1);
        let v: Vec<f64> = (0..100_000).map(|_| r.next_f64()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
    }

    #[test]
    fn below_is_uniform_enough_and_in_range() {
        let mut r = Rng::new(7);
        let mut counts = [0usize; 6];
        for _ in 0..60_000 {
            counts[r.below(6)] += 1;
        }
        for c in counts {
            assert!((c as f64 - 10_000.0).abs() < 600.0, "{counts:?}");
        }
    }

    #[test]
    fn subsample_is_sorted_distinct_and_reproducible() {
        let a = Rng::new(9).choose_sorted(1000, 100);
        assert_eq!(a, Rng::new(9).choose_sorted(1000, 100));
        assert!(a.windows(2).all(|w| w[0] < w[1]) && a.len() == 100 && *a.last().unwrap() < 1000);
        assert_eq!(Rng::new(9).choose_sorted(5, 50).len(), 5);
    }
}
