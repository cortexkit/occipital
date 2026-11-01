//! The apps' tool display operations: views by pin, templates by digest, a
//! session's records by call, and a call's detail. The watch that tells an
//! app when to read again is in the app reader, over [`ToolDisplay::hints`].
//!
//! Pages carry only each call's `callKey` and `schemaPin`; everything here is
//! read beside them, so a provider's write never changes a page's version.
//!
//! A write joins a call only through a view its own writer serves for the pin
//! the runner stored for the call, as the `cortexkit-role-tool-display`
//! contract requires. That pin comes from the call pins kept while pages were
//! built, never from the app or the write. A
//! call two writers could join shows neither, since one of them is wrong and
//! nothing says which.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use cortexkit_role_tool_display::DisplayKind;
use cortexkit_role_tool_provider::views::ViewManifest;
use occipital_app_reader::source::ReadError;
use serde_json::{json, Map, Value};
use subc_protocol::Principal;
use tokio::sync::broadcast;

use super::store::{DisplayStore, KeptView, Stored, ViewState};
use super::views::Fetcher;

pub const VIEWS: &str = "display.views";
pub const TEMPLATE: &str = "display.template";
pub const RECORDS: &str = "session.tool_records";
pub const DETAIL: &str = "session.tool_detail";
pub const RECORDS_WATCH: &str = "session.tool_records_watch";

/// The most pins one `display.views` asks about.
pub const MAX_PINS: usize = 64;
/// The most calls one `session.tool_records` asks about: a page's worth.
pub const MAX_CALLS: usize = 100;
/// The longest pin or call key accepted. A pin is at most 256 bytes; call
/// keys are shorter.
const MAX_ID_BYTES: usize = 256;

/// What makes an app read again, published to every records watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A write was stored for a call of `runner`.
    Write { runner: String, call_key: String },
    /// A newer template revision was kept for a pin.
    Pin(String),
}

/// How many hints wait for a slow watch before it falls behind. A watch that
/// falls behind ends, and the app reads everything again when it reopens.
pub const HINTS: usize = 1024;

pub struct ToolDisplay {
    store: Arc<DisplayStore>,
    views: Option<Fetcher>,
    hints: broadcast::Sender<Hint>,
    /// Each kept view's checked templates, keyed by the view's and its input
    /// schema's bytes. A view is checked once per process, so a checker fix
    /// in a new build re-checks every view.
    verdicts: Mutex<HashMap<Vec<u8>, Verdict>>,
    /// Every passing template by digest, filled as views are checked.
    templates: Mutex<HashMap<String, Value>>,
}

/// The digests of a view's templates that passed the checks.
#[derive(Debug, Clone, Default)]
struct Verdict {
    row: Option<String>,
    page: Option<String>,
}

/// A call's writes from the one writer that may join it.
#[derive(Debug, Default)]
struct Joined {
    record: Option<Value>,
    completion: Option<Value>,
    detail: Option<Value>,
}

impl ToolDisplay {
    /// Serve `store`'s writes, asking `views` for views not fetched yet.
    /// `hints` is where stored writes and kept revisions are announced.
    pub fn new(
        store: Arc<DisplayStore>,
        views: Option<Fetcher>,
        hints: broadcast::Sender<Hint>,
    ) -> Self {
        Self {
            store,
            views,
            hints,
            verdicts: Mutex::new(HashMap::new()),
            templates: Mutex::new(HashMap::new()),
        }
    }

    /// Where stored writes and kept revisions are announced.
    pub fn hints(&self) -> &broadcast::Sender<Hint> {
        &self.hints
    }

    /// `display.views {pins, held?}`: for each pin, its view, `"unknown"` or
    /// `"unavailable"`. A pin `held` at the kept revision or a higher one is
    /// left out, so an app drawing a chat afresh learns only what changed.
    pub async fn views(self: &Arc<Self>, params: &Value) -> Result<Value, ReadError> {
        let pins = ids(params, "pins", MAX_PINS)?;
        let held = held(params)?;
        let this = self.clone();
        blocking(move || {
            let mut answers = Map::new();
            for pin in pins {
                let kept = this.store.views_for_pin(&pin).map_err(unavailable)?;
                if let Some(answer) = this.answer(&pin, &kept, held.get(&pin).copied()) {
                    answers.insert(pin, answer);
                }
            }
            Ok(json!({ "views": answers }))
        })
        .await
    }

