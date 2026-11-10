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
//!   it rather than keep a copy of the rule. Routing is moving from module id
//!   `prefrontal-routing` to `striatum`, so it is asked under either
//!   (`ask_routing`).
//!
//! An id that still matches no model, a bare id without its provider (the
//! same model id exists under many providers), and a retired model (its last
//! name isn't served) get an empty answer, and the app shows the id itself.
//!
//! The whole catalog's names are read at once and kept for 30 minutes.
//! fusiform's `catalog_version` doesn't move when only curated names change,
//! so the age alone decides when to read again. A failed read keeps serving the
//! names already held.
//!
//! The same read takes each model's attachment limits, which a session's
//! details report and its sends are held to ([`ModelNames::attachments`]).

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

/// How long a session's details or a send waits for the first catalog read
/// after the module starts, before taking the fallback limits. Later reads
/// happen in the background, so only that first one is ever waited for.
const FIRST_READ_WAIT: Duration = Duration::from_secs(2);

/// The facts one read of the catalog asks for: the names, and the attachment
/// limits.
const FACTS: [&str; 5] = [
    "model.name",
    "provider.name",
    IMAGES_PER_REQUEST,
    IMAGE_BYTES,
    IMAGE_LONG_EDGE_PX,
];
const IMAGES_PER_REQUEST: &str = "limit.images_per_request";
const IMAGE_BYTES: &str = "limit.image_bytes";
const IMAGE_LONG_EDGE_PX: &str = "limit.image_long_edge_px";

/// How many images, and how many other files, a message may carry when the
/// catalog states no count for its model. The apps use the same numbers.
pub const FALLBACK_FILES: u64 = 4;

/// Routing's module ids: the one it serves under today, then the one it is
/// moving to. Until the move only the first exists; after it, only the second.
const ROUTING: [&str; 2] = ["prefrontal-routing", "striatum"];

/// The refusals that mean routing isn't serving under that id right now, so
/// the other id is asked: no module has the id, or its module is stopped or
/// warming up. Any other refusal is routing's answer.
const ROUTING_ABSENT: [&str; 3] = ["unknown_module", "target_unavailable", "module_warming"];

/// How long a route to one routing id may take to open. A stopped or warming
/// module is asked again only within this, rather than for the daemon client's
/// default 90 s, since the read holds up an app's request and the other id may
/// be the one serving.
const ROUTING_OPEN: Duration = Duration::from_secs(1);

/// Ask routing under its current id, and under the id it is moving to when
/// the first isn't serving (`ROUTING_ABSENT`). When neither is, the read fails
/// with both refusals; the names are then read again a minute later
/// (`FRESH_WITHOUT_ALIASES`), which is the wait before asking the pair again.
async fn ask_routing<F, Asked>(mut ask: F) -> Result<Value, CallFailure>
where
    F: FnMut(&'static str) -> Asked,
    Asked: std::future::Future<Output = Result<Value, CallFailure>>,
{
    let [current, moving_to] = ROUTING;
    let first = match ask(current).await {
        Err(failure) if absent(&failure) => failure,
        answer => return answer,
    };
    ask(moving_to).await.map_err(|second| CallFailure {
        message: format!(
            "{current}: {}; {moving_to}: {}",
            first.message, second.message
        ),
        code: second.code,
    })
}

fn absent(failure: &CallFailure) -> bool {
    failure
        .code
        .as_deref()
        .is_some_and(|code| ROUTING_ABSENT.contains(&code))
}

/// Where the names and the provider map come from.
#[async_trait]
pub trait NameSources: Send + Sync {
    /// One page of fusiform's `catalog.get` reply for the facts in `FACTS`,
    /// starting at `page_token`.
    async fn names_page(&self, page_token: Option<&str>) -> Result<Value, CallFailure>;
    /// Routing's `route.catalog_targets` reply, `{"result": {targets, …}}`.
    async fn catalog_targets(&self) -> Result<Value, CallFailure>;
}

/// The names as one read of the catalog left them.
struct Names {
    /// `<provider>/<model>` to what the catalog says of the model.
    models: HashMap<String, Model>,
    /// Credential provider to catalog provider.
    aliases: HashMap<String, String>,
    fresh_until: Instant,
}

/// One model's row: its name and its provider's, when served, and its
/// attachment limits, each absent when the catalog states none.
#[derive(Debug, Default)]
struct Model {
    name: Option<String>,
    provider: Option<String>,
    images_per_request: Option<u64>,
    image_bytes: Option<u64>,
    image_long_edge_px: Option<u64>,
}

/// What a message to a session may carry: `max_images` files whose mime
/// starts with `image/`, and `max_files` others, counted separately; and, when
/// known, the most bytes and the longest edge in pixels of each image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentLimits {
    pub max_images: u64,
    pub max_files: u64,
    pub image_bytes: Option<u64>,
    pub image_long_edge_px: Option<u64>,
}

