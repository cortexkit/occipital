//! The language's number type (§4.2): an exact decimal held as a signed
//! integer coefficient n with |n| ≤ 10^38 − 1, standing for n × 10^-18.
//!
//! Every operation works on the integer coefficients. Products and quotients
//! are formed exactly in 256-bit integers and rounded once, so results are
//! bit-identical on every target. No numeric dependency is used: none of the
//! common decimal crates has this exact range, scale and rounding behavior.

mod wide;

use std::cmp::Ordering;
use std::fmt;
use std::ops::Neg;

use wide::U256;

/// 10^18: the coefficient of the number 1.
const SCALE: i128 = 1_000_000_000_000_000_000;

/// 10^38 − 1: the largest coefficient magnitude (§4.2).
const MAX_COEFFICIENT: i128 = 99_999_999_999_999_999_999_999_999_999_999_999_999;

/// Number of decimal places a coefficient carries.
const PLACES: i128 = 18;

/// The largest exponent magnitude `**` accepts (§4.2).
const MAX_POWER: u128 = 1024;

/// A language number. Ordering and equality are numeric, and there is no
/// negative zero, infinity or NaN.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal(i128);

/// Why a number operation produced an error value instead of a number.
/// Each variant is one §12.4 code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberError {
    /// `E_OVERFLOW`: the result is outside the range.
    Overflow,
    /// `E_DIV_ZERO`: division or remainder by zero, or a negative power of zero.
    DivZero,
    /// `E_DOMAIN`: `Sqrt` of a negative, or a fractional power.
    Domain,
    /// `E_ARG`: an argument of the right kind with an unusable value.
    Arg,
    /// `E_NUMBER_PARSE`: `Number` text that does not match its pattern.
    NumberParse,
    /// `E_NUMBER_LITERAL`: a source literal that is not exactly representable.
    NumberLiteral,
}

impl NumberError {
    /// The §12.4 diagnostic code.
    pub fn code(self) -> &'static str {
        match self {
            NumberError::Overflow => "E_OVERFLOW",
            NumberError::DivZero => "E_DIV_ZERO",
            NumberError::Domain => "E_DOMAIN",
            NumberError::Arg => "E_ARG",
            NumberError::NumberParse => "E_NUMBER_PARSE",
            NumberError::NumberLiteral => "E_NUMBER_LITERAL",
        }
    }
}

/// A failed power: either the value error the power produced, or whatever
/// the fuel callback returned to stop the evaluation.
enum PowerFailure<E> {
    Value(NumberError),
    Fuel(E),
}

impl<E> From<NumberError> for PowerFailure<E> {
    fn from(error: NumberError) -> Self {
        PowerFailure::Value(error)
    }
}

impl Decimal {
    pub(crate) const ZERO: Decimal = Decimal(0);
    pub(crate) const ONE: Decimal = Decimal(SCALE);

    /// Integer division uses coefficient division, not a rounded decimal quotient.
    pub(crate) fn div_integer(self, rhs: Decimal) -> Result<Decimal, NumberError> {
        if rhs.0 == 0 {
            return Err(NumberError::DivZero);
        }
        Self::from_integer(self.0 / rhs.0)
    }

    pub(crate) fn integer(self) -> Option<i128> {
        self.is_integer().then_some(self.0 / SCALE)
    }

    /// Canonical text length without allocating the text during output preflight.
    pub(crate) fn text_len(self) -> u64 {
        let magnitude = self.0.unsigned_abs();
        let integer = magnitude / SCALE as u128;
        let mut fraction = magnitude % SCALE as u128;
        let mut places = 18;
        if fraction == 0 {
            places = 0;
        } else {
            while fraction.is_multiple_of(10) {
                fraction /= 10;
                places -= 1;
            }
        }
        u64::from(integer.checked_ilog10().unwrap_or(0) + 1)
            + u64::from(self.0 < 0)
            + if places == 0 { 0 } else { 1 + places }
    }

    /// The number with this coefficient, which counts units of 10^-18.
    pub fn from_coefficient(coefficient: i128) -> Result<Decimal, NumberError> {
        if coefficient.unsigned_abs() > MAX_COEFFICIENT.unsigned_abs() {
            Err(NumberError::Overflow)
        } else {
            Ok(Decimal(coefficient))
        }
    }

    /// The signed coefficient, in units of 10^-18.
    pub fn coefficient(self) -> i128 {
        self.0
    }

    /// The integer `value` as a number; the range ends below ±10^20.
    pub fn from_integer(value: i128) -> Result<Decimal, NumberError> {
        value
            .checked_mul(SCALE)
            .ok_or(NumberError::Overflow)
            .and_then(Decimal::from_coefficient)
    }

