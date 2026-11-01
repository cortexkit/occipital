//! The program examples of §16.1–§16.7, checked in their profiles and driven
//! through a session with scripted answers. Each produces a golden trace.
//!
//! Visible-text assertions are derived by hand from the example source, the
//! fixtures and the spec (§6.4 formatting, §8.3–§8.6 envelopes). The rest of
//! each golden (full trees, effect identities, epochs, fuel) is a recorded
//! regression snapshot.

// This module runs only on the committed §16 golden fixtures (in tests and
// with the `traces` feature), never on program or reply input. A panic here
// is a failed trace, not a crash on untrusted input, so the library's
// no-panic lints do not apply.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::fmt::Write;

use super::identity::ReadVersion;
use super::support::*;
use super::*;
use crate::eval::tree::{Tree, TreeNode, TreeValue};
#[cfg(test)]
use crate::section16::golden;
use crate::section16::{self, fixture, host_value, visible_text, write_tree};
use crate::{
    CheckContext, CheckResult, DataFormat, Profile, ReferenceRecord, ReferenceState, Severity,
    ShowUiArguments, ToolInputs,
};

/// Read versions for every test registry entry. They only make identities
/// concrete; they authorize nothing.
fn section_reads() -> Reads {
    let mut reads = reads();
    reads.registry = section16::registry();
    reads.versions = reads
        .registry
        .0
        .keys()
        .map(|name| {
            (
                name.clone(),
                ReadVersion {
                    operation: format!("{name}-v1"),
                    schema: format!("{name}-schema-1"),
                },
            )
        })
        .collect();
    reads
}

struct Example {
    name: &'static str,
    source: &'static str,
    profile: Profile,
    tool: Option<ToolInputs>,
    show_ui: ShowUiArguments,
    data: Value,
}

impl Example {
    fn new(name: &'static str, source: &'static str, profile: Profile) -> Self {
        Self {
            name,
            source,
            profile,
            tool: None,
            show_ui: ShowUiArguments::default(),
            data: Value::null(),
        }
    }

    fn with_tool(mut self, tool: ToolInputs) -> Self {
        self.tool = Some(tool);
        self
    }

    fn context(&self) -> CheckContext {
        let mut context = CheckContext::new(self.profile);
        // Only a page names operations (§2.3); rows and these show_ui
        // examples read none.
        if self.profile == Profile::ToolPage {
            context.registry = section16::registry();
        }
        context.tool = self.tool.clone();
        context.show_ui = self.show_ui.clone();
        context
    }

    fn inputs(&self) -> Inputs {
        let mut inputs = Inputs::new(self.profile);
        if let Some(tool) = &self.tool {
            inputs.input = host_value(&tool.input.value);
            inputs.view = host_value(&tool.view.value);
        }
        inputs.data = self.data.clone();
        inputs
    }

    /// The §12.2 check result. Every §16.1–§16.7 example checks with no
    /// error-severity diagnostic in its profile.
    fn check(&self) -> CheckResult {
        let result = crate::check::trial::check_program(
            self.source.as_bytes(),
            &self.context(),
            &self.inputs(),
        )
        .unwrap();
        assert!(
            result
                .diagnostics()
                .iter()
                .all(|d| d.severity() != Severity::Error),
            "{}: {:?}",
            self.name,
            result.diagnostics()
        );
        result
    }

    fn session(&self) -> Session {
        let program = crate::check::check(self.source.as_bytes(), &self.context()).unwrap();
        let placement = if self.profile == Profile::ToolPage {
            Placement::Page
        } else {
            Placement::Row
        };
        Session::new(
            program,
            self.inputs(),
            section_reads(),
            LayoutContext {
                placement,
                ..layout(320)
            },
            DisplayText::new("caller fallback"),
        )
    }

