//! One-shot syntax preparation. The pinned syntax crate owns all grammar work.

pub(crate) mod resolve;
pub(crate) mod types;

use cortexkit_rich_ui_syntax::{parse_bytes, Document, Mode, Node, Span};

use crate::{Diagnostic, DiagnosticCode, DiagnosticPhase, SourceSpan};

pub(crate) fn prepare(
    source: &[u8],
    mode: Mode,
    context: &types::TypeContext,
) -> Result<resolve::Resolved, crate::accounts::Refused> {
    Ok(resolve::resolve(parse(source, mode)?, context))
}

/// Keeps the syntax tree, including opaque failed constructs and reference nodes,
/// intact for later catalog/profile checks. A fallback is never executable.
pub(crate) struct Parsed {
    pub document: Document,
    pub diagnostics: Vec<Diagnostic>,
    pub positions: Positions,
    /// Source bytes before UTF-8 repair, for the conservative two-per-byte tariff.
    pub accounted_bytes: usize,
    source: String,
}

impl Parsed {
    /// A newer syntax kind is inert source in replies and a refusal in programs.
    /// Keeping the repaired source avoids interpreting an unknown node's children.
    pub(crate) fn unsupported(&self, node: &Node) -> Result<&str, DiagnosticCode> {
        if self.document.mode == Mode::Reply {
            Ok(self
                .source
                .get(node.span.start..node.span.end)
                .unwrap_or(""))
        } else {
            Err(DiagnosticCode::UnsupportedVersion)
        }
    }
}

fn unsupported_mode<T>() -> Result<T, crate::accounts::Refused> {
    Err(crate::accounts::Refused::UnsupportedMode)
}

fn mode_spec(mode: Mode) -> Result<(crate::AccountScope, usize), crate::accounts::Refused> {
    match mode {
        Mode::Program => Ok((
            crate::AccountScope::Interface,
            cortexkit_rich_ui_syntax::PROGRAM_BYTES,
        )),
        Mode::Reply => Ok((
            crate::AccountScope::Reply,
            cortexkit_rich_ui_syntax::REPLY_BYTES,
        )),
        _ => unsupported_mode(),
    }
}

pub(crate) fn parse(source: &[u8], mode: Mode) -> Result<Parsed, crate::accounts::Refused> {
    let (scope, _) = mode_spec(mode)?;
    let accounts =
        crate::accounts::Accounts::isolated(scope).map_err(crate::accounts::Refused::Scope)?;
    parse_accounted(source, mode, &accounts)
}

pub(crate) fn parse_accounted(
    source: &[u8],
    mode: Mode,
    accounts: &crate::accounts::Accounts,
) -> Result<Parsed, crate::accounts::Refused> {
    let (_, cap) = mode_spec(mode)?;
    with_parse_account(accounts, bounded_source_bytes(source, cap), || {
        parse_reserved(source, mode)
    })
}

/// The parser caps raw input before repair. Expanded U+FFFD bytes are not
/// additional received source bytes and must not consume the final reservation.
fn bounded_source_bytes(source: &[u8], cap: usize) -> usize {
    source_prefix(source, cap).len()
}

/// Mirror only the dependency's UTF-8 boundary cut, not any grammar rule.
/// A valid scalar crossing the raw cap is omitted rather than repaired.
pub(crate) fn source_prefix(source: &[u8], cap: usize) -> &[u8] {
    let end = source.len().min(cap);
    let mut start = end;
    while start > end.saturating_sub(3) && source.get(start).is_some_and(|byte| byte & 0xc0 == 0x80)
    {
        start -= 1;
    }
    if start < end {
        let length = match source[start] {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 0,
        };
        if start + length > end
            && source
                .get(start..start + length)
                .is_some_and(|s| std::str::from_utf8(s).is_ok())
        {
            return &source[..start];
        }
    }
    &source[..end]
}

