import OccipitalCore

/// The stateless entry points of the C library: version, measurement,
/// program checks and tool templates.
///
/// Every JSON parameter and result follows the formats documented in
/// `occipital.h`. These may be called from any thread, but never from inside
/// a `TextMetrics` method.
public enum Occipital {
    /// The version descriptor JSON: `{"executionDialect": "...", "catalog":
    /// {"version", "digest", "layout"}, "toolCatalog": {"version",
    /// "digest", "layout", "aliases"}}`; `aliases` is left out of a catalog
    /// that has none. The runner stamps each rich UI reply with the digest of
    /// the catalog it was written for. A reply stamped with `catalog.digest`
    /// (or an alias, when it has any) draws with this library's catalog; any
    /// other draws as plain Markdown. A tool
    /// template is for this library when its `catalog.digest` is `sha256:`
    /// followed by `toolCatalog.digest` or by one of `toolCatalog.aliases`.
    public static func version() throws(OccipitalError) -> String {
        try call { occ_version($0) }.get()
    }

    /// Measures one component node of a tree as a one-block tree.
    ///
    /// - Parameters:
    ///   - tree: the tree JSON.
    ///   - path: a non-empty JSON array of child indices, such as `[1]` or
    ///     `[0,2]`: the first picks one of the root's blocks, each later one
    ///     a child of the node before.
    ///   - layout: the layout context JSON.
    ///   - metrics: the platform text metrics.
    /// - Returns: the measurement JSON.
    public static func measureBlock(
        tree: String, path: String, layout: String, metrics: some TextMetrics
    ) throws(OccipitalError) -> String {
        try withCStrings(tree, path) { tree, path in
            layout.withCString { layout in
                withMetricsTable(metrics) { table in
                    call { occ_measure_block(tree, path, layout, table, $0) }
                }
            }
        }.get()
    }

    /// Measures a whole tree.
    ///
    /// - Parameters:
    ///   - tree: the tree JSON.
    ///   - layout: the layout context JSON.
    ///   - metrics: the platform text metrics.
    /// - Returns: the measurement JSON.
    public static func measureTree(
        tree: String, layout: String, metrics: some TextMetrics
    ) throws(OccipitalError) -> String {
        try withCStrings(tree, layout) { tree, layout in
            withMetricsTable(metrics) { table in
                call { occ_measure_tree(tree, layout, table, $0) }
            }
        }.get()
    }

    /// Checks program source and runs its trial evaluation.
    ///
    /// The source bytes are passed as they are: invalid UTF-8 is repaired and
    /// reported as a `W_ENCODING` diagnostic, not rejected. A completed check
    /// returns its result even when it found error diagnostics.
    ///
    /// - Parameters:
    ///   - source: the program source bytes.
    ///   - context: `{"context": ..., "host": ...}`; `host` may be left out.
    /// - Returns: the check result JSON.
    public static func checkProgram(
        _ source: some Sequence<UInt8>, context: String
    ) throws(OccipitalError) -> String {
        try withByteBuffer(source) { buffer, count in
            context.withCString { context in
                call { occ_program_check(buffer, count, context, $0) }
            }
        }.get()
    }
}
