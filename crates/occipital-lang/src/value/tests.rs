use std::cmp::Ordering;
use std::sync::Arc;

use super::*;

const SPAN: SourceSpan = SourceSpan {
    line: 1,
    column: 2,
    end_line: 1,
    end_column: 4,
};

fn number(value: i128) -> Value {
    Value::number(Decimal::from_integer(value).unwrap())
}
fn string(value: &str) -> Value {
    Value::string(value).unwrap()
}
fn error(code: &str) -> Value {
    Value::error(ValueError::new(code, SPAN))
}
fn list(values: Vec<Value>) -> Value {
    Value::list(values).unwrap()
}
fn record(fields: Vec<(&str, Value)>) -> Value {
    Value::record(
        fields
            .into_iter()
            .map(|(key, value)| (ScalarString::new(key), value))
            .collect(),
    )
    .unwrap()
}
fn empty_record() -> Record {
    let ValueKind::Record(record) = record(vec![]).kind else {
        unreachable!()
    };
    record
}
fn eq(left: &Value, right: &Value) -> (Result<bool, ValueFailure<()>>, u64) {
    let mut fuel = 0;
    let result = left.equals(right, SPAN, &mut |amount| {
        fuel += amount;
        Ok(())
    });
    (result, fuel)
}
fn opaque_values() -> Vec<Value> {
    vec![
        Value::component("Badge", empty_record(), vec![]).unwrap(),
        Value::action(vec![]).unwrap(),
        Value::snapshot(Snapshot {
            record_id: "snapshot-1".into(),
            members: empty_record(),
        }),
        Value::handle(Handle {
            record_id: "record-1".into(),
            identity: HandleIdentity::Unresolved {
                scoped_record_id: "scope/record-1".into(),
            },
        }),
        Value::site_envelope(SiteEnvelope {
            name: "job".into(),
            members: empty_record(),
        }),
    ]
}

#[test]
fn values_and_revisions_are_send_sync_and_share_with_arc() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Value>();
    assert_send_sync::<ValueKind>();
    assert_send_sync::<Revision>();
    let original = list(vec![string("shared")]);
    let copy = original.clone();
    let (ValueKind::List(a), ValueKind::List(b)) = (original.kind(), copy.kind()) else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(a, b));
    let ValueKind::String(text) = a[0].kind() else {
        unreachable!()
    };
    let text_copy = text.clone();
    assert!(Arc::ptr_eq(&text.text, &text_copy.text));
}

#[test]
fn all_thirteen_kinds_are_distinct() {
    let mut values = vec![
        number(1),
        string("x"),
        Value::boolean(true),
        Value::null(),
        Value::missing(),
        error("E_ARG"),
        list(vec![]),
        record(vec![]),
    ];
    values.extend(opaque_values());
    assert_eq!(
        values.iter().map(Value::value_type).collect::<Vec<_>>(),
        vec![
            ValueType::Number,
            ValueType::String,
            ValueType::Boolean,
            ValueType::Null,
            ValueType::Missing,
            ValueType::Error,
            ValueType::List,
            ValueType::Record,
            ValueType::Component,
            ValueType::Action,
            ValueType::Snapshot,
            ValueType::Handle,
            ValueType::SiteEnvelope,
        ]
    );
}

#[test]
fn strings_count_scalars_and_never_normalize() {
    let composed = string("é");
    let decomposed = string("e\u{301}");
    let ValueKind::String(a) = composed.kind() else {
        unreachable!()
    };
    let ValueKind::String(b) = decomposed.kind() else {
        unreachable!()
    };
    assert_eq!((a.len(), b.len()), (1, 2));
    assert_eq!(eq(&composed, &decomposed).0, Ok(false));
    let ValueKind::String(scalars) = string("💻\u{301}a").kind else {
        unreachable!()
    };
    assert_eq!(scalars.len(), 3);
    assert_eq!(composed.logical_size(), 20);
    assert_eq!(decomposed.logical_size(), 24);
    assert!(ScalarString::new("").is_empty());
}

