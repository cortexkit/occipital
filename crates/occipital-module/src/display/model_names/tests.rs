//! `display.model_names` against fusiform's and routing's own sample replies
//! (`fixtures/README.md` says where each comes from).

use std::collections::VecDeque;
use std::sync::Mutex;

use super::*;

pub(crate) fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/display/model_names/fixtures")
        .join(name);
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

/// Serves fixture replies as catalog pages, in order, and records each page
/// token it was asked with. Each queued reply is one call's answer.
#[derive(Default)]
pub(crate) struct Fake {
    pages: Mutex<VecDeque<Result<Value, CallFailure>>>,
    targets: Mutex<VecDeque<Result<Value, CallFailure>>>,
    asked: Mutex<Vec<Option<String>>>,
}

impl Fake {
    /// The three pages of the whole catalog: fusiform's first provider page
    /// (which hands out a token), then two single-model replies, the middle one
    /// given a token so the read goes on.
    pub(crate) fn catalog(&self) {
        let mut middle = fixture("fusiform-one-model-current.json");
        middle["next_page_token"] = json!("v1:test:page-3");
        let mut last = fixture("fusiform-provider-default-endpoint.json");
        let curated = fixture("fusiform-curated-decision-model.json");
        for (key, facts) in curated["models"].as_object().unwrap() {
            last["models"][key] = facts.clone();
        }
        let mut pages = self.pages.lock().unwrap();
        pages.push_back(Ok(fixture("fusiform-first-provider-page.json")));
        pages.push_back(Ok(middle));
        pages.push_back(Ok(last));
    }

    pub(crate) fn page(&self, reply: Result<Value, CallFailure>) {
        self.pages.lock().unwrap().push_back(reply);
    }

    pub(crate) fn targets(&self, reply: Result<Value, CallFailure>) {
        self.targets.lock().unwrap().push_back(reply);
    }
}

#[async_trait]
impl NameSources for Fake {
    async fn names_page(&self, page_token: Option<&str>) -> Result<Value, CallFailure> {
        self.asked
            .lock()
            .unwrap()
            .push(page_token.map(str::to_owned));
        self.pages
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(refused(None, "no page queued")))
    }

    async fn catalog_targets(&self) -> Result<Value, CallFailure> {
        self.targets
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(refused(None, "no reply queued")))
    }
}

fn refused(code: Option<&str>, message: &str) -> CallFailure {
    CallFailure {
        code: code.map(str::to_owned),
        message: message.into(),
    }
}

fn names(fake: &Arc<Fake>) -> ModelNames {
    ModelNames::new(fake.clone())
}

async fn ask(names: &ModelNames, ids: &[&str]) -> Value {
    names.answer(&json!({ "ids": ids })).await.unwrap()["models"].clone()
}

/// Every resolution rule on real catalog rows: a whole key, an effort suffix,
/// a credential provider routing maps, a model id holding `/`, a model without
/// a provider name, and the ids no model matches.
#[tokio::test(start_paused = true)]
async fn ids_resolve_to_the_catalogs_names() {
    let fake = Arc::new(Fake::default());
    fake.catalog();
    fake.targets(Ok(fixture("routing-resolved.json")));
    let names = names(&fake);
    let models = ask(
        &names,
        &[
            "anthropic/claude-sonnet-4-5",
            "openai/gpt-6.1-sol:xhigh",
            "anthropic-platform/claude-sonnet-4-5:high",
            "cloudflare-workers-ai/@cf/cloudflare/clef",
            "claude-sonnet-4-5",
            "poe/google/veo-3",
            "anthropic/claude-sonnet-4-5:",
            "anthropic/claude-sonnet-4-5",
        ],
    )
    .await;
    assert_eq!(
        models,
        json!({
            "anthropic/claude-sonnet-4-5":
                {"name": "Claude Sonnet 4.5 (latest)", "provider": "Anthropic"},
            "openai/gpt-6.1-sol:xhigh":
                {"name": "GPT-6.1 Sol", "provider": "OpenAI", "variant": "xhigh"},
            "anthropic-platform/claude-sonnet-4-5:high":
                {"name": "Claude Sonnet 4.5 (latest)", "provider": "Anthropic", "variant": "high"},
            "cloudflare-workers-ai/@cf/cloudflare/clef": {"name": "Clef"},
            "claude-sonnet-4-5": {},
            "poe/google/veo-3": {},
            "anthropic/claude-sonnet-4-5:":
                {"name": "Claude Sonnet 4.5 (latest)", "provider": "Anthropic"},
        })
    );
    // The read followed each page token in turn.
    assert_eq!(
        *fake.asked.lock().unwrap(),
        [
            None,
            Some("v1:0:1000:000-page".to_owned()),
            Some("v1:test:page-3".to_owned())
        ]
    );
}

/// An id that matches whole keeps its `:` part: a model id can itself hold one.
#[tokio::test(start_paused = true)]
async fn a_colon_inside_a_model_id_is_not_taken_as_a_variant() {
    let fake = Arc::new(Fake::default());
    let mut page = fixture("fusiform-one-model-current.json");
    page["models"]["amazon-bedrock/anthropic.claude-v2:1"] =
        json!({"model.name": "Claude 2.1", "provider.name": "Amazon Bedrock"});
    fake.page(Ok(page));
    fake.targets(Ok(fixture("routing-resolved.json")));
    let models = ask(&names(&fake), &["amazon-bedrock/anthropic.claude-v2:1"]).await;
    assert_eq!(
        models["amazon-bedrock/anthropic.claude-v2:1"],
        json!({"name": "Claude 2.1", "provider": "Amazon Bedrock"})
    );
}

