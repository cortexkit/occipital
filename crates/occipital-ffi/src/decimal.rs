//! Decimals in JSON envelopes are canonical decimal strings: the text form
//! that `Decimal`'s `Display` writes (language spec §4.2). occipital-lang has no public parser
//! for it, so the decoder lives here and accepts exactly that form.

use occipital_lang::Decimal;

/// Fraction digits a decimal carries: a coefficient counts units of 10^-18.
const PLACES: usize = 18;

/// The canonical text of `value`.
pub(crate) fn encode(value: Decimal) -> String {
    value.to_string()
}

/// Parses canonical decimal text: an optional `-`, integer digits without a
/// leading zero (except `0` itself), and optionally `.` with 1 to 18 fraction
/// digits not ending in `0`. `-0`, `+1`, `1.`, `.5`, `01`, `1.50` and
/// exponents are refused, as is any value outside the decimal range.
pub(crate) fn parse(text: &str) -> Option<Decimal> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (integer, fraction) = match unsigned.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (unsigned, ""),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(integer) || (integer.len() > 1 && integer.starts_with('0')) {
        return None;
    }
    if unsigned.contains('.')
        && (!digits(fraction) || fraction.len() > PLACES || fraction.ends_with('0'))
    {
        return None;
    }
    let mut coefficient: i128 = 0;
    for byte in integer.bytes().chain(fraction.bytes()) {
        coefficient = coefficient
            .checked_mul(10)?
            .checked_add(i128::from(byte - b'0'))?;
    }
    for _ in fraction.len()..PLACES {
        coefficient = coefficient.checked_mul(10)?;
    }
    if negative {
        if coefficient == 0 {
            return None;
        }
        coefficient = -coefficient;
    }
    Decimal::from_coefficient(coefficient).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coefficient(text: &str) -> Option<i128> {
        parse(text).map(Decimal::coefficient)
    }

    #[test]
    fn parses_canonical_text_to_the_exact_coefficient() {
        const ONE: i128 = 1_000_000_000_000_000_000;
        assert_eq!(coefficient("0"), Some(0));
        assert_eq!(coefficient("390"), Some(390 * ONE));
        assert_eq!(coefficient("-12.5"), Some(-125 * ONE / 10));
        assert_eq!(coefficient("0.000000000000000001"), Some(1));
        assert_eq!(
            coefficient("99999999999999999999.999999999999999999"),
            Some(99_999_999_999_999_999_999_999_999_999_999_999_999)
        );
    }

    #[test]
    fn refuses_text_that_is_not_canonical() {
        for text in [
            "", "-", "-0", "+1", "1.", ".5", "01", "00", "1.50", "1e3", "1.0", " 1", "1 ", "0x10",
            "1..2", "--1", "\u{0661}",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn refuses_values_outside_the_range() {
        assert_eq!(parse("100000000000000000000"), None);
        assert_eq!(parse("0.0000000000000000001"), None);
        assert_eq!(parse(&"9".repeat(60)), None);
    }

    #[test]
    fn encoding_round_trips() {
        for text in ["0", "1", "-1", "390", "12.5", "-0.000001", "123.456789"] {
            let value = parse(text).unwrap();
            assert_eq!(encode(value), text);
        }
    }
}