#[test]
fn null_and_missing_keep_the_section_4_5_distinctions() {
    let value = record(vec![("a", Value::null()), ("b", Value::missing())]);
    let ValueKind::Record(fields) = value.kind() else {
        unreachable!()
    };
    assert_eq!(fields.keys().collect::<Vec<_>>(), ["a"]);
    assert_eq!(
        fields.values().map(Value::value_type).collect::<Vec<_>>(),
        [ValueType::Null]
    );
    assert_eq!(fields.get("a").value_type(), ValueType::Null);
    assert_eq!(fields.get("b").value_type(), ValueType::Missing);
    assert_eq!(
        list(vec![Value::missing()]).list_element(0).value_type(),
        ValueType::Missing
    );
    assert_eq!(
        list(vec![]).list_element(0).value_type(),
        ValueType::Missing
    );
    for absent in [Value::null(), Value::missing()] {
        assert!(absent.is_absent());
        assert_eq!(absent.record_field("a").value_type(), ValueType::Missing);
        assert_eq!(absent.condition(SPAN), Ok(false));
        assert_eq!(absent.to_text(SPAN), Ok(String::new()));
        assert_eq!(eq(&absent, &Value::null()).0, Ok(true));
        assert_eq!(eq(&absent, &Value::missing()).0, Ok(true));
    }
    assert_eq!(number(1).record_field("a").value_type(), ValueType::Missing);
    assert_eq!(value.canonical_json().unwrap(), r#"{"a":null}"#);
    assert_eq!(
        list(vec![Value::null(), Value::missing()])
            .canonical_json()
            .unwrap(),
        "[null,null]"
    );
}

#[test]
fn conditions_are_typed_and_errors_propagate() {
    assert_eq!(Value::boolean(true).condition(SPAN), Ok(true));
    assert_eq!(Value::boolean(false).condition(SPAN), Ok(false));
    for value in [number(0), string(""), list(vec![]), record(vec![])] {
        assert_eq!(value.condition(SPAN), Err(ValueError::type_error(SPAN)));
    }
    assert_eq!(
        error("E_DIV_ZERO").condition(SPAN),
        Err(ValueError::new("E_DIV_ZERO", SPAN))
    );
}

#[test]
fn text_conversion_matches_section_4_11() {
    let decimal = Value::number(Decimal::from_coefficient(-2_500_000_000_000_000_000).unwrap());
    for (value, expected) in [
        (string("**literal**"), "**literal**"),
        (decimal, "-2.5"),
        (Value::boolean(true), "true"),
        (Value::boolean(false), "false"),
        (Value::null(), ""),
        (Value::missing(), ""),
    ] {
        assert_eq!(value.to_text(SPAN).unwrap(), expected);
    }
    assert_eq!(
        error("E_OVERFLOW").to_text(SPAN),
        Err(ValueError::new("E_OVERFLOW", SPAN))
    );
    for value in opaque_values()
        .into_iter()
        .chain([list(vec![]), record(vec![])])
    {
        assert_eq!(value.to_text(SPAN), Err(ValueError::type_error(SPAN)));
    }
}

#[test]
fn equality_obeys_kind_and_error_precedence() {
    assert_eq!(eq(&number(1), &number(1)).0, Ok(true));
    assert_eq!(eq(&number(1), &string("1")).0, Ok(false));
    assert_eq!(
        eq(&Value::boolean(false), &Value::boolean(true)).0,
        Ok(false)
    );
    assert_eq!(eq(&Value::null(), &number(0)).0, Ok(false));
    assert_eq!(
        eq(&error("E_ARG"), &error("E_DOMAIN")).0,
        Err(ValueFailure::Value(ValueError::new("E_ARG", SPAN)))
    );
    assert_eq!(
        eq(&number(1), &error("E_DOMAIN")).0,
        Err(ValueFailure::Value(ValueError::new("E_DOMAIN", SPAN)))
    );
    for value in opaque_values()
        .into_iter()
        .filter(|v| v.value_type() != ValueType::Handle)
    {
        assert_eq!(
            eq(&value, &Value::null()).0,
            Err(ValueFailure::Value(ValueError::type_error(SPAN)))
        );
        assert_eq!(
            eq(&Value::null(), &value).0,
            Err(ValueFailure::Value(ValueError::type_error(SPAN)))
        );
        assert_eq!(
            eq(&value, &error("E_ARG")).0,
            Err(ValueFailure::Value(ValueError::new("E_ARG", SPAN)))
        );
    }
}

#[test]
fn lists_visit_in_order_and_stop_before_later_errors() {
    assert_eq!(
        eq(
            &list(vec![number(1), error("E_ARG")]),
            &list(vec![number(2), number(0)])
        ),
        (Ok(false), 2)
    );
    assert_eq!(
        eq(&list(vec![error("E_DOMAIN")]), &list(vec![number(0)])).0,
        Err(ValueFailure::Value(ValueError::new("E_DOMAIN", SPAN)))
    );
    assert_eq!(
        eq(&list(vec![error("E_ARG")]), &list(vec![])),
        (Ok(false), 1)
    );
    assert_eq!(
        eq(&list(vec![number(1)]), &list(vec![number(1)])),
        (Ok(true), 2)
    );
}

#[test]
fn record_equality_sorts_keys_and_values_independent_of_insertion_order() {
    let left = record(vec![("b", number(2)), ("a", number(1))]);
    let right = record(vec![("a", number(1)), ("b", number(2))]);
    // n=2,K=1: 10 per sort; root 1, keys 2, values 2 => 25.
    assert_eq!(eq(&left, &right), (Ok(true), 25));
    assert_eq!(eq(&right, &left), (Ok(true), 25));
    assert_eq!(eq(&record(vec![]), &record(vec![])), (Ok(true), 1));
    assert_eq!(
        eq(&record(vec![("a", error("E_ARG"))]), &record(vec![])),
        (Ok(false), 1)
    );
}

#[test]
fn record_equality_checks_all_keys_before_values_and_uses_depth_first_order() {
    let left = record(vec![("b", error("E_DOMAIN")), ("a", number(1))]);
    let right = record(vec![("a", number(2)), ("b", error("E_ARG"))]);
    assert_eq!(eq(&left, &right), (Ok(false), 24));
    let different_keys = record(vec![("a", error("E_ARG")), ("c", number(0))]);
    assert_eq!(eq(&left, &different_keys), (Ok(false), 23));
    let left = record(vec![
        ("b", error("E_DOMAIN")),
        ("a", list(vec![error("E_ARG")])),
    ]);
    let right = record(vec![("a", list(vec![number(1)])), ("b", number(2))]);
    assert_eq!(
        eq(&left, &right).0,
        Err(ValueFailure::Value(ValueError::new("E_ARG", SPAN)))
    );
}

#[test]
fn record_key_order_is_unicode_scalar_not_utf16_order() {
    let left = record(vec![
        ("\u{10000}", error("E_DOMAIN")),
        ("\u{e000}", error("E_ARG")),
    ]);
    let right = record(vec![("\u{e000}", number(0)), ("\u{10000}", number(0))]);
    assert_eq!(
        eq(&left, &right).0,
        Err(ValueFailure::Value(ValueError::new("E_ARG", SPAN)))
    );
}

#[test]
fn record_sort_fuel_is_precharged_atomically_and_stops_at_the_exact_boundary() {
    let left = record(vec![("b", number(2)), ("a", number(1))]);
    let right = record(vec![("a", number(1)), ("b", number(2))]);
    for (budget, expected_spent, expected_attempts) in [
        (20, 1, vec![1, 20]),
        (24, 24, vec![1, 20, 1, 1, 1, 1]),
        (25, 25, vec![1, 20, 1, 1, 1, 1]),
    ] {
        let mut spent = 0;
        let mut attempts = Vec::new();
        let result = left.equals(&right, SPAN, &mut |amount| {
            attempts.push(amount);
            if spent + amount > budget {
                Err("E_FUEL")
            } else {
                spent += amount;
                Ok(())
            }
        });
        assert_eq!(spent, expected_spent);
        assert_eq!(attempts, expected_attempts);
        assert_eq!(
            result,
            if budget < 25 {
                Err(ValueFailure::Fuel("E_FUEL"))
            } else {
                Ok(true)
            }
        );
    }
}

#[test]
fn record_sort_tariff_rounds_log_and_longest_scalar_key_up() {
    let key = "é".repeat(65);
    let value = record(vec![(&key, number(1)), ("b", number(2)), ("c", number(3))]);
    // n=3,K=65: each sort costs 3+2*3*2*3=39. Key comparisons
    // cost 5+1+1; root and three values add 4, for a total of 89.
    assert_eq!(eq(&value, &value), (Ok(true), 89));
    let empty_key = record(vec![("", Value::null())]);
    // n=1,K=0: each sort costs 3; root/key/value add 3.
    assert_eq!(eq(&empty_key, &empty_key), (Ok(true), 9));
}

#[test]
fn string_comparisons_charge_examined_prefix_incrementally() {
    assert_eq!(eq(&string("x"), &string("y")), (Ok(false), 1));
    assert_eq!(eq(&string(""), &string("")), (Ok(true), 1));
    assert_eq!(
        eq(&string(&"é".repeat(16)), &string(&"é".repeat(16))),
        (Ok(true), 2)
    );
    assert_eq!(
        eq(
            &list(vec![string(&"a".repeat(16))]),
            &list(vec![string(&"a".repeat(16))])
        ),
        (Ok(true), 4)
    );
    let mut attempts = Vec::new();
    let result = string(&"a".repeat(100)).equals(&string(&"a".repeat(100)), SPAN, &mut |n| {
        attempts.push(n);
        if attempts.len() == 2 {
            Err("E_FUEL")
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err(ValueFailure::Fuel("E_FUEL")));
    assert_eq!(attempts, [1, 1]);
}

#[test]
fn handle_equality_uses_scoped_immutable_identity() {
    fn resolved(record: &str, kind: &str, resource: &str, version: &str) -> Value {
        Value::handle(Handle {
            record_id: record.into(),
            identity: HandleIdentity::Resolved {
                kind: kind.into(),
                scoped_resource_id: resource.into(),
                version: version.into(),
            },
        })
    }
    let a = resolved("record-1", "file", "computer/session/a", "hash-1");
    assert_eq!(
        eq(
            &a,
            &resolved("record-2", "file", "computer/session/a", "hash-1")
        )
        .0,
        Ok(true)
    );
    for b in [
        resolved("record-1", "image", "computer/session/a", "hash-1"),
        resolved("record-1", "file", "other/session/a", "hash-1"),
        resolved("record-1", "file", "computer/session/a", "hash-2"),
    ] {
        assert_eq!(eq(&a, &b).0, Ok(false));
    }
    let unresolved = |id: &str| {
        Value::handle(Handle {
            record_id: "same".into(),
            identity: HandleIdentity::Unresolved {
                scoped_record_id: id.into(),
            },
        })
    };
    assert_eq!(
        eq(&unresolved("scope/a"), &unresolved("scope/a")).0,
        Ok(true)
    );
    assert_eq!(
        eq(&unresolved("scope/a"), &unresolved("scope/b")).0,
        Ok(false)
    );
    assert_eq!(eq(&a, &unresolved("scope/a")).0, Ok(false));
}

#[test]
fn handle_identity_comparisons_charge_strings_and_short_circuit() {
    let handle = |kind: &str| {
        Value::handle(Handle {
            record_id: "record".into(),
            identity: HandleIdentity::Resolved {
                kind: kind.into(),
                scoped_resource_id: "r".into(),
                version: "v".into(),
            },
        })
    };
    assert_eq!(eq(&handle("k"), &handle("k")), (Ok(true), 3));
    assert_eq!(eq(&handle("k"), &handle("x")), (Ok(false), 1));
}

#[test]
fn relational_operators_follow_their_own_absent_and_type_rules() {
    for relation in [
        Relation::Less,
        Relation::LessEqual,
        Relation::Greater,
        Relation::GreaterEqual,
    ] {
        for absent in [Value::null(), Value::missing()] {
            assert_eq!(
                absent.relates(&number(1), relation, SPAN, &mut |_| Ok::<_, ()>(())),
                Ok(false)
            );
            assert_eq!(
                number(1).relates(&absent, relation, SPAN, &mut |_| Ok::<_, ()>(())),
                Ok(false)
            );
            assert_eq!(
                absent.relates(&error("E_ARG"), relation, SPAN, &mut |_| Ok::<_, ()>(())),
                Err(ValueFailure::Value(ValueError::new("E_ARG", SPAN)))
            );
        }
        assert_eq!(
            Value::boolean(false).relates(&Value::boolean(true), relation, SPAN, &mut |_| Ok::<
                _,
                (),
            >(
                ()
            )),
            Err(ValueFailure::Value(ValueError::type_error(SPAN)))
        );
    }
    let less = |a: Value, b: Value| a.relates(&b, Relation::Less, SPAN, &mut |_| Ok::<_, ()>(()));
    assert_eq!(less(number(2), number(10)), Ok(true));
    assert_eq!(less(string("\u{e000}"), string("\u{10000}")), Ok(true));
    assert_eq!(
        less(number(1), string("1")),
        Err(ValueFailure::Value(ValueError::type_error(SPAN)))
    );
    for (relation, expected) in [
        (Relation::Less, false),
        (Relation::LessEqual, true),
        (Relation::Greater, false),
        (Relation::GreaterEqual, true),
    ] {
        assert_eq!(
            number(2).relates(&number(2), relation, SPAN, &mut |_| Ok::<_, ()>(())),
            Ok(expected)
        );
    }
    assert_eq!(
        number(2).relates(
            &number(1),
            Relation::Greater,
            SPAN,
            &mut |_| Ok::<_, ()>(())
        ),
        Ok(true)
    );
}

#[test]
fn sort_preorder_is_stable_opaque_and_keeps_absent_and_errors_last() {
    let mut values = vec![
        error("E_ARG"),
        Value::missing(),
        Value::boolean(true),
        string("b"),
        number(2),
        list(vec![error("E_DOMAIN")]),
        record(vec![]),
        Value::null(),
        Value::boolean(false),
        string("a"),
        number(1),
    ];
    values.sort_by(|a, b| a.sort_order(b, false));
    assert_eq!(
        values.iter().map(Value::value_type).collect::<Vec<_>>(),
        [
            ValueType::Number,
            ValueType::Number,
            ValueType::String,
            ValueType::String,
            ValueType::Boolean,
            ValueType::Boolean,
            ValueType::List,
            ValueType::Record,
            ValueType::Missing,
            ValueType::Null,
            ValueType::Error
        ]
    );
    assert_eq!(values[0].to_text(SPAN).unwrap(), "1");
    assert_eq!(values[2].to_text(SPAN).unwrap(), "a");
    assert_eq!(values[4].to_text(SPAN).unwrap(), "false");
    values.sort_by(|a, b| a.sort_order(b, true));
    assert_eq!(
        values.iter().map(Value::value_type).collect::<Vec<_>>(),
        [
            ValueType::List,
            ValueType::Record,
            ValueType::Boolean,
            ValueType::Boolean,
            ValueType::String,
            ValueType::String,
            ValueType::Number,
            ValueType::Number,
            ValueType::Missing,
            ValueType::Null,
            ValueType::Error
        ]
    );
    assert_eq!(values[2].to_text(SPAN).unwrap(), "true");
    assert_eq!(values[4].to_text(SPAN).unwrap(), "b");
    assert_eq!(values[6].to_text(SPAN).unwrap(), "2");
    for value in opaque_values() {
        assert_eq!(value.sort_order(&record(vec![]), false), Ordering::Equal);
    }
}

#[test]
fn logical_sizes_and_shallow_allocations_use_recorded_parts_not_sharing() {
    let s = string("💻a"); // L=2: 24 logical and shallow bytes.
    assert_eq!(
        (s.logical_size(), s.allocation_size(Origin::Evaluation)),
        (24, 24)
    );
    let repeated = list(vec![s.clone(), s.clone()]);
    assert_eq!(
        (
            repeated.logical_size(),
            repeated.allocation_size(Origin::Evaluation)
        ),
        (64, 32)
    );
    let r = record(vec![("💻", repeated), ("a", Value::null())]);
    // 16 + (16+4+64) + (16+4+16) = 136; shallow=16+2*16.
    assert_eq!(
        (r.logical_size(), r.allocation_size(Origin::Evaluation)),
        (136, 48)
    );
    let omitted = record(vec![("absent", Value::missing())]);
    assert_eq!(
        (
            omitted.logical_size(),
            omitted.allocation_size(Origin::Evaluation)
        ),
        (16, 16)
    );
    assert_eq!(
        (
            list(vec![]).logical_size(),
            list(vec![]).allocation_size(Origin::Evaluation)
        ),
        (16, 16)
    );
    for atom in [
        number(1),
        Value::boolean(false),
        Value::null(),
        Value::missing(),
        error("E_ARG"),
    ] {
        assert_eq!(
            (
                atom.logical_size(),
                atom.allocation_size(Origin::Evaluation)
            ),
            (16, 16)
        );
        assert_eq!(atom.allocation_size(Origin::Host), 0);
    }
    for value in [s, r] {
        assert_eq!(value.allocation_size(Origin::Host), 0);
    }
}

#[test]
fn badge_text_child_and_repeated_children_have_exact_component_sizes() {
    let ValueKind::Record(props) = record(vec![("tone", string("info"))]).kind else {
        unreachable!()
    };
    let badge = Value::component("Badge", props, vec![string("x")]).unwrap();
    // <Badge tone="info">x</Badge>: 32 + (16+4*4) + (16+4*1) = 84.
    assert_eq!(
        (
            badge.logical_size(),
            badge.allocation_size(Origin::Evaluation)
        ),
        (84, 48)
    );
    let parent = Value::component("Card", empty_record(), vec![badge.clone(), badge]).unwrap();
    assert_eq!(
        (
            parent.logical_size(),
            parent.allocation_size(Origin::Evaluation)
        ),
        (200, 48)
    );
}

#[test]
fn actions_charge_arguments_once_and_host_wrappers_remain_sixteen_bytes() {
    let text = string("abc"); // 28 bytes.
    let action = Value::action(vec![
        ActionStep {
            operation: ActionOperation::Compose,
            arguments: vec![text.clone(), text].into(),
        },
        ActionStep {
            operation: ActionOperation::Reset,
            arguments: vec![number(1)].into(),
        },
    ])
    .unwrap();
    assert_eq!(
        (
            action.logical_size(),
            action.allocation_size(Origin::Evaluation)
        ),
        (88, 40)
    );
    assert_eq!(
        Value::action(vec![])
            .unwrap()
            .allocation_size(Origin::Evaluation),
        16
    );
    let ValueKind::Record(members) = record(vec![("text", string(&"x".repeat(1000)))]).kind else {
        unreachable!()
    };
    let host_values = [
        Value::snapshot(Snapshot {
            record_id: "snapshot".into(),
            members: members.clone(),
        }),
        Value::site_envelope(SiteEnvelope {
            name: "out".into(),
            members,
        }),
        opaque_values().remove(3),
    ];
    for value in host_values {
        assert_eq!(value.logical_size(), 16);
        assert_eq!(value.allocation_size(Origin::Evaluation), 0);
        assert_eq!(value.allocation_size(Origin::Host), 0);
    }
}

#[test]
fn logical_size_overflow_is_a_breach_not_a_wrap_or_graph_walk() {
    let mut value = Value::component("Card", empty_record(), vec![]).unwrap();
    // Only 59 shared nodes are allocated, but the expanded logical size doubles
    // at each level. A full graph walk would never finish this regression test.
    for _ in 0..58 {
        value = list(vec![value.clone(), value]).clone();
    }
    assert!(matches!(
        Value::list(vec![value.clone(), value]),
        Err(ValueBuildError::SizeOverflow)
    ));
    assert_eq!(slots(u64::MAX, 1, 8), Err(ValueBuildError::SizeOverflow));
}

#[test]
fn ordered_codec_has_exact_hand_written_vectors_for_all_eight_kinds() {
    let vectors = [
        (number(2), r#"["number","2000000000000000000"]"#),
        (string("é\n"), r#"["string","é\n"]"#),
        (Value::boolean(true), r#"["boolean",true]"#),
        (Value::null(), r#"["null"]"#),
        (Value::missing(), r#"["missing"]"#),
        (
            list(vec![Value::missing(), number(1)]),
            r#"["list",[["missing"],["number","1000000000000000000"]]]"#,
        ),
        (
            record(vec![("b", number(2)), ("a", number(1))]),
            r#"["record",[["b",["number","2000000000000000000"]],["a",["number","1000000000000000000"]]]]"#,
        ),
        (
            error("E_ARG"),
            r#"["error","E_ARG",{"line":1,"column":2,"endLine":1,"endColumn":4}]"#,
        ),
    ];
    for (value, expected) in vectors {
        let encoded = value.encode_ordered(CodecMode::Event).unwrap();
        // Span object key order is metadata, unlike record-pair array order.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
            serde_json::from_str::<serde_json::Value>(expected).unwrap()
        );
        let restored = Value::decode_ordered(expected.as_bytes(), CodecMode::Event).unwrap();
        assert_eq!(restored.value_type(), value.value_type());
        assert_eq!(restored.logical_size(), value.logical_size());
        assert_eq!(restored.encode_ordered(CodecMode::Event).unwrap(), encoded);
    }
}

#[test]
fn restoring_ordered_records_keeps_keys_b_then_a_and_missing_list_elements() {
    let source = br#"["record",[["b",["number","2000000000000000000"]],["a",["number","1000000000000000000"]]]]"#;
    let restored = Value::decode_ordered(source, CodecMode::State).unwrap();
    let ValueKind::Record(fields) = restored.kind() else {
        unreachable!()
    };
    assert_eq!(fields.keys().collect::<Vec<_>>(), ["b", "a"]);
    assert_eq!(
        fields
            .values()
            .map(|v| v.to_text(SPAN).unwrap())
            .collect::<Vec<_>>(),
        ["2", "1"]
    );
    assert_eq!(
        restored
            .encode_ordered(CodecMode::State)
            .unwrap()
            .as_bytes(),
        source
    );
    let value = list(vec![Value::null(), Value::missing()]);
    let restored = Value::decode_ordered(
        value.encode_ordered(CodecMode::State).unwrap().as_bytes(),
        CodecMode::State,
    )
    .unwrap();
    assert_eq!(restored.list_element(0).value_type(), ValueType::Null);
    assert_eq!(restored.list_element(1).value_type(), ValueType::Missing);
}

#[test]
fn ordered_codec_keeps_exact_adjacent_ids_and_extreme_coefficients() {
    for coefficient in [
        99_999_999_999_999_999_999_999_999_999_999_999_999i128,
        -99_999_999_999_999_999_999_999_999_999_999_999_999,
        1,
        -1,
        0,
        18_446_744_073_709_551_615_000_000_000_000_000_000,
        18_446_744_073_709_551_614_000_000_000_000_000_000,
    ] {
        let source = format!(r#"["number","{coefficient}"]"#);
        let decoded = Value::decode_ordered(source.as_bytes(), CodecMode::State).unwrap();
        let ValueKind::Number(value) = decoded.kind() else {
            unreachable!()
        };
        assert_eq!(value.coefficient(), coefficient);
        assert_eq!(decoded.encode_ordered(CodecMode::State).unwrap(), source);
    }
}

#[test]
fn codec_rejects_each_non_data_kind_on_both_sides_and_at_nested_positions() {
    for (value, tag) in opaque_values().into_iter().zip([
        "component",
        "action",
        "snapshot",
        "handle",
        "siteEnvelope",
    ]) {
        for mode in [CodecMode::Event, CodecMode::State] {
            let expected = CodecError::UnsupportedKind(value.value_type());
            assert_eq!(value.encode_ordered(mode), Err(expected.clone()));
            assert!(
                matches!(Value::decode_ordered(format!(r#"["{tag}"]"#).as_bytes(), mode), Err(ref error) if error == &expected)
            );
            assert_eq!(
                list(vec![value.clone()]).encode_ordered(mode),
                Err(expected.clone())
            );
            assert!(
                matches!(Value::decode_ordered(format!(r#"["list",[["{tag}"]]]"#).as_bytes(), mode), Err(ref error) if error == &expected)
            );
        }
        assert_eq!(
            value.canonical_json(),
            Err(CodecError::UnsupportedKind(value.value_type()))
        );
    }
}

#[test]
fn ordered_codec_errors_are_events_not_persisted_state() {
    let value = record(vec![("a", list(vec![error("E_DIV_ZERO")]))]);
    let event = value.encode_ordered(CodecMode::Event).unwrap();
    assert_eq!(
        value.encode_ordered(CodecMode::State),
        Err(CodecError::ErrorInState)
    );
    assert!(matches!(
        Value::decode_ordered(event.as_bytes(), CodecMode::State),
        Err(CodecError::ErrorInState)
    ));
    let restored = Value::decode_ordered(event.as_bytes(), CodecMode::Event).unwrap();
    assert_eq!(restored.encode_ordered(CodecMode::Event).unwrap(), event);
    assert_eq!(
        error("E_ARG").canonical_json(),
        Err(CodecError::UnsupportedKind(ValueType::Error))
    );
}

#[test]
fn ordered_codec_rejects_malformed_tags_payloads_numbers_and_duplicate_fields() {
    for source in [
        r#"{}"#,
        r#"[]"#,
        r#"["unknown"]"#,
        r#"["null",0]"#,
        r#"["boolean","true"]"#,
        r#"["string",1]"#,
        r#"["list",{}]"#,
        r#"["record",[["a"]]]"#,
        r#"["record",[[1,["null"]]]]"#,
        r#"["error","E_ARG",{"line":0,"column":1,"endLine":1,"endColumn":2}]"#,
        r#"["error","E_ARG",{"line":2,"column":1,"endLine":1,"endColumn":2}]"#,
    ] {
        assert!(
            Value::decode_ordered(source.as_bytes(), CodecMode::Event).is_err(),
            "{source}"
        );
    }
    for payload in [
        "1",
        r#""1.5""#,
        r#""1e3""#,
        r#""+1""#,
        r#""01""#,
        r#""-0""#,
        r#""100000000000000000000000000000000000000""#,
    ] {
        assert!(
            matches!(
                Value::decode_ordered(
                    format!(r#"["number",{payload}]"#).as_bytes(),
                    CodecMode::Event
                ),
                Err(CodecError::InvalidNumber)
            ),
            "{payload}"
        );
    }
    let duplicate = br#"["record",[["a",["null"]],["a",["null"]]]]"#;
    assert!(
        matches!(Value::decode_ordered(duplicate, CodecMode::State), Err(CodecError::Build(ValueBuildError::DuplicateKey(key))) if key == "a")
    );
    assert!(
        matches!(Value::record(vec![(ScalarString::new("a"), Value::null()), (ScalarString::new("a"), Value::null())]), Err(ValueBuildError::DuplicateKey(key)) if key == "a")
    );
}

#[test]
fn canonical_identity_sorts_scalars_escapes_minimally_and_does_not_reorder_values() {
    let value = record(vec![
        ("b", number(2)),
        (
            "a",
            list(vec![
                Value::missing(),
                Value::null(),
                string("\"\\\n\u{1}é/\u{2028}"),
            ]),
        ),
        ("\u{10000}", number(1)),
        ("\u{e000}", number(2)),
        ("omitted", Value::missing()),
    ]);
    let expected = "{\"a\":[null,null,\"\\\"\\\\\\n\\u0001é/\u{2028}\"],\"b\":2,\"\u{e000}\":2,\"\u{10000}\":1}";
    assert_eq!(value.canonical_json().unwrap(), expected);
    let ValueKind::Record(fields) = value.kind() else {
        unreachable!()
    };
    assert_eq!(
        fields.keys().collect::<Vec<_>>(),
        ["b", "a", "\u{10000}", "\u{e000}"]
    );
    assert_eq!(
        Value::number(Decimal::from_literal("0.562500").unwrap())
            .canonical_json()
            .unwrap(),
        "0.5625"
    );
    assert_eq!(
        number(18_446_744_073_709_551_615).canonical_json().unwrap(),
        "18446744073709551615"
    );
    assert_eq!(
        Value::number(Decimal::from_coefficient(-1).unwrap())
            .canonical_json()
            .unwrap(),
        "-0.000000000000000001"
    );
}
