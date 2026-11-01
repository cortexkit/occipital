//! The lexical rules template values follow: read ids, digests and dialects.

/// Whether `id` is a read id: `^[a-z][a-z0-9_]{0,47}$`.
pub fn is_operation_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    match bytes.split_first() {
        Some((first, rest)) => {
            first.is_ascii_lowercase()
                && rest.len() <= 47
                && rest
                    .iter()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        }
        None => false,
    }
}

/// Whether `digest` is `sha256:` followed by 64 lowercase hex characters.
pub fn is_sha256_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Whether `dialect` matches `^[0-9]+\.[0-9]+$`.
pub fn is_dialect(dialect: &str) -> bool {
    match dialect.split_once('.') {
        Some((major, minor)) => is_digits(major) && is_digits(minor),
        None => false,
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}
