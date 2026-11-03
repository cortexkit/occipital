use super::*;

/// Adds pruning metadata only to pages serving the affected stored messages.
struct PrunedRoute {
    inner: Arc<FakeRoute>,
    pruned: Vec<Value>,
}

#[async_trait]
impl BrocaRoute for PrunedRoute {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let mut page = self.inner.read(op, params).await?;
        if let Some(messages) = page["messages"].as_array() {
            let pruned: Vec<_> = self
                .pruned
                .iter()
                .filter(|part| messages.iter().any(|entry| entry["mid"] == part["mid"]))
                .cloned()
                .collect();
            if !pruned.is_empty() {
                page["pruned_reasoning"] = json!(pruned);
            }
        }
        Ok(page)
    }

    async fn subscribe(&self, live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        self.inner.subscribe(live_ids).await
    }
}

/// The runner's part keys are its own opaque strings: `m20` serves them in
/// `part_digests`, `m22` in `part_keys`, the form a pruning runner uses for an
/// entry without digests. Neither can be derived from a part's position.
fn route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    let digests: Vec<_> = ["r-a", "r-b", "r-c", "t-d", "c-e", "r-f"]
        .into_iter()
        .map(|part| json!({"part":part, "digest":"b".repeat(64)}))
        .collect();
    *route.entries.lock().unwrap() = vec![
        json!({"mid":"m20", "ordinal":20, "part_digests":digests, "message":{"role":"assistant", "content":[
            {"type":"reasoning", "text":""},
            {"type":"reasoning", "text":""},
            {"type":"reasoning", "text":"Readable summary"},
            {"type":"text", "text":"Checking."},
            {"type":"tool_call", "tool_call_id":"call_date", "tool_name":"bash", "input":{"command":"date"}},
            {"type":"reasoning", "text":"Unchanged reasoning"}]}}),
        json!({"mid":"m21", "ordinal":21, "message":{"role":"tool", "content":[
            {"type":"tool_result", "tool_call_id":"call_date", "output":"2026"}]}}),
        json!({"mid":"m22", "ordinal":22, "part_keys":["s-a", "s-b"], "message":{"role":"assistant", "content":[
            {"type":"reasoning", "text":""}, {"type":"text", "text":"Done."}]}}),
    ];
    route
}

fn pruning() -> Vec<Value> {
    [
        ("m20", "r-b", "reasoning"),
        ("m20", "r-c", "opaque_reasoning"),
        ("m20", "t-d", "signature"),
        ("m20", "c-e", "signature"),
        ("m22", "s-a", "reasoning"),
        // A key built from the ordinal and position, which the entry doesn't
        // serve, names nothing.
        ("m20", "m20#0", "reasoning"),
        // Neither a matching mid alone nor a served key alone names a part.
        ("m20", "s-b", "reasoning"),
        ("another-mid", "r-a", "reasoning"),
    ]
    .into_iter()
    .map(|(mid, part, kind)| {
        json!({"mid":mid, "part":part, "kind":kind, "bytes":80, "sha256":"a".repeat(64)})
    })
    .collect()
}

#[tokio::test]
async fn pruned_reasoning_is_hidden_without_moving_surviving_parts() {
    let source = BrocaSource::new(
        resolved(),
        Arc::new(PrunedRoute {
            inner: route(),
            pruned: pruning(),
        }),
    );
    // The fake serves at most two entries per read, so metadata must survive merging pages.
    let page = source.newest(3).await.unwrap();
    let parts = &page.rows[0].display["parts"];
    assert_eq!(parts.as_array().unwrap().len(), 5);
    assert_eq!(
        parts[0],
        json!({"type":"reasoning", "text":"", "partKey":"0", "partIndex":0})
    );
    assert_eq!(
        parts[1],
        json!({"type":"reasoning", "text":"Readable summary", "partKey":"2", "partIndex":2})
    );
    assert_eq!(
        parts[2],
        json!({"type":"text", "text":"Checking.", "partKey":"3", "partIndex":3})
    );
    assert_eq!(
        parts[3],
        json!({"type":"tool", "tool":"bash", "arguments":"{\"command\":\"date\"}",
        "argumentsTruncated":false, "outputBytes":4, "partKey":"4", "partIndex":4, "callId":"call_date"})
    );
    assert_eq!(
        parts[4],
        json!({"type":"reasoning", "text":"Unchanged reasoning", "partKey":"5", "partIndex":5})
    );
    assert_eq!(page.rows[0].display["toolCalls"][0]["callId"], "call_date");
    assert_eq!(
        page.rows[2].display["parts"],
        json!([
        {"type":"text", "text":"Done.", "partKey":"1", "partIndex":1}])
    );

    let history = source
        .history_window(
            source.window_head().await.unwrap(),
            HistoryRequest::from_ordinal(20).with_limit(3),
        )
        .await
        .unwrap();
    for (raw, numbered) in history.rows.iter().zip(&page.rows) {
        assert_eq!(raw.display, numbered.display);
    }
    let exact = source.exact(&Anchor::Ordinal(20)).await.unwrap().unwrap();
    let exact_parts = exact.display["parts"].as_array().unwrap();
    assert_eq!(exact_parts.len(), 5);
    for (full, paged) in exact_parts.iter().zip(parts.as_array().unwrap()) {
        assert_eq!(full["partKey"], paged["partKey"]);
        assert_eq!(full["partIndex"], paged["partIndex"]);
        assert_eq!(full["callId"], paged["callId"]);
    }
    assert_eq!(exact_parts[1]["text"], "Readable summary");
    assert_eq!(exact_parts[3]["content"], "2026");
}

#[tokio::test]
async fn a_page_without_pruning_metadata_keeps_the_original_projection_bytes() {
    let route = route();
    let entries = route.entries.lock().unwrap().clone();
    let source = BrocaSource::new(resolved(), route);
    let page = source.newest(3).await.unwrap();
    let expected = broca_entries::rows(&entries, false).unwrap();
    for (mut actual, expected) in page.rows.into_iter().zip(expected) {
        actual.display.as_object_mut().unwrap().remove("network");
        assert_eq!(
            serde_json::to_vec(&actual.display).unwrap(),
            serde_json::to_vec(&expected.display).unwrap()
        );
    }
    assert_eq!(
        source
            .exact(&Anchor::Ordinal(20))
            .await
            .unwrap()
            .unwrap()
            .display["parts"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
}
