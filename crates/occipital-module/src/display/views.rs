//! Fetching each writer's view for a pin (`tool.views`), off the write path.
//!
//! A write's view authorizes the join only when the writer itself serves it for
//! the pin and the pin belongs to a tool in that writer's `tool.catalog` when
//! the view is first fetched (tool-display §6). The answer is kept with the
//! writer's `module_version`, so a call recorded before its provider changed
//! its catalog still joins. When the writer's version changes, a kept view or
//! `unknown` is fetched again (tool-provider §2.1): a view with a higher
//! template revision replaces the kept one, and any other difference keeps it.
//! A failed fetch keeps nothing new: the next write or page that names the pin
//! asks again.
//!
//! A pin can also be wanted without a writer, when a page or an app names a
//! call no provider has written for yet: rows draw from a call's input alone.
//! Then every module that serves views (`tool-views/v1`) is asked; the one
//! whose catalog holds the pin answers, and the others keep it as not in
//! their catalog without being asked again.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use cortexkit_role_tool_provider::call::SchemaPin;
use cortexkit_role_tool_provider::catalog::{CatalogAnswer, CatalogTool};
use cortexkit_role_tool_provider::views::{
    ViewChange, ViewEntry, ViewManifest, ViewsReply, ViewsRequest, MAX_PINS,
};
use serde_json::{json, Value};
use subc_protocol::RouteTarget;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, Instant};

use super::app::Hint;
use super::daemon::DaemonCalls;
use super::store::{now_ms, DisplayStore, KeptView, ViewState};

/// How many wanted pins wait to be fetched before more are dropped. A dropped
/// pin is asked for again the next time a write or page names it.
const QUEUE: usize = 1024;

/// After a failed fetch, a writer isn't asked again for this long, so a
/// provider that is down isn't asked once per write.
const REST_AFTER_FAILURE: Duration = Duration::from_secs(60);

/// How long the writers' versions read from the daemon are used before they
/// are read again, so a burst of wants costs one daemon read.
const VERSIONS_FRESH: Duration = Duration::from_secs(10);

/// A registered module as the fetcher needs it, from the daemon's
/// `catalog.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFacts {
    pub version: String,
    /// Whether it declares `tool-views/v1`, so has views to ask for.
    pub serves_views: bool,
}

/// What the fetcher asks: the daemon for the registered modules, and a writer
/// for its catalog and views, on its tool-provider route.
#[async_trait]
pub trait Providers: Send + Sync {
    /// Every registered module, by module id.
    async fn modules(&self) -> Result<HashMap<String, ModuleFacts>, String>;
    /// The writer's catalog, every tool it serves.
    async fn catalog(&self, writer: &str) -> Result<Vec<CatalogTool>, String>;
    /// The writer's `tool.views` answer for `request`.
    async fn views(&self, writer: &str, request: &ViewsRequest) -> Result<ViewsReply, String>;
}

/// A view the fetcher is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// One writer's view of a pin: the writer of a stored write.
    Writer(String, String),
    /// A pin's view from whichever module serves views has it.
    Pin(String),
}

/// Asks for views to be fetched; the fetch itself runs in the fetcher's task.
#[derive(Clone)]
pub struct Fetcher {
    queue: mpsc::Sender<Want>,
}

impl Fetcher {
    /// A fetcher whose wants arrive on `queue`, for tests of who asks.
    #[cfg(test)]
    pub(crate) fn from_queue(queue: mpsc::Sender<Want>) -> Self {
        Self { queue }
    }

    /// Fetch `writer`'s view for `pin`, unless it is already kept under the
    /// writer's current version. Never waits: when the queue is full the pin
    /// is dropped and asked for later.
    pub fn want(&self, writer: &str, pin: &str) {
        let _ = self
            .queue
            .try_send(Want::Writer(writer.to_owned(), pin.to_owned()));
    }

    /// Fetch `pin`'s view from every module that serves views and hasn't
    /// answered for it under its current version. Never waits, as for
    /// [`Fetcher::want`].
    pub fn want_pin(&self, pin: &str) {
        let _ = self.queue.try_send(Want::Pin(pin.to_owned()));
    }
}

