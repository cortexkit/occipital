//! The machine-readable function table and profile rules (catalog `open`
//! item "the language crate's machine-readable export", catalog-v1.json:870).
//!
//! Everything here is read from the tables the checker itself uses, so the
//! export cannot describe a rule the checker does not apply.

use serde_json::{json, Value as Json};

use super::CatalogDescriptor;
use crate::catalog::Catalog;
use crate::host::EXECUTION_DIALECT;
use crate::prepare::types::{function_table, RESERVED_WRITES, SITE_CONSTRUCTORS};
use crate::Profile;

/// The function table and profile rules as pretty-printed JSON, ending in a
/// newline. Object keys are sorted, so the output is byte-identical on every
/// platform. The crate commits a copy at `export/functions-and-profiles.json`.
///
/// The document holds:
/// - `language` and `versions`: the execution dialect and the catalog entry
///   in the §13.3 declaration shape (`dialects`, `catalogs`);
/// - `functions`: each function name the checker accepts, with its arity
///   (`maxArgs` null when variadic) and the signature diagnostics quote;
/// - `reservedWrites`: the names refused with `E_WRITE_NOT_ALLOWED` in every
///   profile (§7.4);
/// - `profiles`: per profile, its mode, the host values it supplies, the
///   site constructors it allows, whether references may appear, and the
///   catalog components it accepts (§2.3).
pub fn export_json() -> String {
    let catalog = Catalog::v1();
    let functions: Vec<Json> = function_table()
        .into_iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "signature": entry.signature,
                "minArgs": entry.min,
                "maxArgs": entry.max,
            })
        })
        .collect();
    let profiles: Vec<Json> = Profile::ALL
        .iter()
        .map(|profile| {
            let components: Vec<&str> = catalog
                .components
                .iter()
                .filter(|(_, component)| component.profiles.iter().any(|p| p == profile.name()))
                .map(|(name, _)| name.as_str())
                .collect();
            let sites: &[&str] = if profile.sites() {
                &SITE_CONSTRUCTORS
            } else {
                &[]
            };
            json!({
                "name": profile.name(),
                "mode": profile.mode(),
                "hostValues": Profile::HOST_VALUES
                    .iter()
                    .filter(|name| profile.supplies(name))
                    .collect::<Vec<_>>(),
                "sites": sites,
                "references": profile.references(),
                "components": components,
            })
        })
        .collect();
    let document = json!({
        "language": EXECUTION_DIALECT,
        "versions": {
            "dialects": [EXECUTION_DIALECT],
            "catalogs": [CatalogDescriptor::embedded()],
        },
        "functions": functions,
        "reservedWrites": RESERVED_WRITES,
        "profiles": profiles,
    });
    // A JSON value's alternate display is `to_string_pretty`'s output, without
    // a `Result` that serializing a plain JSON value can never fail.
    format!("{document:#}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_matches_the_committed_copy() {
        let committed = include_str!("../../export/functions-and-profiles.json");
        let actual = export_json();
        assert!(
            actual == committed,
            "the export changed; review it and update export/functions-and-profiles.json:\n{actual}"
        );
    }

    #[test]
    fn export_lists_every_function_the_checker_accepts_and_profile_rules() {
        let document: Json = serde_json::from_str(&export_json()).unwrap();
        let names: Vec<&str> = document["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        // §6.1–§6.5 name these; spot-check one from each section.
        for name in [
            "Count",
            "Round",
            "Upper",
            "FormatBytes",
            "Parse",
            "Query",
            "Open",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        assert!(!names.contains(&"Mutation") && !names.contains(&"Run"));
        let min = &document["functions"][names.iter().position(|n| *n == "Min").unwrap()];
        assert_eq!(min["maxArgs"], Json::Null);
        let profile = |name: &str| {
            document["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap()
                .clone()
        };
        // §2.3: hand-copied from the profile table.
        assert_eq!(profile("reply")["hostValues"], json!([]));
        assert_eq!(
            profile("show_ui")["hostValues"],
            json!(["data", "now", "zone"])
        );
        assert_eq!(
            profile("tool_row")["hostValues"],
            json!(["input", "view", "now", "zone"])
        );
        assert_eq!(profile("tool_row")["sites"], json!([]));
        assert_eq!(
            profile("tool_page")["sites"],
            json!(["Query", "Watch", "PagedList"])
        );
        assert_eq!(profile("tool_page")["references"], json!(false));
        assert_eq!(profile("reply")["mode"], json!("reply"));
        // catalog states.tool_row: no table in a row.
        let row = profile("tool_row")["components"].clone();
        assert!(row.as_array().unwrap().contains(&json!("Badge")));
        assert!(!row.as_array().unwrap().contains(&json!("Table")));
        assert_eq!(document["versions"]["dialects"], json!(["1.0"]));
        assert_eq!(
            document["versions"]["catalogs"][0]["digest"],
            json!(include_str!("../../catalog/catalog-v1.digest").trim())
        );
        assert_eq!(document["versions"]["catalogs"][0]["layout"], json!("1"));
    }

    #[test]
    fn listed_names_are_the_checkers_known_functions() {
        use crate::{CheckContext, DiagnosticCode};
        let context = CheckContext::new(Profile::ShowUi);
        let unknown = |source: &str| {
            crate::check::check(source.as_bytes(), &context)
                .unwrap()
                .diagnostics
                .iter()
                .any(|d| d.code() == DiagnosticCode::UnknownFunction)
        };
        assert!(!unknown("{Abs(1)}"));
        assert!(unknown("{Gauge(1)}"));
    }
}