/// Names are kept for 30 minutes, then read again whole.
#[tokio::test(start_paused = true)]
async fn names_are_read_once_and_again_after_thirty_minutes() {
    let fake = Arc::new(Fake::default());
    fake.page(Ok(fixture("fusiform-one-model-current.json")));
    fake.targets(Ok(fixture("routing-resolved.json")));
    let names = names(&fake);
    ask(&names, &["anthropic/claude-sonnet-4-5"]).await;
    tokio::time::advance(FRESH - Duration::from_secs(1)).await;
    ask(&names, &["anthropic/claude-sonnet-4-5"]).await;
    assert_eq!(
        fake.asked.lock().unwrap().len(),
        1,
        "served from the names held"
    );

    tokio::time::advance(Duration::from_secs(2)).await;
    fake.page(Ok(fixture("fusiform-provider-default-endpoint.json")));
    fake.targets(Ok(fixture("routing-resolved.json")));
    let models = ask(
        &names,
        &["anthropic/claude-sonnet-4-5", "openai/gpt-6.1-sol"],
    )
    .await;
    assert_eq!(fake.asked.lock().unwrap().len(), 2);
    // The new read replaced the old names whole.
    assert_eq!(models["anthropic/claude-sonnet-4-5"], json!({}));
    assert_eq!(models["openai/gpt-6.1-sol"]["name"], "GPT-6.1 Sol");
}

/// A catalog that changes between pages is read again from page one.
#[tokio::test(start_paused = true)]
async fn a_catalog_change_mid_read_starts_over() {
    let fake = Arc::new(Fake::default());
    fake.page(Ok(fixture("fusiform-first-provider-page.json")));
    fake.page(Err(refused(Some("catalog_changed"), "the catalog changed")));
    fake.page(Ok(fixture("fusiform-one-model-current.json")));
    fake.targets(Ok(fixture("routing-resolved.json")));
    let models = ask(
        &names(&fake),
        &["anthropic/claude-sonnet-4-5", "000-page/m"],
    )
    .await;
    assert_eq!(
        *fake.asked.lock().unwrap(),
        [None, Some("v1:0:1000:000-page".to_owned()), None]
    );
    // Only the completed read's rows count.
    assert_eq!(models["000-page/m"], json!({}));
    assert_eq!(
        models["anthropic/claude-sonnet-4-5"]["provider"],
        "Anthropic"
    );
}

/// While routing warms up, names still resolve without its provider map, and
/// the read is repeated a minute later to pick the map up.
#[tokio::test(start_paused = true)]
async fn a_warming_router_leaves_aliases_out_for_a_minute() {
    let fake = Arc::new(Fake::default());
    let warming = fixture("routing-warming.json");
    assert_eq!(warming["code"], "routing_not_ready");
    fake.page(Ok(fixture("fusiform-one-model-current.json")));
    fake.targets(Err(refused(
        Some("routing_not_ready"),
        "routing is warming",
    )));
    let names = names(&fake);
    let alias = "anthropic-platform/claude-sonnet-4-5";
    let models = ask(&names, &[alias, "anthropic/claude-sonnet-4-5"]).await;
    assert_eq!(models[alias], json!({}));
    assert_eq!(
        models["anthropic/claude-sonnet-4-5"]["name"],
        "Claude Sonnet 4.5 (latest)"
    );

    tokio::time::advance(FRESH_WITHOUT_ALIASES + Duration::from_secs(1)).await;
    fake.page(Ok(fixture("fusiform-one-model-current.json")));
    fake.targets(Ok(fixture("routing-resolved.json")));
    let models = ask(&names, &[alias]).await;
    assert_eq!(models[alias]["provider"], "Anthropic");
}

/// A failed read serves the names held; with none held it is a retryable
/// refusal.
#[tokio::test(start_paused = true)]
async fn a_failed_read_serves_the_names_held_or_refuses_retryably() {
    let fake = Arc::new(Fake::default());
    fake.page(Err(refused(None, "fusiform is restarting")));
    let names = names(&fake);
    let refusal = names
        .answer(&json!({"ids": ["anthropic/claude-sonnet-4-5"]}))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, "provider_unavailable");
    assert_eq!(refusal.detail, Some(json!({"retryable": true})));

    fake.page(Ok(fixture("fusiform-one-model-current.json")));
    fake.targets(Ok(fixture("routing-resolved.json")));
    ask(&names, &["anthropic/claude-sonnet-4-5"]).await;
    tokio::time::advance(FRESH + Duration::from_secs(1)).await;
    fake.page(Err(refused(None, "fusiform is restarting")));
    let models = ask(&names, &["anthropic/claude-sonnet-4-5"]).await;
    assert_eq!(
        models["anthropic/claude-sonnet-4-5"]["name"],
        "Claude Sonnet 4.5 (latest)"
    );
}

#[tokio::test(start_paused = true)]
async fn malformed_ids_are_refused() {
    let fake = Arc::new(Fake::default());
    let names = names(&fake);
    let too_many: Vec<String> = (0..=MAX_IDS).map(|n| format!("p/m{n}")).collect();
    for params in [
        json!({}),
        json!({"ids": []}),
        json!({"ids": "anthropic/claude"}),
        json!({"ids": [1]}),
        json!({"ids": [""]}),
        json!({"ids": ["x".repeat(MAX_ID_BYTES + 1)]}),
        json!({ "ids": too_many }),
    ] {
        let refusal = names.answer(&params).await.unwrap_err();
        assert_eq!(refusal.code, "invalid_params", "{params}");
    }
    assert!(fake.asked.lock().unwrap().is_empty(), "nothing was read");
}
