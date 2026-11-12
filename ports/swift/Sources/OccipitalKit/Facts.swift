import OccipitalCore

extension Occipital {
    /// Words one fact a module reports, such as a fact in a command's
    /// preview, so the phone draws it exactly as the desktop does.
    ///
    /// A fact is a number with a unit (`count` with an optional `noun`,
    /// `tokens`, `bytes`, `duration` in milliseconds, `ratio`) or a state
    /// (`ready`, `building` with an optional `progress`, `paused`, `stale`,
    /// `unavailable`, `off`). The module's label is kept as given; the value
    /// is worded by the library ("about 18k tokens", "Building, 40%"), and
    /// a state's tone is fixed.
    ///
    /// - Parameter fact: the fact as the JSON text the module sent.
    /// - Returns: `{"label", "text", "tone"}`, with `tone` one of `neutral`,
    ///   `info`, `success`, `warning`, `danger`; or `{"error": "<code>"}` for
    ///   a fact the rules refuse, which the app shows as missing.
    /// - Throws: `.invalidArgument` when `fact` is not JSON.
    public static func formatFact(_ fact: String) throws(OccipitalError) -> String {
        try fact.withCString { fact in
            call { occ_format_fact(fact, $0) }
        }.get()
    }
}
