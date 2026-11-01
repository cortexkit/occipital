use occipital_lang::{
    CodecMode, Decimal, Origin, Revision, ScalarString, SourceSpan, Value, ValueKind,
};

#[test]
fn public_values_restore_order_and_cross_a_thread_without_copying_the_tree() {
    let value = Value::record(vec![
        (
            ScalarString::new("b"),
            Value::number(Decimal::from_integer(2).unwrap()),
        ),
        (
            ScalarString::new("a"),
            Value::number(Decimal::from_integer(1).unwrap()),
        ),
    ])
    .unwrap();
    let encoded = value.encode_ordered(CodecMode::State).unwrap();
    let restored = Value::decode_ordered(encoded.as_bytes(), CodecMode::State).unwrap();
    let revision = Revision {
        instance_epoch: 1,
        revision: 2,
        root: restored,
    };
    let revision = std::thread::spawn(move || {
        let ValueKind::Record(record) = revision.root.kind() else {
            panic!("expected a record")
        };
        assert_eq!(record.keys().collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(revision.root.logical_size(), 88);
        assert_eq!(revision.root.allocation_size(Origin::Host), 0);
        revision
    })
    .join()
    .unwrap();
    let span = SourceSpan {
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 2,
    };
    let mut fuel = 0;
    assert_eq!(
        value.equals(&revision.root, span, &mut |n| {
            fuel += n;
            Ok::<_, ()>(())
        }),
        Ok(true)
    );
    assert_eq!(fuel, 25);
    assert_eq!(revision.root.canonical_json().unwrap(), r#"{"a":1,"b":2}"#);
}
