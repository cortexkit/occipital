//! A tool row evaluated with and without its record measures the same: the
//! Slot rule, checked before evaluation, is what makes that hold.

use occipital_lang::{
    evaluate_template, measure, CatalogVersion, CheckContext, Decimal, Height, HostError,
    HostProjection, HostValues, LayoutContext, MeasureRevision, Metrics, Network, Placement,
    Profile, Schema, TextStyle, ToolInputs, Tree,
};
use serde_json::{json, Value};

fn d(n: i64) -> Decimal {
    Decimal::from_integer(i128::from(n)).unwrap()
}

/// Every style 20 units tall, every character 6 units wide.
struct Fixed;

impl Metrics for Fixed {
    fn l(&self, _: TextStyle) -> Decimal {
        d(20)
    }
    fn t(&self, text: &str, _: TextStyle, width: Decimal, max_lines: Option<u32>) -> Decimal {
        let width = width.to_string().parse::<f64>().unwrap().max(1.0);
        let lines = ((text.chars().count() * 6) as f64 / width).ceil().max(1.0) as i64;
        d(lines.min(i64::from(max_lines.unwrap_or(u32::MAX))) * 20)
    }
    fn tw(&self, text: &str, _: TextStyle) -> Decimal {
        d(text.chars().count() as i64 * 6)
    }
}

const ROW: &str = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.command} />\n<Slot width=\"medium\" needs={view.record}>\n<TextContent text={\"took \" + Text(view.record.durationMs) + \" ms and wrote a long line of text that does not fit\"} />\n</Slot>\n</Card>\n";

fn context(view: Value) -> CheckContext {
    let mut context = CheckContext::new(Profile::ToolRow);
    context.catalog = CatalogVersion::Tool1_1;
    context.tool = Some(ToolInputs {
        input: HostProjection {
            value: json!({"command": "cargo test"}),
            schema: Schema::default(),
        },
        view: HostProjection {
            value: view,
            schema: Schema::default(),
        },
        truncated: false,
        truncated_fields: vec![],
    });
    context
}

fn row(record: Value) -> Tree {
    let view = json!({"call": {"phase": "answered"}, "record": record, "completion": null});
    let evaluation =
        evaluate_template(ROW.as_bytes(), &context(view), &HostValues::default()).unwrap();
    assert!(evaluation.abort.is_none(), "{:?}", evaluation.diagnostics);
    evaluation.tree.unwrap()
}

fn height(tree: &Tree) -> Height {
    let revision = MeasureRevision {
        instance_epoch: 0,
        revision: 0,
        state_hash: "",
        tree,
        first: None,
    };
    let layout = LayoutContext {
        width: d(320),
        content_size_category: "body-normal".into(),
        network: Network::Unknown,
        placement: Placement::Row,
    };
    measure(&revision, &layout, &Fixed).unwrap().height
}

#[test]
fn a_row_measures_the_same_before_and_after_its_record_arrives() {
    let waiting = row(Value::Null);
    let filled = row(json!({"durationMs": 1234, "outcome": "success"}));
    assert_ne!(waiting, filled, "the record fills the Slot in");
    assert_eq!(height(&waiting), height(&filled));
    // The tallest part is the code chip, one line plus 4 of padding. The
    // record's text would wrap to several lines unclipped, so the Slot's
    // one-line box is what keeps the row at this height.
    assert_eq!(height(&filled), Height::Known(d(24)));
}

#[test]
fn only_a_tool_template_is_evaluated_and_a_failing_one_is_refused() {
    let mut show_ui = context(json!({}));
    show_ui.profile = Profile::ShowUi;
    assert!(matches!(
        evaluate_template(b"<Divider />", &show_ui, &HostValues::default()),
        Err(HostError::Internal(_))
    ));
    let unslotted = "<Badge text={view.call.phase} />\n";
    let view = json!({"call": {"phase": "answered"}, "record": null, "completion": null});
    assert!(matches!(
        evaluate_template(unslotted.as_bytes(), &context(view), &HostValues::default()),
        Err(HostError::Rejected(_))
    ));
}
