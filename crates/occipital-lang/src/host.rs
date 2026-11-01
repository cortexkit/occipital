//! Host contract records describe inputs, not authorization.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::SourceSpan;

/// The one exact execution dialect this crate implements (§13.1, §13.2).
/// The checker refuses any other with `E_UNSUPPORTED_VERSION`; there is no
/// older evaluator to select and no newer one to reinterpret under.
pub(crate) const EXECUTION_DIALECT: &str = "1.0";

/// The checking environment selected by the host, never by source text (§2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// `reply`: reply mode, no host values or sites.
    Reply,
    /// `show_ui`: a program with `data`, `now`, `zone` and read sites.
    ShowUi,
    /// `tool_row`: a program with `input`, `view`, `now`, `zone` and no sites.
    ToolRow,
    /// `tool_page`: a program with `input`, `view`, `now`, `zone` and read sites.
    ToolPage,
}

impl Profile {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::ShowUi => "show_ui",
            Self::ToolRow => "tool_row",
            Self::ToolPage => "tool_page",
        }
    }

    pub(crate) fn supplies(self, name: &str) -> bool {
        match self {
            Self::Reply => false,
            Self::ShowUi => matches!(name, "data" | "now" | "zone"),
            Self::ToolRow | Self::ToolPage => matches!(name, "input" | "view" | "now" | "zone"),
        }
    }

    /// Every profile, in the order of the §2.3 table.
    pub(crate) const ALL: [Profile; 4] = [Self::Reply, Self::ShowUi, Self::ToolRow, Self::ToolPage];

    /// The reserved host value names, each supplied by some profiles (§2.3).
    pub(crate) const HOST_VALUES: [&'static str; 5] = ["data", "input", "view", "now", "zone"];

    /// Whether the profile checks reply mode or program mode (§2.3).
    pub(crate) fn mode(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::ShowUi | Self::ToolRow | Self::ToolPage => "program",
        }
    }

    /// Whether `Query`, `Watch` and `PagedList` sites may be declared (§2.3):
    /// a reply runs no operations and a tool row is measured before insertion.
    pub(crate) fn sites(self) -> bool {
        matches!(self, Self::ShowUi | Self::ToolPage)
    }

    /// Whether `$$tc(N)` references may appear (§2.3): only replies and
    /// `show_ui` programs are stored in a transcript.
    pub(crate) fn references(self) -> bool {
        matches!(self, Self::Reply | Self::ShowUi)
    }
}

/// The supported JSON Schema subset for host contracts.
/// Unknown keywords are rejected rather than silently granting a wider shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    /// `type`.
    pub kind: Option<SchemaType>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    /// `properties`: the schema of each named property.
    pub properties: BTreeMap<String, Schema>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// `required`: properties that must be present.
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// `items`: the schema of every array element.
    pub items: Option<Box<Schema>>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    /// `enum`: the allowed values.
    pub enumeration: Option<Vec<Json>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// `nullable`: whether `null` is also allowed.
    pub nullable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// A JSON Schema `type`.
pub enum SchemaType {
    /// `null`.
    Null,
    /// `boolean`.
    Boolean,
    /// `number`.
    Number,
    /// `string`.
    String,
    /// `array`.
    Array,
    /// `object`.
    Object,
}

impl Schema {
    /// Validate already decoded host data, without coercion or property removal.
    /// Unspecified properties remain legal: the subset has no additionalProperties.
    pub fn accepts(&self, value: &Json) -> bool {
        let kind_matches = self.kind.is_none_or(|kind| match kind {
            SchemaType::Null => value.is_null(),
            SchemaType::Boolean => value.is_boolean(),
            SchemaType::Number => value.is_number(),
            SchemaType::String => value.is_string(),
            SchemaType::Array => value.is_array(),
            SchemaType::Object => value.is_object(),
        });
        if !(kind_matches || self.nullable && value.is_null())
            || self
                .enumeration
                .as_ref()
                .is_some_and(|values| !values.iter().any(|candidate| enum_equal(candidate, value)))
        {
            return false;
        }
        match value {
            Json::Object(fields) => {
                self.required.iter().all(|key| fields.contains_key(key))
                    && self.properties.iter().all(|(key, schema)| {
                        fields.get(key).is_none_or(|value| schema.accepts(value))
                    })
            }
            Json::Array(values) => self
                .items
                .as_ref()
                .is_none_or(|schema| values.iter().all(|value| schema.accepts(value))),
            _ => true,
        }
    }
}

fn enum_equal(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(a), Json::Number(b)) => crate::Decimal::from_literal(&a.to_string())
            .ok()
            .zip(crate::Decimal::from_literal(&b.to_string()).ok())
            .is_some_and(|(a, b)| a == b),
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| enum_equal(a, b))
        }
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| enum_equal(a, b)))
        }
        _ => a == b,
    }
}

