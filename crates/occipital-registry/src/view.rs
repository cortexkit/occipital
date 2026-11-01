//! Checking a tool's view, as a provider serves it from `tool.views`: each of
//! its templates against the tool catalog, with `input` and `view` typed the
//! way Occipital supplies them.
//!
//! A row template sees the call's arguments as `input` and, as `view`, the
//! call's phase and times with the record and completion the provider wrote
//! for it. A page template also sees the whole output and the detail. Every
//! part of `view` but `call.phase` may be null, since the data arrives after
//! the row is drawn; a row reads it only inside a `Slot` (`E_UNSLOTTED_READ`).

use std::collections::BTreeMap;

use cortexkit_role_tool_display::{schema, FieldKind, RecordSchema};
use occipital_lang::{
    Catalog, CatalogVersion, CheckContext, Decimal, Diagnostic, Height, HostProjection, HostValues,
    LayoutContext, MeasureRevision, Metrics, Network, Placement, Profile, Schema, SchemaType,
    Severity, TextStyle, ToolInputs, Tree,
};
use serde_json::{json, Value as Json};

use crate::Template;

/// The catalog version every tool template names.
pub const TOOL_CATALOG_VERSION: &str = "1.1";

/// The digest a tool template's `catalog.digest` must hold: the tool
/// catalog's, as `sha256:` and lowercase hex.
pub fn tool_catalog_digest() -> String {
    format!("sha256:{}", Catalog::tool_templates_digest())
}

/// The verdict on one of a view's templates.
#[derive(Debug)]
pub struct TemplateReport {
    /// `row` or `page`: which of the view's templates this is.
    pub slot: &'static str,
    /// The template's digest, when it is a well-formed template document.
    pub digest: Option<String>,
    /// What makes the template unusable before its source is read, each in a
    /// sentence: it isn't a template document, or names the wrong profile or
    /// catalog, or declares reads.
    pub problems: Vec<String>,
    /// The language checker's diagnostics for the source.
    pub diagnostics: Vec<Diagnostic>,
}

impl TemplateReport {
    /// Whether an app may draw the template: no problem and no error.
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity() != Severity::Error)
    }
}

/// Check the templates of one view. `template` is the view's `template`
/// member, `{row?, page?}`; `record_schema` is the view's own; `input_schema`
/// is the input schema of the catalog tool the view's pin names. An `Err` is
/// a `template` member that isn't such an object.
pub fn check_view(
    template: &Json,
    record_schema: &RecordSchema,
    input_schema: &Json,
) -> Result<Vec<TemplateReport>, String> {
    let Some(members) = template.as_object() else {
        return Err("the view's template is not an object".into());
    };
    if let Some(unknown) = members
        .keys()
        .find(|key| !matches!(key.as_str(), "row" | "page"))
    {
        return Err(format!(
            "the view's template has a member {unknown:?}; it holds only row and page"
        ));
    }
    let mut reports = Vec::new();
    for (slot, profile) in [("row", Profile::ToolRow), ("page", Profile::ToolPage)] {
        if let Some(document) = members.get(slot) {
            reports.push(check_template(
                slot,
                profile,
                document,
                record_schema,
                input_schema,
            ));
        }
    }
    Ok(reports)
}