    /// `value × 10^-places`, built without a fallible call. Every `i64` is
    /// below 10^19, so even at zero places the coefficient stays under
    /// 10^37, inside the range. `places` above 18 counts as 18.
    pub(crate) const fn from_scaled(value: i64, places: u32) -> Decimal {
        let places = if places > 18 { 18 } else { places };
        Decimal(value as i128 * 10_i128.pow(18 - places))
    }

    /// The integer `value` as a number. Unlike [`Decimal::from_integer`] it
    /// cannot fail: every `i64` is inside the range.
    pub(crate) const fn from_i64(value: i64) -> Decimal {
        Decimal::from_scaled(value, 0)
    }

    /// The count `value` as a number. Every `u64` is below 2 × 10^19, so its
    /// coefficient is under 2 × 10^37, inside the range.
    pub(crate) const fn from_u64(value: u64) -> Decimal {
        Decimal(value as i128 * SCALE)
    }

    /// Whether the fraction is zero ("an integer" in §4.2).
    pub(crate) fn is_integer(self) -> bool {
        self.0 % SCALE == 0
    }

    /// The absolute value: exact, because the range is symmetric.
    pub(crate) fn abs(self) -> Decimal {
        Decimal(self.0.abs())
    }

    /// `a + b`: exact, or `E_OVERFLOW` outside the range.
    pub(crate) fn checked_add(self, rhs: Decimal) -> Result<Decimal, NumberError> {
        // Two in-range coefficients sum to below 2 × 10^38, which can exceed
        // i128; a failed checked add is therefore also out of range.
        self.0
            .checked_add(rhs.0)
            .ok_or(NumberError::Overflow)
            .and_then(Decimal::from_coefficient)
    }

    /// `a - b`: exact, or `E_OVERFLOW` outside the range.
    pub(crate) fn checked_sub(self, rhs: Decimal) -> Result<Decimal, NumberError> {
        self.checked_add(-rhs)
    }

    /// `a * b`: the exact product rounded to 18 places, half to even.
    pub(crate) fn checked_mul(self, rhs: Decimal) -> Result<Decimal, NumberError> {
        // (A × 10^-18)(B × 10^-18) = (A × B / 10^18) × 10^-18.
        let product = U256::mul_u128(self.0.unsigned_abs(), rhs.0.unsigned_abs());
        let magnitude = divide_half_to_even(product, SCALE.unsigned_abs())?;
        with_sign((self.0 < 0) != (rhs.0 < 0), magnitude)
    }

    /// `a / b`: `E_DIV_ZERO` for a zero divisor, otherwise the exact quotient
    /// rounded to 18 places, half to even.
    pub(crate) fn checked_div(self, rhs: Decimal) -> Result<Decimal, NumberError> {
        if rhs.0 == 0 {
            return Err(NumberError::DivZero);
        }
        // (A × 10^-18) / (B × 10^-18) = (A × 10^18 / B) × 10^-18.
        let numerator = U256::mul_u128(self.0.unsigned_abs(), SCALE.unsigned_abs());
        let magnitude = divide_half_to_even(numerator, rhs.0.unsigned_abs())?;
        with_sign((self.0 < 0) != (rhs.0 < 0), magnitude)
    }

    /// `a % b`: the coefficient A − B·q with q = trunc-toward-zero(A / B),
    /// by exact integer division, so the sign follows `a` (§4.2).
    pub(crate) fn checked_rem(self, rhs: Decimal) -> Result<Decimal, NumberError> {
        // Rust's integer remainder is exactly that truncating definition. Its
        // magnitude is below |B|, so the result is always in range.
        self.0
            .checked_rem(rhs.0)
            .map(Decimal)
            .ok_or(NumberError::DivZero)
    }

    /// `a ** b` without fuel accounting; see [`Decimal::pow_with_fuel`].
    pub(crate) fn pow(self, exponent: Decimal) -> Result<Decimal, NumberError> {
        let unmetered: Result<_, std::convert::Infallible> =
            self.pow_with_fuel(exponent, || Ok(()));
        match unmetered {
            Ok(result) => result,
            Err(never) => match never {},
        }
    }

