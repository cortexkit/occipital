//! RFC 3492 Bootstring with the Punycode parameters and checked arithmetic.

fn adapt(mut delta: u64, points: u64, first: bool) -> u64 {
    delta /= if first { 700 } else { 2 };
    delta += delta / points;
    let mut k = 0;
    while delta > 455 {
        delta /= 35;
        k += 36;
    }
    k + (36 * delta) / (delta + 38)
}

fn threshold(k: u64, bias: u64) -> u64 {
    k.saturating_sub(bias).clamp(1, 26)
}

fn digit(n: u64) -> Option<char> {
    match n {
        0..=25 => Some((b'a' + n as u8) as char),
        26..=35 => Some((b'0' + n as u8 - 26) as char),
        _ => None,
    }
}

pub(super) fn encode(input: &str) -> Option<String> {
    let points: Vec<u64> = input.chars().map(|c| c as u64).collect();
    let mut output: String = input.chars().filter(|c| c.is_ascii()).collect();
    let basic = output.len() as u64;
    let mut handled = basic;
    if basic != 0 {
        output.push('-');
    }
    let (mut n, mut delta, mut bias) = (128, 0u64, 72);
    while handled < points.len() as u64 {
        let m = *points.iter().filter(|&&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(handled + 1)?)?;
        n = m;
        for &c in &points {
            if c < n {
                delta = delta.checked_add(1)?;
            } else if c == n {
                let mut q = delta;
                let mut k = 36;
                loop {
                    let t = threshold(k, bias);
                    if q < t {
                        break;
                    }
                    output.push(digit(t + (q - t) % (36 - t))?);
                    q = (q - t) / (36 - t);
                    k += 36;
                }
                output.push(digit(q)?);
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta = delta.checked_add(1)?;
        n += 1;
    }
    Some(output)
}

pub(super) fn decode(input: &str) -> Option<String> {
    if !input.is_ascii() {
        return None;
    }
    let (mut output, rest) = match input.rsplit_once('-') {
        Some((basic, rest)) => (basic.chars().collect::<Vec<_>>(), rest),
        None => (Vec::new(), input),
    };
    let mut bytes = rest.bytes().peekable();
    let (mut n, mut i, mut bias) = (128u64, 0u64, 72);
    while bytes.peek().is_some() {
        let old_i = i;
        let mut weight = 1u64;
        let mut k = 36;
        loop {
            let value = match bytes.next()? {
                c @ b'a'..=b'z' => (c - b'a') as u64,
                c @ b'0'..=b'9' => (c - b'0') as u64 + 26,
                _ => return None,
            };
            i = i.checked_add(value.checked_mul(weight)?)?;
            let t = threshold(k, bias);
            if value < t {
                break;
            }
            weight = weight.checked_mul(36 - t)?;
            k += 36;
        }
        let count = output.len() as u64 + 1;
        bias = adapt(i - old_i, count, old_i == 0);
        n = n.checked_add(i / count)?;
        i %= count;
        let c = char::from_u32(u32::try_from(n).ok()?)?;
        output.insert(i as usize, c);
        i += 1;
    }
    Some(output.into_iter().collect())
}