/// Only entries already narrowed to the calling session's approved read scope
/// belong here (§2.3). A registry entry is not itself an authorization grant.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationRegistry(pub BTreeMap<String, Operation>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// One registry entry: an operation's kind and schemas (§8.3–§8.6).
pub enum Operation {
    /// A `Query` read.
    Query {
        /// The schema of its arguments record.
        arguments: Schema,
        /// The schema of its answer.
        result: Schema,
    },
    /// A `Watch` stream.
    Watch {
        /// The schema of its arguments record.
        arguments: Schema,
        /// The schema of each item.
        item: Schema,
        /// Whether a resubscription can resume after a sequence number.
        resume: bool,
        /// Whether a resubscription replays earlier items.
        replay: bool,
    },
    /// A `PagedList` read.
    Paged {
        /// The schema of its arguments record.
        arguments: Schema,
        /// The schema of each page: `items`, `next` and optional `total`.
        page: Schema,
        #[serde(default)]
        /// The item field that identifies an item, if the registry names one.
        item_key: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A host value with the schema it was validated against (§2.3).
pub struct HostProjection {
    /// The value.
    pub value: Json,
    /// Its schema.
    pub schema: Schema,
}

/// The host supplies complete field-wise projections, never truncated JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolInputs {
    /// The tool call's `input` projection.
    pub input: HostProjection,
    /// The tool call's `view` record.
    pub view: HostProjection,
    /// Whether the host shortened the arguments.
    pub truncated: bool,
    /// Which fields were shortened.
    pub truncated_fields: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// The `show_ui` call's arguments other than the source (§10.2, §11.1).
pub struct ShowUiArguments {
    /// `title`, the fallback header's first line.
    pub title: String,
    /// `summary`, the fallback header's second line.
    pub summary: String,
    /// `data_format`: how to parse the `data` snapshot.
    pub data_format: Option<DataFormat>,
    /// `data_columns`: column conversions for the `data` snapshot.
    pub data_columns: BTreeMap<String, ColumnType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// A snapshot format hint (§10.2).
pub enum DataFormat {
    /// JSON.
    Json,
    /// CSV.
    Csv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// A column conversion (§10.2).
pub enum ColumnType {
    /// Keep the text.
    Text,
    /// Convert to a number.
    Number,
    /// Convert to a boolean.
    Boolean,
}

/// Immutable snapshot metadata stamped at store time (§4.8). Missing records
/// remain unavailable; the checker never resolves a transcript number itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRecord {
    /// Whether the snapshot is available.
    pub state: ReferenceState,
    /// Why an unresolved snapshot is unavailable.
    pub reason: Option<String>,
    /// The tool that produced the output.
    pub tool: Option<String>,
    /// The output's size in bytes.
    pub bytes: Option<u64>,
    /// The call key, for a held snapshot the app follows (§4.8).
    pub call_key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// The state of a stamped snapshot (§4.8).
pub enum ReferenceState {
    /// The output is stored and can be fetched.
    Resolved,
    /// The output is not available.
    Unresolved,
    /// The output is larger than a snapshot may be.
    TooLarge,
    /// The call has not finished; the snapshot stays pending.
    Held,
}

/// Scoped immutable resource metadata (§4.9), not an ambient file path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandleRecord {
    /// The record's id.
    pub record_id: String,
    /// The computer that holds the resource.
    pub computer: String,
    /// The session that produced it.
    pub session: String,
    /// The scope it is valid in.
    pub scope: String,
    /// An artifact or a file.
    pub kind: HandleKind,
    /// Where the handle is written in the source.
    pub span: SourceSpan,
    /// The scoped resource id, when resolved.
    pub resource_id: Option<String>,
    /// The resource version, when resolved.
    pub version: Option<String>,
    /// The content hash, when known.
    pub content_hash: Option<String>,
    /// The MIME type, when known.
    pub mime: Option<String>,
    /// The size in bytes, when known.
    pub bytes: Option<u64>,
    /// Whether the resource can be fetched now.
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// What a handle names (§4.9).
pub enum HandleKind {
    /// `Artifact(…)`.
    Artifact,
    /// `File(…)`.
    File,
}

/// The catalog a program is checked against, as its template or stamp names
/// it (§13.3 `catalogs[].version`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogVersion {
    /// Catalog 1.0: every reply and `show_ui` program, and tool templates
    /// written for it.
    #[default]
    #[serde(rename = "1.0")]
    V1_0,
    /// The tool catalog 1.1: catalog 1.0 plus `Slot`, for tool templates only.
    /// A tool row checked against it reads the call's view and the time only
    /// inside a Slot (`E_UNSLOTTED_READ`), since it has the means to.
    #[serde(rename = "1.1")]
    Tool1_1,
}

/// Pure inputs for catalog/profile checking. Records never authorize a site.
pub struct CheckContext {
    /// The profile to check under.
    pub profile: Profile,
    /// The catalog the template names. Only tool profiles use 1.1; any other
    /// profile is checked against 1.0 whatever this says.
    pub catalog: CatalogVersion,
    /// The exact execution dialect; anything but `1.0` is `E_UNSUPPORTED_VERSION`.
    pub execution_dialect: String,
    /// The operations the program may read.
    pub registry: OperationRegistry,
    /// `input` and `view`, for tool views.
    pub tool: Option<ToolInputs>,
    /// The `show_ui` call's arguments.
    pub show_ui: ShowUiArguments,
    /// Snapshot records stamped at store time, keyed by N in `$$tc(N)`.
    pub references: BTreeMap<u64, ReferenceRecord>,
    /// Handle records stamped at store time.
    pub handles: Vec<HandleRecord>,
}

impl CheckContext {
    /// A context with the current dialect and no registry, tool or records.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            catalog: CatalogVersion::V1_0,
            execution_dialect: EXECUTION_DIALECT.into(),
            registry: OperationRegistry::default(),
            tool: None,
            show_ui: ShowUiArguments::default(),
            references: BTreeMap::new(),
            handles: vec![],
        }
    }
}
