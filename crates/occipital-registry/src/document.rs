//! Reading templates from bytes and writing typed templates as canonical
//! bytes. Both directions run every rule: writing serializes the template and
//! reads the bytes back through the same parser, so a typed template is
//! written only when its bytes would be accepted.

use crate::canon::{self, Integer, Object, Value};
use crate::digest::sha256_digest;
use crate::error::RegistryError;
use crate::model::{CatalogRef, ReadRef, Template, TEMPLATE_SCHEMA};
use crate::rules;
use crate::shape;
use crate::walk::Faults;

impl Template {
    /// Reads a template from raw bytes: canonical JSON first, then every
    /// template rule, reporting one fault as [`crate::model`] describes.
    pub fn parse(input: &[u8]) -> Result<Self, RegistryError> {
        Self::from_value(&canon::parse(input)?)
    }

    /// Reads a template from an already decoded value.
    pub fn from_value(value: &Value) -> Result<Self, RegistryError> {
        let template = shape::decode_template(value)?;
        let mut faults = Faults::default();
        rules::check_template(&template, &mut faults);
        match faults.into_error() {
            Some(error) => Err(error),
            None => Ok(template),
        }
    }

    /// The canonical bytes of this template, or the fault its bytes would be
    /// refused with, including `input_too_large` when they exceed 1,048,576
    /// bytes.
    pub fn to_canonical(&self) -> Result<Vec<u8>, RegistryError> {
        let bytes = self.to_value().to_canonical();
        Self::parse(&bytes)?;
        Ok(bytes)
    }

    /// The template digest, `sha256:` and the lowercase hex SHA-256 of the
    /// canonical bytes.
    pub fn digest(&self) -> Result<String, RegistryError> {
        self.to_canonical().map(|bytes| sha256_digest(&bytes))
    }

    /// This template as a JSON value, without checking any rule.
    pub(crate) fn to_value(&self) -> Value {
        members([
            ("schema", string(TEMPLATE_SCHEMA)),
            ("profile", string(&self.profile)),
            ("executionDialect", string(&self.execution_dialect)),
            ("catalog", catalog(&self.catalog)),
            ("reads", Value::Array(self.reads.iter().map(read).collect())),
            ("source", string(&self.source)),
        ])
    }
}

fn members<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut object = Object::new();
    for (name, value) in entries {
        object.insert(name, value);
    }
    Value::Object(object)
}

fn string(text: &str) -> Value {
    Value::String(text.to_owned())
}

fn integer(number: Integer) -> Value {
    Value::Integer(number)
}

fn catalog(catalog: &CatalogRef) -> Value {
    members([
        ("version", string(&catalog.version)),
        ("digest", string(&catalog.digest)),
    ])
}

fn read(read: &ReadRef) -> Value {
    members([("id", string(&read.id)), ("version", integer(read.version))])
}
