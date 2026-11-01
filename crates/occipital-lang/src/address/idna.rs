//! UTS #46 rev. 33 ToASCII, nontransitional, with all validation flags enabled.

use super::{normalize, punycode, tables};

fn mapping(c: char) -> (u8, &'static str) {
    let cp = c as u32;
    let row = &tables::mapping::ROWS[tables::mapping::ROWS.partition_point(|row| row.1 < cp)];
    (row.2, row.3)
}

fn property(c: char, ranges: &[(u32, u32, &'static str)], default: &'static str) -> &'static str {
    let cp = c as u32;
    let i = ranges.partition_point(|row| row.1 < cp);
    ranges
        .get(i)
        .filter(|row| row.0 <= cp)
        .map_or(default, |row| row.2)
}

fn bidi(c: char) -> &'static str {
    property(c, tables::bidi::RANGES, "L")
}

fn joining(c: char) -> &'static str {
    property(c, tables::joining::RANGES, "U")
}

fn mark(c: char) -> bool {
    let cp = c as u32;
    let i = tables::nfc::MARKS.partition_point(|row| row.1 < cp);
    tables::nfc::MARKS.get(i).is_some_and(|row| row.0 <= cp)
}

pub(super) fn to_ascii(input: &str) -> Option<String> {
    let mut mapped = String::new();
    for c in input.chars() {
        match mapping(c) {
            (1, value) => mapped.push_str(value),
            (2, _) => (),
            _ => mapped.push(c),
        }
    }
    let normalized = normalize::nfc(&mapped);
    let mut labels = Vec::new();
    for label in normalized.split('.') {
        let label = if let Some(suffix) = label.strip_prefix("xn--") {
            let decoded = punycode::decode(suffix)?;
            // A-labels are validated as decoded, not mapped or repaired by NFC.
            if decoded.is_ascii() || normalize::nfc(&decoded) != decoded {
                return None;
            }
            decoded
        } else {
            label.to_owned()
        };
        if !valid_label(&label) {
            return None;
        }
        labels.push(label);
    }
    // A right-to-left character in any decoded label triggers RFC 5893 for
    // every label, including otherwise innocuous ASCII-only labels.
    let bidi_domain = labels
        .iter()
        .flat_map(|s| s.chars())
        .any(|c| matches!(bidi(c), "R" | "AL" | "AN"));
    let mut output = String::new();
    for label in labels {
        if bidi_domain && !valid_bidi(&label) {
            return None;
        }
        let ascii = if label.is_ascii() {
            label
        } else {
            format!("xn--{}", punycode::encode(&label)?)
        };
        if ascii.is_empty() || ascii.len() > 63 {
            return None;
        }
        if !output.is_empty() {
            output.push('.');
        }
        output.push_str(&ascii);
    }
    (output.len() <= 253).then_some(output)
}

fn valid_label(label: &str) -> bool {
    let chars: Vec<char> = label.chars().collect();
    let Some(&first) = chars.first() else {
        return false;
    };
    if first == '-'
        || chars.last() == Some(&'-')
        || (chars.get(2) == Some(&'-') && chars.get(3) == Some(&'-'))
        || mark(first)
    {
        return false;
    }
    for &c in &chars {
        if c == '.'
            || !matches!(mapping(c).0, 0 | 3)
            || (c.is_ascii() && !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        {
            return false;
        }
    }
    valid_joiners(&chars)
}

fn valid_joiners(chars: &[char]) -> bool {
    for (i, &c) in chars.iter().enumerate() {
        if !matches!(c, '\u{200C}' | '\u{200D}') {
            continue;
        }
        if i > 0 && normalize::combining_class(chars[i - 1]) == 9 {
            continue;
        }
        if c == '\u{200D}' {
            return false;
        }
        let before = chars[..i]
            .iter()
            .rev()
            .map(|&c| joining(c))
            .find(|&ty| ty != "T");
        let after = chars[i + 1..]
            .iter()
            .map(|&c| joining(c))
            .find(|&ty| ty != "T");
        if !matches!(before, Some("L" | "D")) || !matches!(after, Some("R" | "D")) {
            return false;
        }
    }
    true
}

fn valid_bidi(label: &str) -> bool {
    let classes: Vec<_> = label.chars().map(bidi).collect();
    let rtl = match classes[0] {
        "R" | "AL" => true,
        "L" => false,
        _ => return false,
    };
    let mut en = false;
    let mut an = false;
    for &class in &classes {
        let allowed = if rtl {
            matches!(
                class,
                "R" | "AL" | "AN" | "EN" | "ES" | "CS" | "ET" | "ON" | "BN" | "NSM"
            )
        } else {
            matches!(class, "L" | "EN" | "ES" | "CS" | "ET" | "ON" | "BN" | "NSM")
        };
        if !allowed {
            return false;
        }
        en |= class == "EN";
        an |= class == "AN";
    }
    let last = classes.iter().rev().find(|&&class| class != "NSM").copied();
    if rtl {
        !(en && an) && matches!(last, Some("R" | "AL" | "EN" | "AN"))
    } else {
        matches!(last, Some("L" | "EN"))
    }
}
