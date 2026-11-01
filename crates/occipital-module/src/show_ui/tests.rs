use super::*;
use serde_json::json;

pub(crate) fn arguments() -> Value {
    json!({"kind":"native", "title":"Title 🦉", "summary":"Summary", "program":"source\n雪"})
}

pub(crate) fn stamp() -> RouteStamp {
    let bind: Value = serde_json::from_str(include_str!("../../tests/fixtures/pinned/subc-protocol-0.33.1/module_control_request_route_bind_with_scope.json")).unwrap();
    RouteStamp {
        scope: Some(serde_json::from_value(bind["scope"].clone()).unwrap()),
        principal: Some(serde_json::from_value(bind["principal"].clone()).unwrap()),
    }
}

pub(crate) fn descriptor(count: u64) -> Value {
    let sample: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/pinned/commons-d75cf9966453/output-refs.json"
    ))
    .unwrap();
    let mut descriptor = sample["descriptor"].clone();
    descriptor["count"] = count.into();
    descriptor
}

fn admit(args: &Value, refs: Option<&Value>, route: &RouteStamp) -> Result<PreparedCall, Refusal> {
    let refs = refs.map(|value| serde_json::to_vec(value).unwrap());
    prepare(
        args,
        refs.as_deref(),
        route,
        1234,
        Decimal::from_integer(1234).unwrap(),
    )
}

fn code(args: &Value, refs: Option<&Value>, route: &RouteStamp) -> Refusal {
    match admit(args, refs, route) {
        Ok(_) => panic!("expected refusal"),
        Err(refusal) => refusal,
    }
}

#[test]
fn kind_html_is_kind_unsupported_before_every_other_fault() {
    assert_eq!(
        code(
            &json!({"kind":"html","owner":false}),
            Some(&Value::Null),
            &RouteStamp::default()
        )
        .code,
        "kind_unsupported"
    );
}

#[test]
fn missing_scope_is_route_unscoped_before_unknown_arguments() {
    let mut args = arguments();
    args["owner"] = true.into();
    assert_eq!(
        code(&args, None, &RouteStamp::default()).code,
        "route_unscoped"
    );
}

#[test]
fn provenance_arguments_are_unknown_argument_and_selected_in_name_order() {
    for name in ["owner", "ref", "session", "harness"] {
        let mut args = arguments();
        args[name] = "spoofed".into();
        assert_eq!(
            code(&args, None, &stamp()),
            Refusal::member("unknown_argument", name)
        );
    }
    assert_eq!(
        code(&json!({"z":0,"a":0}), None, &stamp()),
        Refusal::member("unknown_argument", "a")
    );
}

#[test]
fn absent_kind_is_argument_missing_before_invalid_members() {
    assert_eq!(
        code(&json!({"data":false}), None, &stamp()),
        Refusal::member("argument_missing", "kind")
    );
    for name in REQUIRED {
        let mut args = arguments();
        args.as_object_mut().unwrap().remove(name);
        assert_eq!(
            code(&args, None, &stamp()),
            Refusal::member("argument_missing", name)
        );
    }
}

#[test]
fn unknown_kind_is_argument_invalid() {
    let mut args = arguments();
    args["kind"] = "foo".into();
    assert_eq!(code(&args, None, &stamp()).code, "argument_invalid");
}

#[test]
fn data_leading_zero_is_argument_invalid() {
    let mut args = arguments();
    args["data"] = "$$tc(01)".into();
    assert_eq!(code(&args, None, &stamp()), invalid("data"));
}

#[test]
fn data_past_u64_is_argument_invalid() {
    let mut args = arguments();
    args["data"] = "$$tc(18446744073709551616)".into();
    assert_eq!(code(&args, None, &stamp()), invalid("data"));
    for value in ["$$tc(0)", "$$tc(18446744073709551615)"] {
        args["data"] = value.into();
        assert_eq!(code(&args, None, &stamp()).code, "data_reference_unknown");
    }
    for value in [
        "$$tc()",
        "$$tc(-1)",
        "$$tc(+1)",
        "$$tc( 1)",
        "$$tc(1)tail",
        "$$tc(١)",
    ] {
        args["data"] = value.into();
        assert_eq!(code(&args, None, &stamp()), invalid("data"));
    }
}

#[test]
fn wrong_member_types_and_values_are_argument_invalid_in_name_order() {
    for name in MEMBERS {
        let mut args = arguments();
        args[name] = Value::Null;
        assert_eq!(code(&args, None, &stamp()), invalid(name));
    }
    for (name, value) in [
        ("data_format", json!("xml")),
        ("data_columns", json!({"x":"date"})),
        ("data_columns", json!({"x":1})),
    ] {
        let mut args = arguments();
        args[name] = value;
        assert_eq!(code(&args, None, &stamp()), invalid(name));
    }
    let mut args = arguments();
    args["title"] = false.into();
    args["data"] = false.into();
    assert_eq!(code(&args, None, &stamp()), invalid("data"));
}

#[test]
fn oversized_data_columns_is_argument_invalid_at_canonical_byte_limit() {
    let mut args = arguments();
    args["data"] = "$$tc(7)".into();
    args["data_format"] = "csv".into();
    let overhead = serde_json::to_vec(&json!({"":"text"})).unwrap().len();
    for (length, expected) in [
        (1_048_576, "data_reference_unknown"),
        (1_048_577, "argument_invalid"),
    ] {
        args["data_columns"] = Value::Object(Map::from_iter([(
            "x".repeat(length - overhead),
            "text".into(),
        )]));
        assert_eq!(
            serde_json::to_vec(&args["data_columns"]).unwrap().len(),
            length
        );
        assert_eq!(code(&args, None, &stamp()).code, expected);
    }
}

