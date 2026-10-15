//! A per-event membership bitmask. FlowJo's engine computes Boolean gates as cached mask gates;
//! this is the same idea: one bit per event, combined with word-wise AND/OR/NOT.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    len: usize,
    words: Vec<u64>,
}

impl Mask {
    pub fn zeros(len: usize) -> Mask {
        Mask { len, words: vec![0; len.div_ceil(64)] }
    }

    pub fn ones(len: usize) -> Mask {
        let mut m = Mask { len, words: vec![u64::MAX; len.div_ceil(64)] };
        m.trim();
        m
    }

    pub fn from_fn(len: usize, mut f: impl FnMut(usize) -> bool) -> Mask {
        let mut m = Mask::zeros(len);
        for i in 0..len {
            if f(i) {
                m.set(i, true);
            }
        }
        m
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> bool {
        debug_assert!(i < self.len);
        self.words[i >> 6] >> (i & 63) & 1 == 1
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: bool) {
        debug_assert!(i < self.len);
        if v {
            self.words[i >> 6] |= 1 << (i & 63);
        } else {
            self.words[i >> 6] &= !(1 << (i & 63));
        }
    }

    /// Number of events in the population.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn and(&self, o: &Mask) -> Mask {
        self.zip(o, |a, b| a & b)
    }

    pub fn or(&self, o: &Mask) -> Mask {
        self.zip(o, |a, b| a | b)
    }

    pub fn not(&self) -> Mask {
        let mut m = Mask { len: self.len, words: self.words.iter().map(|w| !w).collect() };
        m.trim();
        m
    }

    /// Indices of the member events, ascending.
    pub fn indices(&self) -> Vec<usize> {
        (0..self.len).filter(|&i| self.get(i)).collect()
    }

    fn zip(&self, o: &Mask, f: impl Fn(u64, u64) -> u64) -> Mask {
        assert_eq!(self.len, o.len, "mask length mismatch: {} vs {}", self.len, o.len);
        Mask { len: self.len, words: self.words.iter().zip(&o.words).map(|(a, b)| f(*a, *b)).collect() }
    }

    /// Clear the unused high bits of the last word so `count` and equality stay exact after NOT.
    fn trim(&mut self) {
        let rem = self.len & 63;
        if rem != 0 {
            if let Some(last) = self.words.last_mut() {
                *last &= (1u64 << rem) - 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_logic_with_a_length_that_is_not_a_multiple_of_64() {
        let n = 130;
        let a = Mask::from_fn(n, |i| i % 2 == 0);
        let b = Mask::from_fn(n, |i| i % 3 == 0);
        assert_eq!(a.count(), 65);
        assert_eq!(a.and(&b).count(), (0..n).filter(|i| i % 6 == 0).count());
        assert_eq!(a.or(&b).count(), (0..n).filter(|i| i % 2 == 0 || i % 3 == 0).count());
        // NOT must not count the padding bits beyond `len`
        assert_eq!(a.not().count(), n - 65);
        assert_eq!(Mask::ones(n).count(), n);
        assert_eq!(Mask::zeros(n).not(), Mask::ones(n));
    }

    #[test]
    fn de_morgan_holds() {
        let n = 1000;
        let a = Mask::from_fn(n, |i| (i * 7) % 5 < 2);
        let b = Mask::from_fn(n, |i| (i * 11) % 7 < 3);
        assert_eq!(a.and(&b).not(), a.not().or(&b.not()));
        assert_eq!(a.or(&b).not(), a.not().and(&b.not()));
        assert_eq!(a.not().not(), a);
    }

    #[test]
    fn indices_round_trip() {
        let m = Mask::from_fn(70, |i| i == 0 || i == 63 || i == 64 || i == 69);
        assert_eq!(m.indices(), vec![0, 63, 64, 69]);
    }

    #[test]
    #[should_panic(expected = "mask length mismatch")]
    fn mixing_lengths_is_a_bug_not_silent() {
        Mask::zeros(10).and(&Mask::zeros(11));
    }
}
