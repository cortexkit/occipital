use super::{Cursor, FallbackText, Output, PROGRAM_BYTES, PROGRAM_SCALARS};

pub(super) fn scan(source: &[u8], title: &str, summary: &str) -> FallbackText {
    let mut scanner = Scanner {
        input: Cursor::new(source, PROGRAM_BYTES),
        output: Output::new(PROGRAM_SCALARS, true),
        prefix: Some(0),
    };
    for header in [title, summary] {
        for ch in header.chars().take(512) {
            scanner.output.push(ch);
        }
        scanner.output.push('\n');
    }
    if scanner.input.starts(b"\xef\xbb\xbf") {
        scanner.input.pos += 3;
    }
    scanner.body();
    scanner.output.finish(&scanner.input, false)
}

struct Scanner<'a> {
    input: Cursor<'a>,
    output: Output,
    /// Spaces since the physical line start, or None after other content.
    prefix: Option<usize>,
}

impl Scanner<'_> {
    fn take(&mut self) -> Option<char> {
        let ch = self.input.source_char()?;
        self.prefix = match (ch, self.prefix) {
            ('\n', _) => Some(0),
            (' ', Some(n)) => Some(n + 1),
            _ => None,
        };
        Some(ch)
    }

    fn body(&mut self) {
        let mut fence = None;
        let mut closing = false;
        while !self.output.full() && self.input.peek().is_some() {
            let byte = self.input.peek().unwrap_or_default();
            if self.prefix.is_some_and(|n| n <= 3) && matches!(byte, b'`' | b'~') {
                let mut run = 0;
                while self.input.peek() == Some(byte) && !self.output.full() {
                    self.take();
                    self.output.push(char::from(byte));
                    run += 1;
                }
                match fence {
                    None if run >= 3 => fence = Some((byte, run)),
                    Some((mark, count)) if mark == byte && run >= count => closing = true,
                    _ => {}
                }
                continue;
            }
            if fence.is_some() {
                let Some(ch) = self.take() else { break };
                self.output.push(ch);
                if ch == '\n' {
                    if closing {
                        fence = None;
                    }
                    closing = false;
                } else if !matches!(ch, ' ' | '\t') {
                    closing = false;
                }
                continue;
            }
            if self.prefix == Some(0) && self.input.starts(b"export ") {
                self.declaration();
            } else if self.tag_start() {
                self.tag();
            } else if byte == b'{' {
                self.output.push('…');
                if !self.output.full() {
                    self.braced();
                }
            } else if self.input.starts(b"$$tc(") {
                self.reference();
            } else if let Some(ch) = self.take() {
                self.output.push(ch);
            } else {
                break;
            }
        }
    }

    fn declaration(&mut self) {
        let mut brackets = Brackets::default();
        loop {
            let mut last = None;
            let mut blank = true;
            while let Some(ch) = self.take() {
                if ch == '\n' {
                    break;
                }
                if !matches!(ch, ' ' | '\t') {
                    blank = false;
                    last = Some(ch);
                }
                brackets.push(ch);
            }
            if blank || self.input.peek().is_none() {
                return;
            }
            let can_end = brackets.depth == 0
                && brackets.quote.is_none()
                && !last.is_some_and(|ch| "+-*/%=<>&|?:,!".contains(ch));
            let mut spaces = 0;
            while self.input.peek() == Some(b' ') {
                self.take();
                spaces += 1;
            }
            if can_end && !matches!(self.input.peek(), Some(b'?' | b':' | b'&' | b'|' | b'.')) {
                // The indentation was consumed to decide continuation. Preserve
                // it without scanning those bytes again when body text resumes.
                for _ in 0..spaces {
                    self.output.push(' ');
                    if self.output.full() {
                        break;
                    }
                }
                return;
            }
        }
    }

    fn tag_start(&self) -> bool {
        let rest = self.input.rest();
        rest.first() == Some(&b'<')
            && rest
                .get(if rest.get(1) == Some(&b'/') { 2 } else { 1 })
                .is_some_and(u8::is_ascii_uppercase)
    }

    fn tag(&mut self) {
        self.take(); // '<'
        let closing = self.input.peek() == Some(b'/');
        if closing {
            self.take();
        }
        self.identifier(); // tag name
        while !self.output.full() {
            match self.input.peek() {
                None => break,
                Some(b'>') => {
                    self.take();
                    break;
                }
                Some(b'{') => self.braced(),
                Some(b'\'' | b'"') => self.quoted(false),
                Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => {
                    let start = self.input.pos;
                    self.identifier();
                    // The cursor only moves forward over read bytes, so the
                    // range is always in bounds; a miss would keep nothing.
                    let keep = !closing
                        && matches!(
                            self.input.bytes.get(start..self.input.pos),
                            Some(
                                b"title"
                                    | b"subtitle"
                                    | b"label"
                                    | b"text"
                                    | b"description"
                                    | b"caption"
                                    | b"alt"
                            )
                        );
                    self.spaces();
                    if self.input.peek() == Some(b'=') {
                        self.take();
                        self.spaces();
                        match self.input.peek() {
                            Some(b'\'' | b'"') => self.quoted(keep),
                            Some(b'{') => self.braced(),
                            _ => {}
                        }
                    }
                }
                _ => {
                    self.take();
                }
            }
        }
    }

    fn identifier(&mut self) {
        while self
            .input
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.take();
        }
    }

    fn spaces(&mut self) {
        while matches!(self.input.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.take();
        }
    }

    fn quoted(&mut self, keep: bool) {
        let quote = self.take();
        if keep {
            self.output.line_start();
        }
        while !self.output.full() {
            let Some(mut ch) = self.take() else { break };
            if Some(ch) == quote {
                break;
            }
            if ch == '\\' {
                ch = match self.take() {
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    Some('u') if self.input.peek() == Some(b'{') => self.unicode_escape(),
                    Some(ch) => ch,
                    None => break,
                };
            }
            if keep {
                self.output.push(ch);
            }
        }
        if keep {
            self.output.push('\n');
        }
    }

    fn unicode_escape(&mut self) -> char {
        self.take();
        let mut value = 0_u32;
        let mut digits = 0;
        while let Some(byte) = self.input.peek() {
            let Some(hex) = char::from(byte).to_digit(16) else {
                break;
            };
            self.take();
            digits += 1;
            value = value.saturating_mul(16).saturating_add(hex);
        }
        if self.input.peek() == Some(b'}') {
            self.take();
            if (1..=6).contains(&digits) {
                return char::from_u32(value).unwrap_or('\u{fffd}');
            }
        }
        '\u{fffd}'
    }

    fn braced(&mut self) {
        let mut brackets = Brackets::default();
        while let Some(ch) = self.take() {
            brackets.push(ch);
            if brackets.depth == 0 && brackets.quote.is_none() {
                break;
            }
        }
    }

    fn reference(&mut self) {
        // The token's digits need no numeric conversion. Retain only offsets,
        // so even an invalid, extremely long candidate uses constant memory.
        let start = self.input.pos;
        for _ in 0..5 {
            self.take();
        }
        let digits = self.input.pos;
        let zero = self.input.peek() == Some(b'0');
        while self.input.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.take();
        }
        let count = self.input.pos - digits;
        if count > 0 && (!zero || count == 1) && self.input.peek() == Some(b')') {
            self.take();
            self.output.write("[tool output]");
        } else {
            // The consumed prefix is ASCII. Copy it without rescanning grammar.
            for i in start..self.input.pos {
                self.output.push(char::from(self.input.bytes[i]));
                if self.output.full() {
                    self.output.cut |= i + 1 < self.input.pos;
                    break;
                }
            }
        }
    }
}

#[derive(Default)]
struct Brackets {
    depth: u8,
    quote: Option<char>,
    escaped: bool,
}

impl Brackets {
    fn push(&mut self, ch: char) {
        if let Some(quote) = self.quote {
            if self.escaped {
                self.escaped = false;
            } else if ch == '\\' {
                self.escaped = true;
            } else if ch == quote {
                self.quote = None;
            }
        } else {
            match ch {
                '\'' | '"' => self.quote = Some(ch),
                '(' | '[' | '{' => self.depth = (self.depth + 1).min(64),
                ')' | ']' | '}' => self.depth = self.depth.saturating_sub(1),
                _ => {}
            }
        }
    }
}