    /// `a ** b` by the binary algorithm of §4.2, calling `charge` before
    /// each multiplication (one fuel unit each, §6.2).
    ///
    /// When `charge` fails, the power stops there and its error is returned
    /// as the outer `Err`, so an evaluation can abort at the exact point its
    /// budget runs out. The inner result is the power's value or value error.
    /// The reciprocal taken for a negative exponent is a division, not a
    /// multiplication, so it is not charged.
    pub(crate) fn pow_with_fuel<E>(
        self,
        exponent: Decimal,
        charge: impl FnMut() -> Result<(), E>,
    ) -> Result<Result<Decimal, NumberError>, E> {
        match self.power(exponent, charge) {
            Ok(value) => Ok(Ok(value)),
            Err(PowerFailure::Value(error)) => Ok(Err(error)),
            Err(PowerFailure::Fuel(error)) => Err(error),
        }
    }

    fn power<E>(
        self,
        exponent: Decimal,
        mut charge: impl FnMut() -> Result<(), E>,
    ) -> Result<Decimal, PowerFailure<E>> {
        if !exponent.is_integer() {
            return Err(NumberError::Domain.into());
        }
        let steps = (exponent.0 / SCALE).unsigned_abs();
        if steps > MAX_POWER {
            return Err(NumberError::Arg.into());
        }
        if steps == 0 {
            // Including 0 ** 0.
            return Ok(Decimal::ONE);
        }
        // A negative power inverts first, so a small base does not underflow
        // to zero before the inversion. 1 / 0 gives E_DIV_ZERO.
        let base = if exponent.0 < 0 {
            Decimal::ONE.checked_div(self)?
        } else {
            self
        };
        let mut result = Decimal::ONE;
        for index in (0..u128::BITS - steps.leading_zeros()).rev() {
            charge().map_err(PowerFailure::Fuel)?;
            result = result.checked_mul(result)?;
            if (steps >> index) & 1 == 1 {
                charge().map_err(PowerFailure::Fuel)?;
                result = result.checked_mul(base)?;
            }
        }
        Ok(result)
    }

    /// `Sqrt(x)`: `E_DOMAIN` below zero, otherwise the exact square root
    /// rounded to 18 places, half to even.
    pub(crate) fn sqrt(self) -> Result<Decimal, NumberError> {
        if self.0 < 0 {
            return Err(NumberError::Domain);
        }
        // sqrt(N × 10^-18) = sqrt(N × 10^18) × 10^-18.
        let radicand = U256::mul_u128(self.0.unsigned_abs(), SCALE.unsigned_abs());
        let (root, remainder) = radicand.isqrt_rem();
        // The exact root is above root + 1/2 exactly when the radicand is
        // above root² + root + 1/4, which for integers means remainder > root.
        // An integer's square root is never exactly halfway between two
        // integers, so the half-to-even tie rule never has a tie to break.
        let rounded = if remainder > U256::from_u128(root) {
            root + 1
        } else {
            root
        };
        with_sign(false, rounded)
    }

    /// `Round(x, digits)`: rounded to `digits` places, half away from zero
    /// (§6.2). `digits` must be an integer in −20..18, else `E_ARG`.
    pub(crate) fn round(self, digits: Decimal) -> Result<Decimal, NumberError> {
        if !digits.is_integer() {
            return Err(NumberError::Arg);
        }
        let digits = digits.0 / SCALE;
        if !(-20..=PLACES).contains(&digits) {
            return Err(NumberError::Arg);
        }
        // The rounding unit in coefficient terms: 10^0 ..= 10^38.
        let unit = 10_u128.pow((PLACES - digits) as u32);
        let magnitude = self.0.unsigned_abs();
        let (quotient, remainder) = (magnitude / unit, magnitude % unit);
        // Half away from zero, applied to the magnitude.
        let quotient = if remainder >= unit - remainder {
            quotient + 1
        } else {
            quotient
        };
        // quotient × unit ≤ magnitude + unit < 2 × 10^38, which fits u128.
        with_sign(self.0 < 0, quotient * unit)
    }

    /// `Floor(x)`: the largest integer ≤ x. Below the range: `E_OVERFLOW`.
    pub(crate) fn floor(self) -> Result<Decimal, NumberError> {
        Decimal::from_integer(self.0.div_euclid(SCALE))
    }

    /// `Ceil(x)`: the smallest integer ≥ x. Above the range: `E_OVERFLOW`.
    pub(crate) fn ceil(self) -> Result<Decimal, NumberError> {
        Decimal::from_integer(-(-self.0).div_euclid(SCALE))
    }