#[test]
fn data_format_without_data_is_argument_forbidden() {
    let mut args = arguments();
    args["data_format"] = "json".into();
    assert_eq!(
        code(&args, Some(&descriptor(4)), &stamp()),
        Refusal::member("argument_forbidden", "data_format")
    );
}

#[test]
fn data_columns_without_data_format_is_argument_forbidden() {
    let mut args = arguments();
    args["data_columns"] = json!({});
    assert_eq!(
        code(&args, None, &stamp()),
        Refusal::member("argument_forbidden", "data_columns")
    );
}

#[test]
fn malformed_output_descriptor_is_argument_invalid_before_forbidden_arguments() {
    for refs in [Value::Null, json!([]), json!("descriptor")] {
        assert_eq!(
            code(&arguments(), Some(&refs), &stamp()),
            invalid("output_refs")
        );
    }
    for name in ["set_id", "mid", "dialect", "count", "digest"] {
        let mut refs = descriptor(0);
        refs.as_object_mut().unwrap().remove(name);
        assert_eq!(
            code(&arguments(), Some(&refs), &stamp()),
            invalid(&format!("output_refs.{name}"))
        );
        let mut refs = descriptor(0);
        refs[name] = false.into();
        assert_eq!(
            code(&arguments(), Some(&refs), &stamp()),
            invalid(&format!("output_refs.{name}"))
        );
    }
    for (member, value) in [
        ("set_id", json!("")),
        ("mid", json!("")),
        ("dialect", json!("")),
        ("count", json!(-1)),
        ("count", json!(1.0)),
        ("digest", json!("A".repeat(64))),
        ("digest", json!("g".repeat(64))),
        ("digest", json!("a".repeat(63))),
        ("call_key", json!(null)),
        ("call_key", json!("")),
    ] {
        let mut refs = descriptor(0);
        refs[member] = value;
        assert_eq!(
            code(&arguments(), Some(&refs), &stamp()),
            invalid(&format!("output_refs.{member}"))
        );
    }
    let mut args = arguments();
    args["data_columns"] = json!({});
    assert_eq!(
        code(&args, Some(&json!({})), &stamp()).code,
        "argument_invalid"
    );
    let mut args = arguments();
    args["title"] = false.into();
    assert_eq!(
        code(&args, Some(&json!({})), &stamp()),
        invalid("output_refs.count")
    );
    args["data"] = false.into();
    assert_eq!(code(&args, Some(&json!({})), &stamp()), invalid("data"));
}

#[test]
fn descriptor_cap_is_4096_compact_bytes_and_unknown_members_are_open() {
    let mut refs = descriptor(0);
    refs["future"] = "".into();
    let overhead = serde_json::to_vec(&refs).unwrap().len();
    refs["future"] = "x".repeat(4096 - overhead).into();
    assert!(admit(&arguments(), Some(&refs), &stamp()).is_ok());
    let pretty = serde_json::to_vec_pretty(&refs).unwrap();
    assert!(pretty.len() > 4096);
    assert!(prepare(
        &arguments(),
        Some(&pretty),
        &stamp(),
        0,
        Decimal::from_integer(0).unwrap()
    )
    .is_ok());
    refs["future"] = "x".repeat(4097 - overhead).into();
    assert_eq!(
        code(&arguments(), Some(&refs), &stamp()),
        invalid("output_refs")
    );
}

#[test]
fn nonempty_descriptor_is_references_unavailable_before_data_reference_unknown() {
    let mut args = arguments();
    args["data"] = "$$tc(7)".into();
    assert_eq!(
        code(&args, Some(&descriptor(4)), &stamp()).code,
        "references_unavailable"
    );
}

#[test]
fn data_with_no_records_is_data_reference_unknown() {
    let mut args = arguments();
    args["data"] = "$$tc(7)".into();
    for refs in [None, Some(descriptor(0))] {
        assert_eq!(
            code(&args, refs.as_ref(), &stamp()).code,
            "data_reference_unknown"
        );
    }
}

#[test]
fn prepared_call_preserves_source_and_empty_trial_inputs_without_fetching() {
    for refs in [None, Some(descriptor(0))] {
        let prepared = admit(&arguments(), refs.as_ref(), &stamp()).unwrap();
        assert_eq!(prepared.source.as_bytes(), "source\n雪".as_bytes());
        assert_eq!(prepared.context.profile, Profile::ShowUi);
        assert_eq!(prepared.context.execution_dialect, "1.0");
        assert!(prepared.context.registry.0.is_empty());
        assert!(prepared.context.references.is_empty());
        assert!(prepared.context.tool.is_none());
        assert!(prepared.context.handles.is_empty());
        assert_eq!(prepared.context.show_ui.title, "Title 🦉");
        assert_eq!(prepared.context.show_ui.summary, "Summary");
        assert!(prepared.host.data.is_none());
        assert!(prepared.host.snapshots.is_empty());
        assert!(!prepared.host.network);
        assert_eq!(prepared.host.zone, "UTC");
        assert_eq!(prepared.host.now, Decimal::from_integer(1234).unwrap());
        assert_eq!(prepared.now_ms, 1234);
        assert!(prepared.host.data.is_none());
        assert!(prepared.context.show_ui.data_format.is_none());
    }
}
