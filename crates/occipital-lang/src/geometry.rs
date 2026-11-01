//! Catalog geometry uses only committed tree inputs and explicit host metrics.

use std::collections::BTreeMap;

use crate::eval::tree::{Tree, TreeNode, TreeValue};
use crate::{Decimal, DiagnosticCode, NumberError};

mod layout;
mod work;

pub(crate) use layout::render_state;

pub(crate) use work::known_work as known_work_visits;

/// The named styles passed by the catalog to the platform text system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextStyle {
    /// The body style.
    Body,
    /// The subheadline style.
    Subheadline,
    /// The footnote style.
    Footnote,
    /// The first caption style.
    Caption1,
    /// The headline style.
    Headline,
    /// The second title style.
    Title2,
    /// The third title style.
    Title3,
    /// Monospaced body text.
    Mono,
    /// Monospaced footnote text.
    MonoFootnote,
}

/// The only platform callbacks used by formula geometry. `None` means `inf`
/// for T's line limit. Lengths use the rich UI language's Decimal layout units.
pub trait Metrics {
    /// `L(style)`: the line height of one line in `style`.
    fn l(&self, style: TextStyle) -> Decimal;
    /// `T(text, style, width, lines)`: the height of `text` wrapped to
    /// `width`, cut at `max_lines` lines (`None` for no limit).
    fn t(&self, text: &str, style: TextStyle, width: Decimal, max_lines: Option<u32>) -> Decimal;
    /// `TW(text, style)`: the width of `text` on one line.
    fn tw(&self, text: &str, style: TextStyle) -> Decimal;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Where a revision is drawn.
pub enum Placement {
    /// In a transcript row.
    Row,
    /// On its own page.
    Page,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The recorded network policy that changes what images and maps draw.
pub enum Network {
    /// Network content may load.
    Yes,
    /// Network content may not load.
    No,
    /// The policy is not known.
    Unknown,
}

/// Inputs that change a measurement without changing the evaluated revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutContext {
    /// `W`: the available width in layout units.
    pub width: Decimal,
    /// The platform's content size category, such as `large`.
    pub content_size_category: String,
    /// The recorded network policy.
    pub network: Network,
    /// Row or page placement.
    pub placement: Placement,
}

/// An immutable evaluated tree plus the session's identity and state digest.
/// State hashes are supplied by evaluation, not reconstructed from display text.
pub struct MeasureRevision<'a> {
    /// The instance epoch the revision belongs to.
    pub instance_epoch: u64,
    /// The revision number.
    pub revision: u64,
    /// A digest of the state the revision was evaluated with.
    pub state_hash: &'a str,
    /// The evaluated tree.
    pub tree: &'a Tree,
    /// The defaults tree used only by host.RowFrame, never a saved height.
    pub first: Option<&'a Tree>,
}

/// Exactly the catalog cacheKey fields, with an epoch to scope revision ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutKey {
    /// The instance epoch.
    pub instance_epoch: u64,
    /// The revision number.
    pub revision: u64,
    /// The state digest.
    pub state_hash: String,
    /// `W`.
    pub width: Decimal,
    /// The content size category.
    pub content_size_category: String,
    /// The catalog digest (§13).
    pub catalog_digest: String,
    /// The catalog layout version (§13).
    pub layout_version: String,
    /// The network policy.
    pub network: Network,
    /// The placement.
    pub placement: Placement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The outcome of a revision's `perRevision` reservation, sent back with
