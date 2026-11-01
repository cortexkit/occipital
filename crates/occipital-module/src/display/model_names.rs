//! `display.model_names`: each model id as the apps show it, such as
//! `anthropic/claude-opus-5-5` as "Claude Opus 5.5" by "Anthropic".
//!
//! Both apps ask here, so they always show a model the same way and the phone
//! needs no route of its own to the model catalog. The names come from
//! fusiform's model catalog (`catalog.get`, the `model.name` and
//! `provider.name` facts), which keys each model `<provider>/<model>`. Core's
//! model ids can differ from those keys in two ways, both undone here:
//!
//! - **Effort:** an id may end in `:<variant>`, such as `openai/gpt-6.1-sol:xhigh`.
//!   The variant is answered beside the name, for the app to show.
//! - **Credential providers:** an id may name the provider the person's
//!   credential is filed under, such as `anthropic-platform`. Routing's
//!   `route.catalog_targets` maps each to its catalog provider; Occipital asks
//!   it rather than keep a copy of the rule.
//!
//! An id that still matches no model, a bare id without its provider (the
//! same model id exists under many providers), and a retired model (its last
//! name isn't served) get an empty answer, and the app shows the id itself.
//!
//! The whole catalog's names are read at once and kept for 30 minutes.
//! fusiform's `catalog_version` doesn't move when only curated names change,
//! so the age alone decides when to read again. A failed read keeps serving the
//! names already held.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use occipital_app_reader::source::ReadError;
use serde_json::{json, Map, Value};
use subc_protocol::RouteTarget;
use tokio::time::{Duration, Instant};

use super::daemon::{CallFailure, DaemonCalls};

pub const OPERATION: &str = "display.model_names";

/// The most ids one request asks about.
pub const MAX_IDS: usize = 64;

/// The longest id accepted.
const MAX_ID_BYTES: usize = 256;

/// How long the names read from the catalog are served before they are read
/// again.
const FRESH: Duration = Duration::from_secs(30 * 60);

/// How soon names are read again when routing's provider map couldn't be read
/// with them, such as while routing starts up.
const FRESH_WITHOUT_ALIASES: Duration = Duration::from_secs(60);

/// The most catalog pages one read follows, so a catalog that kept handing
/// out page tokens can't hold a request forever.
const MAX_PAGES: usize = 1_000;

/// How many times a read starts over after the catalog changed under it.
const MAX_RESTARTS: usize = 3;

/// Where the names and the provider map come from.
#[async_trait]
pub trait NameSources: Send + Sync {
    /// One page of fusiform's `catalog.get` reply for the two name facts,
    /// starting at `page_token`.
    async fn names_page(&self, page_token: Option<&str>) -> Result<Value, CallFailure>;
    /// Routing's `route.catalog_targets` reply, `{"result": {targets, …}}`.
    async fn catalog_targets(&self) -> Result<Value, CallFailure>;
}

/// The names as one read of the catalog left them.
struct Names {
    /// `<provider>/<model>` to the model's name and its provider's name.
    models: HashMap<String, (String, Option<String>)>,
    /// Credential provider to catalog provider.
    aliases: HashMap<String, String>,
    fresh_until: Instant,
}

pub struct ModelNames {
    sources: Arc<dyn NameSources>,
    held: tokio::sync::Mutex<Option<Arc<Names>>>,
}

impl ModelNames {
    pub fn new(sources: Arc<dyn NameSources>) -> Self {
        Self {
            sources,
            held: tokio::sync::Mutex::new(None),
        }
    }

    /// `display.model_names {ids}`: `{"models": {id: answer}}`, where an
    /// answer is `{name, provider?, variant?}`, or `{}` when the id names no
    /// model the catalog can name.
    pub async fn answer(&self, params: &Value) -> Result<Value, ReadError> {
        let ids = ids(params)?;
        let names = self.names().await?;
        let models: Map<String, Value> = ids
            .into_iter()
            .map(|id| {
                let answer = names.resolve(&id);
                (id, answer)
            })
            .collect();
        Ok(json!({ "models": models }))
    }

    /// The names held, read again first when they are old. A read that fails
    /// keeps serving the names held, which are only ever old, never wrong.
    async fn names(&self) -> Result<Arc<Names>, ReadError> {
        let mut held = self.held.lock().await;
        if let Some(names) = held
            .as_ref()
            .filter(|names| Instant::now() < names.fresh_until)
        {
            return Ok(names.clone());
        }
        match self.read(held.as_deref()).await {
            Ok(names) => {
                let names = Arc::new(names);
                *held = Some(names.clone());
                Ok(names)
            }
            Err(failure) => match held.as_ref() {
                Some(names) => {
                    tracing::warn!(%failure, "cannot read model names again; serving the names held");
                    Ok(names.clone())
                }
                None => Err(ReadError::new(
                    "provider_unavailable",
                    format!("model names are unavailable: {failure}"),
                )
                .with_detail(json!({ "retryable": true }))),
            },
        }
    }

