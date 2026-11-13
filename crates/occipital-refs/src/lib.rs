//! Finds mentions of issues, pull requests and agents in text, so an app can
//! draw them as links or chips: "PR #499" in an agent's reply becomes a link
//! to pull request 499 of the repository the agent works in, and `@CKDESK`
//! names the agent CKDESK.
//!
//! The text itself is never changed. An app passes the text it is about to
//! draw and the project the text belongs to, and gets back byte ranges with
//! a URL for each. Drawing a link over a range changes the text's colour, not
//! its words, so copying, quoting and row heights stay as they were.
//!
//! The rules are in this crate's README. In short:
//!
//! - "Issue N", "issue #N", "PR N", "PR #N" and "pull request N" (any case)
//!   name issue or pull request N in the project's repository, kind
//!   [`ISSUE`]. Without a repository they name nothing.
//! - `owner/repo#N` names issue or pull request N in that repository, kind
//!   [`REPOSITORY_ISSUE`], whatever the project.
//! - A bare `#N` names nothing: in agents' text it is usually a room post.
//! - `@NAME` names an agent, kind [`MENTION`], with the name as typed for the
//!   app to look up; `@all` comes back as the name `all`.
//! - Nothing inside code (code spans and fenced code blocks), Markdown links
//!   (inline, reference and their definitions), HTML tags, autolinks or bare
//!   URLs is a mention.
//!
//! The crate has no dependencies, so an app can depend on it without its
//! feature choices reaching the app's own crates.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

/// The version of the rules [`references`] applies. It goes up whenever the
/// same text and project could give different references, so apps can tell
/// which rules a set of test vectors pins.
pub const RULES_VERSION: u32 = 2;

/// The kind of "Issue N", "PR N" and the like: an issue or pull request in
/// the project's own repository.
pub const ISSUE: &str = "issue";

/// The kind of `owner/repo#N`: an issue or pull request in the repository it
/// names.
pub const REPOSITORY_ISSUE: &str = "repository_issue";

/// The kind of `@NAME`: a name, which an app draws as a chip when it names
/// an agent it knows, comparing without regard to case. Its target is the
/// name, without the `@`.
pub const MENTION: &str = "mention";

/// The host whose URLs version 1 knows how to build.
const GITHUB: &str = "github.com";

/// What the text's project tells the rules. Apps fill it from the project
/// record core serves beside each agent; every member is optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Project<'a> {
    /// The repository the project pushes to.
    pub repository: Option<Repository<'a>>,
}

/// A code repository, as `{host, owner, name}`: `github.com`, `cortexkit`,
/// `occipital`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repository<'a> {
    pub host: &'a str,
    pub owner: &'a str,
    pub name: &'a str,
}

/// One mention found in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The UTF-8 byte offset where the mention starts, at the first byte of
    /// its keyword or of `owner`.
    pub start: usize,
    /// The UTF-8 byte offset just past the mention's last digit.
    pub end: usize,
    /// [`ISSUE`], [`REPOSITORY_ISSUE`] or [`MENTION`]. Later rule versions
    /// add kinds, so an app that doesn't know a kind should draw the text as
    /// it is.
    pub kind: &'static str,
    /// The URL an issue mention links to, or the name a [`MENTION`] names.
    pub target: String,
}

/// Every mention in `text`, in order, none overlapping another.
///
/// `text` is the Markdown source of what the app draws, before it is
/// rendered, so the rules can leave code and existing links alone.
pub fn references(text: &str, project: &Project<'_>) -> Vec<Reference> {
    let home = project.repository.filter(Repository::is_supported);
    // Where `owner/repo#N` points: the project's own host when it has a
    // repository, else GitHub. A project on a host the rules don't know yet
    // gets no links of either kind.
    let host_known = project
        .repository
        .is_none_or(|repository| repository.host.eq_ignore_ascii_case(GITHUB));
    let mut found = Vec::new();
    let stretches = prose(text);
    let defined = definitions(text, &stretches);
    for &(start, end) in &stretches {
        Stretch::new(text, start, end, &defined).scan(start, home, host_known, &mut found);
    }
    found
}

