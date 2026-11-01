import OccipitalCore

/// One streaming reply: feed it the reply's bytes as they arrive, finish it
/// with the stored records, then deliver late host events and measure it.
///
/// Every JSON parameter and result follows the formats documented in
/// `occipital.h`. A session must have one caller at a time, so it is not
/// `Sendable`: keep it inside the one actor that owns it.
public final class ReplySession {
    private let handle: OpaquePointer

    /// Starts a session for one reply under the reply profile.
    ///
    /// - Parameters:
    ///   - stamp: `{"dialect": "..."}`.
    ///   - layout: the layout context JSON (`width`,
    ///     `content_size_category`, `network`, `placement`).
    ///   - chat: the chat account of the reply's window.
    ///   - app: the app account.
    /// - Throws: `OccipitalError.refused(errorJSON: nil)` when an account
    ///   refuses the session (the C call cannot return the refusal JSON).
    public init(
        stamp: String, layout: String, chat: OccipitalAccount, app: OccipitalAccount
    ) throws(OccipitalError) {
        var out: OpaquePointer? = nil
        let status = withCStrings(stamp, layout) { stamp, layout in
            withExtendedLifetime((chat, app)) {
                occ_reply_new(stamp, layout, chat.handle, app.handle, &out)
            }
        }
        guard status == OCC_OK else {
            throw OccipitalError(status: status, output: nil)
        }
        guard let out else { throw .internal }
        handle = out
    }

    deinit {
        occ_reply_free(handle)
    }

    /// Feeds streamed reply bytes. A UTF-8 sequence may be split across
    /// calls and the finish tail.
    ///
    /// - Returns: the provisional revision JSON, or the JSON text `null` when
    ///   nothing visible changed.
    public func chunk(_ bytes: some Sequence<UInt8>) throws(OccipitalError) -> String {
        try withByteBuffer(bytes) { buffer, count in
            call { occ_reply_chunk(handle, buffer, count, $0) }
        }.get()
    }

    /// Feeds the final bytes and finishes the reply.
    ///
    /// - Parameters:
    ///   - tail: the last bytes of the reply, possibly empty.
    ///   - records: `{"references": {...}, "handles": [...]}`, the records
    ///     stamped when the reply was stored.
    /// - Returns: the authoritative revision JSON.
    public func finish(
        _ tail: some Sequence<UInt8>, records: String
    ) throws(OccipitalError) -> String {
        try withByteBuffer(tail) { buffer, count in
            records.withCString { records in
                call { occ_reply_finish(handle, buffer, count, records, $0) }
            }
        }.get()
    }

    /// Finishes the reply with no further bytes.
    public func finish(records: String) throws(OccipitalError) -> String {
        try finish([UInt8](), records: records)
    }

    /// Delivers one late host event, tagged by `"kind"`.
    ///
    /// - Returns: the resulting revision JSON, or the JSON text `null` when
    ///   nothing changed.
    public func event(_ event: String) throws(OccipitalError) -> String {
        try event.withCString { event in
            call { occ_reply_event(handle, event, $0) }
        }.get()
    }

    /// Measures the current candidate revision under `layout` with `metrics`.
    /// Only a successful measurement is delivered to the session.
    ///
    /// - Returns: the measurement JSON.
    public func measure(
        layout: String, metrics: some TextMetrics
    ) throws(OccipitalError) -> String {
        try layout.withCString { layout in
            withMetricsTable(metrics) { table in
                call { occ_reply_measure(handle, layout, table, $0) }
            }
        }.get()
    }
}

// A session has one caller at a time, so it must never cross isolation
// domains. Spelling the conformance out as unavailable makes that explicit.
@available(*, unavailable)
extension ReplySession: Sendable {}