    /// Read every page of names, starting over when the catalog changes
    /// between pages, and routing's provider map beside them.
    async fn read(&self, previous: Option<&Names>) -> Result<Names, CallFailure> {
        let mut restarts = 0;
        let models = loop {
            match self.read_pages().await {
                Ok(models) => break models,
                Err(failure)
                    if failure.code.as_deref() == Some("catalog_changed")
                        && restarts < MAX_RESTARTS =>
                {
                    restarts += 1;
                }
                Err(failure) => return Err(failure),
            }
        };
        let (aliases, fresh_for) = match self.sources.catalog_targets().await {
            Ok(reply) => (targets(&reply)?, FRESH),
            Err(failure) => {
                tracing::warn!(%failure, "cannot read routing's provider map; reading it again in a minute");
                (
                    previous
                        .map(|names| names.aliases.clone())
                        .unwrap_or_default(),
                    FRESH_WITHOUT_ALIASES,
                )
            }
        };
        Ok(Names {
            models,
            aliases,
            fresh_until: Instant::now() + fresh_for,
        })
    }

    async fn read_pages(&self) -> Result<HashMap<String, (String, Option<String>)>, CallFailure> {
        let mut models = HashMap::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = self.sources.names_page(token.as_deref()).await?;
            let rows = page["models"].as_object().ok_or_else(|| CallFailure {
                code: None,
                message: "catalog.get replied without `models`".into(),
            })?;
            for (key, facts) in rows {
                if let Some(name) = text(&facts["model.name"]) {
                    models.insert(key.clone(), (name, text(&facts["provider.name"])));
                }
            }
            match page["next_page_token"].as_str() {
                Some(next) => token = Some(next.to_owned()),
                None => return Ok(models),
            }
        }
        Err(CallFailure {
            code: None,
            message: format!("catalog.get handed out more than {MAX_PAGES} pages"),
        })
    }
}

impl Names {
    fn resolve(&self, id: &str) -> Value {
        // The whole id first: a model id can itself contain `:`, such as a
        // Bedrock version, so only an id that doesn't match whole loses its
        // last `:` part as the variant.
        if let Some(answer) = self.named(id, None) {
            return answer;
        }
        if let Some((model, variant)) = id.rsplit_once(':') {
            if let Some(answer) = self.named(model, Some(variant)) {
                return answer;
            }
        }
        json!({})
    }

    fn named(&self, id: &str, variant: Option<&str>) -> Option<Value> {
        let (provider, model) = id.split_once('/')?;
        let found = self.models.get(id).or_else(|| {
            let target = self.aliases.get(provider)?;
            self.models.get(&format!("{target}/{model}"))
        })?;
        let (name, provider_name) = found;
        let mut answer = json!({ "name": name });
        if let Some(provider_name) = provider_name {
            answer["provider"] = json!(provider_name);
        }
        if let Some(variant) = variant.filter(|variant| !variant.is_empty()) {
            answer["variant"] = json!(variant);
        }
        Some(answer)
    }
}

/// A non-empty string fact.
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Routing's `targets`, credential provider to catalog provider.
fn targets(reply: &Value) -> Result<HashMap<String, String>, CallFailure> {
    let targets = reply["result"]["targets"]
        .as_object()
        .ok_or_else(|| CallFailure {
            code: None,
            message: "route.catalog_targets replied without `targets`".into(),
        })?;
    Ok(targets
        .iter()
        .filter_map(|(from, to)| Some((from.clone(), to.as_str()?.to_owned())))
        .collect())
}

/// The request's `ids`: 1 to 64 strings of 1 to 256 bytes, repeats dropped.
fn ids(params: &Value) -> Result<Vec<String>, ReadError> {
    let invalid = || {
        ReadError::new("invalid_params", "ids is missing or malformed")
            .with_detail(json!({ "field": "ids" }))
    };
    let items = params["ids"]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= MAX_IDS)
        .ok_or_else(invalid)?;
    let mut ids: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let id = item
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= MAX_ID_BYTES)
            .ok_or_else(invalid)?;
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// fusiform and routing, reached through the daemon as this module.
pub struct DaemonNameSources {
    calls: Arc<DaemonCalls>,
}

impl DaemonNameSources {
    pub fn new(calls: Arc<DaemonCalls>) -> Self {
        Self { calls }
    }
}

#[async_trait]
impl NameSources for DaemonNameSources {
    async fn names_page(&self, page_token: Option<&str>) -> Result<Value, CallFailure> {
        // A paged read, since an unpaged one over 4 MiB is refused, and no
        // `at_ms`, since a dated read leaves out the hand-added models.
        let mut arguments =
            json!({"fact_prefixes": ["model.name", "provider.name"], "paged": true});
        if let Some(token) = page_token {
            arguments["page_token"] = json!(token);
        }
        self.calls
            .call(
                RouteTarget::ToolProvider {
                    module_id: "fusiform".into(),
                },
                "model-names",
                &json!({"name": "catalog.get", "arguments": arguments}),
                None,
            )
            .await
    }

    async fn catalog_targets(&self) -> Result<Value, CallFailure> {
        self.calls
            .call(
                RouteTarget::ManagementSurface {
                    module_id: "prefrontal-routing".into(),
                },
                "model-names",
                &json!({"method": "route.catalog_targets", "params": {}}),
                None,
            )
            .await
    }
}

#[cfg(test)]
pub(crate) mod tests;