impl Repository<'_> {
    /// Whether links into this repository can be built: a host the rules
    /// know, and an owner and name GitHub allows, so the URL needs no
    /// escaping.
    fn is_supported(&self) -> bool {
        self.host.eq_ignore_ascii_case(GITHUB) && is_owner(self.owner) && is_name(self.name)
    }
}

/// The byte ranges of `text` outside fenced code blocks.
///
/// A fence is a line whose first non-blank characters are three or more
/// backticks or tildes, at any indentation, so a fence inside a list item
/// counts too. A backtick fence's info string has no backtick. The block ends
/// at a line of the same character, at least as long, and nothing else but
/// blanks; an unclosed block runs to the end of the text.
fn prose(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut prose_start = 0;
    let mut open: Option<(u8, usize)> = None;
    let mut line_start = 0;
    while line_start < text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |at| line_start + at + 1);
        let line = text[line_start..line_end].trim_end_matches(['\n', '\r']);
        let trimmed = line.trim_start_matches([' ', '\t']);
        match open {
            None => {
                if let Some(fence) = fence(trimmed) {
                    ranges.push((prose_start, line_start));
                    open = Some(fence);
                }
            }
            Some((mark, length)) => {
                let run = trimmed.bytes().take_while(|&byte| byte == mark).count();
                if run >= length && trimmed[run..].trim_matches([' ', '\t']).is_empty() {
                    open = None;
                    prose_start = line_end;
                }
            }
        }
        line_start = line_end;
    }
    if open.is_none() {
        ranges.push((prose_start, text.len()));
    }
    ranges.retain(|(start, end)| start < end);
    ranges
}

/// The normalized labels of the link reference definitions in the prose
/// stretches: lines whose first non-blank character opens `[label]:`.
/// Definitions apply to the whole text, wherever they sit.
fn definitions(text: &str, stretches: &[(usize, usize)]) -> HashSet<String> {
    let mut defined = HashSet::new();
    for &(start, end) in stretches {
        for line in text[start..end].split('\n') {
            let Some(rest) = line.trim_start_matches([' ', '\t']).strip_prefix('[') else {
                continue;
            };
            let Some(close) = rest.find(']') else {
                continue;
            };
            if rest[close + 1..].starts_with(':') && !rest[..close].contains('[') {
                defined.insert(normalize(&rest[..close]));
            }
        }
    }
    defined
}

/// A link label as CommonMark matches it: blanks collapsed to one space,
/// trimmed, and ASCII letters in lower case.
fn normalize(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Whether only blanks come before `at` on its line.
fn line_starts_at(text: &str, at: usize) -> bool {
    text[..at]
        .bytes()
        .rev()
        .take_while(|&byte| byte != b'\n')
        .all(|byte| byte == b' ' || byte == b'\t')
}

/// The fence a line opens, as its character and length, if it opens one.
fn fence(trimmed: &str) -> Option<(u8, usize)> {
    let mark = *trimmed.as_bytes().first()?;
    if mark != b'`' && mark != b'~' {
        return None;
    }
    let run = trimmed.bytes().take_while(|&byte| byte == mark).count();
    if run < 3 || (mark == b'`' && trimmed[run..].contains('`')) {
        return None;
    }
    Some((mark, run))
}

/// One stretch of text outside fenced code, with what its scan looks up
/// worked out once, so text full of unclosed markup takes time in proportion
/// to its length.
struct Stretch<'t> {
    text: &'t str,
    /// The labels of the text's link reference definitions, normalized.
    defined: &'t HashSet<String>,
    end: usize,
    /// Where each line holding only blanks starts, in order. No code span or
    /// link crosses one.
    breaks: Vec<usize>,
    /// Where each `[` and `(` that a `]` or `)` closes in the same paragraph
    /// is closed, counting nested pairs and skipping escaped brackets.
    closers: HashMap<usize, usize>,
    /// The next `>` and the next line end found so far. Positions are asked
    /// about in order, so each is searched for once.
    next_angle: Option<usize>,
    next_line: Option<usize>,
}

