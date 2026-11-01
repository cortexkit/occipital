//! Every expected value here is derived by hand from §4.2 and §6.2, never
//! recorded from this implementation.

use super::{Decimal, NumberError};

/// A number from `Number`-style text; the tests write exact values only.
fn n(text: &str) -> Decimal {
    Decimal::parse_text(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
}

/// The largest number, 10^20 − 10^-18.
fn max() -> Decimal {
    n("99999999999999999999.999999999999999999")
}

/// Runs `a ** b` and counts the multiplications it charged.
fn pow_counting(a: Decimal, b: Decimal) -> (Result<Decimal, NumberError>, u32) {
    let mut charged = 0;
    let result = a.pow_with_fuel(b, || -> Result<(), ()> {
        charged += 1;
        Ok(())
    });
    (result.expect("the counting callback never stops"), charged)
}

#[test]
fn representation_is_an_integer_coefficient_at_scale_18() {
    assert_eq!(n("2.5").coefficient(), 2_500_000_000_000_000_000);
    assert_eq!(n("0.000000000000000001").coefficient(), 1);
    assert_eq!(
        max().coefficient(),
        99_999_999_999_999_999_999_999_999_999_999_999_999
    );
    assert_eq!(
        Decimal::from_coefficient(-99_999_999_999_999_999_999_999_999_999_999_999_999),
        Ok(-max())
    );
    assert_eq!(
        Decimal::from_coefficient(100_000_000_000_000_000_000_000_000_000_000_000_000),
        Err(NumberError::Overflow)
    );
    // Every unsigned 64-bit id is an exact integer.
    let id = Decimal::from_integer(i128::from(u64::MAX)).unwrap();
    assert_eq!(id.to_string(), "18446744073709551615");
    assert_eq!(
        Decimal::from_integer(100_000_000_000_000_000_000),
        Err(NumberError::Overflow)
    );
}

#[test]
fn error_codes_are_the_section_12_4_codes() {
    let codes = [
        (NumberError::Overflow, "E_OVERFLOW"),
        (NumberError::DivZero, "E_DIV_ZERO"),
        (NumberError::Domain, "E_DOMAIN"),
        (NumberError::Arg, "E_ARG"),
        (NumberError::NumberParse, "E_NUMBER_PARSE"),
        (NumberError::NumberLiteral, "E_NUMBER_LITERAL"),
    ];
    for (error, code) in codes {
        assert_eq!(error.code(), code);
    }
}

#[test]
fn addition_is_exact() {
    assert_eq!(n("0.1").checked_add(n("0.2")), Ok(n("0.3")));
    assert_eq!(n("-0.1").checked_add(n("0.1")), Ok(Decimal::ZERO));
    assert_eq!(n("0.3").checked_sub(n("0.1")), Ok(n("0.2")));
    assert_eq!(
        n("99999999999999999999").checked_add(n("0.999999999999999999")),
        Ok(max())
    );
}

#[test]
fn addition_and_subtraction_outside_the_range_overflow() {
    let tiny = n("0.000000000000000001");
    assert_eq!(max().checked_add(tiny), Err(NumberError::Overflow));
    assert_eq!((-max()).checked_sub(tiny), Err(NumberError::Overflow));
    // Twice the maximum coefficient exceeds i128 as well as the range.
    assert_eq!(max().checked_add(max()), Err(NumberError::Overflow));
    assert_eq!((-max()).checked_sub(max()), Err(NumberError::Overflow));
}

#[test]
fn negation_and_absolute_value_are_exact() {
    assert_eq!(-n("2.5"), n("-2.5"));
    assert_eq!(-max(), n("-99999999999999999999.999999999999999999"));
    assert_eq!(n("-2.5").abs(), n("2.5"));
    assert_eq!((-Decimal::ZERO).to_string(), "0");
}

#[test]
fn multiplication_rounds_half_to_even() {
    assert_eq!(n("1.5").checked_mul(n("-4")), Ok(n("-6")));
    // 10^-9 × 5 × 10^-10 = 0.5 × 10^-18: a tie, rounded to the even 0.
    let nano = n("0.000000001");
    assert_eq!(nano.checked_mul(n("0.0000000005")), Ok(Decimal::ZERO));
    // 1.5 × 10^-18 rounds up to the even 2 × 10^-18; 2.5 × 10^-18 down to it.
    let two_units = n("0.000000000000000002");
    assert_eq!(nano.checked_mul(n("0.0000000015")), Ok(two_units));
    assert_eq!(nano.checked_mul(n("0.0000000025")), Ok(two_units));
    assert_eq!(nano.checked_mul(n("-0.0000000025")), Ok(-two_units));
    // Above and below a tie round to nearest.
    assert_eq!(
        nano.checked_mul(n("0.00000000051")),
        Ok(n("0.000000000000000001"))
    );
    assert_eq!(nano.checked_mul(n("0.00000000049")), Ok(Decimal::ZERO));
}

#[test]
fn multiplication_uses_a_256_bit_product() {
    // The coefficient product, about 1.85 × 10^46, needs more than 128 bits.
    // 12345678901.234567890123456789 × 1.5 = 18518518351.8518518351851851835
    // exactly: a tie at the 19th place, rounded up to the even ...184.
    let a = n("12345678901.234567890123456789");
    assert_eq!(
        a.checked_mul(n("1.5")),
        Ok(n("18518518351.851851835185185184"))
    );
    // × 2.5 = 30864197253.0864197253086419725: a tie kept at the even ...972.
    assert_eq!(
        a.checked_mul(n("2.5")),
        Ok(n("30864197253.086419725308641972"))
    );
}

#[test]
fn multiplication_outside_the_range_overflows() {
    let ten_10 = n("10000000000");
    assert_eq!(ten_10.checked_mul(ten_10), Err(NumberError::Overflow));
    assert_eq!(ten_10.checked_mul(-ten_10), Err(NumberError::Overflow));
    assert_eq!(
        ten_10.checked_mul(n("9999999999.999999999999999999")),
        Ok(n("99999999999999999999.99999999"))
    );
    // A product that only reaches the range by rounding still overflows:
    // max × 1.000000000000000001 is above max by about 10^2.
    assert_eq!(
        max().checked_mul(n("1.000000000000000001")),
        Err(NumberError::Overflow)
    );
}

#[test]
fn division_rounds_half_to_even() {
    assert_eq!(n("1").checked_div(n("3")), Ok(n("0.333333333333333333")));
    assert_eq!(n("2").checked_div(n("3")), Ok(n("0.666666666666666667")));
    assert_eq!(n("-2").checked_div(n("3")), Ok(n("-0.666666666666666667")));
    assert_eq!(n("7.5").checked_div(n("-2.5")), Ok(n("-3")));
    // 10^-18 / 2 is a tie rounded to the even 0; 3 × 10^-18 / 2 to 2 × 10^-18.
    let unit = n("0.000000000000000001");
    assert_eq!(unit.checked_div(n("2")), Ok(Decimal::ZERO));
    assert_eq!(
        n("0.000000000000000003").checked_div(n("2")),
        Ok(n("0.000000000000000002"))
    );
}

#[test]
fn division_uses_a_256_bit_dividend() {
    // 99999999999999999999 / 7 = 14285714285714285714 remainder 1, so the
    // fraction is 0.142857 repeating; the 19th place is 1, rounded down.
    assert_eq!(
        n("99999999999999999999").checked_div(n("7")),
        Ok(n("14285714285714285714.142857142857142857"))
    );
}

#[test]
fn division_by_zero_and_overflow() {
    assert_eq!(n("1").checked_div(Decimal::ZERO), Err(NumberError::DivZero));
    assert_eq!(
        Decimal::ZERO.checked_div(Decimal::ZERO),
        Err(NumberError::DivZero)
    );
    assert_eq!(
        n("10000000000000000000").checked_div(n("0.1")),
        Err(NumberError::Overflow)
    );
}

#[test]
fn remainder_truncates_and_follows_the_dividend() {
    assert_eq!(n("-7").checked_rem(n("3")), Ok(n("-1")));
    assert_eq!(n("7").checked_rem(n("-3")), Ok(n("1")));
    assert_eq!(n("-7").checked_rem(n("-3")), Ok(n("-1")));
    assert_eq!(n("7").checked_rem(n("3")), Ok(n("1")));
    assert_eq!(n("7.5").checked_rem(n("2")), Ok(n("1.5")));
    // Coefficients: 10^18 − (3 × 10^17)·3 = 10^17, with no rounded division.
    assert_eq!(n("1").checked_rem(n("0.3")), Ok(n("0.1")));
    assert_eq!(n("1").checked_rem(Decimal::ZERO), Err(NumberError::DivZero));
}

#[test]
fn ordering_is_numeric() {
    assert!(n("-2") < n("-1.5"));
    assert!(n("0.000000000000000001") > Decimal::ZERO);
    assert!(n("10") > n("9.999999999999999999"));
    assert_eq!(n("2.50"), n("2.5"));
}

#[test]
fn power_worked_values_and_fuel() {
    // 0.1 ** -19: c = 1 / 0.1 = 10 (a division, not charged). 19 is 10011
    // in binary: five squarings and three multiplications by c, 8 in all.
    let (result, fuel) = pow_counting(n("0.1"), n("-19"));
    assert_eq!(result, Ok(n("10000000000000000000")));
    assert_eq!(fuel, 8);
    // b == 0 gives 1 with no multiplication, including 0 ** 0.
    assert_eq!(
        pow_counting(Decimal::ZERO, Decimal::ZERO),
        (Ok(Decimal::ONE), 0)
    );
    assert_eq!(
        pow_counting(n("-5.5"), Decimal::ZERO),
        (Ok(Decimal::ONE), 0)
    );
}

#[test]
fn power_follows_the_binary_algorithm() {
    // 10 is 1010: four squarings and two multiplications.
    assert_eq!(pow_counting(n("2"), n("10")), (Ok(n("1024")), 6));
    assert_eq!(pow_counting(n("-2"), n("3")), (Ok(n("-8")), 4));
    assert_eq!(pow_counting(n("1.5"), n("2")), (Ok(n("2.25")), 3));
    // 1023 is ten 1 bits: the most multiplications any allowed b needs.
    assert_eq!(
        pow_counting(Decimal::ONE, n("1023")),
        (Ok(Decimal::ONE), 20)
    );
    assert_eq!(
        pow_counting(Decimal::ONE, n("-1024")),
        (Ok(Decimal::ONE), 12)
    );
}

#[test]
fn power_rounds_every_step() {
    // 18 is 10010: 1·1 → 1, ×0.1 → 0.1, then squarings to 10^-2, 10^-4,
    // 10^-8, ×0.1 → 10^-9, and a last squaring to 10^-18: exact.
    assert_eq!(
        pow_counting(n("0.1"), n("18")),
        (Ok(n("0.000000000000000001")), 7)
    );
    // 19 is 10011: the same steps reach 10^-18, and × 0.1 gives
    // 0.1 × 10^-18, which rounds to 0; the inversion order of a negative
    // exponent is what avoids this.
    assert_eq!(pow_counting(n("0.1"), n("19")), (Ok(Decimal::ZERO), 8));
}

#[test]
fn power_errors() {
    assert_eq!(
        pow_counting(n("2"), n("0.5")),
        (Err(NumberError::Domain), 0)
    );
    assert_eq!(
        pow_counting(n("2"), n("-1.5")),
        (Err(NumberError::Domain), 0)
    );
    // A fractional exponent above 1024 is still E_DOMAIN: it is not an integer.
    assert_eq!(
        pow_counting(n("2"), n("2000.5")),
        (Err(NumberError::Domain), 0)
    );
    assert_eq!(pow_counting(n("2"), n("1025")), (Err(NumberError::Arg), 0));
    assert_eq!(pow_counting(n("2"), n("-1025")), (Err(NumberError::Arg), 0));
    assert_eq!(
        pow_counting(Decimal::ZERO, n("-1")),
        (Err(NumberError::DivZero), 0)
    );
    // 1024 is 1 followed by ten 0 bits: 1·1, ×2 → 2, then squarings to
    // 4, 16, 256, 65536, 2^32, 2^64 (about 1.8 × 10^19, in range) and 2^128,
    // which overflows on the 9th multiplication.
    assert_eq!(
        pow_counting(n("2"), n("1024")),
        (Err(NumberError::Overflow), 9)
    );
    assert_eq!(n("2").pow(n("1024")), Err(NumberError::Overflow));
}

#[test]
fn power_stops_where_the_fuel_callback_fails() {
    let mut remaining = 3;
    let result = n("2").pow_with_fuel(n("10"), || {
        if remaining == 0 {
            return Err("E_FUEL");
        }
        remaining -= 1;
        Ok(())
    });
    assert_eq!(result, Err("E_FUEL"));
}

#[test]
fn square_root_rounds_to_nearest() {
    assert_eq!(Decimal::ZERO.sqrt(), Ok(Decimal::ZERO));
    assert_eq!(n("4").sqrt(), Ok(n("2")));
    assert_eq!(n("2.25").sqrt(), Ok(n("1.5")));
    assert_eq!(n("0.000000000000000001").sqrt(), Ok(n("0.000000001")));
    // √2 = 1.41421356237309504880…: the 19th place is 8, rounded up.
    assert_eq!(n("2").sqrt(), Ok(n("1.414213562373095049")));
    // √5 = 2.23606797749978969640…: the 19th place is 4, rounded down.
    assert_eq!(n("5").sqrt(), Ok(n("2.236067977499789696")));
    // √max = 10^10 × √(1 − 10^-38) ≈ 10^10 − 5 × 10^-29, which rounds to 10^10.
    assert_eq!(max().sqrt(), Ok(n("10000000000")));
}

#[test]
fn square_root_of_a_negative_is_a_domain_error() {
    assert_eq!(n("-1").sqrt(), Err(NumberError::Domain));
    assert_eq!(n("-0.000000000000000001").sqrt(), Err(NumberError::Domain));
}

#[test]
fn round_is_half_away_from_zero() {
    assert_eq!(n("2.675").round(n("2")), Ok(n("2.68")));
    assert_eq!(n("2.5").round(Decimal::ZERO), Ok(n("3")));
    assert_eq!(n("-2.5").round(Decimal::ZERO), Ok(n("-3")));
    assert_eq!(n("1250").round(n("-2")), Ok(n("1300")));
    assert_eq!(n("2.4999").round(Decimal::ZERO), Ok(n("2")));
    assert_eq!(n("-2.675").round(n("2")), Ok(n("-2.68")));
    assert_eq!(n("2.674").round(n("2")), Ok(n("2.67")));
    // 18 places leaves every number unchanged.
    assert_eq!(max().round(n("18")), Ok(max()));
    assert_eq!(n("49999999999999999999").round(n("-20")), Ok(Decimal::ZERO));
}

#[test]
fn round_digit_bounds_and_overflow() {
    assert_eq!(n("1").round(n("19")), Err(NumberError::Arg));
    assert_eq!(n("1").round(n("-21")), Err(NumberError::Arg));
    assert_eq!(n("1").round(n("0.5")), Err(NumberError::Arg));
    // Rounding up to 10^20 leaves the range.
    assert_eq!(
        n("50000000000000000000").round(n("-20")),
        Err(NumberError::Overflow)
    );
    assert_eq!(
        n("-99999999999999999999.5").round(Decimal::ZERO),
        Err(NumberError::Overflow)
    );
}

#[test]
fn floor_and_ceil_go_toward_minus_and_plus_infinity() {
    assert_eq!(n("2.7").floor(), Ok(n("2")));
    assert_eq!(n("-2.7").floor(), Ok(n("-3")));
    assert_eq!(n("-2").floor(), Ok(n("-2")));
    assert_eq!(n("2.1").ceil(), Ok(n("3")));
    assert_eq!(n("-2.1").ceil(), Ok(n("-2")));
    assert_eq!(n("2").ceil(), Ok(n("2")));
    assert_eq!(n("0.000000000000000001").ceil(), Ok(n("1")));
    assert_eq!(n("-0.000000000000000001").ceil(), Ok(Decimal::ZERO));
}

#[test]
fn floor_and_ceil_past_the_range_overflow() {
    assert_eq!(max().ceil(), Err(NumberError::Overflow));
    assert_eq!((-max()).floor(), Err(NumberError::Overflow));
    assert_eq!(max().floor(), Ok(n("99999999999999999999")));
    assert_eq!((-max()).ceil(), Ok(n("-99999999999999999999")));
}

#[test]
fn canonical_text_form() {
    let literal = |token| Decimal::from_literal(token).unwrap().to_string();
    assert_eq!(literal("2.50"), "2.5");
    assert_eq!(literal("1e3"), "1000");
    // `-0.0` in source is unary minus applied to the literal 0.0.
    assert_eq!((-Decimal::from_literal("0.0").unwrap()).to_string(), "0");
    assert_eq!(n("-0.0").to_string(), "0");
    assert_eq!(n("-2.5").to_string(), "-2.5");
    assert_eq!(
        n("0.000000000000000001").to_string(),
        "0.000000000000000001"
    );
    assert_eq!(n("-0.05").to_string(), "-0.05");
    assert_eq!(max().to_string(), "99999999999999999999.999999999999999999");
    assert_eq!(n("1E+2").to_string(), "100");
}

#[test]
fn literals_are_exact_or_rejected() {
    assert_eq!(Decimal::from_literal("0"), Ok(Decimal::ZERO));
    assert_eq!(Decimal::from_literal("1.5e1"), Ok(n("15")));
    assert_eq!(
        Decimal::from_literal("1e-18"),
        Ok(n("0.000000000000000001"))
    );
    assert_eq!(Decimal::from_literal("25E-1"), Ok(n("2.5")));
    assert_eq!(
        Decimal::from_literal("99999999999999999999.999999999999999999"),
        Ok(max())
    );
    // Zero is representable at any exponent.
    assert_eq!(
        Decimal::from_literal("0e99999999999999999999999"),
        Ok(Decimal::ZERO)
    );
    let rejected = [
        // Leading zeros.
        "01",
        "00.5",
        // More than 18 digits after the point once the exponent applies,
        // even when the extra digits are zeros.
        "1e-19",
        "1.10e-17",
        "0.1234567890123456789",
        "2.5000000000000000000",
        // Out of range.
        "100000000000000000000",
        "1e20",
        "1e99999999999999999999999",
        // Not a §3.3 number token.
        "-1",
        "+1",
        "1.",
        ".5",
        "1e",
        "",
    ];
    for token in rejected {
        assert_eq!(
            Decimal::from_literal(token),
            Err(NumberError::NumberLiteral),
            "{token:?}"
        );
    }
}

#[test]
fn number_text_rounds_half_to_even() {
    assert_eq!(Decimal::parse_text("007"), Ok(n("7")));
    assert_eq!(Decimal::parse_text("+1.5"), Ok(n("1.5")));
    assert_eq!(Decimal::parse_text("2.50"), Ok(n("2.5")));
    // 0.1, 0.5 and 0.9 of the last unit; 0.5 is a tie rounded to even 0.
    assert_eq!(Decimal::parse_text("1e-19"), Ok(Decimal::ZERO));
    assert_eq!(Decimal::parse_text("5e-19"), Ok(Decimal::ZERO));
    assert_eq!(Decimal::parse_text("9e-19"), Ok(n("0.000000000000000001")));
    // A 5 followed by any nonzero digit is above the tie.
    assert_eq!(
        Decimal::parse_text("0.0000000000000000005000001"),
        Ok(n("0.000000000000000001"))
    );
    // 1.5 and 2.5 units both round to the even 2 units.
    let two_units = n("0.000000000000000002");
    assert_eq!(Decimal::parse_text("15e-19"), Ok(two_units));
    assert_eq!(Decimal::parse_text("25e-19"), Ok(two_units));
    assert_eq!(Decimal::parse_text("-25e-19"), Ok(-two_units));
    assert_eq!(
        Decimal::parse_text("1e-99999999999999999999999"),
        Ok(Decimal::ZERO)
    );
}

#[test]
fn number_text_errors() {
    for text in [
        "", "abc", "1.", ".5", "1e", "1e+", "--1", "1,000", " 1", "0x10", "1_0",
    ] {
        assert_eq!(
            Decimal::parse_text(text),
            Err(NumberError::NumberParse),
            "{text:?}"
        );
    }
    assert_eq!(
        Decimal::parse_text("100000000000000000000"),
        Err(NumberError::Overflow)
    );
    // Rounding the last place up carries the value out of the range.
    assert_eq!(
        Decimal::parse_text("99999999999999999999.9999999999999999995"),
        Err(NumberError::Overflow)
    );
    assert_eq!(
        Decimal::parse_text("-1e99999999999999999999999"),
        Err(NumberError::Overflow)
    );
}
