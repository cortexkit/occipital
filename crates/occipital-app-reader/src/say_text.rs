//! The text of a `say` call while the model is still writing it.
//!
//! The runner streams a call's arguments as chunks of JSON text, in the order
//! the model writes them, split anywhere: inside a key, inside an escape, or
//! between the two halves of a `\u` surrogate pair. `say` puts its message in
//! the top-level `text` string, and the apps draw that string live as it
//! grows, the way they draw a reply's words. So this reads the chunks as they
//! come and gives back only the newly decoded characters of `text`, ignoring
//! every other member whatever its place, nested values included.
//!
//! It reads one call from the start of its arguments. Input that isn't a JSON
//! object stops it for good, and it gives back nothing more: the stored
//! message, parsed whole, then replaces whatever was drawn.

/// The member whose string is the message.
const TEXT: &str = "text";

/// A key longer than this can't be `text`, so its characters aren't kept.
const KEY_KEPT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Before the opening `{`.
    Start,
    /// Inside the object, before a key or its closing `}`.
    BeforeKey,
    /// Reading a key; `kept` is its decoded start, up to [`KEY_KEPT`] characters.
    Key { kept: String, escape: Escape },
    /// After a key, before its `:`.
    Colon { key_is_text: bool },
    /// After the `:`, before the value's first character.
    BeforeValue { key_is_text: bool },
    /// Inside the `text` string: its characters are given back.
    Text { escape: Escape },
    /// Inside some other value, skipped: `depth` counts open brackets, and
    /// `in_string` tells whether the skip is inside a string.
    Skip {
        depth: usize,
        in_string: bool,
        escaped: bool,
    },
    /// After a value, before `,` or `}`.
    AfterValue,
    /// The object closed, or the input wasn't one: nothing more is read.
    Done,
}

