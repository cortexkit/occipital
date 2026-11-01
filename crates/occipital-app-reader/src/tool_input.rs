//! A tool call's arguments as a tool template reads them: `input` on the app's
//! tool part, with `inputTruncatedFields` naming what was shortened.
//!
//! Only a part that names its tool version (`schemaPin`) carries them, since
//! only such a call can have a template. A row's template gets a projection
//! that stays parseable JSON: the page's `arguments` member is cut as a string
//! at 2 KiB, which can't be parsed once cut. An exact message read, which
//! backs the call's page, carries the whole arguments.
//!
//! The projection shortens string fields only, never cuts JSON, and keeps the
//! whole within [`INPUT_BYTES`] of compact JSON. It shortens the longest
//! strings first: every string longer than one shared cap is cut on a
//! character boundary and ends with `…`, and the cap is the largest that
//! fits. Arguments that don't fit even with every string shortened to `…`, or
//! that aren't an object, give no `input`, and the call draws as the plain
//! chip.

use serde_json::{json, Map, Value};

/// The most bytes of compact JSON a row's `input` holds.
pub const INPUT_BYTES: usize = 2_048;

const ELLIPSIS: &str = "…";

/// Arguments shortened to fit [`INPUT_BYTES`], and the JSON Pointer of each
/// string it shortened, in walk order.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    pub input: Value,
    pub truncated_fields: Vec<String>,
}

/// The row projection of `arguments`, or `None` when they aren't an object or
/// can't fit even with every string shortened.
pub fn project(arguments: &Value) -> Option<Projection> {
    if !arguments.is_object() {
        return None;
    }
    if size(arguments) <= INPUT_BYTES {
        return Some(Projection {
            input: arguments.clone(),
            truncated_fields: Vec::new(),
        });
    }
    // The serialized size only grows with the cap, so the largest cap that
    // fits is found by bisection.
    let longest = longest_string(arguments);
    let fits =
        |cap: usize| size(&shorten(arguments, cap, &mut String::new(), &mut None)) <= INPUT_BYTES;
    if !fits(0) {
        return None;
    }
    let (mut low, mut high) = (0, longest);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if fits(middle) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let mut truncated_fields = Vec::new();
    let input = shorten(
        arguments,
        low,
        &mut String::new(),
        &mut Some(&mut truncated_fields),
    );
    Some(Projection {
        input,
        truncated_fields,
    })
}

/// Put `input` and `inputTruncatedFields` on an app tool part that names its
/// tool version. `full` is an exact message read, which carries the whole
/// arguments; a list page carries the projection.
pub fn attach(part: &mut Value, arguments: Option<&Value>, full: bool) {
    if !part["schemaPin"].is_string() {
        return;
    }
    let Some(arguments) = arguments.filter(|arguments| arguments.is_object()) else {
        return;
    };
    let projection = if full {
        Some(Projection {
            input: arguments.clone(),
            truncated_fields: Vec::new(),
        })
    } else {
        project(arguments)
    };
    if let Some(projection) = projection {
        part["input"] = projection.input;
        part["inputTruncatedFields"] = json!(projection.truncated_fields);
    }
}

fn size(value: &Value) -> usize {
    value.to_string().len()
}

fn longest_string(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(longest_string).max().unwrap_or(0),
        Value::Object(members) => members.values().map(longest_string).max().unwrap_or(0),
        _ => 0,
    }
}

/// `value` with every string longer than `cap` bytes cut to at most `cap`
/// bytes, the `…` included. Each cut string's pointer is pushed to `cut`.
fn shorten(
    value: &Value,
    cap: usize,
    pointer: &mut String,
    cut: &mut Option<&mut Vec<String>>,
) -> Value {
    match value {
        Value::String(text) if text.len() > cap => {
            if let Some(cut) = cut {
                cut.push(pointer.clone());
            }
            let mut end = cap.saturating_sub(ELLIPSIS.len()).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!("{}{ELLIPSIS}", &text[..end]))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let length = pointer.len();
                    pointer.push('/');
                    pointer.push_str(&index.to_string());
                    let item = shorten(item, cap, pointer, cut);
                    pointer.truncate(length);
                    item
                })
                .collect(),
        ),
        Value::Object(members) => {
            let mut shortened = Map::new();
            for (name, member) in members {
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&name.replace('~', "~0").replace('/', "~1"));
                shortened.insert(name.clone(), shorten(member, cap, pointer, cut));
                pointer.truncate(length);
            }
            Value::Object(shortened)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_arguments_are_kept_whole() {
        let arguments = json!({"command": "date", "timeout": 5});
        assert_eq!(
            project(&arguments),
            Some(Projection {
                input: arguments.clone(),
                truncated_fields: vec![]
            })
        );
    }

    #[test]
    fn the_longest_strings_are_shortened_to_one_shared_cap() {
        let arguments = json!({
            "content": "é".repeat(3_000),
            "path": "src/main.rs",
            "edits": [{"old": "a".repeat(900), "new": "b".repeat(40)}],
            "a/b~c": "x".repeat(800),
        });
        let projection = project(&arguments).unwrap();
        let input = &projection.input;
        assert!(size(input) <= INPUT_BYTES, "{}", size(input));
        // Short fields stay whole.
        assert_eq!(input["path"], "src/main.rs");
        assert_eq!(input["edits"][0]["new"], json!("b".repeat(40)));
        // Long ones end with the ellipsis, cut on a character boundary.
        let content = input["content"].as_str().unwrap();
        assert!(content.ends_with('…') && content.trim_end_matches('…').chars().all(|c| c == 'é'));
        // Pointers in walk order, with `~` and `/` escaped.
        assert_eq!(
            projection.truncated_fields,
            ["/a~1b~0c", "/content", "/edits/0/old"]
        );
        // The cap is the largest that fits: one more byte for each cut field
        // would not.
        let cap = input["edits"][0]["old"].as_str().unwrap().len();
        assert!(size(&shorten(&arguments, cap + 1, &mut String::new(), &mut None)) > INPUT_BYTES);
    }

    #[test]
    fn arguments_that_cannot_fit_or_are_not_an_object_give_no_input() {
        let numbers: Vec<u32> = (0..1_000).collect();
        assert_eq!(project(&json!({"numbers": numbers})), None);
        assert_eq!(project(&json!("text")), None);
        assert_eq!(project(&json!([1, 2])), None);
    }

    #[test]
    fn only_a_part_with_a_schema_pin_gets_input() {
        let arguments = json!({"command": "x".repeat(4_000)});
        let mut plain = json!({"type": "tool"});
        attach(&mut plain, Some(&arguments), false);
        assert_eq!(plain, json!({"type": "tool"}));

        let mut row = json!({"type": "tool", "schemaPin": "tp1:bash:00:1"});
        attach(&mut row, Some(&arguments), false);
        assert_eq!(row["inputTruncatedFields"], json!(["/command"]));
        assert!(size(&row["input"]) <= INPUT_BYTES);

        let mut page = json!({"type": "tool", "schemaPin": "tp1:bash:00:1"});
        attach(&mut page, Some(&arguments), true);
        assert_eq!(page["input"], arguments);
        assert_eq!(page["inputTruncatedFields"], json!([]));

        let mut no_arguments = json!({"type": "tool", "schemaPin": "tp1:bash:00:1"});
        attach(&mut no_arguments, None, false);
        attach(&mut no_arguments, Some(&Value::Null), false);
        assert!(no_arguments.get("input").is_none());
    }
}