/// `measure_complete` and applied when the revision commits.
pub enum ReservationOutcome {
    /// Known work alone does not decide whether a revision fits its resource
    /// limits. That decision waits for text-run and decoded-byte reservation rules.
    Deferred,
    /// Carries an existing evaluation abort reported by the host. Reservation
    /// admission remains deferred until text-run and decoded-byte rules are supplied.
    Abort(crate::eval::Abort),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A measured height.
pub enum Height {
    /// A height in whole layout units.
    Known(Decimal),
    /// Markdown and prose-measured children need the prose measurement rules.
    ProseDeferred,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// How a measured component draws.
pub enum RenderState {
    /// Its data is drawn.
    Ready,
    /// Its data is loading.
    Loading,
    /// Its snapshot is held: the call has not finished.
    Held,
    /// Its data is unavailable.
    Unavailable,
    /// Its fixed-size error state, with the code.
    Error(DiagnosticCode),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One measured component.
pub struct MeasuredBox {
    /// Indices in blocks/children, with prop names and list indices for nodes
    /// stored in props. This remains stable when late data changes state.
    pub path: String,
    /// Its height.
    pub height: Decimal,
    /// Its width.
    pub width: Decimal,
    /// How it draws.
    pub state: RenderState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The result of [`measure`]: send it back in a `MeasureComplete` event.
pub struct Measurement {
    /// The content height.
    pub height: Height,
    /// The layout key the measurement was made under.
    pub key: LayoutKey,
    /// Sum of catalog work.visits only, not the complete perRevision tariff.
    pub known_work_visits: u64,
    /// The reservation outcome.
    pub reservation: ReservationOutcome,
    /// Every measured component, by path.
    pub boxes: Vec<MeasuredBox>,
    pub(crate) tables: Vec<work::TableLayout>,
    /// The row frame, for a `show_ui` revision in row placement.
    pub row_frame: Option<RowFrame>,
}

/// The defaults-tree height, frame height, and clipped flag are separate from
/// the measured content height.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowFrame {
    /// The unframed height of the defaults tree, `H(first)`.
    pub baseline: Height,
    /// The frame height, `min(H(first), cap) + k.tap`.
    pub height: Height,
    /// Whether the content is taller than the frame ("Open to see all").
    pub clipped: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Why a revision could not be measured.
pub enum MeasureError {
    /// Decimal arithmetic failed.
    Number(NumberError),
    /// The tree breaks a catalog shape the formulas need; the text says how.
    InvalidTree(String),
    /// The app owns this component's measurement; it is not prose geometry.
    GenericToolRow,
}

impl From<NumberError> for MeasureError {
    fn from(error: NumberError) -> Self {
        Self::Number(error)
    }
}

pub(crate) fn layout_key(revision: &MeasureRevision<'_>, context: &LayoutContext) -> LayoutKey {
    let catalog = crate::catalog::Catalog::shared();
    LayoutKey {
        instance_epoch: revision.instance_epoch,
        revision: revision.revision,
        state_hash: revision.state_hash.to_owned(),
        width: context.width,
        content_size_category: context.content_size_category.clone(),
        catalog_digest: crate::catalog::Catalog::embedded_digest().to_owned(),
        layout_version: catalog.layout_version().to_owned(),
        network: context.network,
        placement: context.placement,
    }
}

/// Pure evaluation of formula-defined heights. Admission is deliberately
/// deferred, even when the known work alone exceeds the perRevision cap.
pub fn measure(
    revision: &MeasureRevision<'_>,
    context: &LayoutContext,
    metrics: &impl Metrics,
) -> Result<Measurement, MeasureError> {
    let mut measured = measure_content(revision, context, metrics)?;
    if let Some(first) = revision
        .first
        .filter(|_| context.placement == Placement::Row)
    {
        let baseline = measure_content(
            &MeasureRevision {
                tree: first,
                first: None,
                ..*revision
            },
            context,
            metrics,
        )?
        .height;
        let (height, clipped) = match (baseline, measured.height) {
            (Height::Known(baseline), Height::Known(content)) => {
                let cap = k("rowCapMin")?.max(d(16).checked_mul(metrics.l(TextStyle::Body))?);
                let height = baseline.min(cap).checked_add(k("tap")?)?.ceil()?;
                (Height::Known(height), Some(content > height))
            }
            _ => (Height::ProseDeferred, None),
        };
        measured.row_frame = Some(RowFrame {
            baseline,
            height,
            clipped,
        });
    }
    Ok(measured)
}

fn measure_content(
    revision: &MeasureRevision<'_>,
    context: &LayoutContext,
    metrics: &impl Metrics,
) -> Result<Measurement, MeasureError> {
    valid(context.width)?;
    // Count known work before consulting metrics. It does not reserve text
    // runs or decoded bytes, whose pre-layout rules are not defined yet.
    let known_work_visits = known_work_visits(revision.tree, context.placement)?;
    let mut engine = Engine {
        context,
        metrics,
        lines: BTreeMap::new(),
        boxes: Vec::new(),
        tables: Vec::new(),
        citations: citation_ids(revision.tree),
        first: revision.first,
    };
    let height = match engine.stack(&revision.tree.blocks, context.width, "", k("gap.m")?) {
        Ok(height) => Height::Known(height),
        Err(LayoutError::Prose) => Height::ProseDeferred,
        Err(LayoutError::Measure(error)) => return Err(error),
    };
    Ok(Measurement {
        height,
        key: layout_key(revision, context),
        known_work_visits,
        reservation: ReservationOutcome::Deferred,
        boxes: engine.boxes,
        tables: engine.tables,
        row_frame: None,
    })
}

struct Engine<'a, M> {
    context: &'a LayoutContext,
    metrics: &'a M,
    lines: BTreeMap<TextStyle, Decimal>,
    boxes: Vec<MeasuredBox>,
    tables: Vec<work::TableLayout>,
    citations: Vec<String>,
    first: Option<&'a Tree>,
}

enum LayoutError {
    Prose,
    Measure(MeasureError),
}

impl From<MeasureError> for LayoutError {
    fn from(error: MeasureError) -> Self {
        Self::Measure(error)
    }
}

impl From<NumberError> for LayoutError {
    fn from(error: NumberError) -> Self {
        Self::Measure(MeasureError::Number(error))
    }
}

type ResultH = Result<Decimal, LayoutError>;
type Props = [(crate::DisplayText, TreeValue)];

/// A whole number for a formula: a small literal or an in-memory count, all
/// far inside `i64`, so the conversion cannot fail. A value beyond `i64`
/// would saturate rather than fail.
fn d(n: i128) -> Decimal {
    let n = i64::try_from(n).unwrap_or(if n < 0 { i64::MIN } else { i64::MAX });
    Decimal::from_i64(n)
}

/// A catalog layout constant such as `gap.m`. A name the catalog does not
/// define can only come from a tree that breaks its shape, such as a host-built
/// `Card` with an unknown `gap`; evaluated trees only hold catalog values.
fn k(name: &str) -> Result<Decimal, MeasureError> {
    crate::catalog::Catalog::shared()
        .layout_constant(name)
        .ok_or_else(|| MeasureError::InvalidTree(format!("layout constant {name}")))
}

fn valid(value: Decimal) -> Result<Decimal, MeasureError> {
    if value < Decimal::ZERO {
        Err(MeasureError::Number(NumberError::Arg))
    } else {
        Ok(value)
    }
}

fn prop<'a>(props: &'a Props, key: &str) -> Option<&'a TreeValue> {
    props
        .iter()
        .find(|(name, _)| name.as_str() == key)
        .map(|(_, value)| value)
}

fn text<'a>(props: &'a Props, key: &str) -> Option<&'a str> {
    match prop(props, key) {
        Some(TreeValue::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn number(props: &Props, key: &str, default: i128) -> Decimal {
    match prop(props, key) {
        Some(TreeValue::Number(value)) => *value,
        _ => d(default),
    }
}

fn has(props: &Props, key: &str) -> bool {
    !matches!(prop(props, key), None | Some(TreeValue::Null))
}

fn list<'a>(props: &'a Props, key: &str) -> &'a [TreeValue] {
    match prop(props, key) {
        Some(TreeValue::List(values)) => values,
        _ => &[],
    }
}