impl<'t> Stretch<'t> {
    fn new(text: &'t str, start: usize, end: usize, defined: &'t HashSet<String>) -> Self {
        let bytes = text.as_bytes();
        let mut breaks = Vec::new();
        let mut line_start = start;
        while line_start < end {
            let line_end = text[line_start..end]
                .find('\n')
                .map_or(end, |offset| line_start + offset + 1);
            if line_start > start && text[line_start..line_end].trim().is_empty() {
                breaks.push(line_start);
            }
            line_start = line_end;
        }
        let mut closers = HashMap::new();
        let (mut brackets, mut parens) = (Vec::new(), Vec::new());
        let mut next_break = breaks.iter().peekable();
        let mut at = start;
        while at < end {
            if next_break.next_if(|&&blank| blank <= at).is_some() {
                brackets.clear();
                parens.clear();
            }
            match bytes[at] {
                b'\\' if at + 1 < end && bytes[at + 1].is_ascii_punctuation() => at += 1,
                b'[' => brackets.push(at),
                b'(' => parens.push(at),
                b']' => {
                    if let Some(open) = brackets.pop() {
                        closers.insert(open, at);
                    }
                }
                b')' => {
                    if let Some(open) = parens.pop() {
                        closers.insert(open, at);
                    }
                }
                _ => {}
            }
            at += 1;
        }
        Self {
            text,
            defined,
            end,
            breaks,
            closers,
            next_angle: None,
            next_line: None,
        }
    }

    /// Finds the mentions in the stretch starting at `start`.
    fn scan(
        &mut self,
        start: usize,
        home: Option<Repository<'_>>,
        host_known: bool,
        found: &mut Vec<Reference>,
    ) {
        let (text, end) = (self.text, self.end);
        let bytes = text.as_bytes();
        let mut at = start;
        while at < end {
            let byte = bytes[at];
            let skip = match byte {
                b'\\' => (at + 1 < end && bytes[at + 1].is_ascii_punctuation()).then_some(at + 2),
                b'`' => Some(self.code_span(at)),
                b'[' => self.link(at),
                b'<' => self.tag(at),
                b'h' | b'H' => url(text, at, end),
                _ => None,
            };
            if let Some(next) = skip {
                at = next;
                continue;
            }
            if byte == b'@' && starts_word(text, at) {
                if let Some(name_end) = agent_name(text, at + 1, end) {
                    found.push(Reference {
                        start: at,
                        end: name_end,
                        kind: MENTION,
                        target: text[at + 1..name_end].to_owned(),
                    });
                    at = name_end;
                    continue;
                }
            }
            if starts_word(text, at) {
                if let Some((mention_end, number)) = keyword_mention(text, at, end) {
                    if let Some(repository) = home {
                        found.push(Reference {
                            start: at,
                            end: mention_end,
                            kind: ISSUE,
                            target: issue_url(repository.owner, repository.name, number),
                        });
                    }
                    at = mention_end;
                    continue;
                }
                if let Some((mention_end, owner, name, number)) = repository_mention(text, at, end)
                {
                    if host_known {
                        found.push(Reference {
                            start: at,
                            end: mention_end,
                            kind: REPOSITORY_ISSUE,
                            target: issue_url(owner, name, number),
                        });
                    }
                    at = mention_end;
                    continue;
                }
            }
            at += utf8_len(byte);
        }
    }

    /// Where the paragraph holding `at` ends: at the next blank line, or the
    /// end of the stretch.
    fn paragraph_end(&self, at: usize) -> usize {
        let next = self.breaks.partition_point(|&blank| blank <= at);
        self.breaks.get(next).copied().unwrap_or(self.end)
    }

