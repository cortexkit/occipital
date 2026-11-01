//! The single address parser/serializer used by `Open` and address props (§3.11).

use serde::Serialize;

use crate::{DiagnosticCode, DisplayText};

mod idna;
mod normalize;
mod punycode;
mod tables {
    pub(super) mod bidi;
    pub(super) mod joining;
    pub(super) mod mapping;
    pub(super) mod nfc;
}

const MAX_SCALARS: usize = 2_048;

/// A validated, canonical HTTP(S) address (§3.11).
///
/// Open only `as_str()`, never the original input or the sheet representation.
/// Validation does not authorize a fetch or opening: network policy, the full
/// app-owned address sheet and the person's second tap remain host duties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Address(String);

impl Address {
    /// Parse an absolute HTTP(S) URL using pinned Unicode 16 IDNA data.
    ///
    /// All refusals return `E_SCHEME`. Both input and canonical output must fit
    /// 2,048 scalars. Local/numeric hosts and browser-reinterpreted paths are
    /// refused so the canonical address is also a WHATWG serialization fixed point.
    pub fn parse(input: &str) -> Result<Self, DiagnosticCode> {
        parse(input).map(Self).ok_or(DiagnosticCode::Scheme)
    }

    /// The exact URL to open after confirmation, with no platform re-parsing of input.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The complete escaped sheet representation; never use it as an opened URL.
    pub fn display(&self) -> DisplayText {
        DisplayText::address_sheet(&self.0)
    }
}

fn parse(input: &str) -> Option<String> {
    if input.chars().take(MAX_SCALARS + 1).count() > MAX_SCALARS
        || input
            .chars()
            .any(|c| matches!(c, '\u{0}'..=' ' | '\u{7F}'..='\u{9F}' | '\\'))
        || !valid_escapes(input)
    {
        return None;
    }
    let (scheme, rest) = input.split_once("://")?;
    let (scheme, default_port) = if scheme.eq_ignore_ascii_case("http") {
        ("http", 80)
    } else if scheme.eq_ignore_ascii_case("https") {
        ("https", 443)
    } else {
        return None;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, rest) = rest.split_at(end);
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let host = idna::to_ascii(host)?;
    if forbidden_host(&host) {
        return None;
    }
    let port = match port {
        Some(port) => {
            if port.starts_with('0') || !port.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let port: u16 = port.parse().ok()?;
            (port != default_port).then_some(port)
        }
        None => None,
    };
    let (rest, fragment) = match rest.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (rest, None),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let path = if path.is_empty() { "/" } else { path };
    if path.split('/').any(dot_segment) {
        return None;
    }
    let mut output = format!("{scheme}://{host}");
    if let Some(port) = port {
        output.push_str(&format!(":{port}"));
    }
    encode(path, Part::Path, &mut output);
    if let Some(query) = query {
        output.push('?');
        encode(query, Part::Query, &mut output);
    }
    if let Some(fragment) = fragment {
        output.push('#');
        encode(fragment, Part::Fragment, &mut output);
    }
    // Serialization is entirely ASCII, so its byte length is its scalar count.
    (output.len() <= MAX_SCALARS).then_some(output)
}

fn valid_escapes(input: &str) -> bool {
    let mut bytes = input.bytes();
    while let Some(c) = bytes.next() {
        if c == b'%'
            && !(bytes.next().is_some_and(|c| c.is_ascii_hexdigit())
                && bytes.next().is_some_and(|c| c.is_ascii_hexdigit()))
        {
            return false;
        }
    }
    true
}

fn forbidden_host(host: &str) -> bool {
    let Some((_, last)) = host.rsplit_once('.') else {
        return true;
    };
    // WHATWG treats even a bare "0x" as numeric zero. Refuse it too: a numeric
    // final label can turn an apparently named host into an IPv4 address.
    last.is_empty()
        || last.bytes().all(|c| c.is_ascii_digit())
        || last
            .strip_prefix("0x")
            .is_some_and(|s| s.bytes().all(|c| c.is_ascii_hexdigit()))
        || host == "localhost"
        || host == "home.arpa"
        || [".localhost", ".local", ".internal", ".home.arpa"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

fn dot_segment(segment: &str) -> bool {
    let segment = segment.replace("%2e", ".").replace("%2E", ".");
    segment == "." || segment == ".."
}

#[derive(Clone, Copy)]
enum Part {
    Path,
    Query,
    Fragment,
}

fn encode(input: &str, part: Part, output: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut bytes = input.bytes();
    while let Some(c) = bytes.next() {
        if c == b'%' {
            output.push('%');
            // The whole input was checked before any component was split,
            // so two hex digits follow every '%'.
            for digit in bytes.by_ref().take(2) {
                output.push(digit.to_ascii_uppercase() as char);
            }
        } else if (c.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&c)
            || (!matches!(part, Part::Path) && c == b'?'))
            // HTTP(S) is a WHATWG special scheme, whose query set also escapes '.
            && !(matches!(part, Part::Query) && c == b'\'')
        {
            output.push(c as char);
        } else {
            output.push('%');
            output.push(HEX[(c >> 4) as usize] as char);
            output.push(HEX[(c & 15) as usize] as char);
        }
    }
}

#[cfg(test)]
mod tests;
