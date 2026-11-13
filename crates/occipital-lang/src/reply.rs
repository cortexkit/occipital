//! Reply streaming, authoritative finalization and late host inputs (§3.10).

mod render;

// Program sessions prepare `Parse` results at the same source positions.
pub(crate) use render::parse_span;

use std::{collections::BTreeMap, sync::Arc};

use cortexkit_rich_ui_syntax::{Mode, StreamParser};

use crate::accounts::{Accounts, Refused, ReplyWork, Reservation, Retention, UnitKind};
use crate::catalog::Catalog;
use crate::eval::{self, snapshot, tree::Tree, Inputs};
use crate::geometry::{self, LayoutContext, LayoutKey, MeasureRevision, Measurement};
use crate::prepare::resolve::Resolved;
use crate::{
    CheckContext, Diagnostic, DisplayText, HandleRecord, Profile, ReferenceRecord, ReferenceState,
    SourceSpan,
};

#[derive(Clone, Debug)]
/// One reply revision: provisional while streaming, authoritative after the
/// final call (§3.10).
pub struct Revision {
    /// The revision number within the reply.
    pub id: u64,
    /// Whether this is a provisional streaming view.
    pub provisional: bool,
    /// The tree to draw.
    pub tree: Arc<Tree>,
    /// The source ranges drawn as literal text: failed tags and their text forms.
    pub literals: Vec<SourceSpan>,
    /// The diagnostics, for reports from the app (§12.3).
    pub diagnostics: Vec<Diagnostic>,
    /// The §11.3 plain text.
    pub plain_text: DisplayText,
    /// The reply as plain text for an app that doesn't hold its catalog (a
    /// reply stamped with a retired digest): the plain text with a table as
    /// its rows (header first, cells tab-separated, at most 200 rows) and a
    /// badge as its bare label. A provisional revision carries the words
    /// streamed so far, one block per line.
    pub readable_text: DisplayText,
    state_hash: String,
    _retention: Arc<Retention>,
}

impl Revision {
    /// The revision as input to [`measure`](crate::measure).
    pub fn measure_revision(&self) -> MeasureRevision<'_> {
        MeasureRevision {
            instance_epoch: 0,
            revision: self.id,
            state_hash: &self.state_hash,
            tree: &self.tree,
            first: None,
        }
    }

    /// The layout key this revision measures under in `layout`.
    pub fn layout_key(&self, layout: &LayoutContext) -> LayoutKey {
        geometry::layout_key(&self.measure_revision(), layout)
    }
}