    /// Start a trace: the source name, the §12.2 check result and a fresh
    /// session that has received its load event.
    fn start(&self, trace: &mut String, case: &str) -> (Session, Output) {
        let check = self.check();
        writeln!(
            trace,
            "example={} case={case} profile={}\ncheck={}",
            self.name,
            self.profile.name(),
            serde_json::to_string(&check).unwrap()
        )
        .unwrap();
        let mut session = self.session();
        let out = session.batch(vec![Event::Load {
            epoch: 42,
            inputs: Box::new(self.inputs()),
            restored: BTreeMap::new(),
        }]);
        record(trace, "load", &out);
        settle(trace, &mut session, "load", &out);
        (session, out)
    }
}

/// v1 is read-only (§7.4, §8.4): a session may ask for reads and timers but
/// never for a write or a confirmation. None of these examples hands off to
/// the composer or an address sheet either, so every effect is a read effect
/// or a timer.
fn read_only(out: &Output) {
    for effect in &out.effects {
        assert!(
            matches!(
                effect,
                Effect::Issue { .. }
                    | Effect::Subscribe { .. }
                    | Effect::Credit { .. }
                    | Effect::Page { .. }
                    | Effect::Cancel { .. }
                    | Effect::Arm(..)
                    | Effect::Stop(..)
            ),
            "unexpected effect {effect:?}"
        );
    }
}

fn effect_line(effect: &Effect) -> String {
    let epoch = |e: &Epoch| format!("{}:{}", e.instance, e.request);
    match effect {
        Effect::Issue {
            site,
            identity,
            epoch: e,
        } => format!(
            "issue {site} {} {} epoch={} refresh={:?}",
            identity.operation,
            identity.arguments,
            epoch(e),
            identity.refresh.map(|r| r.to_string())
        ),
        Effect::Subscribe {
            site,
            identity,
            epoch: e,
            after,
        } => format!(
            "subscribe {site} {} {} epoch={} after={after:?}",
            identity.operation,
            identity.arguments,
            epoch(e)
        ),
        Effect::Page {
            site,
            identity,
            epoch: e,
            cursor,
            limit,
        } => format!(
            "page {site} {} {} epoch={} cursor={cursor:?} limit={limit}",
            identity.operation,
            identity.arguments,
            epoch(e)
        ),
        Effect::Cancel {
            site,
            identity,
            epoch: e,
        } => format!("cancel {site} {} epoch={}", identity.operation, epoch(e)),
        other => format!("{other:?}"),
    }
}

/// One batch's output: counters, effects, diagnostics, the candidate tree and
/// any commit, in that order.
fn record(trace: &mut String, label: &str, out: &Output) {
    read_only(out);
    writeln!(
        trace,
        "{label}: evaluations={} fuel={} rejected={:?} abort={:?}",
        out.evaluations, out.fuel, out.rejected, out.abort
    )
    .unwrap();
    for effect in &out.effects {
        writeln!(trace, "  {}", effect_line(effect)).unwrap();
    }
    for diagnostic in &out.diagnostics {
        writeln!(
            trace,
            "  diagnostic {}",
            serde_json::to_string(diagnostic).unwrap()
        )
        .unwrap();
    }
    if let Some(candidate) = &out.candidate {
        writeln!(
            trace,
            "  candidate revision={} loading={} controls={} bindings={:?}",
            candidate.revision,
            candidate.loading,
            candidate.controls.len(),
            candidate.bindings.values().collect::<Vec<_>>()
        )
        .unwrap();
        write_tree(trace, &candidate.tree);
    }
    if let Some(committed) = &out.committed {
        writeln!(
            trace,
            "  committed revision={} fallback={}",
            committed.candidate.revision, committed.fallback
        )
        .unwrap();
    }
}

/// Measure a new candidate as the host would, and record the commit.
fn settle(trace: &mut String, session: &mut Session, label: &str, out: &Output) {
    if let Some(candidate) = &out.candidate {
        let measured = session.batch(vec![measure(candidate)]);
        record(trace, &format!("{label}/measure"), &measured);
        assert_eq!(
            session.committed.as_ref().map(|c| c.candidate.revision),
            Some(candidate.revision),
            "{label}: the current candidate commits"
        );
    }
}

/// Run one batch of events, record it and commit its candidate.
fn drive(
    trace: &mut String,
    session: &mut Session,
    label: &str,
    events: impl FnOnce(&Session) -> Vec<Event>,
) -> Output {
    let events = events(session);
    let out = session.batch(events);
    record(trace, label, &out);
    settle(trace, session, label, &out);
    out
}