    fn answer(&self, pin: &str, kept: &[(String, KeptView)], held: Option<u64>) -> Option<Value> {
        let views: Vec<(&String, &Vec<u8>, &Vec<u8>)> = kept
            .iter()
            .filter_map(|(writer, kept)| match &kept.state {
                ViewState::View {
                    manifest,
                    input_schema,
                } => Some((writer, manifest, input_schema)),
                _ => None,
            })
            .collect();
        match views.as_slice() {
            [(writer, manifest, input_schema)] => {
                let Ok(view) = serde_json::from_slice::<ViewManifest>(manifest) else {
                    tracing::warn!(%pin, %writer, "a kept view no longer decodes");
                    return Some(json!("unavailable"));
                };
                if held.is_some_and(|held| held >= view.template_revision) {
                    return None;
                }
                let verdict = self.verdict(manifest, input_schema, &view);
                let mut answer = json!({
                    "provider": writer,
                    "recordSchema": view.record_schema,
                    "reads": view.reads,
                    "templateRevision": view.template_revision,
                });
                if let Some(row) = verdict.row {
                    answer["row"] = json!(row);
                }
                if let Some(page) = verdict.page {
                    answer["page"] = json!(page);
                }
                Some(answer)
            }
            // A module whose catalog holds the pin said it has no view.
            [] if kept
                .iter()
                .any(|(_, kept)| matches!(kept.state, ViewState::Unknown)) =>
            {
                Some(json!("unknown"))
            }
            [] => {
                if let Some(views) = &self.views {
                    views.want_pin(pin);
                }
                Some(json!("unavailable"))
            }
            _ => {
                let writers: Vec<&str> = views.iter().map(|(writer, ..)| writer.as_str()).collect();
                tracing::warn!(%pin, ?writers, "several modules serve a view for one pin; drawing none");
                Some(json!("unknown"))
            }
        }
    }

    /// Check a kept view's templates once, keeping each passing one by digest.
    fn verdict(&self, manifest: &[u8], input_schema: &[u8], view: &ViewManifest) -> Verdict {
        let key = [manifest, b"\0", input_schema].concat();
        if let Some(verdict) = self.verdicts.lock().ok().and_then(|v| v.get(&key).cloned()) {
            return verdict;
        }
        let mut verdict = Verdict::default();
        let input_schema: Value = serde_json::from_slice(input_schema).unwrap_or(Value::Null);
        match occipital_registry::view::check_view(
            &view.template,
            &view.record_schema,
            &input_schema,
        ) {
            Ok(reports) => {
                for report in reports {
                    let Some(digest) = report.digest.clone() else {
                        continue;
                    };
                    if !report.passed() {
                        tracing::warn!(
                            slot = report.slot,
                            %digest,
                            problems = ?report.problems,
                            diagnostics = report.diagnostics.len(),
                            "a provider's template fails the checks; its calls draw as the plain chip"
                        );
                        continue;
                    }
                    if let Some(document) = canonical_document(&view.template[report.slot]) {
                        if let Ok(mut templates) = self.templates.lock() {
                            templates.insert(digest.clone(), document);
                        }
                    }
                    match report.slot {
                        "row" => verdict.row = Some(digest),
                        _ => verdict.page = Some(digest),
                    }
                }
            }
            Err(problem) => tracing::warn!(%problem, "a provider's view has a malformed template"),
        }
        if let Ok(mut verdicts) = self.verdicts.lock() {
            verdicts.insert(key, verdict.clone());
        }
        verdict
    }

