//! Canonical decomposition, ordering and composition (UAX #15, Unicode 16).

use super::tables::nfc::{CCC, COMPOSITION, DECOMPOSITION};

pub(super) fn combining_class(c: char) -> u8 {
    let cp = c as u32;
    let i = CCC.partition_point(|row| row.1 < cp);
    CCC.get(i).filter(|row| row.0 <= cp).map_or(0, |row| row.2)
}

fn decompose(c: char, output: &mut Vec<char>) {
    let cp = c as u32;
    if (0xAC00..0xD7A4).contains(&cp) {
        // Hangul syllables have algorithmic, rather than tabulated, decompositions.
        // Each jamo lies in U+1100..U+11C3, so `from_u32` always yields one.
        let index = cp - 0xAC00;
        output.extend(char::from_u32(0x1100 + index / 588));
        output.extend(char::from_u32(0x1161 + (index % 588) / 28));
        if !index.is_multiple_of(28) {
            output.extend(char::from_u32(0x11A7 + index % 28));
        }
    } else if let Ok(i) = DECOMPOSITION.binary_search_by_key(&cp, |row| row.0) {
        for part in DECOMPOSITION[i].1.chars() {
            decompose(part, output);
        }
    } else {
        output.push(c);
    }
}

fn compose(a: char, b: char) -> Option<char> {
    let (a, b) = (a as u32, b as u32);
    if (0x1100..0x1113).contains(&a) && (0x1161..0x1176).contains(&b) {
        return char::from_u32(0xAC00 + (a - 0x1100) * 588 + (b - 0x1161) * 28);
    }
    if (0xAC00..0xD7A4).contains(&a)
        && (a - 0xAC00).is_multiple_of(28)
        && (0x11A8..0x11C3).contains(&b)
    {
        return char::from_u32(a + b - 0x11A7);
    }
    COMPOSITION
        .binary_search_by_key(&(a, b), |row| row.0)
        .ok()
        .and_then(|i| char::from_u32(COMPOSITION[i].1))
}

pub(super) fn nfc(input: &str) -> String {
    let mut decomposed = Vec::new();
    for c in input.chars() {
        decompose(c, &mut decomposed);
    }
    let mut start = 0;
    for i in 0..decomposed.len() {
        if combining_class(decomposed[i]) == 0 {
            decomposed[start..i].sort_by_key(|c| combining_class(*c));
            start = i + 1;
        }
    }
    decomposed[start..].sort_by_key(|c| combining_class(*c));
    let mut output = Vec::new();
    let mut starter = None;
    let mut last_class = 0;
    for c in decomposed {
        let class = combining_class(c);
        if let Some(index) = starter {
            if last_class == 0 || last_class < class {
                if let Some(composed) = compose(output[index], c) {
                    output[index] = composed;
                    continue;
                }
            }
        }
        if class == 0 {
            starter = Some(output.len());
        }
        output.push(c);
        last_class = class;
    }
    output.into_iter().collect()
}
