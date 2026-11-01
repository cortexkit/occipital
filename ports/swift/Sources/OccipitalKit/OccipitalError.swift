import OccipitalCore

/// A non-OK status returned by the OccipitalCore C library.
///
/// There is one case per status code in `occipital.h`. Only `refused` and
/// `rejected` carry a payload: the error JSON the library left in its output.
public enum OccipitalError: Error, Sendable, Equatable {
    /// `OCC_INVALID_ARGUMENT`: a JSON parameter did not decode (bad JSON, an
    /// unknown or missing member) or a value was out of its documented range.
    case invalidArgument

    /// `OCC_REFUSED`: an account or limit refused the work. The payload is
    /// the error JSON `{"error": "refused", "refused": {...}}`. It is `nil`
    /// when the refusing call returns a handle (creating a reply session),
    /// because such calls have no string output to carry the JSON.
    case refused(errorJSON: String?)

    /// `OCC_REJECTED`: the library rejected the input with diagnostics. The
    /// payload is the error JSON `{"error": "rejected", "diagnostics": [...]}`.
    case rejected(errorJSON: String?)

    /// `OCC_MEASURE_ERROR`: a `TextMetrics` method returned NaN, a negative
    /// value or an infinity, or the tree could not be measured. The
    /// measurement was discarded and no session state changed.
    case measureError

    /// `OCC_PANIC`: the library panicked and recovered. A reply session that
    /// reported this should be dropped rather than used again.
    case panic

    /// `OCC_SESSION_STATE`: the call is not allowed in the reply session's
    /// current state, such as finishing a reply twice.
    case sessionState

    /// `OCC_INTERNAL`: the library reported a broken invariant. This case also
    /// covers the library breaking its own contract: a status code the header
    /// does not define, or a success without the promised output.
    case `internal`

    /// Maps a non-OK status and the string the call left in its output.
    init(status: Int32, output: String?) {
        switch status {
        case OCC_INVALID_ARGUMENT: self = .invalidArgument
        case OCC_REFUSED: self = .refused(errorJSON: output)
        case OCC_REJECTED: self = .rejected(errorJSON: output)
        case OCC_MEASURE_ERROR: self = .measureError
        case OCC_PANIC: self = .panic
        case OCC_SESSION_STATE: self = .sessionState
        default: self = .internal
        }
    }
}