fn committed(session: &Session) -> &Candidate {
    &session.committed.as_ref().unwrap().candidate
}

fn shown(session: &Session) -> String {
    visible_text(&committed(session).tree)
}

fn has(session: &Session, expected: &str) {
    let text = shown(session);
    assert!(
        text.contains(expected),
        "expected {expected:?} in the committed tree:\n{text}"
    );
}

fn lacks(session: &Session, unexpected: &str) {
    let text = shown(session);
    assert!(
        !text.contains(unexpected),
        "did not expect {unexpected:?} in the committed tree:\n{text}"
    );
}

/// The component that owns a control's action prop, found by following the
/// control's path through the tree (§7.3 paths: props before children).
fn owner<'a>(tree: &'a Tree, path: &[PathPart]) -> Option<&'a TreeNode> {
    enum At<'a> {
        Root,
        Node(&'a TreeNode),
        Value(&'a TreeValue),
    }
    let (_, prefix) = path.split_last()?;
    let mut at = At::Root;
    for part in prefix {
        at = match (at, part) {
            (At::Root, PathPart::Child(i)) => At::Node(tree.blocks.get(*i)?),
            (
                At::Node(
                    TreeNode::Component { children, .. } | TreeNode::Markdown { children, .. },
                ),
                PathPart::Child(i),
            ) => At::Node(children.get(*i)?),
            (At::Node(TreeNode::Component { props, .. }), PathPart::Prop(key)) => {
                At::Value(&props.iter().find(|(k, _)| k.as_str() == key)?.1)
            }
            (At::Value(TreeValue::List(items)), PathPart::Index(i)) => At::Value(items.get(*i)?),
            (At::Value(TreeValue::Record(fields)), PathPart::Field(key)) => {
                At::Value(&fields.iter().find(|(k, _)| k.as_str() == key)?.1)
            }
            _ => return None,
        };
        if let At::Value(TreeValue::Node(node)) = at {
            at = At::Node(node);
        }
    }
    match at {
        At::Node(node) => Some(node),
        _ => None,
    }
}

/// The committed control whose component has this `label` prop.
fn labelled(session: &Session, label: &str) -> ControlId {
    let candidate = committed(session);
    candidate
        .controls
        .keys()
        .find(|id| {
            matches!(
                owner(&candidate.tree, &id.0),
                Some(TreeNode::Component { props, .. })
                    if props.iter().any(|(k, v)| k.as_str() == "label"
                        && *v == TreeValue::String(DisplayText::new(label)))
            )
        })
        .unwrap_or_else(|| panic!("no committed control labelled {label:?}"))
        .clone()
}

fn press(session: &Session, label: &str, id: u64) -> Event {
    let candidate = committed(session);
    Event::Activate(Activation {
        instance_epoch: 42,
        revision: candidate.revision,
        control: labelled(session, label),
        id,
    })
}

/// A control writing `state` through its binding (§7.6), as the person
/// moving a slider or choosing an item would.
fn set(session: &Session, state: &str, value: Value) -> Event {
    let candidate = committed(session);
    let control = candidate
        .bindings
        .iter()
        .find(|(_, binding)| binding.as_str() == state)
        .unwrap_or_else(|| panic!("no binding for ${state}"))
        .0
        .clone();
    Event::Input {
        instance: 42,
        revision: candidate.revision,
        control,
        binding: state.into(),
        value,
    }
}

fn requests(out: &Output) -> Vec<(String, Identity, Epoch)> {
    issues(out)
}

fn reply(request: &(String, Identity, Epoch), body: Result<&str, &str>) -> Event {
    answer(request, body)
}

/// Run a trace builder twice in one process; both runs must be identical
/// before the result is compared with the committed golden.
#[cfg(test)]
fn replayed(name: &str, expected: &str, build: fn() -> String) {
    let first = build();
    assert_eq!(
        first,
        build(),
        "{name}: a second run in one process differs"
    );
    golden(name, expected, &first);
}

