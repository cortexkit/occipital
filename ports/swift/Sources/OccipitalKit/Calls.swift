import OccipitalCore

// Helpers that call the C library and turn its output into Swift values.
// Every string the library returns is copied and then freed here, with
// `defer` right after the call, so no caller ever holds a library-owned
// pointer.

/// The status of one C call and a Swift copy of the string it returned.
struct CallResult {
    let status: Int32
    let output: String?

    /// The output of a successful call, or the matching `OccipitalError`.
    func get() throws(OccipitalError) -> String {
        guard status == OCC_OK else {
            throw OccipitalError(status: status, output: output)
        }
        guard let output else {
            // OCC_OK always comes with a result; a missing one breaks the
            // library's contract.
            throw .internal
        }
        return output
    }
}

/// Calls a C entry point whose result comes back through `char **out`.
///
/// The returned string is copied and freed with `occ_string_free` whatever
/// the status. On `OCC_REFUSED` and `OCC_REJECTED` the copy is the error
/// JSON, which `CallResult.get()` puts into the thrown `OccipitalError`.
func call(
    _ body: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32
) -> CallResult {
    var out: UnsafeMutablePointer<CChar>? = nil
    let status = withUnsafeMutablePointer(to: &out) { body($0) }
    defer { occ_string_free(out) }
    return CallResult(status: status, output: out.map { String(cString: $0) })
}

/// Runs `body` with a NUL-terminated UTF-8 copy of each of two strings.
func withCStrings<R>(
    _ first: String, _ second: String,
    _ body: (UnsafePointer<CChar>, UnsafePointer<CChar>) -> R
) -> R {
    first.withCString { a in second.withCString { b in body(a, b) } }
}

/// Runs `body` with a contiguous copy of `bytes`: a pointer that may be NULL
/// when the buffer is empty (the C library allows that) and the byte count.
func withByteBuffer<R>(
    _ bytes: some Sequence<UInt8>,
    _ body: (UnsafePointer<UInt8>?, Int) -> R
) -> R {
    Array(bytes).withUnsafeBufferPointer { buffer in
        body(buffer.baseAddress, buffer.count)
    }
}
