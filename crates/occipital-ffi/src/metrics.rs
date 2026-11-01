//! The `OccMetrics` callback table and the adapter that turns it into the
//! library's [`Metrics`].
//!
//! The adapter converts each returned `double` into the library's decimal by
//! rounding it up to the next multiple of 10^-6 pt, so measured text is never
//! shorter than the text the platform draws. A callback value that is NaN,
//! negative, infinite or outside the decimal range cannot be measured with:
//! the adapter records the failure and answers zero, and the entry point
//! checks [`Adapter::failed`] after `measure` returns and discards the whole
//! measurement.

use std::cell::Cell;
use std::ffi::{c_char, c_void};

use occipital_lang::{Decimal, Metrics, TextStyle};

use crate::boundary::Failure;

/// `l` in the header: the height of one line in `style`.
pub type LineHeight = unsafe extern "C" fn(ctx: *mut c_void, style: i32) -> f64;
/// `t` in the header: the height of `text` wrapped to `width`, cut at
/// `max_lines` lines (-1 for no limit).
pub type TextHeight = unsafe extern "C" fn(
    ctx: *mut c_void,
    text: *const c_char,
    len: usize,
    style: i32,
    width: f64,
    max_lines: i32,
) -> f64;
/// `tw` in the header: the width of `text` set on one line.
pub type TextWidth =
    unsafe extern "C" fn(ctx: *mut c_void, text: *const c_char, len: usize, style: i32) -> f64;

/// The platform text measurements (`OccMetrics` in the header). The function
/// pointers are `Option`s so a NULL pointer from C is representable and
/// refused as `OCC_INVALID_ARGUMENT` instead of being called.
#[repr(C)]
pub struct OccMetrics {
    /// Passed back unchanged to every callback.
    pub ctx: *mut c_void,
    /// The line height callback.
    pub l: Option<LineHeight>,
    /// The wrapped text height callback.
    pub t: Option<TextHeight>,
    /// The one-line text width callback.
    pub tw: Option<TextWidth>,
}

/// A [`Metrics`] implementation that calls an [`OccMetrics`] table.
pub(crate) struct Adapter {
    ctx: *mut c_void,
    l: LineHeight,
    t: TextHeight,
    tw: TextWidth,
    failed: Cell<bool>,
    /// What a callback with an unusable value answers. occipital-lang has no
    /// public zero constant, so it is built once here.
    zero: Decimal,
}

impl Adapter {
    /// The adapter for `metrics`, or `OCC_INVALID_ARGUMENT` when the table
    /// pointer or any of its callbacks is NULL.
    ///
    /// # Safety
    ///
    /// `metrics` must be NULL or point to an `OccMetrics` valid for the call.
    /// Its callbacks must be safe to call with its `ctx` for as long as the
    /// adapter is used, and must not call back into this library.
    pub(crate) unsafe fn new(metrics: *const OccMetrics) -> Result<Self, Failure> {
        if metrics.is_null() {
            return Err(Failure::invalid());
        }
        // SAFETY: `metrics` is not NULL, and the caller guarantees it points
        // to a valid table.
        let metrics = unsafe { &*metrics };
        let zero = Decimal::from_coefficient(0).map_err(|_| Failure::internal())?;
        match (metrics.l, metrics.t, metrics.tw) {
            (Some(l), Some(t), Some(tw)) => Ok(Self {
                ctx: metrics.ctx,
                l,
                t,
                tw,
                failed: Cell::new(false),
                zero,
            }),
            _ => Err(Failure::invalid()),
        }
    }

    /// Whether any callback returned a value the adapter could not use. A
    /// measurement made while this is true must be discarded.
    pub(crate) fn failed(&self) -> bool {
        self.failed.get()
    }

    /// `value` rounded up to 10^-6 pt, or zero after recording the failure.
    fn length(&self, value: f64) -> Decimal {
        points_up(value).unwrap_or_else(|| {
            self.failed.set(true);
            self.zero
        })
    }
}

impl Metrics for Adapter {
    fn l(&self, style: TextStyle) -> Decimal {
        // SAFETY: `Adapter::new`'s caller guarantees the callback is safe to
        // call with `ctx`.
        let value = unsafe { (self.l)(self.ctx, ordinal(style)) };
        self.length(value)
    }

    fn t(&self, text: &str, style: TextStyle, width: Decimal, max_lines: Option<u32>) -> Decimal {
        let max_lines = max_lines.map_or(-1, |lines| i32::try_from(lines).unwrap_or(i32::MAX));
        // SAFETY: as in `l`; `text` is valid for `text.len()` bytes during the
        // call.
        let value = unsafe {
            (self.t)(
                self.ctx,
                text.as_ptr().cast(),
                text.len(),
                ordinal(style),
                to_f64(width),
                max_lines,
            )
        };
        self.length(value)
    }