// §16.1 `row-bash` (profile `tool_row`).

pub(crate) fn row_bash() -> String {
    let source = fixture!("sources/16.01-row-bash.ui");
    let mut trace = String::new();
    // Expected status badge and duration per view fixture, from the three
    // conditionals in the source and FormatNumber's half-away rounding.
    let expected = [
        (
            "1-running",
            r#"<Badge text="Running" tone="info" />"#,
            r#"<TextContent text="Duration unavailable" size="small" />"#,
        ),
        (
            "2-succeeded",
            r#"<Badge text="Succeeded" tone="success" />"#,
            r#"<TextContent text="4.2 s" size="small" />"#,
        ),
        (
            "3-failed",
            r#"<Badge text="Failed, exit 2" tone="danger" />"#,
            r#"<TextContent text="Duration unavailable" size="small" />"#,
        ),
        (
            "4-exit-unavailable",
            r#"<Badge text="Finished, exit unavailable" tone="neutral" />"#,
            r#"<TextContent text="61.1 s" size="small" />"#,
        ),
    ];
    let hosts = section16::hosts("16.01-row-bash");
    assert_eq!(hosts.len(), expected.len());
    for (case, badge, duration) in expected {
        let example =
            Example::new("16.01-row-bash", source, Profile::ToolRow).with_tool(hosts[case].clone());
        let (session, _) = example.start(&mut trace, case);
        has(&session, r#"<InlineCode code="cargo test --workspace" />"#);
        has(&session, badge);
        has(&session, duration);
        // Output text stays off this row.
        lacks(&session, "no output");
    }
    trace
}

#[test]
fn section_16_1_row_bash_golden_trace() {
    replayed(
        "16.01-row-bash.trace",
        fixture!("golden/16.01-row-bash.trace"),
        row_bash,
    );
}

// §16.2 `row-edit` (profile `tool_row`).

pub(crate) fn row_edit() -> String {
    let source = fixture!("sources/16.02-row-edit.ui");
    let mut trace = String::new();
    // FormatNumber groups thousands with "," (§6.4); hunks of 1 is singular.
    let expected = [
        ("1-running", "Editing…"),
        ("2-one-place", "+1,234 −5 · 1 changed place"),
        ("3-many-places", "+3 −0 · 4 changed places"),
        (
            "4-counts-unavailable",
            "Counts unavailable · Changed places unavailable",
        ),
    ];
    let hosts = section16::hosts("16.02-row-edit");
    assert_eq!(hosts.len(), expected.len());
    for (case, summary) in expected {
        let example =
            Example::new("16.02-row-edit", source, Profile::ToolRow).with_tool(hosts[case].clone());
        let (session, _) = example.start(&mut trace, case);
        has(
            &session,
            &format!(r#"<InlineCode code="src/main.rs" /> {summary}"#),
        );
    }
    trace
}

#[test]
fn section_16_2_row_edit_golden_trace() {
    replayed(
        "16.02-row-edit.trace",
        fixture!("golden/16.02-row-edit.trace"),
        row_edit,
    );
}

// §16.3 `detail-bash` (profile `tool_page`).

pub(crate) fn detail_bash() -> String {
    let hosts = section16::hosts("16.03-detail-bash");
    let example = Example::new(
        "16.03-detail-bash",
        fixture!("sources/16.03-detail-bash.ui"),
        Profile::ToolPage,
    )
    .with_tool(hosts["1-page"].clone());
    let mut trace = String::new();
    let (mut s, out) = example.start(&mut trace, "1-page");
    // Both queries are demanded at load (§8.3): status with no arguments and
    // the first output page. Until they settle the tree is the loading root.
    let first = requests(&out);
    assert_eq!(
        first
            .iter()
            .map(|(site, identity, _)| (
                site.as_str(),
                identity.operation.as_str(),
                identity.arguments.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            ("job", "bash_status", "{}"),
            ("out", "bash_output", "{\"page\":1}")
        ]
    );
    assert!(out.candidate.as_ref().unwrap().loading);
    has(&s, "LoadingRoot");

    drive(&mut trace, &mut s, "answers", |_| {
        vec![
            reply(&first[0], Ok(r#"{"running":true,"exitCode":null}"#)),
            reply(
                &first[1],
                Ok(r#"{"lines":["Compiling occipital-lang","Running tests"],"page":1,"pages":3}"#),
            ),
        ]
    });
    has(&s, r#"<CardHeader title="Shell command" />"#);
    has(&s, "### Command");
    has(&s, r#"codeString="cargo test --workspace""#);
    has(&s, r#"<Badge text="Running" tone="info" />"#);
    has(
        &s,
        r#"<Col label="Lines, page 1" data=["Compiling occipital-lang", "Running tests"] />"#,
    );
    has(&s, "Page 1 of 3");
    has(&s, r#"label="Next""#);
    lacks(&s, r#"label="Previous""#);
    lacks(&s, "Loading the page…");

    // Next sets $page to 2; the output identity changes, so the old lines
    // stay visible, marked stale, while page 2 loads (§8.3).
    let next = drive(&mut trace, &mut s, "next", |s| vec![press(s, "Next", 1)]);
    let second = requests(&next);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].1.arguments, "{\"page\":2}");
    has(
        &s,
        r#"<Col label="Lines, page 2" data=["Compiling occipital-lang", "Running tests"] />"#,
    );
    has(&s, "Loading the page…");
    has(&s, "Page 2 of 3");
    has(&s, r#"label="Previous""#);
    has(&s, r#"label="Next""#);
    assert_field(&s, "out", "stale", "true");

    drive(&mut trace, &mut s, "page-2", |_| {
        vec![reply(
            &second[0],
            Ok(r#"{"lines":["test result: ok"],"page":2,"pages":3}"#),
        )]
    });
    has(
        &s,
        r#"<Col label="Lines, page 2" data=["test result: ok"] />"#,
    );
    lacks(&s, "Loading the page…");

    // Refresh is an explicit read: the same status identity is issued again
    // with a fresh epoch. An error after a ready answer shows the error
    // branch of the status tag (§8.3).
    let refresh = drive(&mut trace, &mut s, "refresh-status", |s| {
        vec![press(s, "Refresh status", 2)]
    });
    let again = requests(&refresh);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].1, first[0].1);
    assert!(again[0].2.request > first[1].2.request);
    drive(&mut trace, &mut s, "status-error", |_| {
        vec![reply(&again[0], Err("unavailable"))]
    });
    has(&s, r#"<Badge text="Status unavailable" tone="warning" />"#);
    trace
}

#[test]
fn section_16_3_detail_bash_golden_trace() {
    replayed(
        "16.03-detail-bash.trace",
        fixture!("golden/16.03-detail-bash.trace"),
        detail_bash,
    );
}

// §16.4 `detail-work` (profile `tool_page`).

pub(crate) fn detail_work() -> String {
    let hosts = section16::hosts("16.04-detail-work");
    let example = Example::new(
        "16.04-detail-work",
        fixture!("sources/16.04-detail-work.ui"),
        Profile::ToolPage,
    )
    .with_tool(hosts["1-page"].clone());
    let mut trace = String::new();
    let (mut s, load) = example.start(&mut trace, "1-page");
    let first = requests(&load);
    assert_eq!(first.len(), 1);
    assert_eq!(
        (first[0].1.operation.as_str(), first[0].1.arguments.as_str()),
        ("work_get", "{}")
    );
    assert_eq!(
        first[0].1.refresh.map(|r| r.to_string()).as_deref(),
        Some("5")
    );
    drive(&mut trace, &mut s, "answer", |_| {
        vec![reply(
            &first[0],
            Ok(
                r#"{"state":"running","model":"opus","variant":"high","tokens":{"input":100,"output":50,"cacheRead":300,"cacheWrite":100},"lastLines":["reading the cursor","writing the fix"]}"#,
            ),
        )]
    });
    // Prompt tokens are 100 + 300 + 100 = 500, so the hit ratio is
    // 300 / 500 × 100 = 60, shown with one decimal.
    has(&s, r#"<CardHeader title="Fix the paging cursor" />"#);
    has(&s, r#"<Badge text="running" tone="info" />"#);
    has(&s, "**Model:** opus · **Variant:** high");
    has(
        &s,
        r#"<Col label="Token type" data=["Input", "Output", "Cache read", "Cache write"] />"#,
    );
    has(
        &s,
        r#"<Col label="Tokens" type="number" data=[100, 50, 300, 100] />"#,
    );
    has(&s, r#"<Progress value=60 label="Cache hit ratio 60.0%" />"#);
    has(
        &s,
        r#"<Col label="Transcript" data=["reading the cursor", "writing the fix"] />"#,
    );
    has(&s, "Refreshes every 5 seconds.");

    let refresh = drive(&mut trace, &mut s, "refresh", |s| {
        vec![press(s, "Refresh", 1)]
    });
    let again = requests(&refresh);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].1, first[0].1);
    // An error keeps the earlier answer as a stale value (§8.3): the state
    // text says "Unavailable" while the tone still follows the kept state.
    drive(&mut trace, &mut s, "error", |_| {
        vec![reply(&again[0], Err("unavailable"))]
    });
    has(&s, r#"<Badge text="Unavailable" tone="info" />"#);
    has(&s, "**Model:** opus · **Variant:** high");
    assert_field(&s, "work", "stale", "true");
    trace
}

#[test]
fn section_16_4_detail_work_golden_trace() {
    replayed(
        "16.04-detail-work.trace",
        fixture!("golden/16.04-detail-work.trace"),
        detail_work,
    );
}

// §16.5 `calc-paint` (profile `show_ui`).

pub(crate) fn calc_paint() -> String {
    let example = Example::new(
        "16.05-calc-paint",
        fixture!("sources/16.05-calc-paint.ui"),
        Profile::ShowUi,
    );
    let mut trace = String::new();
    let (mut s, _) = example.start(&mut trace, "defaults");
    // Width 4, length 5, height 2.5: walls 2 × 9 × 2.5 = 45 m². One door and
    // two windows: 1.9 + 3.0 = 4.9 m². Wall area 40.1 m²; two coats at
    // 10 m² per litre: 8.02 L, shown as 8.0; cans ⌈8.02 / 2.5⌉ = ⌈3.208⌉ = 4.
    has(&s, r#"label="Width: 4.0 m""#);
    has(&s, r#"label="Length: 5.0 m""#);
    has(&s, r#"label="Height: 2.5 m""#);
    has(&s, r#"label="Coats: 2""#);
    has(&s, "| Wall area to paint | 40.1 m² |");
    has(&s, "| Paint needed | 8.0 L |");
    has(&s, "| 2.5 L cans to buy | 4 |");
    has(&s, r#"title="Buy 4 cans of 2.5 L""#);
    has(
        &s,
        r#"description="Walls of 45.0 m² minus 4.9 m² of doors and windows, at 10 m² per litre per coat.""#,
    );

    // One coat: 40.1 × 1 / 10 = 4.01 L, shown as 4.0; ⌈4.01 / 2.5⌉ = ⌈1.604⌉ = 2.
    drive(&mut trace, &mut s, "coats-1", |s| {
        vec![set(s, "coats", number(1))]
    });
    has(&s, r#"label="Coats: 1""#);
    has(&s, "| Wall area to paint | 40.1 m² |");
    has(&s, "| Paint needed | 4.0 L |");
    has(&s, "| 2.5 L cans to buy | 2 |");
    has(&s, r#"title="Buy 2 cans of 2.5 L""#);

    // Reset restores every declared value, so the load results return.
    drive(&mut trace, &mut s, "reset", |s| vec![press(s, "Reset", 1)]);
    has(&s, r#"label="Coats: 2""#);
    has(&s, "| Paint needed | 8.0 L |");
    has(&s, r#"title="Buy 4 cans of 2.5 L""#);
    trace
}

#[test]
fn section_16_5_calc_paint_golden_trace() {
    replayed(
        "16.05-calc-paint.trace",
        fixture!("golden/16.05-calc-paint.trace"),
        calc_paint,
    );
}

// §16.6 `data-sales` (profile `show_ui`), with `data: "$$tc(494)"` and
// `data_format: "json"`.

fn sales_example() -> Example {
    let bytes = fixture!("data-sales.json").as_bytes();
    let record = ReferenceRecord {
        state: ReferenceState::Resolved,
        reason: None,
        tool: Some("bash".into()),
        bytes: Some(bytes.len() as u64),
        call_key: Some("call-494".into()),
    };
    let span = crate::SourceSpan {
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1,
    };
    let prepared = crate::data::prepare(
        bytes,
        crate::data::Format::Json,
        &crate::data::Options::default(),
        span,
        &mut |_| Ok::<(), ()>(()),
    )
    .unwrap();
    assert!(prepared.diagnostics.is_empty());
    let mut example = Example::new(
        "16.06-data-sales",
        fixture!("sources/16.06-data-sales.ui"),
        Profile::ShowUi,
    );
    example.show_ui = ShowUiArguments {
        title: "Monthly sales, 2025".into(),
        summary: "Total, best month, chart and table for one region at a time".into(),
        data_format: Some(DataFormat::Json),
        data_columns: BTreeMap::new(),
    };
    example.data = crate::eval::snapshot::snapshot("tc-494", &record, Some(bytes), Some(&prepared));
    example
}

pub(crate) fn data_sales() -> String {
    let example = sales_example();
    let mut trace = String::new();
    let (mut s, _) = example.start(&mut trace, "data-sales.json");
    // North in month order: 2025-01 120, 2025-02 150, 2025-03 150; total 420.
    // The stable descending sort keeps 2025-02 ahead of 2025-03 (§6.1).
    has(&s, r#"subtitle="Region: North""#);
    has(&s, "**Total for the year:** 420");
    has(&s, "**Best month:** 2025-02, 150");
    has(
        &s,
        r#"labels=["2025-01", "2025-02", "2025-03"] series=[<Series category="North" values=[120, 150, 150] />]"#,
    );
    has(
        &s,
        r#"<Col label="Month" data=["2025-01", "2025-02", "2025-03"] />"#,
    );

    // South: 2025-01 90, 2025-02 1,200; the total is grouped (§6.4).
    drive(&mut trace, &mut s, "south", |s| {
        vec![set(s, "region", text("South"))]
    });
    has(&s, r#"subtitle="Region: South""#);
    has(&s, "**Total for the year:** 1,290");
    has(&s, "**Best month:** 2025-02, 1,200");

    // East has no rows: Sum of an empty list is 0 and the best month says so.
    drive(&mut trace, &mut s, "east", |s| {
        vec![set(s, "region", text("East"))]
    });
    has(&s, "**Total for the year:** 0");
    has(&s, "**Best month:** no data");
    has(&s, r#"<Col label="Month" data=[] />"#);
    trace
}

#[test]
fn section_16_6_data_sales_golden_trace() {
    replayed(
        "16.06-data-sales.trace",
        fixture!("golden/16.06-data-sales.trace"),
        data_sales,
    );
}

// §16.7 `Watch` and `PagedList` (profile `tool_page`).

fn stream(effect: &Effect) -> Option<(Identity, Epoch)> {
    match effect {
        Effect::Subscribe {
            identity, epoch, ..
        }
        | Effect::Page {
            identity, epoch, ..
        } => Some((identity.clone(), *epoch)),
        _ => None,
    }
}

pub(crate) fn live_view() -> String {
    let hosts = section16::hosts("16.07-live-view");
    let example = Example::new(
        "16.07-live-view",
        fixture!("sources/16.07-live-view.ui"),
        Profile::ToolPage,
    )
    .with_tool(hosts["1-page"].clone());
    let mut trace = String::new();
    let (mut s, load) = example.start(&mut trace, "1-page");
    let streams: Vec<_> = load.effects.iter().filter_map(stream).collect();
    assert_eq!(streams.len(), 2);
    let (watch, watch_epoch) = streams[0].clone();
    let (paged, first_epoch) = streams[1].clone();
    assert_eq!(
        (watch.operation.as_str(), watch.kind),
        ("bash_watch", "watch")
    );
    assert_eq!(
        (paged.operation.as_str(), paged.page_size),
        ("bash_lines", Some(200))
    );
    assert!(load.effects.iter().any(|e| matches!(
        e,
        Effect::Page {
            cursor: None,
            limit: 200,
            ..
        }
    )));

    // The acknowledgement makes the watch live with no items; the first page
    // has a next cursor, so Load more is offered.
    drive(&mut trace, &mut s, "ack-and-first-page", |_| {
        vec![
            Event::WatchAck {
                identity: Box::new(watch.clone()),
                epoch: watch_epoch,
                after: 0,
            },
            Event::PageAnswer {
                identity: Box::new(paged.clone()),
                epoch: first_epoch,
                cursor: None,
                answer: Ok(br#"{"items":["line 1","line 2"],"next":"c2","total":5}"#.to_vec()),
            },
        ]
    });
    assert_field(&s, "live", "status", "\"live\"");
    has(&s, r#"<Col label="Latest lines" data=[] />"#);
    has(&s, r#"<Col label="Line" data=["line 1", "line 2"] />"#);
    has(&s, r#"label="Load more""#);
    lacks(&s, "The command has finished.");
    lacks(&s, "earlier lines are not shown.");

    // Items 1–2 arrive, then a notice starting at 5: items 3 and 4 are a gap
    // and count as dropped (§8.5). Notices apply on the next apply tick.
    let pending = drive(&mut trace, &mut s, "notices", |_| {
        vec![
            Event::WatchNotice {
                identity: Box::new(watch.clone()),
                epoch: watch_epoch,
                token: 1,
                first: 1,
                bytes: br#"["out 1","out 2"]"#.to_vec(),
            },
            Event::WatchNotice {
                identity: Box::new(watch.clone()),
                epoch: watch_epoch,
                token: 2,
                first: 5,
                bytes: br#"["out 5"]"#.to_vec(),
            },
        ]
    });
    assert_eq!(pending.evaluations, 0);
    let tick = s
        .queries
        .apply_timer
        .clone()
        .expect("unapplied notices arm the apply timer");
    drive(&mut trace, &mut s, "apply", |_| vec![Event::Timer(tick)]);
    has(
        &s,
        r#"<Col label="Latest lines" data=["out 1", "out 2", "out 5"] />"#,
    );
    has(&s, "2 earlier lines are not shown.");

    // Load more asks for the next cursor; the last page has next null, so
    // the list is done and the button goes away.
    let more = drive(&mut trace, &mut s, "load-more", |s| {
        vec![press(s, "Load more", 1)]
    });
    let later: Vec<_> = more
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Page { epoch, cursor, .. } => Some((*epoch, cursor.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].1.as_deref(), Some("c2"));
    assert_field(&s, "history", "loadingMore", "true");
    drive(&mut trace, &mut s, "last-page", |_| {
        vec![Event::PageAnswer {
            identity: Box::new(paged.clone()),
            epoch: later[0].0,
            cursor: Some("c2".into()),
            answer: Ok(br#"{"items":["line 3","line 4","line 5"],"next":null}"#.to_vec()),
        }]
    });
    assert_field(&s, "history", "done", "true");
    has(
        &s,
        r#"<Col label="Line" data=["line 1", "line 2", "line 3", "line 4", "line 5"] />"#,
    );
    lacks(&s, r#"label="Load more""#);

    // A clean end keeps the buffer and says the command finished.
    drive(&mut trace, &mut s, "end", |_| {
        vec![Event::WatchEnd {
            identity: Box::new(watch),
            epoch: watch_epoch,
            error: None,
        }]
    });
    assert_field(&s, "live", "status", "\"ended\"");
    has(&s, "The command has finished.");
    has(
        &s,
        r#"<Col label="Latest lines" data=["out 1", "out 2", "out 5"] />"#,
    );
    has(&s, "2 earlier lines are not shown.");
    lacks(&s, "reached its retained limit");
    trace
}

#[test]
fn section_16_7_live_view_golden_trace() {
    replayed(
        "16.07-live-view.trace",
        fixture!("golden/16.07-live-view.trace"),
        live_view,
    );
}