    /// Converts a number literal token (`number` in §3.3: no sign, no
    /// leading zeros) exactly. A literal with more than 18 digits after the
    /// point once its exponent is applied, or out of range, is
    /// `E_NUMBER_LITERAL` (§3.3, §12.4); so is text that is not a literal.
    pub(crate) fn from_literal(token: &str) -> Result<Decimal, NumberError> {
        let parts = split_number(token).ok_or(NumberError::NumberLiteral)?;
        let leading_zero = parts.integer.len() > 1 && parts.integer.starts_with('0');
        if parts.sign.is_some() || leading_zero {
            return Err(NumberError::NumberLiteral);
        }
        // Digits after the point once the exponent moves it.
        let fraction_digits = parts.fraction.len() as i128 - parts.exponent;
        if fraction_digits > PLACES {
            return Err(NumberError::NumberLiteral);
        }
        // With at most 18 fraction digits nothing is rounded, so the only
        // other failure is the range.
        parts.to_decimal().map_err(|_| NumberError::NumberLiteral)
    }

    /// Converts text for `Number(s)` (§6.2): `[+-]? digits ("." digits)?
    /// ([eE] [+-]? digits)?`, leading zeros allowed, converted exactly and
    /// rounded to 18 places, half to even. Other text is `E_NUMBER_PARSE`;
    /// a result out of range is `E_OVERFLOW`. Trimming white space is the
    /// caller's job, because it uses the §4.3 White_Space table.
    pub(crate) fn parse_text(text: &str) -> Result<Decimal, NumberError> {
        split_number(text)
            .ok_or(NumberError::NumberParse)?
            .to_decimal()
    }
}

/// Signed-magnitude 256-bit coefficient sum for §6.1. Even 10,000 maximum
/// coefficients fit; cancellation and averaging must precede range checking.
pub(crate) struct Accumulator {
    magnitude: U256,
    negative: bool,
    count: u128,
}

impl Accumulator {
    pub(crate) fn new() -> Self {
        Self {
            magnitude: U256::ZERO,
            negative: false,
            count: 0,
        }
    }

    pub(crate) fn push(&mut self, number: Decimal) {
        let term = U256::from_u128(number.0.unsigned_abs());
        let negative = number.0 < 0;
        if self.negative == negative {
            self.magnitude = self.magnitude.add(term);
        } else if self.magnitude >= term {
            self.magnitude = self.magnitude.sub(term);
        } else {
            self.magnitude = term.sub(self.magnitude);
            self.negative = negative;
        }
        self.count += 1;
    }

    pub(crate) fn finish(self, average: bool) -> Result<Option<Decimal>, NumberError> {
        if average && self.count == 0 {
            return Ok(None);
        }
        let magnitude = if average {
            divide_half_to_even(self.magnitude, self.count)?
        } else {
            self.magnitude.to_u128().ok_or(NumberError::Overflow)?
        };
        with_sign(self.negative, magnitude).map(Some)
    }
}

/// The canonical text form of §4.2: an optional `-`, the integer digits, and
/// the fraction digits without trailing zeros when the fraction is not zero.
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let magnitude = self.0.unsigned_abs();
        let scale = SCALE.unsigned_abs();
        if self.0 < 0 {
            f.write_str("-")?;
        }
        write!(f, "{}", magnitude / scale)?;
        let fraction = magnitude % scale;
        if fraction != 0 {
            let digits = format!("{fraction:018}");
            write!(f, ".{}", digits.trim_end_matches('0'))?;
        }
        Ok(())
    }
}

/// `-a`: exact, because the range is symmetric.
impl Neg for Decimal {
    type Output = Decimal;

    fn neg(self) -> Decimal {
        Decimal(-self.0)
    }
}

impl fmt::Debug for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Decimal({self})")
    }
}

/// `numerator / divisor` rounded half to even, as a coefficient magnitude.
fn divide_half_to_even(numerator: U256, divisor: u128) -> Result<u128, NumberError> {
    let (quotient, remainder) = numerator.div_rem_u128(divisor);
    let quotient = quotient.to_u128().ok_or(NumberError::Overflow)?;
    let round_up = match remainder.cmp(&(divisor - remainder)) {
        Ordering::Greater => true,
        Ordering::Equal => quotient % 2 == 1,
        Ordering::Less => false,
    };
    if round_up {
        quotient.checked_add(1).ok_or(NumberError::Overflow)
    } else {
        Ok(quotient)
    }
}

/// The number with this sign and coefficient magnitude, or `E_OVERFLOW`.
fn with_sign(negative: bool, magnitude: u128) -> Result<Decimal, NumberError> {
    if magnitude > MAX_COEFFICIENT.unsigned_abs() {
        return Err(NumberError::Overflow);
    }
    // In range, so the magnitude fits i128.
    let coefficient = magnitude as i128;
    Ok(Decimal(if negative { -coefficient } else { coefficient }))
}

