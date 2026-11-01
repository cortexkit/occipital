//! Lexical rule tests: read ids, digests and dialects.

use occipital_registry::grammar::{is_dialect, is_operation_id, is_sha256_digest};

#[test]
fn read_ids() {
    assert!(is_operation_id("a") && is_operation_id("a0_"));
    assert!(is_operation_id(&format!("a{}", "0".repeat(47))));
    for refused in ["", "a.b", "A", "0a", "_a", "a-b", "é"] {
        assert!(!is_operation_id(refused), "{refused:?}");
    }
    assert!(!is_operation_id(&format!("a{}", "0".repeat(48))));
}

#[test]
fn digests() {
    assert!(is_sha256_digest(&format!("sha256:{}", "0".repeat(64))));
    assert!(is_sha256_digest(&format!("sha256:{}", "f".repeat(64))));
    for refused in [
        format!("SHA256:{}", "0".repeat(64)),
        format!("sha256:{}", "g".repeat(64)),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "0".repeat(63)),
        format!("sha256:{}", "0".repeat(65)),
        "0".repeat(64),
    ] {
        assert!(!is_sha256_digest(&refused), "{refused:?}");
    }
}

#[test]
fn dialects() {
    assert!(is_dialect("1.0") && is_dialect("12.345"));
    for dialect in ["1", "1.", ".1", "1.0.0", "a.b", "1.0 ", "١.٠"] {
        assert!(!is_dialect(dialect), "{dialect:?}");
    }
}
