import OccipitalCore

extension Occipital {
    /// Finds the issue and pull request mentions in a text, so the app can
    /// draw them as links without changing the text.
    ///
    /// The rules are those of the `cortexkit-occipital-refs` crate: "Issue N",
    /// "PR #N", "pull request N" and the like name an issue or pull request in
    /// the project's repository, `owner/repo#N` names one in that repository,
    /// and a bare `#N` names nothing. Code, Markdown links, tags and URLs are
    /// left alone. A link changes the text's colour, never its words, so it
    /// can be applied after a row is measured.
    ///
    /// - Parameters:
    ///   - text: the Markdown source the app is about to draw.
    ///   - project: the project record core serves beside the text's agent,
    ///     as the JSON text it arrived as, or `"null"` for text with no
    ///     project, such as a room post by the person.
    /// - Returns: `{"rules", "references": [{"start", "end", "kind",
    ///   "target"}]}`, in text order. `start` and `end` are UTF-8 byte offsets
    ///   into `text`, so they index `text.utf8`. Draw a `kind` you don't know
    ///   as a plain link to its `target`.
    /// - Throws: `.invalidArgument` when `project` is not JSON.
    public static func textReferences(
        in text: String,
        project: String
    ) throws(OccipitalError) -> String {
        try withByteBuffer(text.utf8) { buffer, count in
            project.withCString { project in
                call { occ_text_references(buffer, count, project, $0) }
            }
        }.get()
    }
}