impl AttachmentLimits {
    /// The limits when nothing is known of the model.
    pub fn fallback() -> Self {
        Self {
            max_images: FALLBACK_FILES,
            max_files: FALLBACK_FILES,
            image_bytes: None,
            image_long_edge_px: None,
        }
    }

    /// As `session.details` carries it: both counts always, each image bound
    /// only when known.
    pub fn to_json(&self) -> Value {
        let mut value = json!({"maxImages": self.max_images, "maxFiles": self.max_files});
        if let Some(bytes) = self.image_bytes {
            value["imageBytes"] = json!(bytes);
        }
        if let Some(edge) = self.image_long_edge_px {
            value["imageLongEdgePx"] = json!(edge);
        }
        value
    }
}

/// [`ModelNames::attachments`] when the catalog can be read, and otherwise
/// the fallbacks with the runner's own image bounds. The one place the session
/// details and the send path get their limits, so the two always agree.
pub async fn attachment_limits(
    names: Option<&Arc<ModelNames>>,
    model: Option<&Value>,
    runner_image_bytes: Option<u64>,
    runner_image_edge: Option<u64>,
) -> AttachmentLimits {
    match names {
        Some(names) => {
            names
                .attachments(model, runner_image_bytes, runner_image_edge)
                .await
        }
        None => limits(None, runner_image_bytes, runner_image_edge),
    }
}