/// Start the fetcher's task. It runs while any [`Fetcher`] is held. A view
/// that replaces a kept answer, such as a newer template revision, is
/// announced to `hints`, so apps showing the pin's calls learn of it.
pub fn start(
    store: Arc<DisplayStore>,
    providers: Arc<dyn Providers>,
    hints: Option<broadcast::Sender<Hint>>,
) -> (Fetcher, tokio::task::JoinHandle<()>) {
    let (queue, mut wanted) = mpsc::channel::<Want>(QUEUE);
    let task = tokio::spawn(async move {
        let mut resting: HashMap<String, Instant> = HashMap::new();
        let mut modules: Option<(Instant, HashMap<String, ModuleFacts>)> = None;
        while let Some(first) = wanted.recv().await {
            // Take everything already queued, so one catalog read and one
            // `tool.views` call serve every pin a writer is waiting on.
            let mut queued = vec![first];
            while let Ok(next) = wanted.try_recv() {
                queued.push(next);
            }
            if modules
                .as_ref()
                .is_none_or(|(read, _)| read.elapsed() >= VERSIONS_FRESH)
            {
                match providers.modules().await {
                    Ok(read) => modules = Some((Instant::now(), read)),
                    Err(error) => {
                        // Nothing can be compared without them; the pins are
                        // asked for again by the next write or page.
                        tracing::warn!(%error, "cannot read the writers' versions");
                        continue;
                    }
                }
            }
            let Some((_, current)) = modules.as_ref() else {
                continue;
            };
            let mut batch: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for want in queued {
                match want {
                    Want::Writer(writer, pin) => {
                        batch.entry(writer).or_default().insert(pin);
                    }
                    Want::Pin(pin) => {
                        for (writer, _) in current.iter().filter(|(_, facts)| facts.serves_views) {
                            batch.entry(writer.clone()).or_default().insert(pin.clone());
                        }
                    }
                }
            }
            for (writer, pins) in batch {
                if resting
                    .get(&writer)
                    .is_some_and(|since| since.elapsed() < REST_AFTER_FAILURE)
                {
                    continue;
                }
                // A writer the daemon doesn't list isn't running to ask.
                let Some(version) = current.get(&writer).map(|facts| &facts.version) else {
                    continue;
                };
                let due: Vec<(String, Option<ViewState>)> = pins
                    .into_iter()
                    .filter_map(|pin| match store.kept_view(&writer, &pin) {
                        Ok(None) => Some((pin, None)),
                        Ok(Some(kept)) if is_due(&kept, version) => Some((pin, Some(kept.state))),
                        _ => None,
                    })
                    .collect();
                if due.is_empty() {
                    continue;
                }
                match fetch(&store, providers.as_ref(), &writer, version, &due).await {
                    Ok(replaced) => {
                        resting.remove(&writer);
                        if let Some(hints) = &hints {
                            for pin in replaced {
                                // No watch open is no one to tell.
                                let _ = hints.send(Hint::Pin(pin));
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%writer, %error, "cannot fetch display views");
                        resting.insert(writer, Instant::now());
                    }
                }
            }
        }
    });
    (Fetcher { queue }, task)
}

/// Whether a kept answer is fetched again under the writer's `version`: a
/// view or `unknown` kept under another version. A pin that wasn't in the
/// writer's catalog at its first fetch never joins, so it isn't asked again.
fn is_due(kept: &KeptView, version: &str) -> bool {
    !matches!(kept.state, ViewState::NotInCatalog) && kept.module_version != version
}

/// Fetch `writer`'s answers for `due`, each pin with what is kept for it, and
/// keep, replace or stamp each as [`decide`] says. Returns the pins whose kept
/// answer was replaced.
async fn fetch(
    store: &DisplayStore,
    providers: &dyn Providers,
    writer: &str,
    version: &str,
    due: &[(String, Option<ViewState>)],
) -> Result<Vec<String>, String> {
    let catalog = providers.catalog(writer).await?;
    let schemas: HashMap<String, &Value> = catalog
        .iter()
        .filter_map(|tool| {
            SchemaPin::new(&tool.name, &tool.schema_digest, tool.semantics)
                .encode()
                .ok()
                .map(|pin| (pin, &tool.input_schema))
        })
        .collect();
    let now = now_ms();
    // A first fetch of a pin outside the catalog is never asked about.
    let mut asked = Vec::new();
    for (pin, kept) in due {
        if kept.is_none() && !schemas.contains_key(pin) {
            put(
                store,
                writer,
                pin,
                Decision::Keep(ViewState::NotInCatalog),
                version,
                now,
            )?;
        } else {
            asked.push((pin, kept));
        }
    }
    let mut replaced = Vec::new();
    for chunk in asked.chunks(MAX_PINS) {
        let request = ViewsRequest::new(chunk.iter().map(|(pin, _)| (*pin).clone()).collect());
        let reply = providers.views(writer, &request).await?;
        reply
            .validate_for(&request)
            .map_err(|refusal| format!("{}: {}", refusal.code, refusal.message))?;
        for (pin, kept) in chunk {
            let decision = decide(
                kept.as_ref(),
                &reply.views[*pin],
                schemas.get(*pin).copied(),
            )?;
            if let Decision::Stamp(Some(reason)) = &decision {
                tracing::warn!(%writer, %pin, %reason, "kept the display view");
            }
            if matches!(decision, Decision::Replace(_)) {
                replaced.push((*pin).clone());
            }
            put(store, writer, pin, decision, version, now)?;
        }
    }
    Ok(replaced)
}

/// What to do with a writer's answer for a pin.
#[derive(Debug, PartialEq)]
enum Decision {
    /// The first answer for the pin: keep it.
    Keep(ViewState),
    /// Put this in place of the kept answer.
    Replace(ViewState),
    /// The kept answer stays, checked under the current version; with a
    /// reason when the writer's answer broke its contract.
    Stamp(Option<String>),
}

/// Decide on `fetched`, the writer's answer for a pin, given what is `kept`
/// for it and the input schema of the writer's catalog tool with that pin,
/// absent when no catalog tool has it any more.
fn decide(
    kept: Option<&ViewState>,
    fetched: &ViewEntry,
    input_schema: Option<&Value>,
) -> Result<Decision, String> {
    let fetched_view = match fetched {
        ViewEntry::Manifest(view) => Some(view),
        _ => None,
    };
    Ok(match (kept, fetched_view) {
        // A first answer: the pin is in the catalog, or it wasn't asked.
        (None, Some(view)) => match input_schema {
            Some(schema) => Decision::Keep(ViewState::View {
                manifest: canonical(view)?,
                input_schema: canonical(schema)?,
            }),
            None => Decision::Keep(ViewState::NotInCatalog),
        },
        (None, None) => Decision::Keep(ViewState::Unknown),
        // A view where the writer had answered `unknown`. Its input schema
        // comes only from the catalog; a tool already gone from it can't join.
        (Some(ViewState::Unknown), Some(view)) => match input_schema {
            Some(schema) => Decision::Replace(ViewState::View {
                manifest: canonical(view)?,
                input_schema: canonical(schema)?,
            }),
            None => Decision::Stamp(Some(
                "its tool left the catalog before it served a view".into(),
            )),
        },
        (Some(ViewState::Unknown | ViewState::NotInCatalog), None) => Decision::Stamp(None),
        (
            Some(ViewState::View {
                manifest,
                input_schema,
            }),
            fetched,
        ) => {
            let Some(view) = fetched else {
                return Ok(Decision::Stamp(Some(
                    "the writer now answers unknown for a pin it served".into(),
                )));
            };
            let Ok(kept_view) = serde_json::from_slice::<ViewManifest>(manifest) else {
                // A kept view this build can't read is replaced by the one
                // the writer serves now, with the input schema it was kept with.
                return Ok(Decision::Replace(ViewState::View {
                    manifest: canonical(view)?,
                    input_schema: input_schema.clone(),
                }));
            };
            match view.compare_with_kept(&kept_view) {
                ViewChange::NewerTemplate => Decision::Replace(ViewState::View {
                    manifest: canonical(view)?,
                    input_schema: input_schema.clone(),
                }),
                ViewChange::Conflict => Decision::Stamp(Some(
                    "its record schema or reads changed, or its template changed without a higher revision"
                        .into(),
                )),
                // Same, or an older revision than the one kept.
                _ => Decision::Stamp(None),
            }
        }
        // Not asked: a pin outside the catalog at its first fetch.
        (Some(ViewState::NotInCatalog), Some(_)) => Decision::Stamp(None),
    })
}

fn put(
    store: &DisplayStore,
    writer: &str,
    pin: &str,
    decision: Decision,
    version: &str,
    now: i64,
) -> Result<(), String> {
    match decision {
        Decision::Keep(state) => store.keep_view(writer, pin, &state, version, now),
        Decision::Replace(state) => store.replace_view(writer, pin, &state, version, now),
        Decision::Stamp(_) => store.stamp_view(writer, pin, version),
    }
    .map_err(|error| error.to_string())
}

fn canonical(value: &impl serde::Serialize) -> Result<Vec<u8>, String> {
    serde_jcs::to_vec(value).map_err(|error| error.to_string())
}

/// The writers, reached through the daemon as this module.
pub struct DaemonProviders {
    calls: Arc<DaemonCalls>,
}

impl DaemonProviders {
    pub fn new(calls: Arc<DaemonCalls>) -> Self {
        Self { calls }
    }

    async fn named(&self, writer: &str, name: &str, arguments: Value) -> Result<Value, String> {
        self.calls
            .call(
                RouteTarget::ToolProvider {
                    module_id: writer.into(),
                },
                "display-views",
                &json!({"name": name, "arguments": arguments}),
                Some(BTreeMap::from([("tool-provider".into(), "v1".into())])),
            )
            .await
            .map_err(|failure| format!("{writer}: {failure}"))
    }
}

#[async_trait]
impl Providers for DaemonProviders {
    async fn modules(&self) -> Result<HashMap<String, ModuleFacts>, String> {
        let consumer = self
            .calls
            .consumer()
            .await
            .map_err(|failure| format!("catalog.list: {failure}"))?;
        let listed = match consumer.catalog_list().await {
            Ok(listed) => listed,
            Err(error) => {
                self.calls.failed(error.code()).await;
                return Err(format!("catalog.list: {error}"));
            }
        };
        // A module listed without a version comes from a daemon that
        // predates the field; its views are kept under the empty version, so
        // they are fetched again only once the daemon reports one.
        Ok(listed
            .modules
            .into_iter()
            .map(|module| {
                let serves_views = module.capabilities.as_ref().is_some_and(|declared| {
                    declared.provides.iter().any(|capability| {
                        capability == cortexkit_role_tool_provider::TOOL_VIEWS_CAPABILITY
                    })
                });
                let facts = ModuleFacts {
                    version: module.module_version.unwrap_or_default(),
                    serves_views,
                };
                (module.module_id, facts)
            })
            .collect())
    }

    async fn catalog(&self, writer: &str) -> Result<Vec<CatalogTool>, String> {
        let answer: CatalogAnswer =
            serde_json::from_value(self.named(writer, "tool.catalog", json!({})).await?)
                .map_err(|error| format!("{writer}: tool.catalog: {error}"))?;
        Ok(answer.tools)
    }

    async fn views(&self, writer: &str, request: &ViewsRequest) -> Result<ViewsReply, String> {
        let arguments = serde_json::to_value(request).map_err(|error| error.to_string())?;
        serde_json::from_value(self.named(writer, "tool.views", arguments).await?)
            .map_err(|error| format!("{writer}: tool.views: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_role_tool_provider::catalog::schema_digest;
    use cortexkit_role_tool_provider::views::{RecordSchema, ViewManifest};
    use cortexkit_test_support::ScratchDir;
    use std::sync::Mutex;

    /// A writer with two tools, `bash` and `edit`, serving `bash_view` for
    /// `bash` (`unknown` while it is `None`) and `unknown` for `edit`, at
    /// module version `version`. It counts its catalog and views calls, and
    /// fails its catalog while `down` is set.
    struct Aft {
        calls: Mutex<Vec<String>>,
        down: Mutex<bool>,
        version: Mutex<String>,
        bash_view: Mutex<Option<ViewManifest>>,
    }

    impl Aft {
        fn new(down: bool) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(vec![]),
                down: Mutex::new(down),
                version: Mutex::new("1.0".into()),
                bash_view: Mutex::new(Some(manifest())),
            })
        }
    }

    fn tool(name: &str) -> CatalogTool {
        let schema = json!({"type": "object", "properties": {name: {"type": "string"}}});
        CatalogTool::new(name, schema_digest(&schema).unwrap(), 1, schema)
    }

    fn pin(name: &str) -> String {
        let tool = tool(name);
        SchemaPin::new(&tool.name, &tool.schema_digest, tool.semantics)
            .encode()
            .unwrap()
    }

    fn manifest() -> ViewManifest {
        let schema: RecordSchema =
            serde_json::from_value(json!({"name": "aft.bash", "revision": 1, "fields": {}}))
                .unwrap();
        ViewManifest::new(schema, vec![], json!({"row": {"source": "x"}}))
    }

    #[async_trait]
    impl Providers for Aft {
        async fn modules(&self) -> Result<HashMap<String, ModuleFacts>, String> {
            let version = self.version.lock().unwrap().clone();
            Ok(HashMap::from([
                (
                    "aft".to_owned(),
                    ModuleFacts {
                        version,
                        serves_views: true,
                    },
                ),
                // A module that serves no views is never asked for a pin.
                (
                    "broca".to_owned(),
                    ModuleFacts {
                        version: "0.3".into(),
                        serves_views: false,
                    },
                ),
            ]))
        }

        async fn catalog(&self, writer: &str) -> Result<Vec<CatalogTool>, String> {
            self.calls.lock().unwrap().push(format!("{writer} catalog"));
            if *self.down.lock().unwrap() {
                return Err("aft is down".into());
            }
            Ok(vec![tool("bash"), tool("edit")])
        }

        async fn views(&self, writer: &str, request: &ViewsRequest) -> Result<ViewsReply, String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{writer} views {}", request.pins.len()));
            Ok(ViewsReply::new(
                request
                    .pins
                    .iter()
                    .map(|asked| {
                        let served = (*asked == pin("bash"))
                            .then(|| self.bash_view.lock().unwrap().clone())
                            .flatten();
                        let entry = served.map_or(ViewEntry::Unknown, ViewEntry::Manifest);
                        (asked.clone(), entry)
                    })
                    .collect(),
            ))
        }
    }

    fn opened(dir: &ScratchDir) -> Arc<DisplayStore> {
        let descriptor = crate::store::tests::descriptor(dir);
        Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap())
    }

    async fn until(mut done: impl FnMut() -> bool) {
        for _ in 0..1_000 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("the fetcher never got there");
    }

    /// One catalog read and one `tool.views` call serve every pin queued for a
    /// writer. Each answer is kept: the view with the catalog tool's input
    /// schema, `unknown` as unknown, and a pin no catalog tool has as not in
    /// the catalog, without asking `tool.views` about it.
    #[tokio::test(start_paused = true)]
    async fn each_answer_is_kept_and_a_pin_outside_the_catalog_is_never_asked() {
        let dir = ScratchDir::new("occipital-views-kept");
        let store = opened(&dir);
        let aft = Aft::new(false);
        let (fetcher, _task) = start(store.clone(), aft.clone(), None);
        let foreign = "tp1:gone:0000000000000000000000000000000000000000000000000000000000000000:1";
        for wanted in [pin("bash"), pin("edit"), foreign.into()] {
            fetcher.want("aft", &wanted);
        }
        until(|| matches!(store.view("aft", foreign), Ok(Some(_)))).await;
        until(|| matches!(store.view("aft", &pin("edit")), Ok(Some(_)))).await;
        assert_eq!(*aft.calls.lock().unwrap(), ["aft catalog", "aft views 2"]);
        assert_eq!(
            store.view("aft", &pin("bash")).unwrap(),
            Some(ViewState::View {
                manifest: serde_jcs::to_vec(&manifest()).unwrap(),
                input_schema: serde_jcs::to_vec(&tool("bash").input_schema).unwrap(),
            })
        );
        assert_eq!(
            store.view("aft", &pin("edit")).unwrap(),
            Some(ViewState::Unknown)
        );
        assert_eq!(
            store.view("aft", foreign).unwrap(),
            Some(ViewState::NotInCatalog)
        );
        // Another writer's view of the same pin is a different question.
        assert_eq!(store.view("core", &pin("bash")).unwrap(), None);
        // A kept pin isn't fetched again while the writer's version stays.
        fetcher.want("aft", &pin("bash"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(aft.calls.lock().unwrap().len(), 2);
    }

    /// When the writer's version changes, a kept `unknown` and a kept view are
    /// fetched again: a view replaces `unknown`, a higher template revision
    /// replaces the kept view, and a template changed without a higher
    /// revision is ignored. A pin outside the catalog is never asked again.
    #[tokio::test(start_paused = true)]
    async fn a_new_writer_version_brings_revised_templates_only() {
        let dir = ScratchDir::new("occipital-views-revised");
        let store = opened(&dir);
        let aft = Aft::new(false);
        *aft.bash_view.lock().unwrap() = None;
        let (hints, mut announced) = broadcast::channel(8);
        let (fetcher, _task) = start(store.clone(), aft.clone(), Some(hints));
        let foreign = "tp1:gone:0000000000000000000000000000000000000000000000000000000000000000:1";
        let kept = |pin: &str| store.kept_view("aft", pin).unwrap();
        // Under each version: set what aft serves, want the pins, and wait
        // until the bash pin was checked under that version.
        let step = |version: &str, served: Option<ViewManifest>| {
            *aft.version.lock().unwrap() = version.into();
            *aft.bash_view.lock().unwrap() = served;
            fetcher.want("aft", &pin("bash"));
            fetcher.want("aft", foreign);
        };
        let bash_at = |version: &str| {
            let version = version.to_owned();
            let store = store.clone();
            move || {
                store
                    .kept_view("aft", &pin("bash"))
                    .unwrap()
                    .is_some_and(|kept| kept.module_version == version)
            }
        };
        let view = |manifest: &ViewManifest| ViewState::View {
            manifest: serde_jcs::to_vec(manifest).unwrap(),
            input_schema: serde_jcs::to_vec(&tool("bash").input_schema).unwrap(),
        };
        let revised = |revision: u64, source: &str| {
            let schema: RecordSchema =
                serde_json::from_value(json!({"name": "aft.bash", "revision": 1, "fields": {}}))
                    .unwrap();
            ViewManifest::new(schema, vec![], json!({"row": {"source": source}}))
                .with_template_revision(revision)
        };

        step("1.0", None);
        until(bash_at("1.0")).await;
        assert_eq!(kept(&pin("bash")).unwrap().state, ViewState::Unknown);
        assert_eq!(kept(foreign).unwrap().state, ViewState::NotInCatalog);

        // Same version: nothing is asked, even though aft serves a view now.
        step("1.0", Some(manifest()));
        tokio::time::sleep(VERSIONS_FRESH).await;
        let calls = aft.calls.lock().unwrap().len();
        fetcher.want("aft", &pin("bash"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(aft.calls.lock().unwrap().len(), calls);

        tokio::time::sleep(VERSIONS_FRESH).await;
        step("1.1", Some(manifest()));
        until(bash_at("1.1")).await;
        assert_eq!(kept(&pin("bash")).unwrap().state, view(&manifest()));

        tokio::time::sleep(VERSIONS_FRESH).await;
        step("1.2", Some(revised(0, "changed without a revision")));
        until(bash_at("1.2")).await;
        assert_eq!(kept(&pin("bash")).unwrap().state, view(&manifest()));

        tokio::time::sleep(VERSIONS_FRESH).await;
        step("1.3", Some(revised(1, "fixed")));
        until(bash_at("1.3")).await;
        assert_eq!(
            kept(&pin("bash")).unwrap().state,
            view(&revised(1, "fixed"))
        );

        // The pin outside the catalog stayed as first kept, never asked again.
        assert_eq!(
            kept(foreign).unwrap(),
            KeptView {
                state: ViewState::NotInCatalog,
                module_version: "1.0".into(),
            }
        );
        assert!(aft
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call != "aft views 2"));
        // Each replacement was announced, the view in place of `unknown` and
        // the fixed template; the ignored change wasn't.
        let mut pins = Vec::new();
        while let Ok(hint) = announced.try_recv() {
            pins.push(hint);
        }
        assert_eq!(pins, [Hint::Pin(pin("bash")), Hint::Pin(pin("bash"))]);
    }

    /// Every refetch outcome the integration test doesn't reach: the same view,
    /// an older revision, a writer answering `unknown` for a pin it served,
    /// and a view for a kept `unknown` whose tool has left the catalog. Each
    /// keeps what is kept, with a reason when the writer broke its contract.
    #[test]
    fn refetches_that_keep_the_kept_answer() {
        let newer = manifest().with_template_revision(2);
        let kept = ViewState::View {
            manifest: serde_jcs::to_vec(&newer).unwrap(),
            input_schema: b"{}".to_vec(),
        };
        let schema = json!({});
        let stamped = |kept: &ViewState, fetched: ViewEntry, schema: Option<&Value>| match decide(
            Some(kept),
            &fetched,
            schema,
        )
        .unwrap()
        {
            Decision::Stamp(reason) => reason.is_some(),
            other => panic!("expected the kept answer to stay, got {other:?}"),
        };
        assert!(!stamped(
            &kept,
            ViewEntry::Manifest(newer.clone()),
            Some(&schema)
        ));
        assert!(!stamped(
            &kept,
            ViewEntry::Manifest(manifest()),
            Some(&schema)
        ));
        assert!(stamped(&kept, ViewEntry::Unknown, Some(&schema)));
        assert!(stamped(
            &ViewState::Unknown,
            ViewEntry::Manifest(manifest()),
            None
        ));
        assert!(!stamped(
            &ViewState::Unknown,
            ViewEntry::Unknown,
            Some(&schema)
        ));
    }

    /// A failed fetch keeps nothing, and the writer isn't asked again until
    /// its rest is over.
    #[tokio::test(start_paused = true)]
    async fn a_failed_fetch_keeps_nothing_and_rests_the_writer() {
        let dir = ScratchDir::new("occipital-views-rest");
        let store = opened(&dir);
        let aft = Aft::new(true);
        let (fetcher, _task) = start(store.clone(), aft.clone(), None);
        fetcher.want("aft", &pin("bash"));
        until(|| aft.calls.lock().unwrap().len() == 1).await;
        fetcher.want("aft", &pin("bash"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(aft.calls.lock().unwrap().len(), 1, "asked while resting");
        assert_eq!(store.view("aft", &pin("bash")).unwrap(), None);

        *aft.down.lock().unwrap() = false;
        tokio::time::sleep(REST_AFTER_FAILURE).await;
        fetcher.want("aft", &pin("bash"));
        until(|| matches!(store.view("aft", &pin("bash")), Ok(Some(_)))).await;
        assert_eq!(aft.calls.lock().unwrap().len(), 3);
    }

    /// A pin wanted without a writer is asked of every module that serves
    /// views, and of no other. The answers are kept as for a writer's pin, so
    /// asking again costs nothing.
    #[tokio::test(start_paused = true)]
    async fn a_pin_wanted_without_a_writer_asks_each_module_that_serves_views() {
        let dir = ScratchDir::new("occipital-views-by-pin");
        let store = opened(&dir);
        let aft = Aft::new(false);
        let (fetcher, _task) = start(store.clone(), aft.clone(), None);
        let foreign = "tp1:gone:0000000000000000000000000000000000000000000000000000000000000000:1";
        fetcher.want_pin(&pin("bash"));
        fetcher.want_pin(foreign);
        until(|| matches!(store.view("aft", foreign), Ok(Some(_)))).await;
        until(|| matches!(store.view("aft", &pin("bash")), Ok(Some(_)))).await;
        assert!(matches!(
            store.view("aft", &pin("bash")).unwrap(),
            Some(ViewState::View { .. })
        ));
        assert_eq!(
            store.view("aft", foreign).unwrap(),
            Some(ViewState::NotInCatalog)
        );
        // Broca serves no views, so it was never asked and keeps nothing.
        assert_eq!(store.view("broca", &pin("bash")).unwrap(), None);
        assert_eq!(*aft.calls.lock().unwrap(), ["aft catalog", "aft views 1"]);
        fetcher.want_pin(&pin("bash"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(aft.calls.lock().unwrap().len(), 2);
    }
}