    fn tw(&self, text: &str, style: TextStyle) -> Decimal {
        // SAFETY: as in `t`.
        let value =
            unsafe { (self.tw)(self.ctx, text.as_ptr().cast(), text.len(), ordinal(style)) };
        self.length(value)
    }
}

/// The header's style ordinal: Body = 0 through MonoFootnote = 8, in the
/// order `TextStyle` declares them.
pub(crate) fn ordinal(style: TextStyle) -> i32 {
    match style {
        TextStyle::Body => 0,
        TextStyle::Subheadline => 1,
        TextStyle::Footnote => 2,
        TextStyle::Caption1 => 3,
        TextStyle::Headline => 4,
        TextStyle::Title2 => 5,
        TextStyle::Title3 => 6,
        TextStyle::Mono => 7,
        TextStyle::MonoFootnote => 8,
    }
}

/// A width for a callback: the nearest `double` to the decimal. Parsing the
/// decimal's exact text gives the correctly rounded value.
fn to_f64(value: Decimal) -> f64 {
    value.to_string().parse().unwrap_or(0.0)
}

/// Fraction digits of a decimal coefficient: it counts units of 10^-18.
const COEFFICIENT_PER_MICRO: u128 = 1_000_000_000_000;
/// Micro-points per point.
const MICROS: u128 = 1_000_000;

