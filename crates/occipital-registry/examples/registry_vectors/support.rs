use occipital_registry::canon::{self, Value};

use super::Result;

pub fn json(text: &str) -> Result<Value> {
    Ok(canon::parse(text.as_bytes())?)
}

pub fn at<'a>(mut value: &'a mut Value, pointer: &str) -> Result<&'a mut Value> {
    for step in pointer.split('/').skip(1) {
        value = match value {
            Value::Object(object) => object.get_mut(step),
            Value::Array(array) => array.get_mut(step.parse::<usize>()?),
            _ => None,
        }
        .ok_or_else(|| format!("no value at {pointer}"))?;
    }
    Ok(value)
}

pub fn set(value: &mut Value, pointer: &str, replacement: Value) -> Result<()> {
    let (head, key) = pointer.rsplit_once('/').ok_or("missing pointer step")?;
    match at(value, head)? {
        Value::Object(object) => {
            object.insert(key, replacement);
        }
        Value::Array(array) => {
            let slot = array
                .get_mut(key.parse::<usize>()?)
                .ok_or("missing array element")?;
            *slot = replacement;
        }
        _ => return Err(format!("scalar parent at {pointer}").into()),
    }
    Ok(())
}

pub fn remove(value: &mut Value, pointer: &str) -> Result<()> {
    let (head, key) = pointer.rsplit_once('/').ok_or("missing pointer step")?;
    match at(value, head)? {
        Value::Object(object) => object
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| format!("no member at {pointer}").into()),
        _ => Err(format!("no member at {pointer}").into()),
    }
}

pub fn edit(mut value: Value, pointer: &str, text: &str) -> Result<Value> {
    set(&mut value, pointer, json(text)?)?;
    Ok(value)
}

pub fn template() -> Result<Value> {
    json(&format!(
        r#"{{"schema":"occipital.template/1","profile":"tool_page","executionDialect":"1.0","catalog":{{"version":"1.0","digest":"sha256:{}"}},"reads":[{{"id":"status","version":1}}],"source":"export const job = Query(\"status\", {{}}, null, 2)\n"}}"#,
        "c".repeat(64)
    ))
}