/// The pieces of `[+-]? digits ("." digits)? ([eE] [+-]? digits)?`.
struct NumberParts<'a> {
    sign: Option<u8>,
    integer: &'a str,
    fraction: &'a str,
    /// The written exponent, saturated far beyond any value that could
    /// change the result (see `parse_exponent`).
    exponent: i128,
}

/// Exponent magnitudes are capped here while parsing. Any input is far
/// shorter than this many digits, so a capped exponent still overflows
/// (positive) or rounds every digit away (negative), exactly as the written
/// one would.
const EXPONENT_CAP: i128 = 1 << 64;

fn split_number(text: &str) -> Option<NumberParts<'_>> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let sign = match bytes.first() {
        Some(&sign @ (b'+' | b'-')) => {
            at = 1;
            Some(sign)
        }
        _ => None,
    };
    let integer = take_digits(text, &mut at)?;
    let fraction = if bytes.get(at) == Some(&b'.') {
        at += 1;
        take_digits(text, &mut at)?
    } else {
        ""
    };
    let exponent = if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        let negative = match bytes.get(at) {
            Some(b'+') => {
                at += 1;
                false
            }
            Some(b'-') => {
                at += 1;
                true
            }
            _ => false,
        };
        let magnitude = parse_exponent(take_digits(text, &mut at)?);
        if negative {
            -magnitude
        } else {
            magnitude
        }
    } else {
        0
    };
    (at == bytes.len()).then_some(NumberParts {
        sign,
        integer,
        fraction,
        exponent,
    })
}

/// One or more ASCII digits starting at `at`, advancing past them.
fn take_digits<'a>(text: &'a str, at: &mut usize) -> Option<&'a str> {
    let start = *at;
    // `at` only advances over bytes of `text`, so this never misses; a miss
    // would read as no digits, which the caller already reports as not a number.
    let length = text
        .as_bytes()
        .get(start..)?
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    *at += length;
    (length > 0).then(|| &text[start..start + length])
}

fn parse_exponent(digits: &str) -> i128 {
    digits.bytes().fold(0, |value: i128, digit| {
        (value * 10 + i128::from(digit - b'0')).min(EXPONENT_CAP)
    })
}

impl NumberParts<'_> {
    /// The exact value rounded to 18 places, half to even, or `E_OVERFLOW`.
    fn to_decimal(&self) -> Result<Decimal, NumberError> {
        let negative = self.sign == Some(b'-');
        let digits: Vec<u8> = self
            .integer
            .bytes()
            .chain(self.fraction.bytes())
            .map(|digit| digit - b'0')
            .collect();
        let (Some(first), Some(last)) = (
            digits.iter().position(|&digit| digit != 0),
            digits.iter().rposition(|&digit| digit != 0),
        ) else {
            // Zero, whatever its sign or exponent: there is no negative zero.
            return Ok(Decimal::ZERO);
        };
        let significant = &digits[first..=last];
        let length = significant.len() as i128;
        // The coefficient is significant × 10^shift; trailing zeros move
        // into the shift.
        let shift = self.exponent - self.fraction.len() as i128
            + PLACES
            + (digits.len() - 1 - last) as i128;
        // Number of significant digits that stay in the integer coefficient.
        let kept = length + shift;
        // A coefficient with more than 38 digits (its first one nonzero) is
        // at least 10^38, beyond the range.
        if kept > 38 {
            return Err(NumberError::Overflow);
        }
        if shift >= 0 {
            let magnitude = digits_value(significant) * 10_u128.pow(shift as u32);
            return with_sign(negative, magnitude);
        }
        let (magnitude, round_up) = if kept < 0 {
            // Below half a unit: the first dropped digit is an implied zero.
            (0, false)
        } else {
            let kept = kept as usize;
            let magnitude = digits_value(&significant[..kept]);
            let round_up = match significant[kept].cmp(&5) {
                Ordering::Greater => true,
                // The last significant digit is nonzero, so any digit after
                // this 5 puts the value above the halfway point.
                Ordering::Equal => kept + 1 < significant.len() || magnitude % 2 == 1,
                Ordering::Less => false,
            };
            (magnitude, round_up)
        };
        with_sign(negative, magnitude + u128::from(round_up))
    }
}

/// The value of at most 38 decimal digits.
fn digits_value(digits: &[u8]) -> u128 {
    digits
        .iter()
        .fold(0, |value, &digit| value * 10 + u128::from(digit))
}

#[cfg(test)]
mod tests;
