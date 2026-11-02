//! The host-facing entry points that need more than one internal module: the
//! program checker, the program session, the §13 version descriptors, the
//! function-table/profile export and, with the `traces` feature, the
//! golden-trace runner. The library root re-exports each of them.

mod export;
mod host;
mod program;
#[cfg(feature = "traces")]
mod traces;

use serde::Serialize;

pub use export::export_json;
pub use host::{DataSnapshot, HostError, HostValues};
pub use program::{check_program, evaluate_template, ProgramSession, TemplateEvaluation};
#[cfg(feature = "traces")]
pub use traces::{run_golden_traces, GoldenTrace};

// The §16 fixtures convert their host values the same way a session does.
#[cfg(any(test, feature = "traces"))]
pub(crate) use host::json_value;

use crate::catalog::Catalog;
use crate::host::EXECUTION_DIALECT;

/// What a stored program or reply carries about the semantics it runs under
/// (§13.2): the exact execution dialect and the catalog it was checked
/// against. Replays compare it with what the app implements (§13.3) instead
/// of reinterpreting old content under the newest rules (§13.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDescriptor {
    /// The exact execution dialect, `1.0` for this crate. It is not the
    /// `requires` feature hint of a check result (§12.2).
    pub execution_dialect: String,
    /// The catalog version, digest and layout version.
    pub catalog: CatalogDescriptor,
}

/// One catalog entry of §13.3: `{"version", "digest", "layout", "aliases"}`.
/// The layout version and catalog digest are the same values every layout
/// key carries (catalog `geometry.cacheKey`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogDescriptor {
    /// The catalog version, such as `1.0`.
    pub version: String,
    /// The catalog digest by its `digestRule`, in lowercase hex.
    pub digest: String,
    /// The catalog's layout version, such as `1`.
    pub layout: String,
    /// Earlier digests of this same catalog, from before a change to its
    /// descriptive text only. A stored reply stamped with one of them draws
    /// exactly as one stamped with `digest`, so an app accepts all of them.
    /// Left out of the JSON when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

impl VersionDescriptor {
    /// The descriptor of what this crate implements: dialect `1.0` and the
    /// embedded catalog v1.
    pub fn current() -> Self {
        Self::for_dialect(EXECUTION_DIALECT)
    }

    /// A descriptor naming `dialect` with the embedded catalog. A session is
    /// only created for a dialect the checker accepts, so in practice this
    /// is the current dialect too.
    pub(crate) fn for_dialect(dialect: &str) -> Self {
        Self {
            execution_dialect: dialect.to_owned(),
            catalog: CatalogDescriptor::embedded(),
        }
    }
}

impl CatalogDescriptor {
    /// The descriptor of the tool catalog (1.1), the catalog tool templates
    /// name, with its earlier digests as aliases: a template naming one of
    /// them is accepted as naming this catalog. Replies and stored programs
    /// never carry it: they stay on catalog 1.0, the one [`VersionDescriptor`]
    /// names.
    pub fn tool_templates() -> Self {
        let catalog = Catalog::tool_templates();
        Self {
            version: catalog.version().to_owned(),
            digest: Catalog::tool_templates_digest().to_owned(),
            layout: catalog.layout_version().to_owned(),
            aliases: crate::catalog::TOOL_ALIASES
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect(),
        }
    }

    /// The descriptor of the embedded catalog, whose digest is computed once.
    pub(crate) fn embedded() -> Self {
        let catalog = Catalog::v1();
        Self {
            version: catalog.version().to_owned(),
            digest: Catalog::embedded_digest().to_owned(),
            layout: catalog.layout_version().to_owned(),
            aliases: crate::catalog::V1_ALIASES
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect(),
        }
    }
}

/// The catalog digest a runner stamps on rich replies (`rich_ui.catalog_digest`).
/// It is catalog 1.0's first published digest, now an alias: stamps never
/// moved when the catalog's descriptive text was rewritten, so app builds that
/// know only that digest keep drawing new replies.
pub fn reply_stamp_digest() -> &'static str {
    crate::catalog::V1_ALIASES[0]
}