/// A late host event for a finished reply.
pub enum Event {
    /// Fetched snapshot bytes, or a failed fetch (§10.1).
    Snapshot {
        /// N in `$$tc(N)`.
        reference: u64,
        /// The bytes, or `Err(())` when the fetch failed.
        result: Result<Vec<u8>, ()>,
    },
    /// A handle record became available or unavailable.
    HandleAvailability {
        /// The record's id.
        record_id: String,
        /// Whether it is available now.
        available: bool,
    },
    /// A shared account released budget, so refused work may be retried.
    AccountRelease,
    /// The layout context changed.
    Layout(LayoutContext),
    /// A measurement of the current candidate.
    MeasureComplete(Box<Measurement>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Why a reply session refused a call.
pub enum Error {
    /// The profile is not `reply`.
    Profile,
    /// The catalog is not the embedded catalog v1.
    Catalog,
    /// The final call was already made.
    Finished,
    /// The event needs the final call first.
    NotFinished,
    /// An account refused the work.
    Account(Refused),
}

impl From<Refused> for Error {
    fn from(error: Refused) -> Self {
        Self::Account(error)
    }
}

/// A host-owned instance. The catalog is the pinned v1 catalog, never source data.
/// Chunk results are provisional views with no actions or fetch identities.
/// Finalization binds the reference and handle records supplied by the host.
pub struct Session {
    context: CheckContext,
    layout: LayoutContext,
    accounts: Accounts,
    work: Option<ReplyWork>,
    parser: Option<StreamParser>,
    source: Vec<u8>,
    _source_retention: [Reservation; 2],
    frozen: bool,
    program: Option<Resolved>,
    snapshots: BTreeMap<u64, Result<Vec<u8>, ()>>,
    snapshot_retention: BTreeMap<u64, Reservation>,
    prepared: BTreeMap<snapshot::ParseKey, crate::data::Prepared>,
    revision: u64,
    /// The latest revision to measure.
    pub candidate: Option<Revision>,
    /// The committed revision and its measurement.
    pub committed: Option<(Revision, Measurement)>,
}

impl Session {
    /// A session for one reply. `dialect` is the dialect the reply was taught;
    /// only `1.0` checks without `E_UNSUPPORTED_VERSION`. `layout` carries the
    /// recorded network policy. Parse preparation is charged to the reply's own
    /// aggregate and to the shared `chat` and `app` accounts (§9.1).
    pub fn new(
        profile: Profile,
        dialect: String,
        catalog: &Catalog,
        layout: LayoutContext,
        chat: crate::SharedAccount,
        app: crate::SharedAccount,
    ) -> Result<Self, Error> {
        if profile != Profile::Reply {
            return Err(Error::Profile);
        }
        // Evaluation and layout use the embedded catalog, never a host
        // override, so a session cannot claim to run under another catalog.
        if !catalog.is_embedded() {
            return Err(Error::Catalog);
        }
        let accounts = Accounts::new(crate::AccountScope::Reply, chat, app)
            .map_err(|e| Error::Account(Refused::Scope(e)))?;
        let work = ReplyWork::new(&accounts)?;
        // Charge both bounded source copies before either can grow: the session
        // keeps a finalization copy even if the streaming parser is discarded.
        let source_lease = || {
            accounts
                .retain(crate::RetainedAllocation {
                    requested: cortexkit_rich_ui_syntax::REPLY_BYTES as u64 + 4,
                    unused_capacity: 0,
                })
                .map_err(Refused::Aggregate)
        };
        let source_retention = [source_lease()?, source_lease()?];
        let mut context = CheckContext::new(profile);
        context.execution_dialect = dialect;
        Ok(Self {
            context,
            layout,
            accounts,
            work: Some(work),
            parser: Some(StreamParser::new(Mode::Reply)),
            source: Vec::with_capacity(cortexkit_rich_ui_syntax::REPLY_BYTES + 4),
            frozen: false,
            _source_retention: source_retention,
            program: None,
            snapshots: BTreeMap::new(),
            snapshot_retention: BTreeMap::new(),
            prepared: BTreeMap::new(),
            revision: 0,
            candidate: None,
            committed: None,
        })
    }

    fn append(&mut self, chunk: &[u8]) {
        // Three bytes complete a split UTF-8 scalar. The fourth records that
        // input exceeded the cap even if the streaming parser was discarded.
        let left = (cortexkit_rich_ui_syntax::REPLY_BYTES + 4).saturating_sub(self.source.len());
        self.source
            .extend_from_slice(&chunk[..chunk.len().min(left)]);
    }

    /// Feed streamed bytes. Returns a provisional revision when the visible
    /// picture changed, or `None`.
    pub fn chunk(&mut self, chunk: &[u8]) -> Result<Option<Revision>, Error> {
        if self.program.is_some() {
            return Err(Error::Finished);
        }
        self.append(chunk);
        let mut latest = None;
        // Canonical byte steps make both promotion work and the freeze point
        // independent of transport chunk boundaries (§3.10).
        for byte in chunk {
            if self.frozen {
                break;
            }
            let parser = self.parser.as_mut().ok_or(Error::Finished)?;
            let picture = parser.feed(std::slice::from_ref(byte));
            let work = parser.work();
            // The existing conservative parse tariff is two fuel per examined byte.
            // The syntax crate reports bounded reexamination separately from scanning.
            let fuel = 2 * (work.scanned as u64 + work.reexamined as u64);
            if self
                .work
                .as_mut()
                .ok_or(Error::Finished)?
                .provisional(fuel)
                .is_err()
            {
                self.frozen = true;
                self.parser = None;
                break;
            }
            let Some(picture) = picture else {
                continue;
            };
            if self
                .work
                .as_mut()
                .ok_or(Error::Finished)?
                .provisional(render::promotion_fuel(&picture))
                .is_err()
            {
                self.frozen = true;
                self.parser = None;
                break;
            }
            let tree = render::provisional(&picture);
            let readable = render::provisional_text(&tree);
            let revision =
                self.publish(tree, vec![], vec![], (DisplayText::new(""), readable), true)?;
            latest = Some(revision);
        }
        Ok(latest)
    }

    /// Append the remaining bytes and bind records stamped at storage. With no
    /// chunk calls this is the stored-reply replay path, not a second parser.
    pub fn finish(
        &mut self,
        tail: &[u8],
        references: BTreeMap<u64, ReferenceRecord>,
        handles: Vec<HandleRecord>,
    ) -> Result<Revision, Error> {
        if self.program.is_some() {
            return Err(Error::Finished);
        }
        self.append(tail);
        let parser = self.parser.take();
        let work = self.work.take().ok_or(Error::Finished)?;
        let document = work.finish(|| {
            let document = if let Some(parser) = parser.filter(|_| tail.is_empty()) {
                parser.finish()
            } else {
                cortexkit_rich_ui_syntax::parse_bytes(&self.source, Mode::Reply)
            };
            let bytes =
                crate::prepare::source_prefix(&self.source, cortexkit_rich_ui_syntax::REPLY_BYTES)
                    .len();
            (document, bytes)
        })?;
        self.context.references = references;
        self.context.handles = handles;
        let parsed = crate::prepare::from_document(&self.source, Mode::Reply, document)?;
        let mut program = crate::check::check_parsed(parsed, &self.context);
        render::literalize(&mut program, &self.source);
        self.program = Some(program);
        self.refresh()
    }

    /// Apply a late event. Returns a revision to measure when the tree changed.
    pub fn event(&mut self, event: Event) -> Result<Option<Revision>, Error> {
        if self.program.is_none() && !matches!(event, Event::Layout(_) | Event::MeasureComplete(_))
        {
            return Err(Error::NotFinished);
        }
        match event {
            Event::MeasureComplete(measured) => {
                if let Some(candidate) = &self.candidate {
                    if candidate.layout_key(&self.layout) == measured.key {
                        // Full admission remains Deferred until the text-run and
                        // decoded-byte reservation rules are defined (§9.1).
                        self.committed = Some((candidate.clone(), *measured));
                    }
                }
                Ok(None)
            }
            Event::Layout(layout) => {
                self.layout = layout;
                Ok(self.candidate.clone())
            }
            Event::Snapshot { reference, result } => {
                if !self.context.references.contains_key(&reference) {
                    return Ok(None);
                }
                if self.snapshots.get(&reference) == Some(&result) {
                    return Ok(None);
                }
                if let Ok(bytes) = &result {
                    let lease = self
                        .accounts
                        .retain(crate::RetainedAllocation {
                            requested: bytes.capacity() as u64,
                            unused_capacity: 0,
                        })
                        .map_err(Refused::Aggregate)?;
                    self.snapshot_retention.insert(reference, lease);
                } else {
                    self.snapshot_retention.remove(&reference);
                }
                self.snapshots.insert(reference, result);
                self.prepared.retain(|key, _| {
                    key.source != snapshot::SnapshotSource::Reference(reference.to_string())
                });
                self.refresh().map(Some)
            }
            Event::HandleAvailability {
                record_id,
                available,
            } => {
                let mut changed = false;
                for record in &mut self.context.handles {
                    if record.record_id == record_id && record.available != available {
                        record.available = available;
                        changed = true;
                    }
                }
                if changed {
                    self.refresh().map(Some)
                } else {
                    Ok(None)
                }
            }
            Event::AccountRelease => self.refresh().map(Some),
        }
    }

    fn refresh(&mut self) -> Result<Revision, Error> {
        let program = self.program.as_mut().ok_or(Error::NotFinished)?;
        let mut inputs = Inputs::new(Profile::Reply);
        inputs.network = self.layout.network != geometry::Network::No;
        inputs.handles = self.context.handles.clone();
        for (n, record) in &self.context.references {
            let fetched = self.snapshots.get(n);
            let mut record = record.clone();
            if matches!(fetched, Some(Err(()))) {
                record.state = ReferenceState::Unresolved;
                record.reason = Some("unavailable".into());
            }
            let bytes = fetched.and_then(|r| r.as_ref().ok()).map(Vec::as_slice);
            inputs.references.insert(
                n.to_string(),
                snapshot::snapshot(&n.to_string(), &record, bytes, None),
            );
        }
        let mut diagnostics = program.diagnostics.clone();
        for key in snapshot::parse_keys(program) {
            let snapshot::SnapshotSource::Reference(n) = &key.source else {
                continue;
            };
            let Some(n) = n.parse::<u64>().ok() else {
                continue;
            };
            if self
                .context
                .references
                .get(&n)
                .is_none_or(|r| r.state != ReferenceState::Resolved)
            {
                continue;
            }
            let Some(Ok(bytes)) = self.snapshots.get(&n) else {
                continue;
            };
            if !self.prepared.contains_key(&key) {
                // A preparation without a source location must not fabricate an
                // error origin if a future syntax shape cannot be traversed.
                let Some(span) = render::parse_span(program, &key) else {
                    continue;
                };
                let prepared = crate::data::prepare_accounted(
                    &self.accounts,
                    UnitKind::DataPreparation,
                    bytes,
                    key.format,
                    &key.options,
                    span,
                )?;
                self.prepared.insert(key.clone(), prepared);
            }
            if let Some(prepared) = self.prepared.get(&key) {
                diagnostics.extend(prepared.diagnostics.clone());
            }
        }
        inputs.prepared = self.prepared.clone();
        let evaluated = eval::evaluate(program, &inputs);
        let evaluation_diagnostics = evaluated.diagnostics;
        diagnostics.extend(evaluation_diagnostics.clone());
        let mut tree = evaluated.tree.unwrap_or(Tree { blocks: vec![] });
        if render::failed_cells(program, &mut tree, &self.source) {
            program.diagnostics.extend(evaluation_diagnostics);
        }
        render::late_states(&mut tree, &self.context, &self.snapshots);
        render::render_limits(&tree, &mut diagnostics);
        let literals = render::literal_spans(program);
        let texts = render::plain_text(program, &self.source, &tree, &self.context);
        self.publish(tree, literals, diagnostics, texts, false)
    }

    fn publish(
        &mut self,
        tree: Tree,
        literals: Vec<SourceSpan>,
        diagnostics: Vec<Diagnostic>,
        (plain_text, readable_text): (DisplayText, DisplayText),
        provisional: bool,
    ) -> Result<Revision, Error> {
        let retention = self.accounts.retain_tree(&tree)?;
        self.revision += 1;
        let state_hash = hmac_sha256::Hash::hash(format!("{tree:?}").as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let revision = Revision {
            id: self.revision,
            provisional,
            state_hash,
            tree: Arc::new(tree),
            literals,
            diagnostics,
            plain_text,
            readable_text,
            _retention: retention,
        };
        self.candidate = Some(revision.clone());
        Ok(revision)
    }
}

// Golden-trace support is compiled for tests and for the `traces` feature,
// so apps can run the committed §16 traces themselves (§14.1).
#[cfg(any(test, feature = "traces"))]
#[cfg_attr(not(test), allow(dead_code))]
mod support;
#[cfg(any(test, feature = "traces"))]
pub(crate) mod traces;

#[cfg(test)]
mod tests;
