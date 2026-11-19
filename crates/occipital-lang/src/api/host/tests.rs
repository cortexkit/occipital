use super::*;
use crate::{Schema, ValueKind};

#[test]
fn host_parity_numbers_and_enum_members() {
    for (token, expected) in [
        ("1.000000000000000001", Some("1")),
        ("-2", Some("-2")),
        ("1e30", None),
        ("18446744073709551615", Some("18446744073709551615")),
    ] {
        let raw = serde_json::from_str(token).unwrap();
        match (json_value(&raw), expected) {
            (Ok(value), Some(expected)) => {
                assert_eq!(value.canonical_json().unwrap(), expected);
                let schema = Schema {
                    enumeration: Some(vec![serde_json::from_str(expected).unwrap()]),
                    ..Default::default()
                };
                assert!(schema.accepts(&raw), "{token}");
            }
            (Err(HostError::InvalidHostValue(_)), None) => {
                let schema = Schema {
                    enumeration: Some(vec![raw.clone()]),
                    ..Default::default()
                };
                assert!(!schema.accepts(&raw));
            }
            other => panic!("{token}: {other:?}"),
        }
    }
}

#[test]
fn host_parity_record_order() {
    let raw = serde_json::from_str(r#"{"z":1,"a":2}"#).unwrap();
    let value = json_value(&raw).unwrap();
    let ValueKind::Record(record) = value.kind() else {
        panic!("expected record")
    };
    assert_eq!(record.keys().collect::<Vec<_>>(), ["a", "z"]);
}
