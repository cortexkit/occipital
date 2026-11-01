//! `occipital.template/1` rule tests. Each refusal starts from a valid
//! template and changes one thing, then checks the exact code and the JSON
//! Pointer it is placed at, so a rule that stopped firing, fired elsewhere or
//! won the wrong precedence contest fails here.

use occipital_registry::canon::{self, Value};
use occipital_registry::model::{CatalogRef, ReadRef};
use occipital_registry::{RegistryError, Template};

const HEX_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn template_text() -> String {
    r#"{"schema": "occipital.template/1", "profile": "tool_page", "executionDialect": "1.0",
 "catalog": {"version": "1.0", "digest": "sha256:CCC"}, "reads": [{"id": "bash_status", "version": 1}],
 "source": "export const job = Query(\"bash_status\", {}, {running: null, exitCode: null}, 2)\n"}"#
        .replace("CCC", HEX_C)
}

fn json(text: &str) -> Value {
    match canon::parse(text.as_bytes()) {
        Ok(value) => value,
        Err(error) => panic!("test JSON {text:?} does not parse: {error}"),
    }
}

fn template() -> Value {
    json(&template_text())
}

/// The value at a JSON Pointer's parent, and the pointer's last step.
fn parent<'v>(doc: &'v mut Value, pointer: &'v str) -> (&'v mut Value, &'v str) {
    let (head, last) = pointer
        .rsplit_once('/')
        .unwrap_or_else(|| panic!("pointer {pointer:?} has no step"));
    let mut current = doc;
    for step in head.split('/').skip(1) {
        current = match current {
            Value::Object(object) => object
                .get_mut(step)
                .unwrap_or_else(|| panic!("no member {step:?} in {pointer:?}")),
            Value::Array(elements) => {
                let index: usize = step.parse().expect("array index");
                elements
                    .get_mut(index)
                    .unwrap_or_else(|| panic!("no element {step:?} in {pointer:?}"))
            }
            _ => panic!("{pointer:?} passes through a scalar"),
        };
    }
    (current, last)
}

/// Applies edits to the base template: `(pointer, Some(json))` sets a member,
/// replaces an element or appends at `-`; `(pointer, None)` removes.
fn edited(edits: &[(&str, Option<&str>)]) -> Value {
    let mut doc = template();
    for (pointer, change) in edits {
        let (parent, last) = parent(&mut doc, pointer);
        match (parent, change) {
            (Value::Object(object), Some(text)) => {
                object.insert(last, json(text));
            }
            (Value::Object(object), None) => {
                object
                    .remove(last)
                    .unwrap_or_else(|| panic!("nothing to remove at {pointer:?}"));
            }
            (Value::Array(elements), Some(text)) if last == "-" => elements.push(json(text)),
            (Value::Array(elements), Some(text)) => {
                elements[last.parse::<usize>().expect("array index")] = json(text);
            }
            (Value::Array(elements), None) => {
                elements.remove(last.parse::<usize>().expect("array index"));
            }
            _ => panic!("{pointer:?} has a scalar parent"),
        }
    }
    doc
}

fn parse(bytes: &[u8]) -> Result<Vec<u8>, RegistryError> {
    Template::parse(bytes).and_then(|template| template.to_canonical())
}

#[track_caller]
fn refused_bytes(bytes: &[u8], code: &str, path: Option<&str>) {
    match parse(bytes) {
        Ok(_) => panic!(
            "accepted, expected {code} at {path:?}: {}",
            String::from_utf8_lossy(bytes)
        ),
        Err(error) => assert_eq!(
            (error.code(), error.path()),
            (code, path),
            "document {}",
            String::from_utf8_lossy(bytes)
        ),
    }
}

#[track_caller]
fn refused(edits: &[(&str, Option<&str>)], code: &str, path: &str) {
    refused_bytes(&edited(edits).to_canonical(), code, Some(path));
}

/// Asserts that the edited template is accepted, and that the typed template
/// it parses to serializes back to the document's canonical bytes.
#[track_caller]
fn accepted(edits: &[(&str, Option<&str>)]) {
    let bytes = edited(edits).to_canonical();
    match parse(&bytes) {
        Ok(round_trip) => assert_eq!(
            String::from_utf8_lossy(&round_trip),
            String::from_utf8_lossy(&bytes)
        ),
        Err(error) => panic!("refused with {error}: {}", String::from_utf8_lossy(&bytes)),
    }
}

