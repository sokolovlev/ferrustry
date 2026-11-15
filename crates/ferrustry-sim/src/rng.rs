//! Детерминированный ГПСЧ (xoshiro256**), состояние которого входит в состояние мира.

use serde::{Deserialize, Serialize};

/// Генератор псевдослучайных чисел xoshiro256**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Создаёт генератор из seed.
    pub fn new(seed: u64) -> Self {
        let mut st = seed;
        Self {
            s: [
                splitmix64(&mut st),
                splitmix64(&mut st),
                splitmix64(&mut st),
                splitmix64(&mut st),
            ],
        }
    }

    /// Следующее 64-битное число.
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Число в диапазоне `0..bound` (`bound > 0`), без смещения (метод Лемира).
    pub fn below(&mut self, bound: u32) -> u32 {
        debug_assert!(bound > 0);
        let bound = u64::from(bound.max(1));
        loop {
            let m = (self.next_u64() >> 32) * bound;
            let low = m & 0xFFFF_FFFF;
            if low >= (1u64 << 32) % bound {
                // m >> 32 < bound <= u32::MAX
                return u32::try_from(m >> 32).unwrap_or(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn below_in_range() {
        let mut r = Rng::new(1);
        for _ in 0..1000 {
            assert!(r.below(7) < 7);
        }
    }
}