fn check_template(
    slot: &'static str,
    profile: Profile,
    document: &Json,
    record_schema: &RecordSchema,
    input_schema: &Json,
) -> TemplateReport {
    let mut report = TemplateReport {
        slot,
        digest: None,
        problems: Vec::new(),
        diagnostics: Vec::new(),
    };
    let parsed = serde_json::to_vec(document)
        .map_err(|error| error.to_string())
        .and_then(|bytes| Template::parse(&bytes).map_err(|error| error.to_string()));
    let template = match parsed {
        Ok(template) => template,
        Err(error) => {
            report.problems.push(format!(
                "it is not an occipital.template/1 document: {error}"
            ));
            return report;
        }
    };
    report.digest = template.digest().ok();
    let expected = match profile {
        Profile::ToolRow => "tool_row",
        _ => "tool_page",
    };
    if template.profile != expected {
        report.problems.push(format!(
            "the {slot} template has profile {:?}; it must be {expected:?}",
            template.profile
        ));
    }
    if template.catalog.version != TOOL_CATALOG_VERSION
        || template.catalog.digest != tool_catalog_digest()
    {
        report.problems.push(format!(
            "it names catalog {} ({}); tool templates name the tool catalog {TOOL_CATALOG_VERSION} ({})",
            template.catalog.version,
            template.catalog.digest,
            tool_catalog_digest()
        ));
    }
    if !template.reads.is_empty() {
        report
            .problems
            .push("it declares reads; templates make no reads in this version".into());
    }
    if !report.problems.is_empty() {
        return report;
    }
    let context = tool_context(
        profile,
        &template,
        json!({}),
        Schema {
            // Checking supplies no arguments, so none may be required.
            required: Vec::new(),
            ..input(input_schema)
        },
        waiting_view(profile),
        record_schema,
    );
    match occipital_lang::check_program(
        template.source.as_bytes(),
        &context,
        &HostValues::default(),
    ) {
        Ok(result) => report.diagnostics = result.diagnostics().to_vec(),
        Err(error) => report
            .problems
            .push(format!("the checker could not run: {error:?}")),
    }
    report
}

/// The context a tool template is checked and evaluated in.
fn tool_context(
    profile: Profile,
    template: &Template,
    input_value: Json,
    input_schema: Schema,
    view_value: Json,
    record_schema: &RecordSchema,
) -> CheckContext {
    let mut context = CheckContext::new(profile);
    context.catalog = CatalogVersion::Tool1_1;
    context.execution_dialect = template.execution_dialect.clone();
    context.tool = Some(ToolInputs {
        input: HostProjection {
            value: input_value,
            schema: input_schema,
        },
        view: HostProjection {
            value: view_value,
            schema: view(record_schema, profile),
        },
        truncated: false,
        truncated_fields: Vec::new(),
    });
    context
}

/// The widths a sample row is measured at: a phone's and a desktop's.
pub const SAMPLE_WIDTHS: [i64; 2] = [320, 720];

/// A sample row's height at one width, in the checker's fixed metrics: every
/// line of text 20 units tall and every character 8 units wide. The numbers
/// compare a row with itself, not with an app's points.
#[derive(Debug, PartialEq, Eq)]
pub struct RowHeight {
    /// The width it was measured at.
    pub width: i64,
    /// The height before anything is written for the call.
    pub waiting: Height,
    /// The height with the sample's record and completion.
    pub filled: Height,
}

/// Evaluate and measure a view's row template for one sample call, at each
/// of [`SAMPLE_WIDTHS`], once before anything is written for it and once
/// with `record` and `completion`. The two heights must be equal; the Slot
/// rule is what makes them so. An `Err` says why the sample couldn't be
/// measured: a record or completion its schema refuses, a template that
/// doesn't check, or an evaluation that stopped.
pub fn row_heights(
    row: &Json,
    record_schema: &RecordSchema,
    input_schema: &Json,
    sample_input: &Json,
    record: Option<&Json>,
    completion: Option<&Json>,
) -> Result<Vec<RowHeight>, String> {
    for (name, value, validate) in [
        ("record", record, schema::validate_record as Validator),
        ("completion", completion, schema::validate_completion),
    ] {
        if let Some(value) = value {
            let object = value
                .as_object()
                .ok_or_else(|| format!("the sample's {name} is not an object"))?;
            validate(object, record_schema).map_err(|error| {
                format!("the record schema refuses the sample's {name}: {error}")
            })?;
        }
    }
    let template = serde_json::to_vec(row)
        .map_err(|error| error.to_string())
        .and_then(|bytes| Template::parse(&bytes).map_err(|error| error.to_string()))?;
    let mut filled = waiting_view(Profile::ToolRow);
    filled["call"] = json!({
        "phase": "answered", "startedAtMs": 1_767_225_600_000_u64,
        "finishedAtMs": 1_767_225_601_500_u64, "isError": false,
        "outputBytes": 2048, "unanswered": null
    });
    filled["record"] = record.cloned().unwrap_or(Json::Null);
    filled["completion"] = completion.cloned().unwrap_or(Json::Null);
    let [waiting, filled] = [waiting_view(Profile::ToolRow), filled].map(|view_value| {
        let context = tool_context(
            Profile::ToolRow,
            &template,
            sample_input.clone(),
            input(input_schema),
            view_value,
            record_schema,
        );
        let evaluated = occipital_lang::evaluate_template(
            template.source.as_bytes(),
            &context,
            &HostValues::default(),
        )
        .map_err(|error| format!("the row could not be evaluated: {error:?}"))?;
        match (evaluated.abort, evaluated.tree) {
            (None, Some(tree)) => Ok(tree),
            (abort, _) => Err(format!("the row's evaluation stopped: {abort:?}")),
        }
    });
    let (waiting, filled) = (waiting?, filled?);
    SAMPLE_WIDTHS
        .iter()
        .map(|&width| {
            Ok(RowHeight {
                width,
                waiting: measure_row(&waiting, width)?,
                filled: measure_row(&filled, width)?,
            })
        })
        .collect()
}