fn rows(props: &Props, placement: Placement, default: i128) -> Decimal {
    let rows = number(props, "displayRows", default);
    if placement == Placement::Row {
        rows.min(d(10))
    } else {
        rows
    }
}

fn aspect(props: &Props, default: &str) -> Decimal {
    match text(props, "aspect").unwrap_or(default) {
        "1:1" => Decimal::ONE,
        "4:3" => Decimal::from_scaled(75, 2),
        _ => Decimal::from_scaled(5625, 4),
    }
}

fn citation_identity(props: &Props) -> String {
    match prop(props, "source") {
        Some(TreeValue::Snapshot { record_id, .. }) => format!("snapshot:{record_id}"),
        Some(TreeValue::Handle(id)) => format!("handle:{id}"),
        _ => "unresolved".to_owned(),
    }
}

// Assign indices in tree order before width planning can visit later siblings.
fn citation_ids(tree: &Tree) -> Vec<String> {
    fn value(item: &TreeValue, ids: &mut Vec<String>) {
        match item {
            TreeValue::Node(node) => visit(node, ids),
            TreeValue::List(items) => {
                for item in items {
                    value(item, ids);
                }
            }
            _ => {}
        }
    }
    fn visit(node: &TreeNode, ids: &mut Vec<String>) {
        match node {
            TreeNode::Component {
                name,
                props,
                children,
            } => {
                if name.as_str() == "Citation" {
                    let identity = citation_identity(props);
                    if !ids.contains(&identity) {
                        ids.push(identity);
                    }
                }
                for (_, item) in props {
                    value(item, ids);
                }
                for child in children {
                    visit(child, ids);
                }
            }
            TreeNode::Markdown { children, .. } => {
                for child in children {
                    visit(child, ids);
                }
            }
            _ => {}
        }
    }
    let mut ids = Vec::new();
    for node in &tree.blocks {
        visit(node, &mut ids);
    }
    ids
}

#[cfg(test)]
mod tests;
