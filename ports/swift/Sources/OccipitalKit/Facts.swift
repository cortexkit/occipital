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

    /// Words a module's recurring schedule in the person's own clock, so the
    /// phone draws it exactly as the desktop does: "Every day at 06:00",
    /// "Mondays and Thursdays at 09:30", "Weekdays at 04:30", "Every hour
    /// at :15".
    ///
    /// - Parameters:
    ///   - schedule: the schedule as the JSON text the module sent, with its
    ///     `time_zone`.
    ///   - shiftMinutes: the device's UTC offset minus the offset of the
    ///     schedule's `time_zone`, both now, in minutes. For example
    ///     `(TimeZone.current.secondsFromGMT() - TimeZone(identifier:
    ///     zone)!.secondsFromGMT()) / 60`; 0 words it as given.
    /// - Returns: `{"text"}`; or `{"error": "schedule_unworded"}` for a
    ///   cron-only schedule and `{"error": "schedule_malformed"}` for any
    ///   other shape, for which the app shows its own fallback.
    /// - Throws: `.invalidArgument` when `schedule` is not JSON.
    public static func formatSchedule(
        _ schedule: String, shiftMinutes: Int32
    ) throws(OccipitalError) -> String {
        try schedule.withCString { schedule in
            call { occ_format_schedule(schedule, shiftMinutes, $0) }
        }.get()
    }
}