    /// Past the code span a backtick run at `at` opens, or past the run alone
    /// when no run of the same length closes it in the same paragraph.
    fn code_span(&self, at: usize) -> usize {
        let bytes = self.text.as_bytes();
        let run = bytes[at..self.end]
            .iter()
            .take_while(|&&byte| byte == b'`')
            .count();
        let limit = self.paragraph_end(at + run);
        let mut look = at + run;
        while look < limit {
            if bytes[look] == b'`' {
                let close = bytes[look..limit]
                    .iter()
                    .take_while(|&&byte| byte == b'`')
                    .count();
                if close == run {
                    return look + close;
                }
                look += close;
            } else {
                look += 1;
            }
        }
        at + run
    }

    /// Past a Markdown link or image starting at its `[`, or past a link
    /// reference definition's line: a mention the agent already linked keeps
    /// the agent's link. The forms are an inline link `[label](destination)`,
    /// a full or collapsed reference `[label][ref]` or `[label][]`, a
    /// shortcut reference `[label]` whose label the text defines, and a
    /// definition `[label]: destination` at the start of a line.
    fn link(&self, at: usize) -> Option<usize> {
        // CommonMark caps a link label at 999 characters.
        const LABEL: usize = 1_000;
        const DESTINATION: usize = 2_048;
        let bytes = self.text.as_bytes();
        let close = *self.closers.get(&at)?;
        if close - at > LABEL {
            return None;
        }
        match bytes.get(close + 1).filter(|_| close + 1 < self.end) {
            Some(b'(') => {
                let after = *self.closers.get(&(close + 1))?;
                (after - close <= DESTINATION).then_some(after + 1)
            }
            Some(b'[') => {
                let after = *self.closers.get(&(close + 1))?;
                (after - close <= LABEL).then_some(after + 1)
            }
            Some(b':') if line_starts_at(self.text, at) => Some(
                self.text[close..self.end]
                    .find('\n')
                    .map_or(self.end, |offset| close + offset + 1),
            ),
            _ => self
                .defined
                .contains(&normalize(&self.text[at + 1..close]))
                .then_some(close + 1),
        }
    }

    /// Past an HTML tag or an autolink (`<https://…>`) on one line, starting
    /// at its `<`.
    fn tag(&mut self, at: usize) -> Option<usize> {
        let bytes = self.text.as_bytes();
        let next = *bytes.get(at + 1).filter(|_| at + 1 < self.end)?;
        if !(next.is_ascii_alphabetic() || matches!(next, b'/' | b'!' | b'?')) {
            return None;
        }
        let angle = next_of(&mut self.next_angle, bytes, at, self.end, b'>');
        let line = next_of(&mut self.next_line, bytes, at, self.end, b'\n');
        (angle < line).then_some(angle + 1)
    }
}

/// The first `byte` at or after `from` before `end`, or `end`, reusing the
/// last answer while it still lies ahead of `from`.
fn next_of(cache: &mut Option<usize>, bytes: &[u8], from: usize, end: usize, byte: u8) -> usize {
    if let Some(found) = cache.filter(|&found| found >= from) {
        return found;
    }
    let found = bytes[from..end]
        .iter()
        .position(|&candidate| candidate == byte)
        .map_or(end, |offset| from + offset);
    *cache = Some(found);
    found
}

/// Past a bare `http://` or `https://` URL, which runs to the next blank.
fn url(text: &str, at: usize, end: usize) -> Option<usize> {
    let rest = &text[at..end];
    let scheme = ["https://", "http://"]
        .into_iter()
        .find(|scheme| starts_with_ignore_case(rest, scheme))?;
    if !starts_word(text, at) {
        return None;
    }
    let length = rest[scheme.len()..]
        .find(char::is_whitespace)
        .map_or(rest.len(), |offset| scheme.len() + offset);
    Some(at + length)
}

