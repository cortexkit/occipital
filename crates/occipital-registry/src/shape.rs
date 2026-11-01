//! Tiers 2 and 3: decoding a canonical-profile JSON value into the typed
//! template while judging its root, its `schema`, its member names and its
//! JSON types. Value rules (tiers 4 and 5) run afterwards on the typed model,
//! in `rules`, so the parser and the typed builders share them.

use crate::canon::{compare_keys, Integer, Object, Value};
use crate::error::{RegistryError, RuleCode};
use crate::model::{CatalogRef, ReadRef, Template, TEMPLATE_SCHEMA};
use crate::walk::{Faults, Path, Tier};

/// Decodes a template document, or returns its tier 2 or tier 3 fault.
pub(crate) fn decode_template(value: &Value) -> Result<Template, RegistryError> {
    let root = Path::ROOT;
    let mut faults = Faults::default();
    let Value::Object(object) = value else {
        faults.report(Tier::Root, RuleCode::WrongType, &root);
        return Err(fault_or_root(faults));
    };
    check_schema(object, &mut faults);
    if faults.any() {
        return Err(fault_or_root(faults));
    }
    let decoded = template(object, &mut faults);
    match (faults.into_error(), decoded) {
        (Some(error), _) => Err(error),
        (None, Some(decoded)) => Ok(decoded),
        // The decoder returns `None` only after reporting a fault, so this
        // arm is never taken; it still answers with a refusal, not a panic.
        (None, None) => Err(fault_or_root(Faults::default())),
    }
}

fn fault_or_root(faults: Faults) -> RegistryError {
    faults.into_error().unwrap_or(RegistryError::Rule {
        code: RuleCode::WrongType,
        path: String::new(),
    })
}

/// Tier 2: the top-level `schema` must be present, a string, and exactly
/// [`TEMPLATE_SCHEMA`]. Another version of it (`occipital.template/<n>`, n a
/// positive integer other than 1) is `registry_schema_unsupported`; any other
/// string is `invalid_value`.
fn check_schema(object: &Object, faults: &mut Faults) {
    let path = Path::ROOT.key("schema");
    let code = match object.get("schema") {
        None => RuleCode::MissingField,
        Some(Value::String(schema)) if schema == TEMPLATE_SCHEMA => return,
        Some(Value::String(schema)) => {
            let prefix = TEMPLATE_SCHEMA
                .rsplit_once('/')
                .map_or(TEMPLATE_SCHEMA, |(prefix, _)| prefix);
            let other_version = schema
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
                .is_some_and(|version| {
                    version != "1"
                        && version.bytes().all(|byte| byte.is_ascii_digit())
                        && version.bytes().next().is_some_and(|byte| byte != b'0')
                });
            if other_version {
                RuleCode::RegistrySchemaUnsupported
            } else {
                RuleCode::InvalidValue
            }
        }
        Some(_) => RuleCode::WrongType,
    };
    faults.report(Tier::Root, code, &path);
}

/// Visits an object's members in walk order: present names and absent
/// required names, sorted by UTF-16 code units, skipping `skip` (`schema`,
/// judged before). An unknown name is `unknown_field` and an absent one
/// `missing_field`; `visit` decodes each member that is present and known.
fn members<'v>(
    object: &'v Object,
    required: &[&'static str],
    skip: &str,
    path: &Path<'_>,
    faults: &mut Faults,
    mut visit: impl FnMut(&str, &'v Value, &Path<'_>, &mut Faults),
) {
    let mut names: Vec<&str> = object.iter().map(|(name, _)| name).collect();
    names.extend(required.iter().filter(|name| object.get(name).is_none()));
    names.sort_by(|a, b| compare_keys(a, b));
    names.dedup();
    for name in names {
        if name == skip {
            continue;
        }
        let member_path = path.key(name);
        match (object.get(name), required.contains(&name)) {
            (Some(value), true) => visit(name, value, &member_path, faults),
            (Some(_), false) => faults.report(Tier::Shape, RuleCode::UnknownField, &member_path),
            (None, _) => faults.report(Tier::Shape, RuleCode::MissingField, &member_path),
        }
    }
}

fn wrong_type<T>(path: &Path<'_>, faults: &mut Faults) -> Option<T> {
    faults.report(Tier::Shape, RuleCode::WrongType, path);
    None
}

fn object<'v>(value: &'v Value, path: &Path<'_>, faults: &mut Faults) -> Option<&'v Object> {
    match value {
        Value::Object(object) => Some(object),
        _ => wrong_type(path, faults),
    }
}

fn string(value: &Value, path: &Path<'_>, faults: &mut Faults) -> Option<String> {
    match value {
        Value::String(string) => Some(string.clone()),
        _ => wrong_type(path, faults),
    }
}

fn integer(value: &Value, path: &Path<'_>, faults: &mut Faults) -> Option<Integer> {
    match value {
        Value::Integer(integer) => Some(*integer),
        _ => wrong_type(path, faults),
    }
}

/// Decodes every element of an array, judging each one even after a fault.
fn each<T>(
    value: &Value,
    path: &Path<'_>,
    faults: &mut Faults,
    mut decode: impl FnMut(&Value, &Path<'_>, &mut Faults) -> Option<T>,
) -> Option<Vec<T>> {
    let Value::Array(elements) = value else {
        return wrong_type(path, faults);
    };
    let mut out = Vec::with_capacity(elements.len());
    let mut complete = true;
    for (index, element) in elements.iter().enumerate() {
        match decode(element, &path.index(index), faults) {
            Some(decoded) => out.push(decoded),
            None => complete = false,
        }
    }
    complete.then_some(out)
}

fn catalog_ref(value: &Value, path: &Path<'_>, faults: &mut Faults) -> Option<CatalogRef> {
    let object = object(value, path, faults)?;
    let (mut version, mut digest) = (None, None);
    members(
        object,
        &["digest", "version"],
        "",
        path,
        faults,
        |name, value, path, faults| match name {
            "digest" => digest = string(value, path, faults),
            "version" => version = string(value, path, faults),
            _ => {}
        },
    );
    Some(CatalogRef {
        version: version?,
        digest: digest?,
    })
}

fn read_ref(value: &Value, path: &Path<'_>, faults: &mut Faults) -> Option<ReadRef> {
    let object = object(value, path, faults)?;
    let (mut id, mut version) = (None, None);
    members(
        object,
        &["id", "version"],
        "",
        path,
        faults,
        |name, value, path, faults| match name {
            "id" => id = string(value, path, faults),
            "version" => version = integer(value, path, faults),
            _ => {}
        },
    );
    Some(ReadRef {
        id: id?,
        version: version?,
    })
}

const TEMPLATE_MEMBERS: &[&str] = &[
    "catalog",
    "executionDialect",
    "profile",
    "reads",
    "schema",
    "source",
];

fn template(object: &Object, faults: &mut Faults) -> Option<Template> {
    let (mut profile, mut execution_dialect, mut catalog) = (None, None, None);
    let (mut reads, mut source) = (None, None);
    members(
        object,
        TEMPLATE_MEMBERS,
        "schema",
        &Path::ROOT,
        faults,
        |name, value, path, faults| match name {
            "catalog" => catalog = catalog_ref(value, path, faults),
            "executionDialect" => execution_dialect = string(value, path, faults),
            "profile" => profile = string(value, path, faults),
            "reads" => reads = each(value, path, faults, read_ref),
            "source" => source = string(value, path, faults),
            _ => {}
        },
    );
    Some(Template {
        profile: profile?,
        execution_dialect: execution_dialect?,
        catalog: catalog?,
        reads: reads?,
        source: source?,
    })
}
