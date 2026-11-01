use occipital_registry::canon::Value;

use super::support::{edit, remove, set, template};
use super::{Kind, Result, Vectors};

pub fn add(v: &mut Vectors) -> Result<()> {
    let template = template()?;
    v.accepted("template-page", template.to_canonical(), Kind::Template)?;
    v.accepted(
        "template-row",
        edit(template.clone(), "/profile", r#""tool_row""#)?.to_canonical(),
        Kind::Template,
    )?;
    values(v, &template)?;
    shapes(v, &template)?;
    source_cap(v, &template)?;
    refusal(
        v,
        "template-duplicate-read",
        &template,
        "/reads",
        r#"[{"id":"status","version":1},{"id":"status","version":1}]"#,
        "duplicate_entry",
    )
}

/// `base` with the value at `pointer` replaced by `text`, refused with `code`.
fn refusal(
    v: &mut Vectors,
    name: &str,
    base: &Value,
    pointer: &str,
    text: &str,
    code: &str,
) -> Result<()> {
    v.refused(
        name,
        edit(base.clone(), pointer, text)?.to_canonical(),
        Kind::Template,
        code,
    )
}

fn values(v: &mut Vectors, template: &Value) -> Result<()> {
    for (name, pointer, text, code) in [
        ("schema", "/schema", r#""other""#, "invalid_value"),
        ("profile", "/profile", r#""other""#, "invalid_value"),
        ("dialect", "/executionDialect", r#""1""#, "invalid_value"),
        ("read-id", "/reads/0/id", r#""Bad""#, "invalid_id"),
        ("read-version", "/reads/0/version", "0", "invalid_value"),
        (
            "catalog-digest",
            "/catalog/digest",
            r#""bad""#,
            "invalid_digest",
        ),
    ] {
        refusal(
            v,
            &format!("template-{name}"),
            template,
            pointer,
            text,
            code,
        )?;
    }
    Ok(())
}

fn shapes(v: &mut Vectors, template: &Value) -> Result<()> {
    v.refused("template-array-root", b"[]", Kind::Template, "wrong_type")?;
    // A root that is not an object and holds a fraction: canonical JSON is
    // judged first.
    v.refused(
        "precedence-template-canon-before-root",
        b"[1.0]",
        Kind::Template,
        "number_not_integer",
    )?;
    v.refused(
        "template-string-root",
        br#""root""#,
        Kind::Template,
        "wrong_type",
    )?;
    let mut missing = template.clone();
    remove(&mut missing, "/schema")?;
    v.refused(
        "template-missing-schema",
        missing.to_canonical(),
        Kind::Template,
        "missing_field",
    )?;
    refusal(
        v,
        "template-schema-type",
        template,
        "/schema",
        "null",
        "wrong_type",
    )?;
    // Another schema version is judged before the members, so the unknown
    // member beside it is never reported.
    let mut unsupported = edit(template.clone(), "/schema", r#""occipital.template/2""#)?;
    set(&mut unsupported, "/extra", Value::Null)?;
    v.refused(
        "precedence-template-unsupported-before-shape",
        unsupported.to_canonical(),
        Kind::Template,
        "registry_schema_unsupported",
    )?;
    refusal(
        v,
        "template-unknown-field",
        template,
        "/extra",
        "null",
        "unknown_field",
    )?;
    let mut doc = template.clone();
    remove(&mut doc, "/source")?;
    v.refused(
        "shape-template-missing",
        doc.to_canonical(),
        Kind::Template,
        "missing_field",
    )?;
    refusal(
        v,
        "shape-template-wrong-type",
        template,
        "/source",
        "null",
        "wrong_type",
    )
}

/// A source of exactly 65,536 UTF-8 bytes is accepted and one more is
/// refused; two-byte characters make the byte count differ from the
/// character count.
fn source_cap(v: &mut Vectors, template: &Value) -> Result<()> {
    for (bytes, accepted) in [(65536, true), (65537, false)] {
        let mut doc = template.clone();
        set(
            &mut doc,
            "/source",
            Value::String("é".repeat(bytes / 2) + if bytes % 2 == 1 { "a" } else { "" }),
        )?;
        if accepted {
            v.accepted("template-source-at-cap", doc.to_canonical(), Kind::Template)?;
        } else {
            v.refused(
                "template-source-over-cap",
                doc.to_canonical(),
                Kind::Template,
                "template_too_large",
            )?;
        }
    }
    Ok(())
}