/// `value` rounded up to the next multiple of 10^-6, exactly: the rounding
/// works on the `double`'s exact binary value, not on a rounded product, so
/// the result is never below the value. `None` for NaN, a negative value, an
/// infinity, or a result outside the decimal range. Negative zero is zero.
pub(crate) fn points_up(value: f64) -> Option<Decimal> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    // A finite non-negative double is mantissa * 2^exponent exactly.
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = u128::from(bits & ((1 << 52) - 1));
    let (mantissa, exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let micros = if exponent >= 0 {
        // A whole number of at least 2^52 * 2^exponent. Beyond 2^20 that is
        // over 2^72, far outside the decimal range of about 10^20.
        if exponent > 20 {
            return None;
        }
        (mantissa << exponent).checked_mul(MICROS)?
    } else {
        // mantissa * 10^6 / 2^shift, rounded up. The numerator is below 2^73.
        let shift = exponent.unsigned_abs();
        let numerator = mantissa * MICROS;
        if shift >= 128 {
            u128::from(numerator != 0)
        } else {
            let quotient = numerator >> shift;
            if quotient << shift == numerator {
                quotient
            } else {
                quotient + 1
            }
        }
    };
    let coefficient = i128::try_from(micros.checked_mul(COEFFICIENT_PER_MICRO)?).ok()?;
    Decimal::from_coefficient(coefficient).ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::ptr;

    /// What a test callback table answers and what it was asked.
    #[derive(Default)]
    pub(crate) struct Script {
        /// Returned by every callback when set; otherwise the stub values.
        pub(crate) value: Option<f64>,
        /// Every style ordinal a callback received.
        pub(crate) styles: Vec<i32>,
        /// Every `max_lines` the `t` callback received.
        pub(crate) max_lines: Vec<i32>,
        /// Every `width` the `t` callback received.
        pub(crate) widths: Vec<f64>,
        /// Every text a callback received.
        pub(crate) texts: Vec<String>,
    }

    fn script<'a>(ctx: *mut c_void) -> &'a mut Script {
        // SAFETY: every table built by `table` passes a live `Script`.
        unsafe { &mut *ctx.cast::<Script>() }
    }

    fn text(text: *const c_char, len: usize) -> String {
        // SAFETY: the adapter passes a valid UTF-8 slice of `len` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) };
        std::str::from_utf8(bytes).unwrap().to_owned()
    }

    unsafe extern "C" fn l(ctx: *mut c_void, style: i32) -> f64 {
        let script = script(ctx);
        script.styles.push(style);
        script.value.unwrap_or(20.0)
    }

    unsafe extern "C" fn t(
        ctx: *mut c_void,
        value: *const c_char,
        len: usize,
        style: i32,
        width: f64,
        max_lines: i32,
    ) -> f64 {
        let script = script(ctx);
        script.styles.push(style);
        script.max_lines.push(max_lines);
        script.widths.push(width);
        script.texts.push(text(value, len));
        script.value.unwrap_or(20.0)
    }

    unsafe extern "C" fn tw(ctx: *mut c_void, value: *const c_char, len: usize, style: i32) -> f64 {
        let script = script(ctx);
        script.styles.push(style);
        let value = text(value, len);
        let width = value.len() as f64;
        script.texts.push(value);
        script.value.unwrap_or(width)
    }

    /// A callback table over `script`: the stub answers 20 for `l` and `t`
    /// and the UTF-8 byte length for `tw`, unless `script.value` is set.
    pub(crate) fn table(script: &mut Script) -> OccMetrics {
        OccMetrics {
            ctx: ptr::from_mut(script).cast(),
            l: Some(l),
            t: Some(t),
            tw: Some(tw),
        }
    }

    fn adapter(metrics: &OccMetrics) -> Adapter {
        // SAFETY: `metrics` is a live table over a live script.
        unsafe { Adapter::new(metrics) }.unwrap()
    }

    fn dec(text: &str) -> Decimal {
        crate::decimal::parse(text).unwrap()
    }

    #[test]
    fn styles_pass_as_their_ordinals() {
        let styles = [
            TextStyle::Body,
            TextStyle::Subheadline,
            TextStyle::Footnote,
            TextStyle::Caption1,
            TextStyle::Headline,
            TextStyle::Title2,
            TextStyle::Title3,
            TextStyle::Mono,
            TextStyle::MonoFootnote,
        ];
        let mut script = Script::default();
        let metrics = table(&mut script);
        let adapter = adapter(&metrics);
        for style in styles {
            adapter.l(style);
        }
        assert_eq!(script.styles, (0..=8).collect::<Vec<_>>());
    }

    #[test]
    fn no_line_limit_is_minus_one_and_a_limit_passes_through() {
        let mut script = Script::default();
        let metrics = table(&mut script);
        let adapter = adapter(&metrics);
        adapter.t("a", TextStyle::Body, dec("12.5"), None);
        adapter.t("b", TextStyle::Body, dec("12.5"), Some(3));
        adapter.t("c", TextStyle::Body, dec("12.5"), Some(u32::MAX));
        assert_eq!(script.max_lines, [-1, 3, i32::MAX]);
        assert_eq!(script.widths, [12.5, 12.5, 12.5]);
        assert_eq!(script.texts, ["a", "b", "c"]);
    }

    #[test]
    fn text_passes_as_its_utf8_bytes() {
        let mut script = Script::default();
        let metrics = table(&mut script);
        let adapter = adapter(&metrics);
        assert_eq!(adapter.tw("é🙂", TextStyle::Mono), dec("6"));
        assert_eq!(script.texts, ["é🙂"]);
        assert_eq!(script.styles, [7]);
    }

    #[test]
    fn doubles_round_up_to_a_millionth_of_a_point() {
        let cases: [(f64, &str); 9] = [
            (0.0, "0"),
            (-0.0, "0"),
            (20.0, "20"),
            (1.5, "1.5"),
            (17.25, "17.25"),
            // 2.0000001 is not a multiple of 10^-6: it rounds up, not to the
            // nearest.
            (2.0000001, "2.000001"),
            (2.000001 - 1e-12, "2.000001"),
            // The double nearest 0.1 is slightly above 0.1, so exact rounding
            // up gives the next millionth.
            (0.1, "0.100001"),
            // The smallest subnormal is still above zero.
            (f64::from_bits(1), "0.000001"),
        ];
        for (value, expected) in cases {
            assert_eq!(points_up(value), Some(dec(expected)), "{value:e}");
        }
        // A large whole number stays exact.
        assert_eq!(
            points_up(90_000_000_000_000_000_000.0),
            Some(dec("90000000000000000000"))
        );
    }

    #[test]
    fn unusable_doubles_have_no_decimal() {
        for value in [
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            -f64::from_bits(1),
            // 10^20 is just past the largest decimal.
            100_000_000_000_000_000_000.0,
            f64::MAX,
        ] {
            assert_eq!(points_up(value), None, "{value:e}");
        }
    }

    #[test]
    fn an_unusable_value_is_recorded_and_answers_zero() {
        for value in [f64::NAN, -1.0, f64::INFINITY, 1e21] {
            let mut script = Script {
                value: Some(value),
                ..Script::default()
            };
            let metrics = table(&mut script);
            let adapter = adapter(&metrics);
            assert!(!adapter.failed());
            assert_eq!(adapter.l(TextStyle::Body), dec("0"));
            assert!(adapter.failed(), "{value:e}");
        }
    }

    #[test]
    fn a_null_table_or_callback_is_an_invalid_argument() {
        // SAFETY: NULL is allowed.
        assert!(unsafe { Adapter::new(ptr::null()) }.is_err());
        let mut script = Script::default();
        for which in 0..3 {
            let mut metrics = table(&mut script);
            match which {
                0 => metrics.l = None,
                1 => metrics.t = None,
                _ => metrics.tw = None,
            }
            // SAFETY: `metrics` is a live table.
            let adapter = unsafe { Adapter::new(&metrics) };
            assert_eq!(adapter.err(), Some(Failure::invalid()), "{which}");
        }
    }
}
