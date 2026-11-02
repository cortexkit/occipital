//! Fetching each writer's view for a pin (`tool.views`), off the write path.
//!
//! A write's view authorizes the join only when the writer itself serves it for
//! the pin, and the pin is the tool's (tool-display §6): the view's own `tool`
//! recomputes to the pin, and its input schema is the one kept; or, for a view
//! without `tool`, the pin belongs to a tool in the writer's current
//! `tool.catalog`, whose input schema is kept. A `tool` that doesn't recompute
//! to the pin makes the answer `unknown`. Every wanted pin is asked about, so a
//! pin that left the catalog still joins when its view carries its tool. When
//! two writers each have an accepted view for one pin, the apps' `display.views`
//! answers `unknown` for it, so neither view is drawn: a match proves the
//! tool's definition, not which writer owns the tool. The answer is kept with the
//! writer's version (see [`ModuleFacts::version`]), so a call recorded before
//! its provider changed its catalog still joins. A kept view or `unknown` is
//! fetched again, as is a pin kept as not in the catalog, when that version
//! changes (tool-provider §2.1), and also when
//! it was last asked an hour or more ago, because a provider that declares no
//! build can serve a revised template under an unchanged `module_version`:
//! many modules keep one across builds.
//! A view with a higher template revision replaces the kept one, and any other
//! difference keeps it.
//! A failed fetch keeps nothing new: the next write or page that names the pin
//! asks again.
//!
//! A pin can also be wanted without a writer, when a page or an app names a
//! call no provider has written for yet: rows draw from a call's input alone.
//! Then every module that serves views (`tool-views/v1`) is asked, and each
//! answer is kept as for a writer's pin.

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

/// How long a kept answer is trusted under an unchanged writer version: past
/// this, the next want for its pin asks the writer again. One `tool.views`
/// call per writer an hour at most, and only while its calls are drawn.
const ASK_AGAIN_AFTER_MS: i64 = 60 * 60 * 1000;