fn typed_template() -> Template {
    Template {
        profile: "tool_row".into(),
        execution_dialect: "1.0".into(),
        catalog: CatalogRef {
            version: "1.0".into(),
            digest: format!("sha256:{HEX_C}"),
        },
        reads: vec![ReadRef {
            id: "bash_status".into(),
            version: canon::Integer::new(2).expect("in range"),
        }],
        source: "export const row = Text(\"é\")\n\u{0001}".into(),
    }
}

#[test]
fn a_template_canonicalizes_and_digests() {
    let bytes = template().to_canonical();
    let parsed = Template::parse(&bytes).expect("valid");
    assert_eq!(parsed.to_canonical().expect("valid"), bytes);
    assert_eq!(
        parsed.digest().expect("valid"),
        occipital_registry::sha256_digest(&bytes)
    );
}

#[test]
fn raw_input_with_whitespace_and_any_member_order_parses_to_the_canonical_form() {
    let raw = template_text();
    let template = Template::parse(raw.as_bytes()).expect("valid");
    assert_eq!(
        template.to_canonical().expect("valid"),
        canon::canonicalize(raw.as_bytes()).expect("valid JSON")
    );
    assert_ne!(raw.as_bytes(), canon::canonicalize(raw.as_bytes()).unwrap());
}

#[test]
fn a_typed_template_round_trips_to_identical_bytes() {
    let template = typed_template();
    let bytes = template.to_canonical().expect("valid");
    let parsed = Template::parse(&bytes).expect("parses back");
    assert_eq!(parsed, template);
    assert_eq!(parsed.to_canonical().expect("valid"), bytes);
    assert_eq!(canon::canonicalize(&bytes).expect("canonical"), bytes);
}

#[test]
fn a_typed_template_runs_the_same_rules_as_parsing() {
    let mut template = typed_template();
    template.source = "x".repeat(65_537);
    let error = template.to_canonical().expect_err("too large");
    assert_eq!(
        (error.code(), error.path()),
        ("template_too_large", Some("/source"))
    );

    // Faults in two tiers: the value fault outranks the repeat, as in parsing.
    let mut template = typed_template();
    template.reads.push(template.reads[0].clone());
    template.profile = "reply".into();
    let error = template.to_canonical().expect_err("two faults");
    assert_eq!(
        (error.code(), error.path()),
        ("invalid_value", Some("/profile"))
    );
}

#[test]
fn a_root_that_is_not_an_object_is_wrong_type() {
    for root in ["[]", r#""root""#, "1", "null", "true"] {
        refused_bytes(root.as_bytes(), "wrong_type", Some(""));
    }
}

#[test]
fn schema_rules() {
    refused(&[("/schema", None)], "missing_field", "/schema");
    refused(&[("/schema", Some("null"))], "wrong_type", "/schema");
    refused(&[("/schema", Some("1"))], "wrong_type", "/schema");
    for other_version in ["occipital.template/2", "occipital.template/10"] {
        refused(
            &[("/schema", Some(&format!("\"{other_version}\"")))],
            "registry_schema_unsupported",
            "/schema",
        );
    }
    for other in [
        "occipital.template/0",
        "occipital.template/01",
        "occipital.template/1x",
        "occipital.template",
        "occipital.manifest/1",
        "other",
    ] {
        refused(
            &[("/schema", Some(&format!("\"{other}\"")))],
            "invalid_value",
            "/schema",
        );
    }
}

