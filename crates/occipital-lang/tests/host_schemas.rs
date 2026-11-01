use occipital_lang::{
    CheckContext, Operation, OperationRegistry, Profile, Schema, ShowUiArguments,
};
use serde_json::{json, Value};

fn schema(value: Value) -> Schema {
    serde_json::from_value(value).unwrap()
}

#[test]
fn provisional_schema_subset_validates_nested_types_required_enum_and_nullable() {
    let schema = schema(json!({
        "type":"object",
        "properties": {
            "phase":{"type":"string","enum":["running","finished"]},
            "exitCode":{"type":"number","nullable":true},
            "lines":{"type":"array","items":{"type":"string"}},
            "meta":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}
        },
        "required":["phase","exitCode"]
    }));
    assert!(schema
        .accepts(&json!({"phase":"running","exitCode":null,"lines":["a","b"],"meta":{"ok":true}})));
    assert!(schema
        .accepts(&json!({"phase":"finished","exitCode":0,"extra":"not forbidden by this subset"})));
    for value in [
        json!({"phase":"unknown","exitCode":null}),
        json!({"phase":"running"}),
        json!({"phase":"running","exitCode":"0"}),
        json!({"phase":"running","exitCode":null,"lines":[1]}),
        json!({"phase":"running","exitCode":null,"meta":{}}),
        json!([]),
    ] {
        assert!(!schema.accepts(&value), "{value}");
    }
    assert!(serde_json::from_value::<Schema>(
        json!({"type":"object","additionalProperties":false})
    )
    .is_err());
    assert!(serde_json::from_value::<Schema>(json!({"type":"integer"})).is_err());
}

#[test]
fn provisional_registry_and_show_ui_arguments_round_trip_without_live_authority() {
    let registry: OperationRegistry = serde_json::from_value(json!({
        "watch_test":{"kind":"watch","arguments":{"type":"object"},"item":{"type":"string"},"resume":true,"replay":false},
        "page_test":{"kind":"paged","arguments":{"type":"object"},"page":{"type":"object","properties":{"items":{"type":"array","items":{"type":"string"}},"next":{"type":"string","nullable":true}},"required":["items","next"]}}
    })).unwrap();
    assert!(matches!(
        registry.0["page_test"],
        Operation::Paged { item_key: None, .. }
    ));
    assert_eq!(
        serde_json::from_value::<OperationRegistry>(serde_json::to_value(&registry).unwrap())
            .unwrap(),
        registry
    );
    let args: ShowUiArguments = serde_json::from_value(json!({"title":"Results","summary":"Counts","data_format":"csv","data_columns":{"count":"number","ok":"boolean"}})).unwrap();
    assert_eq!(
        serde_json::to_value(args).unwrap()["data_columns"]["count"],
        "number"
    );
    let context = CheckContext::new(Profile::ShowUi);
    assert!(context.registry.0.is_empty());
    assert!(context.references.is_empty());
    assert!(context.handles.is_empty());
}

#[test]
fn enum_numbers_use_decimal_equality_not_json_token_spelling() {
    let schema = schema(json!({"type":"number","enum":[1]}));
    assert!(schema.accepts(&serde_json::from_str("1.0").unwrap()));
    assert!(!schema.accepts(&json!(2)));
}