/// The smaller of two bounds, either of which may be unknown.
fn stricter(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub struct ModelNames {
    sources: Arc<dyn NameSources>,
    /// The names held, locked for the whole of a read, so only one read runs.
    held: tokio::sync::Mutex<Option<Arc<Names>>>,
    /// The latest names a read left, for the attachment limits, which never
    /// wait on a read in progress once one has finished.
    latest: tokio::sync::watch::Sender<Option<Arc<Names>>>,
}

impl ModelNames {
    pub fn new(sources: Arc<dyn NameSources>) -> Self {
        Self {
            sources,
            held: tokio::sync::Mutex::new(None),
            latest: tokio::sync::watch::channel(None).0,
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

    /// What a message to a session whose head names `model` (`{provider,
    /// model, variant?}`) may carry. The image count comes from the catalog's
    /// `limit.images_per_request` for the model, else [`FALLBACK_FILES`];
    /// other files are always held to [`FALLBACK_FILES`], since the catalog
    /// states no count for them. Each image bound is the stricter of the
    /// catalog's and the runner's own (`runner_image_bytes` and
    /// `runner_image_edge`, from the head's `input`), because the runner refuses
    /// an image above its own limit whatever the catalog says.
    ///
    /// It never fails, and it never waits on a catalog read once one has
    /// finished: old names are used while a read in the background replaces
    /// them. Before the first read finishes it waits up to [`FIRST_READ_WAIT`];
    /// with no names by then, the fallbacks apply, the same limits the apps
    /// assume for a model they know nothing of.
    pub async fn attachments(
        self: &Arc<Self>,
        model: Option<&Value>,
        runner_image_bytes: Option<u64>,
        runner_image_edge: Option<u64>,
    ) -> AttachmentLimits {
        let mut latest = self.latest.subscribe();
        let current = latest.borrow_and_update().clone();
        let old = current
            .as_ref()
            .is_none_or(|names| Instant::now() >= names.fresh_until);
        // A held lock is a read already running; it will publish its names.
        if old && self.held.try_lock().is_ok() {
            let names = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(error) = names.names().await {
                    tracing::warn!(error = %error.message, "cannot read the model catalog for attachment limits");
                }
            });
        }
        let names = match current {
            Some(names) => Some(names),
            None => tokio::time::timeout(FIRST_READ_WAIT, latest.wait_for(Option::is_some))
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|names| (*names).clone()),
        };
        if names.is_none() {
            tracing::warn!("no model catalog read yet; using the fallback attachment limits");
        }
        let row = names.as_deref().and_then(|names| {
            let model = model?;
            let id = format!(
                "{}/{}",
                model["provider"].as_str()?,
                model["model"].as_str()?
            );
            names.find(&id)
        });
        limits(row, runner_image_bytes, runner_image_edge)
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
                self.latest.send_replace(Some(names.clone()));
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

    async fn read_pages(&self) -> Result<HashMap<String, Model>, CallFailure> {
        let mut models = HashMap::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = self.sources.names_page(token.as_deref()).await?;
            let rows = page["models"].as_object().ok_or_else(|| CallFailure {
                code: None,
                message: "catalog.get replied without `models`".into(),
            })?;
            for (key, facts) in rows {
                models.insert(
                    key.clone(),
                    Model {
                        name: text(&facts["model.name"]),
                        provider: text(&facts["provider.name"]),
                        images_per_request: facts[IMAGES_PER_REQUEST].as_u64(),
                        image_bytes: facts[IMAGE_BYTES].as_u64(),
                        image_long_edge_px: facts[IMAGE_LONG_EDGE_PX].as_u64(),
                    },
                );
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

    /// The row a `<provider>/<model>` id names: the id itself, or with its
    /// provider mapped to the catalog's by routing.
    fn find(&self, id: &str) -> Option<&Model> {
        let (provider, model) = id.split_once('/')?;
        self.models.get(id).or_else(|| {
            let target = self.aliases.get(provider)?;
            self.models.get(&format!("{target}/{model}"))
        })
    }

    fn named(&self, id: &str, variant: Option<&str>) -> Option<Value> {
        let found = self.find(id)?;
        let mut answer = json!({ "name": found.name.as_ref()? });
        if let Some(provider_name) = &found.provider {
            answer["provider"] = json!(provider_name);
        }
        if let Some(variant) = variant.filter(|variant| !variant.is_empty()) {
            answer["variant"] = json!(variant);
        }
        Some(answer)
    }
}

/// The limits for a model whose catalog row is `row`, if known, and whose
/// runner bounds each image as given.
fn limits(
    row: Option<&Model>,
    runner_image_bytes: Option<u64>,
    runner_image_edge: Option<u64>,
) -> AttachmentLimits {
    AttachmentLimits {
        max_images: row
            .and_then(|row| row.images_per_request)
            .unwrap_or(FALLBACK_FILES),
        max_files: FALLBACK_FILES,
        image_bytes: stricter(row.and_then(|row| row.image_bytes), runner_image_bytes),
        image_long_edge_px: stricter(
            row.and_then(|row| row.image_long_edge_px),
            runner_image_edge,
        ),
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
        let mut arguments = json!({"fact_prefixes": FACTS, "paged": true});
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
        let body = json!({"method": "route.catalog_targets", "params": {}});
        ask_routing(|module| {
            self.calls.call_within(
                RouteTarget::ManagementSurface {
                    module_id: module.into(),
                },
                "model-names",
                &body,
                None,
                ROUTING_OPEN,
            )
        })
        .await
    }
}

#[cfg(test)]
pub(crate) mod tests;
