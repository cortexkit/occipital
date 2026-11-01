//! Registry digests: `sha256:` followed by the lowercase hex SHA-256 of
//! canonical bytes.

/// Returns `sha256:` and the 64 lowercase hex characters of the SHA-256 of
/// `canonical`.
pub fn sha256_digest(canonical: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hash = hmac_sha256::Hash::hash(canonical);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in hash {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}
