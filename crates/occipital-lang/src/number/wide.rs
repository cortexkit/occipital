//! Unsigned 256-bit integers for the intermediate products and quotients of
//! §4.2. Only magnitudes are needed: the decimal code tracks signs itself.
//!
//! The largest intermediate is a product of two coefficients below 10^38,
//! which is below 10^76 < 2^256, so no operation here can overflow when used
//! as the decimal code uses it. The arithmetic is plain integer work, so it
//! gives the same bits on every target.

use std::cmp::Ordering;

/// An unsigned 256-bit integer, `hi * 2^128 + lo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct U256 {
    hi: u128,
    lo: u128,
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.hi.cmp(&other.hi).then(self.lo.cmp(&other.lo))
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const LOW_64: u128 = u64::MAX as u128;

impl U256 {
    pub(super) const ZERO: U256 = U256 { hi: 0, lo: 0 };

    pub(super) fn from_u128(value: u128) -> U256 {
        U256 { hi: 0, lo: value }
    }

    /// The value as a `u128`, or `None` when it needs more than 128 bits.
    pub(super) fn to_u128(self) -> Option<u128> {
        (self.hi == 0).then_some(self.lo)
    }

    /// The full product of two 128-bit values, which always fits 256 bits.
    pub(super) fn mul_u128(a: u128, b: u128) -> U256 {
        let (a1, a0) = (a >> 64, a & LOW_64);
        let (b1, b0) = (b >> 64, b & LOW_64);
        let p00 = a0 * b0;
        let p01 = a0 * b1;
        let p10 = a1 * b0;
        let p11 = a1 * b1;
        // Each term is below 2^64, so the sum of three stays below 2^66.
        let mid = (p00 >> 64) + (p01 & LOW_64) + (p10 & LOW_64);
        U256 {
            hi: p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64),
            lo: (p00 & LOW_64) | (mid << 64),
        }
    }

    /// Quotient and remainder of division by a nonzero 128-bit divisor.
    pub(super) fn div_rem_u128(self, divisor: u128) -> (U256, u128) {
        assert!(divisor != 0, "U256 division by zero");
        if self.hi == 0 {
            return (U256::from_u128(self.lo / divisor), self.lo % divisor);
        }
        // Restoring binary long division, one bit at a time from the top.
        // The remainder stays below the divisor; `carry` holds the bit that
        // shifting it left can push past 128 bits.
        let mut quotient = U256::ZERO;
        let mut remainder: u128 = 0;
        for index in (0..256).rev() {
            let carry = remainder >> 127;
            remainder = (remainder << 1) | self.bit(index);
            if carry == 1 || remainder >= divisor {
                remainder = remainder.wrapping_sub(divisor);
                quotient = quotient.with_bit(index);
            }
        }
        (quotient, remainder)
    }

    /// `self - other`; the caller guarantees `other <= self`.
    pub(super) fn sub(self, other: U256) -> U256 {
        let (lo, borrow) = self.lo.overflowing_sub(other.lo);
        U256 {
            hi: self.hi - other.hi - u128::from(borrow),
            lo,
        }
    }

    /// Exact addition for bounded coefficient sums (at most 10,000 terms).
    pub(super) fn add(self, other: U256) -> U256 {
        let (lo, carry) = self.lo.overflowing_add(other.lo);
        U256 {
            hi: self.hi + other.hi + u128::from(carry),
            lo,
        }
    }

    /// The floor of the square root, and `self` minus its square.
    ///
    /// Limited to values below 2^252, so the Newton start and every sum of
    /// two candidate roots fit in 128 bits; the decimal code passes values
    /// below 10^56 < 2^187.
    pub(super) fn isqrt_rem(self) -> (u128, U256) {
        assert!(self.bit_length() <= 252, "U256 square root input too large");
        if self == U256::ZERO {
            return (0, U256::ZERO);
        }
        // Newton's iteration from a start at or above the root decreases
        // monotonically to the floor of the root, then stops decreasing.
        let mut root: u128 = 1 << self.bit_length().div_ceil(2);
        loop {
            let (quotient, _) = self.div_rem_u128(root);
            // With root >= sqrt(self), quotient <= root, so it fits 128 bits
            // and the sum fits. A quotient too wide for 128 bits would be
            // above root, so the next step could not decrease: stop there.
            let Some(quotient) = quotient.to_u128() else {
                break;
            };
            let next = (root + quotient) / 2;
            if next >= root {
                break;
            }
            root = next;
        }
        (root, self.sub(U256::mul_u128(root, root)))
    }

    fn bit_length(self) -> u32 {
        if self.hi != 0 {
            256 - self.hi.leading_zeros()
        } else {
            128 - self.lo.leading_zeros()
        }
    }

    fn bit(self, index: u32) -> u128 {
        if index >= 128 {
            (self.hi >> (index - 128)) & 1
        } else {
            (self.lo >> index) & 1
        }
    }

    fn with_bit(self, index: u32) -> U256 {
        if index >= 128 {
            U256 {
                hi: self.hi | (1 << (index - 128)),
                lo: self.lo,
            }
        } else {
            U256 {
                hi: self.hi,
                lo: self.lo | (1 << index),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_of_two_maximal_values_is_exact() {
        // (2^128 - 1)^2 = 2^256 - 2^129 + 1.
        let product = U256::mul_u128(u128::MAX, u128::MAX);
        assert_eq!(
            product,
            U256 {
                hi: u128::MAX - 1,
                lo: 1
            }
        );
    }

    #[test]
    fn long_division_recovers_the_factors() {
        let a = 99_999_999_999_999_999_999_999_999_999_999_999_999_u128;
        let b = 12_345_678_901_234_567_890_123_456_789_u128;
        let (quotient, remainder) = U256::mul_u128(a, b).div_rem_u128(b);
        assert_eq!((quotient.to_u128(), remainder), (Some(a), 0));
        let (quotient, remainder) = U256::mul_u128(a, b).div_rem_u128(a + 1);
        // a + 1 = 10^38 and a = 10^38 - 1, so a*b = b*10^38 - b.
        assert_eq!(quotient.to_u128(), Some(b - 1));
        assert_eq!(remainder, a + 1 - b);
    }

    #[test]
    fn division_by_a_divisor_above_two_to_the_127() {
        let divisor = u128::MAX - 4;
        let (quotient, remainder) = U256::mul_u128(divisor, 3).div_rem_u128(divisor);
        assert_eq!((quotient.to_u128(), remainder), (Some(3), 0));
    }

    #[test]
    fn integer_square_root_and_remainder() {
        assert_eq!(U256::from_u128(0).isqrt_rem(), (0, U256::ZERO));
        assert_eq!(U256::from_u128(1).isqrt_rem(), (1, U256::ZERO));
        assert_eq!(U256::from_u128(8).isqrt_rem(), (2, U256::from_u128(4)));
        assert_eq!(U256::from_u128(9).isqrt_rem(), (3, U256::ZERO));
        // (10^28)^2 - 1 has root 10^28 - 1 and remainder 2*10^28 - 2.
        let ten_28 = 10_u128.pow(28);
        let square = U256::mul_u128(ten_28, ten_28).sub(U256::from_u128(1));
        assert_eq!(
            square.isqrt_rem(),
            (ten_28 - 1, U256::from_u128(2 * ten_28 - 2))
        );
    }
}
