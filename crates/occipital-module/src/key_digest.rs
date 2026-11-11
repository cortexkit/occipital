//! The keys of the in-memory sets that remember what was already written to a
//! store (a call's pin, a notice, the send a message came from), so a page read
//! again and again costs no store write.
//!
//! Each key is several strings: a runner, a session, a call key or message id,
//! and so on. A set holding them as strings costs about 180 bytes a key; a
//! 16-byte digest of them about 25. The sets are only asked whether a key is
//! present, which the digest answers as well. Two different keys with the same
//! digest would make one write be skipped as already done, which in 128 bits
//! doesn't happen in practice.

use sha2::{Digest, Sha256};

/// The first 16 bytes of the SHA-256 of a key's parts.
pub type KeyDigest = [u8; 16];

/// The digest of a key made of `parts`, in order. Each part is preceded by its
/// length, so `("ab", "c")` and `("a", "bc")` differ.
pub fn key_digest(parts: &[&str]) -> KeyDigest {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let mut digest = [0; 16];
    digest.copy_from_slice(&hasher.finalize()[..16]);
    digest
}

#[cfg(test)]
mod tests {
    use super::key_digest;

    #[test]
    fn parts_are_kept_apart_and_in_order() {
        assert_eq!(key_digest(&["a", "b"]), key_digest(&["a", "b"]));
        assert_ne!(key_digest(&["ab", "c"]), key_digest(&["a", "bc"]));
        assert_ne!(key_digest(&["a", "b"]), key_digest(&["b", "a"]));
        assert_ne!(key_digest(&["a", ""]), key_digest(&["a"]));
    }
}
