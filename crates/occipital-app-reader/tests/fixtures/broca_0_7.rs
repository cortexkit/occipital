use serde_json::Value;

/// Always exercise the small fixtures; an optional directory adds exact server
/// captures without making machine-specific recordings part of the test suite.
pub fn samples(name: &str) -> Vec<(String, Value)> {
    let fixtures: Value = serde_json::from_str(include_str!("broca-0.7.json")).unwrap();
    let fixture = if let Some(event_name) = name.strip_suffix(".frame.json") {
        let event = fixtures[format!("{event_name}.json")].clone();
        assert!(event.is_object(), "missing fixture {name}");
        let unit = if event_name.contains("finished") {
            serde_json::json!({"type":"tool_result", "result":{
                "tool_call_id":event["tool_call_id"], "output":{"kind":"text","text":"result"}, "is_error":false
            }})
        } else {
            serde_json::json!({"type":"message_boundary","mid":"m1"})
        };
        serde_json::json!({"kind":"control", "seq":if event_name.contains("finished") {10} else {9},
            "cursor":{"wal_seq":5,"sub_index":0},
            "unit":unit, "event":event})
    } else {
        fixtures[name].clone()
    };
    assert!(fixture.is_object(), "missing fixture {name}");
    let mut samples = vec![(format!("fixture/{name}"), fixture)];
    if let Some(root) = std::env::var_os("THALAMUS_BROCA_0_7_SAMPLES") {
        let bytes = std::fs::read(std::path::Path::new(&root).join(name)).unwrap();
        samples.push((
            format!("capture/{name}"),
            serde_json::from_slice(&bytes).unwrap(),
        ));
    }
    samples
}