    /// `display.template {digest}`: a passing template of a kept view.
    pub async fn template(self: &Arc<Self>, params: &Value) -> Result<Value, ReadError> {
        let digest = id(params, "digest")?;
        if !occipital_registry::grammar::is_sha256_digest(&digest) {
            return Err(invalid("digest"));
        }
        let this = self.clone();
        blocking(move || {
            if let Some(document) = this.kept_template(&digest) {
                return Ok(json!({ "template": document }));
            }
            // Not checked yet in this process: check every kept view once.
            for (writer, pin, manifest) in this.store.all_views().map_err(unavailable)? {
                let Some(KeptView {
                    state: ViewState::View { input_schema, .. },
                    ..
                }) = this.store.kept_view(&writer, &pin).map_err(unavailable)?
                else {
                    continue;
                };
                if let Ok(view) = serde_json::from_slice::<ViewManifest>(&manifest) {
                    this.verdict(&manifest, &input_schema, &view);
                }
            }
            this.kept_template(&digest)
                .map(|document| json!({ "template": document }))
                .ok_or_else(|| {
                    ReadError::new(
                        "not_found",
                        "no kept view has a passing template with this digest",
                    )
                })
        })
        .await
    }

    fn kept_template(&self, digest: &str) -> Option<Value> {
        self.templates.lock().ok()?.get(digest).cloned()
    }

    /// `session.tool_records`: for each call of `runner`'s `session` that a
    /// writer may join, its record, its completion, and whether it has a
    /// detail. A call with none of these is left out.
    pub async fn records(
        self: &Arc<Self>,
        runner: &str,
        session: &str,
        params: &Value,
    ) -> Result<Value, ReadError> {
        let call_keys = ids(params, "callKeys", MAX_CALLS)?;
        let (this, runner, session) = (self.clone(), runner.to_owned(), session.to_owned());
        blocking(move || {
            let mut calls = Map::new();
            for call_key in call_keys {
                let Some(joined) = this.join(&runner, &session, &call_key)? else {
                    continue;
                };
                let mut call = json!({ "detail": joined.detail.is_some() });
                if let Some(record) = joined.record {
                    call["record"] = record;
                }
                if let Some(completion) = joined.completion {
                    call["completion"] = completion;
                }
                calls.insert(call_key, call);
            }
            Ok(json!({
                "calls": calls,
                "loadingWindowMs": this.store.limits().loading_window_ms,
            }))
        })
        .await
    }

    /// `session.tool_detail {callKey}`: the call's detail, when it has one.
    pub async fn detail(
        self: &Arc<Self>,
        runner: &str,
        session: &str,
        params: &Value,
    ) -> Result<Value, ReadError> {
        let call_key = id(params, "callKey")?;
        let (this, runner, session) = (self.clone(), runner.to_owned(), session.to_owned());
        blocking(move || {
            let detail = this
                .join(&runner, &session, &call_key)?
                .and_then(|joined| joined.detail);
            Ok(match detail {
                Some(detail) => json!({ "detail": detail }),
                None => json!({}),
            })
        })
        .await
    }