type Validator = fn(
    &serde_json::Map<String, Json>,
    &RecordSchema,
) -> Result<(), cortexkit_role_tool_display::InvalidParams>;

fn measure_row(tree: &Tree, width: i64) -> Result<Height, String> {
    let width = Decimal::from_integer(i128::from(width)).map_err(|error| format!("{error:?}"))?;
    let revision = MeasureRevision {
        instance_epoch: 0,
        revision: 0,
        state_hash: "",
        tree,
        first: None,
    };
    let layout = LayoutContext {
        width,
        content_size_category: "large".into(),
        network: Network::Unknown,
        placement: Placement::Row,
    };
    occipital_lang::measure(&revision, &layout, &FixedMetrics::new()?)
        .map(|measurement| measurement.height)
        .map_err(|error| format!("the row could not be measured: {error:?}"))
}

/// Text metrics that need no fonts: every line 20 units tall and every
/// character 8 units wide, wrapping at the width.
struct FixedMetrics {
    /// What a measure falls back to if a count is too large for a number.
    zero: Decimal,
}

impl FixedMetrics {
    fn new() -> Result<Self, String> {
        Ok(Self {
            zero: Decimal::from_integer(0).map_err(|error| format!("{error:?}"))?,
        })
    }

    fn units(&self, n: i128) -> Decimal {
        Decimal::from_integer(n).unwrap_or(self.zero)
    }
}

impl Metrics for FixedMetrics {
    fn l(&self, _: TextStyle) -> Decimal {
        self.units(20)
    }

    fn t(&self, text: &str, _: TextStyle, width: Decimal, max_lines: Option<u32>) -> Decimal {
        let per_line = (width.to_string().parse::<f64>().unwrap_or(8.0) / 8.0)
            .floor()
            .max(1.0);
        let chars = text.chars().count().max(1) as f64;
        let lines = (chars / per_line).ceil() as i128;
        self.units(lines.min(i128::from(max_lines.unwrap_or(u32::MAX))) * 20)
    }

    fn tw(&self, text: &str, _: TextStyle) -> Decimal {
        self.units(text.chars().count() as i128 * 8)
    }
}

/// The tool's JSON Schema in the language's subset: `type`, `properties`,
/// `required`, `items`, `enum` and nullability. Every other keyword is
/// dropped, which only widens what a value may be, so a template is never
/// refused over a constraint the subset can't state.
pub fn input(schema: &Json) -> Schema {
    project(schema, 0)
}

fn project(schema: &Json, depth: usize) -> Schema {
    let mut projected = Schema::default();
    // Deeper input is typed as anything rather than followed further.
    let Some(object) = schema.as_object().filter(|_| depth < 32) else {
        return projected;
    };
    match object.get("type") {
        Some(Json::String(name)) => projected.kind = kind(name),
        Some(Json::Array(names)) => {
            let names: Vec<&str> = names.iter().filter_map(Json::as_str).collect();
            projected.nullable = names.contains(&"null");
            if let [name] = names
                .iter()
                .filter(|name| **name != "null")
                .collect::<Vec<_>>()[..]
            {
                projected.kind = kind(name);
            }
        }
        _ => {}
    }
    if object.get("nullable") == Some(&Json::Bool(true)) {
        projected.nullable = true;
    }
    if let Some(properties) = object.get("properties").and_then(Json::as_object) {
        for (name, property) in properties {
            projected
                .properties
                .insert(name.clone(), project(property, depth + 1));
        }
    }
    if let Some(required) = object.get("required").and_then(Json::as_array) {
        projected.required = required
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect();
    }
    if let Some(items) = object.get("items").filter(|items| items.is_object()) {
        projected.items = Some(Box::new(project(items, depth + 1)));
    }
    if let Some(values) = object.get("enum").and_then(Json::as_array) {
        projected.enumeration = Some(values.clone());
    }
    projected
}

