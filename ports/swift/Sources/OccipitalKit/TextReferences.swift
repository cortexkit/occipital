import OccipitalCore

extension Occipital {
    /// Finds the issue, pull request and `@NAME` mentions in a text, so the
    /// app can draw them as links and chips without changing the text.
    ///
    /// The rules are those of the `cortexkit-occipital-refs` crate: "Issue N",
    /// "PR #N", "pull request N" and the like name an issue or pull request in
    /// the project's repository, `owner/repo#N` names one in that repository,
    /// a bare `#N` names nothing, and `@NAME` names an agent. Code, Markdown
    /// links, tags and URLs are left alone. A link changes the text's colour,
    /// never its words, so it can be applied after a row is measured; a chip
    /// is measured with the row.
    ///
    /// - Parameters:
    ///   - text: the Markdown source the app is about to draw.
    ///   - project: the project record core serves beside the text's agent,
    ///     as the JSON text it arrived as, or `"null"` for text with no
    ///     project, such as a room post by the person.
    /// - Returns: `{"rules", "references": [{"start", "end", "kind",
    ///   "target"}]}`, in text order. `start` and `end` are UTF-8 byte offsets
    ///   into `text`, so they index `text.utf8`. An `issue` or
    ///   `repository_issue` target is a URL; a `mention` target is the name
    ///   after the `@`, to match without regard to case against the agents,
    ///   the person's name and `all`, drawing plain text when nothing matches.
    ///   Draw a `kind` you don't know as plain text.
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