    /// The call's writes from the one writer whose own view of the runner's
    /// pin authorizes it, or `None`: an unknown call, a call of another
    /// session, no such writer, or more than one.
    fn join(
        &self,
        runner: &str,
        session: &str,
        call_key: &str,
    ) -> Result<Option<Joined>, ReadError> {
        let Some((call_session, pin)) =
            self.store.call_pin(runner, call_key).map_err(unavailable)?
        else {
            return Ok(None);
        };
        if call_session != session {
            return Ok(None);
        }
        let runner_principal = Principal::Reserved {
            module_id: runner.to_owned(),
        };
        let mut by_writer: BTreeMap<String, Vec<Stored>> = BTreeMap::new();
        for stored in self
            .store
            .writes(&runner_principal, call_key)
            .map_err(unavailable)?
        {
            if let Principal::Reserved { module_id } = &stored.writer {
                by_writer.entry(module_id.clone()).or_default().push(stored);
            }
        }
        let mut authorized = Vec::new();
        for (writer, writes) in by_writer {
            match self.store.view(&writer, &pin).map_err(unavailable)? {
                Some(ViewState::View { .. }) => authorized.push((writer, writes)),
                // Not fetched yet: ask now, and the next read may join it.
                None => {
                    if let Some(views) = &self.views {
                        views.want(&writer, &pin);
                    }
                }
                Some(_) => {}
            }
        }
        let writes = match authorized.as_slice() {
            [(_, writes)] => writes,
            [] => return Ok(None),
            _ => {
                let writers: Vec<&str> = authorized
                    .iter()
                    .map(|(writer, _)| writer.as_str())
                    .collect();
                tracing::warn!(%runner, %call_key, ?writers, "several writers may join one call; showing none");
                return Ok(None);
            }
        };
        let mut joined = Joined::default();
        for stored in writes {
            let body = serde_json::from_slice::<Value>(&stored.body).ok();
            match stored.kind {
                DisplayKind::Record => joined.record = body,
                DisplayKind::Completion => joined.completion = body,
                DisplayKind::Detail => joined.detail = body,
                // A kind this build doesn't know is never stored (the write
                // is refused), so nothing else can be read back.
                _ => {}
            }
        }
        Ok(
            (joined.record.is_some() || joined.completion.is_some() || joined.detail.is_some())
                .then_some(joined),
        )
    }

    /// Whether a stored write belongs to `runner`'s `session`, so its watch
    /// should say so.
    pub async fn is_session_call(
        self: &Arc<Self>,
        runner: &str,
        session: &str,
        call_key: &str,
    ) -> bool {
        let (this, runner, call_key) = (self.clone(), runner.to_owned(), call_key.to_owned());
        let found =
            tokio::task::spawn_blocking(move || this.store.call_pin(&runner, &call_key)).await;
        matches!(found, Ok(Ok(Some((call_session, _)))) if call_session == session)
    }
}

/// A template document in its canonical form, as its digest covers it.
fn canonical_document(document: &Value) -> Option<Value> {
    let bytes = serde_json::to_vec(document).ok()?;
    let canonical = occipital_registry::Template::parse(&bytes)
        .ok()?
        .to_canonical()
        .ok()?;
    serde_json::from_slice(&canonical).ok()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ReadError> + Send + 'static,
) -> Result<T, ReadError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ReadError::new("internal", format!("display read stopped: {error}")))?
}

/// A required string member, 1 to 256 bytes.
fn id(params: &Value, field: &'static str) -> Result<String, ReadError> {
    params[field]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_ID_BYTES)
        .map(str::to_owned)
        .ok_or_else(|| invalid(field))
}

/// A required array of 1 to `max` distinct such strings, in the order given.
fn ids(params: &Value, field: &'static str, max: usize) -> Result<Vec<String>, ReadError> {
    let items = params[field]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= max)
        .ok_or_else(|| invalid(field))?;
    let mut ids: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let id = item
            .as_str()
            .filter(|text| !text.is_empty() && text.len() <= MAX_ID_BYTES)
            .ok_or_else(|| invalid(field))?;
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// The optional `held` member: the template revision an app holds for each
/// pin, as an object of non-negative integers.
fn held(params: &Value) -> Result<BTreeMap<String, u64>, ReadError> {
    let Some(held) = params.get("held").filter(|held| !held.is_null()) else {
        return Ok(BTreeMap::new());
    };
    let members = held.as_object().ok_or_else(|| invalid("held"))?;
    members
        .iter()
        .map(|(pin, revision)| {
            revision
                .as_u64()
                .map(|revision| (pin.clone(), revision))
                .ok_or_else(|| invalid("held"))
        })
        .collect()
}

fn invalid(field: &str) -> ReadError {
    ReadError::new("invalid_params", format!("{field} is missing or malformed"))
        .with_detail(json!({ "field": field }))
}

fn unavailable(error: impl std::fmt::Display) -> ReadError {
    ReadError::new(
        "provider_unavailable",
        format!("display store unavailable: {error}"),
    )
    .with_detail(json!({ "retryable": true }))
}