fn kind(name: &str) -> Option<SchemaType> {
    Some(match name {
        "null" => SchemaType::Null,
        "boolean" => SchemaType::Boolean,
        "number" | "integer" => SchemaType::Number,
        "string" => SchemaType::String,
        "array" => SchemaType::Array,
        "object" => SchemaType::Object,
        _ => return None,
    })
}

/// The schema of `view` for a template of `profile`, with the record and
/// completion typed by the view's own record schema.
pub fn view(record_schema: &RecordSchema, profile: Profile) -> Schema {
    let typed = |kind: SchemaType| Schema {
        kind: Some(kind),
        nullable: true,
        ..Schema::default()
    };
    let call = object(
        [
            (
                "phase",
                Schema {
                    kind: Some(SchemaType::String),
                    enumeration: Some(vec![
                        json!("running"),
                        json!("answered"),
                        json!("unanswered"),
                    ]),
                    ..Schema::default()
                },
            ),
            ("startedAtMs", typed(SchemaType::Number)),
            ("finishedAtMs", typed(SchemaType::Number)),
            ("isError", typed(SchemaType::Boolean)),
            ("outputBytes", typed(SchemaType::Number)),
            ("unanswered", typed(SchemaType::String)),
        ],
        false,
    );
    let mut written = BTreeMap::from([
        ("durationMs".to_owned(), typed(SchemaType::Number)),
        (
            "outcome".to_owned(),
            Schema {
                kind: Some(SchemaType::String),
                nullable: true,
                enumeration: Some(vec![
                    json!("success"),
                    json!("error"),
                    json!("partial"),
                    json!("held"),
                ]),
                ..Schema::default()
            },
        ),
        ("reason".to_owned(), typed(SchemaType::String)),
        ("queuedMs".to_owned(), typed(SchemaType::Number)),
        ("heldMs".to_owned(), typed(SchemaType::Number)),
        ("location".to_owned(), typed(SchemaType::String)),
    ]);
    for (name, kind) in &record_schema.fields {
        written.insert(name.clone(), field(kind));
    }
    // A record and a completion hold only these fields, each optional.
    let record = Schema {
        kind: Some(SchemaType::Object),
        nullable: true,
        properties: written,
        ..Schema::default()
    };
    let mut members = vec![
        ("call", call),
        ("record", record.clone()),
        ("completion", record),
    ];
    if profile == Profile::ToolPage {
        members.push(("output", typed(SchemaType::String)));
        members.push(("detail", typed(SchemaType::Object)));
    }
    object(members, false)
}

/// An extension field holds one value of its kind or a list of them, which
/// the subset can't state, so it is typed as anything.
fn field(_: &FieldKind) -> Schema {
    Schema::default()
}

/// An object with every member required, so a template may read each
/// without a missing-member check; `nullable` for one that may be null.
fn object<'a>(members: impl IntoIterator<Item = (&'a str, Schema)>, nullable: bool) -> Schema {
    let properties: BTreeMap<String, Schema> = members
        .into_iter()
        .map(|(name, schema)| (name.to_owned(), schema))
        .collect();
    Schema {
        kind: Some(SchemaType::Object),
        required: properties.keys().cloned().collect(),
        properties,
        nullable,
        ..Schema::default()
    }
}

/// The `view` of a call that has just been answered, with nothing written
/// yet: the shape every template must check against.
fn waiting_view(profile: Profile) -> Json {
    let mut view = json!({
        "call": {
            "phase": "answered", "startedAtMs": null, "finishedAtMs": null,
            "isError": null, "outputBytes": null, "unanswered": null
        },
        "record": null,
        "completion": null,
    });
    if profile == Profile::ToolPage {
        view["output"] = Json::Null;
        view["detail"] = Json::Null;
    }
    view
}