/// "Issue N", "issue #N", "PR N", "PR #N" or "pull request N" at `at`: where
/// it ends, and N.
fn keyword_mention(text: &str, at: usize, end: usize) -> Option<(usize, u32)> {
    let rest = &text[at..end];
    let keyword = ["pull request", "issue", "pr"]
        .into_iter()
        .find(|keyword| starts_with_ignore_case(rest, keyword))?;
    let bytes = text.as_bytes();
    let mut look = at + keyword.len();
    let spaces = bytes[look..end]
        .iter()
        .take_while(|&&byte| byte == b' ')
        .count();
    if spaces == 0 {
        return None;
    }
    look += spaces;
    if bytes.get(look) == Some(&b'#') {
        look += 1;
    }
    number(text, look, end)
}

/// `owner/repo#N` at `at`: where it ends, the owner, the repository's name,
/// and N.
fn repository_mention(text: &str, at: usize, end: usize) -> Option<(usize, &str, &str, u32)> {
    let bytes = text.as_bytes();
    if at > 0 && matches!(bytes[at - 1], b'-' | b'.' | b'/' | b'@' | b':' | b'#') {
        return None;
    }
    let owner_end = at
        + bytes[at..end]
            .iter()
            .take_while(|&&byte| byte.is_ascii_alphanumeric() || byte == b'-')
            .count();
    if bytes.get(owner_end) != Some(&b'/') {
        return None;
    }
    let name_start = owner_end + 1;
    let name_end = name_start
        + bytes[name_start..end]
            .iter()
            .take_while(|&&byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            .count();
    if bytes.get(name_end) != Some(&b'#') {
        return None;
    }
    let (owner, name) = (&text[at..owner_end], &text[name_start..name_end]);
    if !is_owner(owner) || !is_name(name) {
        return None;
    }
    let (mention_end, issue) = number(text, name_end + 1, end)?;
    Some((mention_end, owner, name, issue))
}

/// An issue number at `at`: one to nine digits, the first not zero, ending
/// where no letter, digit or `_` follows and no `.` or `,` with a digit after
/// it (so "PR 3.5" and "Issue 1,000" are not mentions).
fn number(text: &str, at: usize, end: usize) -> Option<(usize, u32)> {
    let bytes = text.as_bytes();
    let digits = bytes[at..end]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || digits > 9 || bytes[at] == b'0' {
        return None;
    }
    let after = at + digits;
    let mut rest = text[after..end].chars();
    match rest.next() {
        Some(next) if next.is_alphanumeric() || next == '_' => return None,
        Some('.' | ',') if rest.next().is_some_and(|digit| digit.is_ascii_digit()) => return None,
        _ => {}
    }
    let value = bytes[at..after]
        .iter()
        .fold(0u32, |value, byte| value * 10 + u32::from(byte - b'0'));
    Some((after, value))
}

/// Where the name after an `@` at `at - 1` ends: the longest run of letters,
/// digits (any script), `_` and `-`. None when the run is empty. The name is
/// returned as typed: whether it names an agent is for the app to look up.
fn agent_name(text: &str, at: usize, end: usize) -> Option<usize> {
    let length: usize = text[at..end]
        .chars()
        .take_while(|&c| c.is_alphanumeric() || c == '_' || c == '-')
        .map(char::len_utf8)
        .sum();
    (length > 0).then_some(at + length)
}

/// Whether a mention may start at `at`: at the start of the text, or after a
/// character that is not a letter, digit or `_`.
fn starts_word(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|before| !(before.is_alphanumeric() || before == '_'))
}

fn issue_url(owner: &str, name: &str, number: u32) -> String {
    // GitHub sends an issue URL on to the pull request when N is one, so one
    // form serves both.
    format!("https://github.com/{owner}/{name}/issues/{number}")
}

/// A GitHub owner: letters, digits and `-`, at most 39, not starting with `-`.
fn is_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// A GitHub repository name: letters, digits, `-`, `_` and `.`, at most 100,
/// and not `.` or `..`.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// The length of the UTF-8 character starting with `byte`.
fn utf8_len(byte: u8) -> usize {
    match byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

// Runs the README's example as a doctest, so the documented use can't drift.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExample;

#[cfg(test)]
mod tests;