/// A registered module as the fetcher needs it, from the daemon's
/// `catalog.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFacts {
    /// The module's `module_version`, followed by `+` and its declared build
    /// commit when the catalog carries one (`provenance.build_git_sha`), as in
    /// a semver build suffix. Empty from a daemon that reports no version.
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
                let now = now_ms();
                let due: Vec<(String, Option<ViewState>)> = pins
                    .into_iter()
                    .filter_map(|pin| match store.kept_view(&writer, &pin) {
                        Ok(None) => Some((pin, None)),
                        Ok(Some(kept)) if is_due(&kept, version, now) => {
                            Some((pin, Some(kept.state)))
                        }
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

/// A writer's version as [`ModuleFacts::version`] keeps it: `module_version`,
/// then `+build` when a build commit is declared, so a new build of an
/// unchanged version is a new version here.
fn versioned(module_version: Option<&str>, build: Option<&str>) -> String {
    let version = module_version.unwrap_or_default();
    match build {
        Some(build) if !build.is_empty() => format!("{version}+{build}"),
        _ => version.to_owned(),
    }
}

/// Whether a kept answer is fetched again under the writer's `version` at
/// `now_ms`: one kept under another version, or last asked
/// [`ASK_AGAIN_AFTER_MS`] or more ago. That includes a pin kept as not in the
/// catalog, which a later answer carrying the pin's tool can still accept.
fn is_due(kept: &KeptView, version: &str, now_ms: i64) -> bool {
    kept.module_version != version || now_ms.saturating_sub(kept.asked_at_ms) >= ASK_AGAIN_AFTER_MS
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
    let mut replaced = Vec::new();
    for chunk in due.chunks(MAX_PINS) {
        let request = ViewsRequest::new(chunk.iter().map(|(pin, _)| pin.clone()).collect());
        let reply = providers.views(writer, &request).await?;
        reply
            .validate_for(&request)
            .map_err(|refusal| format!("{}: {}", refusal.code, refusal.message))?;
        for (pin, kept) in chunk {
            let (view, input_schema) = resolve(pin, &reply.views[pin], schemas.get(pin).copied());
            let decision = decide(kept.as_ref(), view, input_schema)?;
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

/// The writer's answer for `pin` as [`decide`] reads it: the view, and the input
/// schema that comes with it. A view whose `tool` recomputes to the pin brings
/// that tool's input schema; a view without `tool` brings `catalog`, the input
/// schema of the writer's current catalog tool with the pin, if any. A view
/// whose `tool` doesn't recompute to the pin is `unknown`, never checked
/// against the catalog instead (tool-display §6).
fn resolve<'a>(
    pin: &str,
    answer: &'a ViewEntry,
    catalog: Option<&'a Value>,
) -> (Option<&'a ViewManifest>, Option<&'a Value>) {
    match answer {
        ViewEntry::Manifest(view) => match view.check_pin(pin) {
            Ok(true) => (
                Some(view),
                view.tool.as_ref().map(|tool| &tool.input_schema),
            ),
            Ok(false) => (Some(view), catalog),
            Err(_) => (None, None),
        },
        _ => (None, catalog),
    }
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

/// Decide on `fetched`, the view the writer serves for a pin (`None` for
/// `unknown`), given what is `kept` for it and the input schema [`resolve`]
/// found, absent when the view carries no tool and no catalog tool has the pin.
fn decide(
    kept: Option<&ViewState>,
    fetched: Option<&ViewManifest>,
    input_schema: Option<&Value>,
) -> Result<Decision, String> {
    Ok(match (kept, fetched) {
        // A first answer: a view with an input schema joins; one without is
        // a pin outside the catalog.
        (None, Some(view)) => match input_schema {
            Some(schema) => Decision::Keep(ViewState::View {
                manifest: canonical(view)?,
                input_schema: canonical(schema)?,
            }),
            None => Decision::Keep(ViewState::NotInCatalog),
        },
        (None, None) => Decision::Keep(ViewState::Unknown),
        // A view where the writer had answered `unknown`. Without its tool,
        // its input schema comes only from the catalog, and a tool already
        // gone from it can't join.
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
        // A pin kept as outside the catalog now joins if the view carries its
        // tool or the pin is back in the catalog.
        (Some(ViewState::NotInCatalog), Some(view)) => match input_schema {
            Some(schema) => Decision::Replace(ViewState::View {
                manifest: canonical(view)?,
                input_schema: canonical(schema)?,
            }),
            None => Decision::Stamp(None),
        },
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
        Decision::Stamp(_) => store.stamp_view(writer, pin, version, now),
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
        // they are fetched again once the daemon reports one or by age.
        Ok(listed
            .modules
            .into_iter()
            .map(|module| {
                let serves_views = module.capabilities.as_ref().is_some_and(|declared| {
                    declared.provides.iter().any(|capability| {
                        capability == cortexkit_role_tool_provider::TOOL_VIEWS_CAPABILITY
                    })
                });
                let build = module
                    .provenance
                    .as_ref()
                    .and_then(|provenance| provenance.build_git_sha.as_deref());
                let facts = ModuleFacts {
                    version: versioned(module.module_version.as_deref(), build),
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
    use cortexkit_role_tool_provider::views::{RecordSchema, ViewManifest, ViewTool};
    use cortexkit_test_support::ScratchDir;
    use std::sync::Mutex;

    /// A writer with two tools in its catalog, `bash` and `edit`, at module
    /// version `version`. It serves `bash_view` for `bash` (`unknown` while it
    /// is `None`), `unknown` for `edit`, and `outside` for pins outside its
    /// catalog. It counts its catalog and views calls, and fails its catalog
    /// while `down` is set.
    struct Aft {
        calls: Mutex<Vec<String>>,
        down: Mutex<bool>,
        version: Mutex<String>,
        bash_view: Mutex<Option<ViewManifest>>,
        outside: Mutex<HashMap<String, ViewManifest>>,
    }

    impl Aft {
        /// Outside its catalog it serves the `old` tool's view carrying that
        /// tool, a view without `tool` for `legacy`, and for `forged` a view
        /// whose `tool` is `old`'s, so it recomputes to another pin.
        fn new(down: bool) -> Arc<Self> {
            let old = manifest().with_tool(ViewTool::from(&tool("old")));
            Arc::new(Self {
                calls: Mutex::new(vec![]),
                down: Mutex::new(down),
                version: Mutex::new("1.0".into()),
                bash_view: Mutex::new(Some(manifest())),
                outside: Mutex::new(HashMap::from([
                    (pin("old"), old.clone()),
                    (pin("legacy"), manifest()),
                    (pin("forged"), old),
                ])),
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
                        let served = if *asked == pin("bash") {
                            self.bash_view.lock().unwrap().clone()
                        } else {
                            self.outside.lock().unwrap().get(asked).cloned()
                        };
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
    /// writer, in its catalog or not. Each answer is kept: a catalog pin's view
    /// with the catalog tool's input schema; a view carrying its tool, outside
    /// the catalog, with that tool's input schema; `unknown` as unknown, and so
    /// is a view whose tool recomputes to another pin; and a view without a
    /// tool for a pin outside the catalog as not in the catalog.
    #[tokio::test(start_paused = true)]
    async fn each_answer_is_kept_and_every_pin_is_asked() {
        let dir = ScratchDir::new("occipital-views-kept");
        let store = opened(&dir);
        let aft = Aft::new(false);
        let (fetcher, _task) = start(store.clone(), aft.clone(), None);
        let foreign = "tp1:gone:0000000000000000000000000000000000000000000000000000000000000000:1";
        let wanted = [
            pin("bash"),
            pin("edit"),
            foreign.into(),
            pin("old"),
            pin("legacy"),
            pin("forged"),
        ];
        for pin in &wanted {
            fetcher.want("aft", pin);
        }
        for pin in &wanted {
            until(|| matches!(store.view("aft", pin), Ok(Some(_)))).await;
        }
        assert_eq!(*aft.calls.lock().unwrap(), ["aft catalog", "aft views 6"]);
        let view = |name: &str, manifest: &ViewManifest| {
            Some(ViewState::View {
                manifest: serde_jcs::to_vec(manifest).unwrap(),
                input_schema: serde_jcs::to_vec(&tool(name).input_schema).unwrap(),
            })
        };
        assert_eq!(
            store.view("aft", &pin("bash")).unwrap(),
            view("bash", &manifest())
        );
        assert_eq!(
            store.view("aft", &pin("old")).unwrap(),
            view("old", &manifest().with_tool(ViewTool::from(&tool("old"))))
        );
        for unknown in [pin("edit"), foreign.into(), pin("forged")] {
            assert_eq!(
                store.view("aft", &unknown).unwrap(),
                Some(ViewState::Unknown),
                "{unknown}"
            );
        }
        assert_eq!(
            store.view("aft", &pin("legacy")).unwrap(),
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
    /// revision is ignored.
    #[tokio::test(start_paused = true)]
    async fn a_new_writer_version_brings_revised_templates_only() {
        let dir = ScratchDir::new("occipital-views-revised");
        let store = opened(&dir);
        let aft = Aft::new(false);
        *aft.bash_view.lock().unwrap() = None;
        let (hints, mut announced) = broadcast::channel(8);
        let (fetcher, _task) = start(store.clone(), aft.clone(), Some(hints));
        let kept = |pin: &str| store.kept_view("aft", pin).unwrap();
        // Under each version: set what aft serves, want the pins, and wait
        // until the bash pin was checked under that version.
        let step = |version: &str, served: Option<ViewManifest>| {
            *aft.version.lock().unwrap() = version.into();
            *aft.bash_view.lock().unwrap() = served;
            fetcher.want("aft", &pin("bash"));
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

        // Each replacement was announced, the view in place of `unknown` and
        // the fixed template; the ignored change wasn't.
        let mut pins = Vec::new();
        while let Ok(hint) = announced.try_recv() {
            pins.push(hint);
        }
        assert_eq!(pins, [Hint::Pin(pin("bash")), Hint::Pin(pin("bash"))]);
    }

    /// A declared build is part of the writer's version, so a new build of the
    /// same `module_version` makes its kept answers due.
    #[test]
    fn a_declared_build_is_part_of_the_version() {
        assert_eq!(versioned(Some("0.1.0"), None), "0.1.0");
        assert_eq!(versioned(Some("0.1.0"), Some("")), "0.1.0");
        assert_eq!(versioned(Some("0.1.0"), Some("4a1d473")), "0.1.0+4a1d473");
        assert_eq!(versioned(None, None), "");
        let kept = KeptView {
            state: ViewState::Unknown,
            module_version: versioned(Some("0.1.0"), Some("4a1d473")),
            asked_at_ms: 1_000,
        };
        assert!(!is_due(
            &kept,
            &versioned(Some("0.1.0"), Some("4a1d473")),
            1_000
        ));
        assert!(is_due(
            &kept,
            &versioned(Some("0.1.0"), Some("f67814a")),
            1_000
        ));
    }

    /// A kept answer is asked again under a new writer version, or once it was
    /// last asked an hour ago under the same one; a pin outside the writer's
    /// catalog never is.
    #[test]
    fn kept_answers_fall_due_by_version_or_age() {
        let kept = |state: ViewState| KeptView {
            state,
            module_version: "1.0".into(),
            asked_at_ms: 1_000,
        };
        let due = |kept: &KeptView, version: &str, now: i64| is_due(kept, version, now);
        let unknown = kept(ViewState::Unknown);
        assert!(!due(&unknown, "1.0", 1_000 + ASK_AGAIN_AFTER_MS - 1));
        assert!(due(&unknown, "1.0", 1_000 + ASK_AGAIN_AFTER_MS));
        assert!(due(&unknown, "1.1", 1_000));
        // A pin kept as outside the catalog falls due by the same rules: a
        // later answer can carry its tool.
        let outside = kept(ViewState::NotInCatalog);
        assert!(!due(&outside, "1.0", 1_000 + ASK_AGAIN_AFTER_MS - 1));
        assert!(due(&outside, "1.0", 1_000 + ASK_AGAIN_AFTER_MS));
        assert!(due(&outside, "1.1", 1_000));
    }

    /// A pin kept as outside the catalog is asked again under the writer's
    /// next version, and joins once the writer's view carries its tool.
    #[tokio::test(start_paused = true)]
    async fn a_pin_outside_the_catalog_joins_once_its_view_carries_its_tool() {
        let dir = ScratchDir::new("occipital-views-outside");
        let store = opened(&dir);
        let aft = Aft::new(false);
        let (fetcher, _task) = start(store.clone(), aft.clone(), None);
        fetcher.want("aft", &pin("legacy"));
        until(|| matches!(store.view("aft", &pin("legacy")), Ok(Some(_)))).await;
        assert_eq!(
            store.view("aft", &pin("legacy")).unwrap(),
            Some(ViewState::NotInCatalog)
        );

        let carried = manifest().with_tool(ViewTool::from(&tool("legacy")));
        aft.outside
            .lock()
            .unwrap()
            .insert(pin("legacy"), carried.clone());
        *aft.version.lock().unwrap() = "1.1".into();
        tokio::time::sleep(VERSIONS_FRESH).await;
        fetcher.want("aft", &pin("legacy"));
        until(|| {
            matches!(
                store.view("aft", &pin("legacy")),
                Ok(Some(ViewState::View { .. }))
            )
        })
        .await;
        assert_eq!(
            store.view("aft", &pin("legacy")).unwrap(),
            Some(ViewState::View {
                manifest: serde_jcs::to_vec(&carried).unwrap(),
                input_schema: serde_jcs::to_vec(&tool("legacy").input_schema).unwrap(),
            })
        );
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
        let stamped =
            |kept: &ViewState, fetched: Option<ViewManifest>, schema: Option<&Value>| match decide(
                Some(kept),
                fetched.as_ref(),
                schema,
            )
            .unwrap()
            {
                Decision::Stamp(reason) => reason.is_some(),
                other => panic!("expected the kept answer to stay, got {other:?}"),
            };
        assert!(!stamped(&kept, Some(newer.clone()), Some(&schema)));
        assert!(!stamped(&kept, Some(manifest()), Some(&schema)));
        assert!(stamped(&kept, None, Some(&schema)));
        assert!(stamped(&ViewState::Unknown, Some(manifest()), None));
        assert!(!stamped(&ViewState::Unknown, None, Some(&schema)));
        assert!(!stamped(&ViewState::NotInCatalog, Some(manifest()), None));
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
            Some(ViewState::Unknown)
        );
        // Broca serves no views, so it was never asked and keeps nothing.
        assert_eq!(store.view("broca", &pin("bash")).unwrap(), None);
        assert_eq!(*aft.calls.lock().unwrap(), ["aft catalog", "aft views 2"]);
        fetcher.want_pin(&pin("bash"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(aft.calls.lock().unwrap().len(), 2);
    }
}
