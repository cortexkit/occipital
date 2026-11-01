import OccipitalCore

/// Which part of a tool call a template draws.
public enum ToolTemplateProfile: String, Sendable, CaseIterable {
    /// The call's row in the chat.
    case row = "tool_row"
    /// The call's page, opened from its row.
    case page = "tool_page"
}

extension Occipital {
    /// Checks a tool template and evaluates it once.
    ///
    /// A tool template makes no reads and keeps no state, so drawing a row or
    /// a page is one evaluation: evaluate again when the call's view changes,
    /// such as when its record arrives. Occipital checked the template against
    /// the tool's input and record schemas before it served the template's
    /// digest, so here `input` and `view` are typed as any value.
    ///
    /// A row's height depends only on its template and input, never on its
    /// view, because everything a view fills sits inside a fixed `Slot` box. A
    /// row's measurement can therefore be kept by template digest, input,
    /// width and text size, and reused as the record and completion arrive.
    ///
    /// - Parameters:
    ///   - source: the template document's `source`.
    ///   - profile: the template document's `profile`.
    ///   - executionDialect: the template document's `executionDialect`.
    ///   - input: the tool part's `input`, as a JSON object text.
    ///   - truncatedFields: the tool part's `inputTruncatedFields`.
    ///   - view: the call's view, as a JSON object text.
    ///   - nowMs: the current time, in Unix milliseconds.
    ///   - zone: the time zone times are shown in, such as `Europe/Madrid`.
    /// - Returns: the evaluation JSON `{"tree", "diagnostics", "outcome"}`.
    ///   `tree` is the tree JSON for `measureTree`, or null when the evaluation
    ///   stopped, and then `outcome` names why.
    /// - Throws: `.rejected` with the diagnostics when the template fails its
    ///   checks, and `.invalidArgument` when `input` or `view` is not JSON.
    public static func evaluateToolTemplate(
        _ source: some Sequence<UInt8>,
        profile: ToolTemplateProfile,
        executionDialect: String,
        input: String,
        truncatedFields: [String],
        view: String,
        nowMs: UInt64,
        zone: String
    ) throws(OccipitalError) -> String {
        let fields = truncatedFields.map(jsonString).joined(separator: ",")
        let context = """
            {"context":{"profile":\(jsonString(profile.rawValue)),\
            "execution_dialect":\(jsonString(executionDialect)),"catalog":"1.1",\
            "registry":{},"tool":{"input":{"value":\(input),"schema":{}},\
            "view":{"value":\(view),"schema":{}},\
            "truncated":\(truncatedFields.isEmpty ? "false" : "true"),\
            "truncated_fields":[\(fields)]},\
            "show_ui":{"title":"","summary":"","data_format":null,"data_columns":{}},\
            "references":{},"handles":[]},\
            "host":{"network":true,"now":"\(nowMs)","zone":\(jsonString(zone)),\
            "data":null,"snapshots":{}}}
            """
        return try withByteBuffer(source) { buffer, count in
            context.withCString { context in
                call { occ_template_evaluate(buffer, count, context, $0) }
            }
        }.get()
    }
}

/// `text` as a JSON string literal.
private func jsonString(_ text: String) -> String {
    var escaped = "\""
    for scalar in text.unicodeScalars {
        switch scalar {
        case "\"": escaped += "\\\""
        case "\\": escaped += "\\\\"
        case "\n": escaped += "\\n"
        case "\r": escaped += "\\r"
        case "\t": escaped += "\\t"
        case _ where scalar.value < 0x20:
            let hex = String(scalar.value, radix: 16)
            escaped += "\\u" + String(repeating: "0", count: 4 - hex.count) + hex
        default: escaped.unicodeScalars.append(scalar)
        }
    }
    return escaped + "\""
}