#[test]
fn every_member_is_required_and_typed() {
    for path in [
        "/catalog",
        "/executionDialect",
        "/profile",
        "/reads",
        "/source",
        "/catalog/digest",
        "/catalog/version",
        "/reads/0/id",
        "/reads/0/version",
    ] {
        refused(&[(path, None)], "missing_field", path);
    }
    for (path, text) in [
        ("/catalog", "[]"),
        ("/executionDialect", "1"),
        ("/profile", "null"),
        ("/reads", "{}"),
        ("/source", "1"),
        ("/catalog/digest", "null"),
        ("/catalog/version", "1"),
        ("/reads/0", r#""bash_status""#),
        ("/reads/0/id", "1"),
        ("/reads/0/version", r#""1""#),
    ] {
        refused(&[(path, Some(text))], "wrong_type", path);
    }
}

#[test]
fn an_unknown_member_is_refused_in_each_object() {
    for path in ["/extra", "/catalog/extra", "/reads/0/extra"] {
        refused(&[(path, Some("null"))], "unknown_field", path);
    }
}

#[test]
fn value_rules() {
    refused(
        &[("/profile", Some(r#""reply""#))],
        "invalid_value",
        "/profile",
    );
    refused(
        &[("/profile", Some(r#""show_ui""#))],
        "invalid_value",
        "/profile",
    );
    for dialect in ["1", "1.", ".1", "1.0.0", "a.b", "1.0 "] {
        refused(
            &[("/executionDialect", Some(&format!("\"{dialect}\"")))],
            "invalid_value",
            "/executionDialect",
        );
    }
    for id in ["", "Bad", "1a", "a.b", &"a".repeat(49)] {
        refused(
            &[("/reads/0/id", Some(&format!("\"{id}\"")))],
            "invalid_id",
            "/reads/0/id",
        );
    }
    for version in ["0", "-1"] {
        refused(
            &[("/reads/0/version", Some(version))],
            "invalid_value",
            "/reads/0/version",
        );
    }
    for digest in [
        format!("sha512:{HEX_C}"),
        format!("sha256:{}", &HEX_C[1..]),
        format!("sha256:{}", HEX_C.to_uppercase()),
        format!("sha256:{}", "g".repeat(64)),
        HEX_C.to_owned(),
    ] {
        refused(
            &[("/catalog/digest", Some(&format!("\"{digest}\"")))],
            "invalid_digest",
            "/catalog/digest",
        );
    }
    accepted(&[("/profile", Some(r#""tool_row""#)), ("/reads", Some("[]"))]);
    accepted(&[("/executionDialect", Some(r#""12.345""#))]);
    accepted(&[("/reads/0/id", Some(&format!("\"a{}\"", "0".repeat(47))))]);
}

#[test]
fn the_source_may_hold_65536_utf8_bytes() {
    // Two-byte characters, so a cap counted in characters would differ.
    let at = format!("\"{}\"", "é".repeat(32_768));
    let over = format!("\"{}a\"", "é".repeat(32_768));
    accepted(&[("/source", Some(&at))]);
    refused(&[("/source", Some(&over))], "template_too_large", "/source");
}

#[test]
fn a_repeated_read_is_refused_at_its_second_occurrence() {
    refused(
        &[("/reads/-", Some(r#"{"id": "bash_status", "version": 1}"#))],
        "duplicate_entry",
        "/reads/1",
    );
    accepted(&[("/reads/-", Some(r#"{"id": "bash_status", "version": 2}"#))]);
}

#[test]
fn canonical_json_is_judged_before_the_root() {
    refused_bytes(b"[1.0]", "number_not_integer", None);
    refused_bytes(br#"{"schema":"x","schema":"y"}"#, "duplicate_key", None);
}

#[test]
fn the_schema_is_judged_before_the_members() {
    // `aaa` comes first in walk order, but `schema` is judged first.
    refused(
        &[
            ("/aaa", Some("1")),
            ("/schema", Some(r#""occipital.template/2""#)),
        ],
        "registry_schema_unsupported",
        "/schema",
    );
}

#[test]
fn shape_outranks_values_and_values_outrank_repeats() {
    refused(
        &[("/profile", Some(r#""reply""#)), ("/source", Some("1"))],
        "wrong_type",
        "/source",
    );
    refused(
        &[
            ("/reads/-", Some(r#"{"id": "bash_status", "version": 1}"#)),
            ("/source", Some(&format!("\"{}\"", "x".repeat(65_537)))),
        ],
        "template_too_large",
        "/source",
    );
}

#[test]
fn within_a_tier_the_first_fault_in_walk_order_wins() {
    // Members walk in canonical order: catalog, executionDialect, profile,
    // reads, source.
    refused(
        &[
            ("/profile", Some(r#""reply""#)),
            ("/executionDialect", Some(r#""1""#)),
            ("/catalog/digest", Some(r#""bad""#)),
        ],
        "invalid_digest",
        "/catalog/digest",
    );
    refused(
        &[
            ("/profile", Some(r#""reply""#)),
            ("/executionDialect", Some(r#""1""#)),
        ],
        "invalid_value",
        "/executionDialect",
    );
    // A missing member sorts by its name among the present ones.
    refused(
        &[("/source", Some("1")), ("/catalog", None)],
        "missing_field",
        "/catalog",
    );
}