/// Where a string is in an escape sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Escape {
    None,
    /// Right after a backslash.
    Backslash,
    /// Reading the hex digits of a `\u` escape.
    Unicode(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SayText {
    state: State,
    /// Whether a `text` string was already read, so a repeated member is skipped.
    text_seen: bool,
    /// The first half of a `\u` surrogate pair, waiting for its second half.
    high_surrogate: Option<u16>,
}

impl Default for SayText {
    fn default() -> Self {
        Self::new()
    }
}

impl SayText {
    pub fn new() -> Self {
        Self {
            state: State::Start,
            text_seen: false,
            high_surrogate: None,
        }
    }

    /// Read the next chunk of the arguments, and give back the characters of
    /// `text` it completes.
    pub fn feed(&mut self, chunk: &str) -> String {
        let mut out = String::new();
        for c in chunk.chars() {
            self.step(c, &mut out);
            if self.state == State::Done {
                break;
            }
        }
        out
    }

    fn step(&mut self, c: char, out: &mut String) {
        let state = std::mem::replace(&mut self.state, State::Done);
        self.state = match state {
            State::Start => match c {
                c if c.is_whitespace() => State::Start,
                '{' => State::BeforeKey,
                _ => State::Done,
            },
            State::BeforeKey => match c {
                c if c.is_whitespace() => State::BeforeKey,
                '"' => State::Key {
                    kept: String::new(),
                    escape: Escape::None,
                },
                _ => State::Done,
            },
            State::Key { mut kept, escape } => match escape {
                Escape::None => match c {
                    '"' => State::Colon {
                        key_is_text: kept == TEXT,
                    },
                    '\\' => State::Key {
                        kept,
                        escape: Escape::Backslash,
                    },
                    c => {
                        keep(&mut kept, c);
                        State::Key {
                            kept,
                            escape: Escape::None,
                        }
                    }
                },
                Escape::Backslash => match simple_escape(c) {
                    Some(decoded) => {
                        keep(&mut kept, decoded);
                        State::Key {
                            kept,
                            escape: Escape::None,
                        }
                    }
                    None if c == 'u' => State::Key {
                        kept,
                        escape: Escape::Unicode(String::new()),
                    },
                    None => State::Done,
                },
                // A key's `\u` escape is read but not decoded, so a key that
                // spells `text` with one isn't taken for it. That only costs
                // the live preview: the message still shows once its call is
                // stored, from the arguments parsed whole.
                Escape::Unicode(mut digits) => {
                    if !c.is_ascii_hexdigit() {
                        State::Done
                    } else {
                        digits.push(c);
                        if digits.len() == 4 {
                            keep(&mut kept, '\u{FFFD}');
                            State::Key {
                                kept,
                                escape: Escape::None,
                            }
                        } else {
                            State::Key {
                                kept,
                                escape: Escape::Unicode(digits),
                            }
                        }
                    }
                }
            },
            State::Colon { key_is_text } => match c {
                c if c.is_whitespace() => State::Colon { key_is_text },
                ':' => State::BeforeValue { key_is_text },
                _ => State::Done,
            },
            State::BeforeValue { key_is_text } => match c {
                c if c.is_whitespace() => State::BeforeValue { key_is_text },
                '"' if key_is_text && !self.text_seen => {
                    self.text_seen = true;
                    State::Text {
                        escape: Escape::None,
                    }
                }
                '"' => State::Skip {
                    depth: 0,
                    in_string: true,
                    escaped: false,
                },
                '{' | '[' => State::Skip {
                    depth: 1,
                    in_string: false,
                    escaped: false,
                },
                ',' | '}' | ']' | ':' => State::Done,
                // A number, `true`, `false` or `null`: skipped up to the next
                // `,` or `}`.
                _ => State::Skip {
                    depth: 0,
                    in_string: false,
                    escaped: false,
                },
            },
            State::Text { escape } => self.text_char(c, escape, out),
            State::Skip {
                depth,
                in_string,
                escaped,
            } => skip(c, depth, in_string, escaped),
            State::AfterValue => after_value(c),
            State::Done => State::Done,
        };
    }

    fn text_char(&mut self, c: char, escape: Escape, out: &mut String) -> State {
        let text = |escape| State::Text { escape };
        match escape {
            Escape::None => match c {
                '"' => {
                    self.flush_surrogate(out);
                    State::AfterValue
                }
                '\\' => text(Escape::Backslash),
                c => {
                    self.flush_surrogate(out);
                    out.push(c);
                    text(Escape::None)
                }
            },
            Escape::Backslash => match simple_escape(c) {
                Some(decoded) => {
                    self.flush_surrogate(out);
                    out.push(decoded);
                    text(Escape::None)
                }
                None if c == 'u' => text(Escape::Unicode(String::new())),
                None => State::Done,
            },
            Escape::Unicode(mut digits) => {
                if !c.is_ascii_hexdigit() {
                    return State::Done;
                }
                digits.push(c);
                if digits.len() < 4 {
                    return text(Escape::Unicode(digits));
                }
                let Ok(unit) = u16::from_str_radix(&digits, 16) else {
                    return State::Done;
                };
                self.unit(unit, out);
                text(Escape::None)
            }
        }
    }

    /// One UTF-16 unit from a `\u` escape, joined with a waiting first half
    /// when it completes a surrogate pair. A half with no partner reads as
    /// U+FFFD.
    fn unit(&mut self, unit: u16, out: &mut String) {
        match (self.high_surrogate.take(), unit) {
            (Some(high), 0xDC00..=0xDFFF) => {
                let joined =
                    0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00);
                out.push(char::from_u32(joined).unwrap_or('\u{FFFD}'));
            }
            (waiting, 0xD800..=0xDBFF) => {
                if waiting.is_some() {
                    out.push('\u{FFFD}');
                }
                self.high_surrogate = Some(unit);
            }
            (waiting, unit) => {
                if waiting.is_some() {
                    out.push('\u{FFFD}');
                }
                out.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'));
            }
        }
    }

    fn flush_surrogate(&mut self, out: &mut String) {
        if self.high_surrogate.take().is_some() {
            out.push('\u{FFFD}');
        }
    }
}

/// The start of a key, kept only as far as it could still be `text`.
fn keep(kept: &mut String, c: char) {
    if kept.chars().count() < KEY_KEPT {
        kept.push(c);
    }
}

/// What a one-character escape stands for: `\"`, `\\`, `\/` and the control
/// escapes.
fn simple_escape(c: char) -> Option<char> {
    Some(match c {
        '"' => '"',
        '\\' => '\\',
        '/' => '/',
        'b' => '\u{8}',
        'f' => '\u{c}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        _ => return None,
    })
}

fn skip(c: char, depth: usize, in_string: bool, escaped: bool) -> State {
    let next = |depth, in_string, escaped| State::Skip {
        depth,
        in_string,
        escaped,
    };
    if in_string {
        return match (escaped, c) {
            (true, _) => next(depth, true, false),
            (false, '\\') => next(depth, true, true),
            (false, '"') if depth == 0 => State::AfterValue,
            (false, '"') => next(depth, false, false),
            _ => next(depth, true, false),
        };
    }
    match c {
        '"' => next(depth, true, false),
        '{' | '[' => next(depth + 1, false, false),
        '}' | ']' if depth > 1 => next(depth - 1, false, false),
        '}' | ']' if depth == 1 => State::AfterValue,
        // A scalar ends at the member's `,` or the object's `}`.
        ',' if depth == 0 => State::BeforeKey,
        '}' if depth == 0 => State::Done,
        _ => next(depth, false, false),
    }
}

fn after_value(c: char) -> State {
    match c {
        c if c.is_whitespace() => State::AfterValue,
        ',' => State::BeforeKey,
        _ => State::Done,
    }
}

#[cfg(test)]
mod tests;
