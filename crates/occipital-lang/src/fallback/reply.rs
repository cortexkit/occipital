use super::{Cursor, FallbackText, Output, REPLY_BYTES, REPLY_SCALARS};
use crate::catalog::Catalog;

/// Final-parse, nonoverlapping replacements in source-byte order. Failed tags
/// are omitted, leaving their exact literal source in the export (§11.3).
pub(crate) struct Replacement<'a> {
    pub start: usize,
    pub end: usize,
    pub content: Content<'a>,
}

pub(crate) enum Content<'a> {
    /// Values for the catalog's placeholders, taken from a successfully rendered
    /// tag (for example label, children, body, citation index, or point count n).
    Tag {
        name: &'a str,
        fields: &'a [(&'a str, &'a str)],
    },
    Reference {
        tool: Option<&'a str>,
    },
}

/// Export the supplied reply source using the final parse's replacements, not
/// any temporary rendering made while chunks were arriving. Replacement offsets
/// refer to the supplied source, before UTF-8 repair.
pub(crate) fn plain_text(source: &[u8], replacements: &[Replacement<'_>]) -> FallbackText {
    let mut input = Cursor::new(source, REPLY_BYTES);
    let mut output = Output::new(REPLY_SCALARS, false);
    let mut replacements = replacements.iter().peekable();
    if input.starts(b"\xef\xbb\xbf") {
        input.pos += 3;
    }
    while !output.full() && input.peek().is_some() {
        while replacements.peek().is_some_and(|r| r.start < input.pos) {
            replacements.next();
        }
        if let Some(replacement) = replacements
            .peek()
            .filter(|r| r.start == input.pos && r.end > r.start && r.end <= input.bytes.len())
        {
            match &replacement.content {
                Content::Tag { name, fields } => {
                    if let Some(template) = Catalog::shared()
                        .components
                        .get(*name)
                        .filter(|component| component.profiles.iter().any(|p| p == "reply"))
                        .and_then(|component| component.rules.get("states"))
                        .and_then(|states| states["text"].as_str())
                    {
                        input.pos = replacement.end;
                        text_form(&mut output, template, fields);
                        replacements.next();
                        continue;
                    }
                }
                Content::Reference { tool } => {
                    input.pos = replacement.end;
                    if let Some(tool) = tool {
                        output.write("[output of ");
                        output.write(tool);
                        output.push(']');
                    } else {
                        output.write("[tool output]");
                    }
                    replacements.next();
                    continue;
                }
            }
        }
        if let Some(ch) = input.source_char() {
            output.push(ch);
        }
    }
    output.finish(&input, true)
}

fn text_form(output: &mut Output, template: &str, fields: &[(&str, &str)]) {
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.write(&rest[..start]);
        let Some(end) = rest[start..].find('}').map(|end| start + end) else {
            output.write(&rest[start..]);
            return;
        };
        let key = &rest[start + 1..end];
        if let Some((_, value)) = fields.iter().find(|(name, _)| *name == key) {
            output.write(value);
        }
        if output.full() {
            output.cut |= end + 1 < rest.len();
            return;
        }
        rest = &rest[end + 1..];
    }
    output.write(rest);
}