fn with_parse_account(
    accounts: &crate::accounts::Accounts,
    prefix_bytes: usize,
    parse: impl FnOnce() -> Result<Parsed, crate::accounts::Refused>,
) -> Result<Parsed, crate::accounts::Refused> {
    // Two fuel per bounded input byte covers both bytes and tokens (§9.2).
    let reservation = accounts.reserve_parse(prefix_bytes)?;
    let parsed = parse()?;
    reservation.settle(parsed.accounted_bytes)?;
    Ok(parsed)
}

fn parse_reserved(source: &[u8], mode: Mode) -> Result<Parsed, crate::accounts::Refused> {
    mode_spec(mode)?;
    let document = parse_bytes(source, mode);
    from_document(source, mode, document)
}

/// Reuse the authoritative streaming parse without parsing its source again.
pub(crate) fn from_document(
    source: &[u8],
    mode: Mode,
    document: Document,
) -> Result<Parsed, crate::accounts::Refused> {
    let (_, cap) = mode_spec(mode)?;
    mode_spec(document.mode)?;
    // Match parse_bytes' raw-byte prefix only to translate its repaired offsets. This
    // is not a second parser or a scan for references (language spec §3.1, §3.9).
    let repaired = String::from_utf8_lossy(source_prefix(source, cap));
    let positions = Positions::new(&repaired);
    let diagnostics = document
        .diagnostics
        .iter()
        .filter_map(|problem| {
            // The syntax crate also has private safety-limit codes not present
            // in the language spec's diagnostic table (§12.4). Preserve those
            // in Document, without inventing a mapping.
            let code = DiagnosticCode::ALL
                .iter()
                .copied()
                .find(|code| code.as_str() == problem.code.as_str())?;
            let phase = match code {
                DiagnosticCode::DuplicateName
                | DiagnosticCode::ReservedName
                | DiagnosticCode::LiteralRequired
                | DiagnosticCode::ReferenceLimit
                | DiagnosticCode::ReplyForbidden
                | DiagnosticCode::LimitTags
                | DiagnosticCode::Encoding => DiagnosticPhase::Check,
                _ => DiagnosticPhase::Parse,
            };
            Some(positioned(code, phase, problem.span, &positions))
        })
        .collect();
    Ok(Parsed {
        document,
        diagnostics,
        positions,
        accounted_bytes: bounded_source_bytes(source, cap),
        source: repaired.into_owned(),
    })
}

pub(crate) fn positioned(
    code: DiagnosticCode,
    phase: DiagnosticPhase,
    span: Span,
    positions: &Positions,
) -> Diagnostic {
    // Positions are one-based by construction; a malformed span from the
    // parser would leave the diagnostic unpositioned rather than fail.
    Diagnostic::new(code, phase).refine(|diagnostic| diagnostic.with_span(positions.span(span)))
}

pub(crate) struct Positions(Vec<(usize, u32, u32)>);

impl Positions {
    fn new(source: &str) -> Self {
        let (mut line, mut column, mut cr) = (1, 1, false);
        let mut positions = Vec::new();
        for (offset, scalar) in source.char_indices() {
            positions.push((offset, line, column));
            match scalar {
                '\u{feff}' if offset == 0 => {}
                '\r' => {
                    line += 1;
                    column = 1;
                }
                '\n' if cr => {}
                '\n' => {
                    line += 1;
                    column = 1;
                }
                _ => column += 1,
            }
            cr = scalar == '\r';
        }
        positions.push((source.len(), line, column));
        Self(positions)
    }

    fn at(&self, offset: usize) -> (u32, u32) {
        let index = self.0.partition_point(|position| position.0 <= offset);
        let (_, line, column) = self.0[index.saturating_sub(1)];
        (line, column)
    }

    pub fn span(&self, span: Span) -> SourceSpan {
        let (line, column) = self.at(span.start);
        let (end_line, end_column) = self.at(span.end);
        SourceSpan {
            line,
            column,
            end_line,
            end_column,
        }
    }
}

#[cfg(test)]
mod tests;
