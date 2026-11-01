//! The status codes every entry point returns. The values and their meanings
//! are fixed by `include/occipital.h`; a test checks the two agree.

/// Success: `*out` holds the result.
pub const OCC_OK: i32 = 0;
/// A NULL required pointer, a JSON parameter that is not UTF-8 or does not
/// parse, an envelope with an unknown or missing member, or an out-of-range
/// value. `*out` is NULL.
pub const OCC_INVALID_ARGUMENT: i32 = 1;
/// An account or limit refused the work. A `char **out` holds the refusal as
/// error JSON.
pub const OCC_REFUSED: i32 = 2;
/// The library rejected the input with diagnostics. A `char **out` holds them
/// as error JSON.
pub const OCC_REJECTED: i32 = 3;
/// A metrics callback returned an unusable value; the measurement was
/// discarded. `*out` is NULL.
pub const OCC_MEASURE_ERROR: i32 = 4;
/// The library panicked; the panic was caught. `*out` is NULL.
pub const OCC_PANIC: i32 = 5;
/// The call is not allowed in the session's current state. `*out` is NULL.
pub const OCC_SESSION_STATE: i32 = 6;
/// An internal error reported instead of a crash. `*out` is NULL.
pub const OCC_INTERNAL: i32 = 7;

/// Whether a failure with `status` hands error JSON back through a
/// `char **out`. Every other non-OK status leaves `*out` NULL.
pub(crate) fn carries_payload(status: i32) -> bool {
    status == OCC_REFUSED || status == OCC_REJECTED
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header is the contract, so its `#define`s must carry exactly the
    /// values the Rust side returns.
    #[test]
    fn header_status_values_match_the_rust_constants() {
        let header = include_str!("../include/occipital.h");
        let defined = |name: &str| -> Option<i32> {
            header.lines().find_map(|line| {
                let mut words = line.split_whitespace();
                (words.next() == Some("#define") && words.next() == Some(name))
                    .then(|| words.next().and_then(|value| value.parse().ok()))
                    .flatten()
            })
        };
        let expected = [
            ("OCC_OK", OCC_OK, 0),
            ("OCC_INVALID_ARGUMENT", OCC_INVALID_ARGUMENT, 1),
            ("OCC_REFUSED", OCC_REFUSED, 2),
            ("OCC_REJECTED", OCC_REJECTED, 3),
            ("OCC_MEASURE_ERROR", OCC_MEASURE_ERROR, 4),
            ("OCC_PANIC", OCC_PANIC, 5),
            ("OCC_SESSION_STATE", OCC_SESSION_STATE, 6),
            ("OCC_INTERNAL", OCC_INTERNAL, 7),
        ];
        for (name, constant, value) in expected {
            assert_eq!(constant, value, "{name}");
            assert_eq!(defined(name), Some(value), "{name} in occipital.h");
        }
    }

    #[test]
    fn only_refused_and_rejected_carry_error_json() {
        let with_json: Vec<i32> = (0..=7).filter(|&s| carries_payload(s)).collect();
        assert_eq!(with_json, [2, 3]);
    }
}
