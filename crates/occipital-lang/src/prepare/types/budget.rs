use super::Type;

/// Fixed charges, rather than platform object sizes, keep checker admission
/// identical on every target. Count owned names as well as structural nodes.
pub(super) const TYPE_BUDGET: usize = 16 * 1024 * 1024;

pub(super) struct Budget(pub usize);

impl Budget {
    fn take(&mut self, amount: usize) -> bool {
        if let Some(left) = self.0.checked_sub(amount) {
            self.0 = left;
            true
        } else {
            false
        }
    }

    pub(super) fn admits(&mut self, ty: &Type) -> bool {
        self.take(64)
            && match ty {
                Type::List(item) => self.admits(item),
                Type::Enum(item, values) => {
                    self.admits(item) && values.iter().all(|v| self.json(v))
                }
                Type::Record(fields) | Type::OpenRecord(fields) | Type::Envelope(_, fields) => {
                    fields
                        .iter()
                        .all(|(name, ty)| self.take(name.len()) && self.admits(ty))
                }
                Type::Union(types) => types.iter().all(|ty| self.admits(ty)),
                Type::Tagged(name) => self.take(name.len()),
                _ => true,
            }
    }

    fn json(&mut self, value: &serde_json::Value) -> bool {
        use serde_json::Value::*;
        self.take(64)
            && match value {
                Object(fields) => fields
                    .iter()
                    .all(|(k, v)| self.take(k.len()) && self.json(v)),
                Array(values) => values.iter().all(|v| self.json(v)),
                String(s) => self.take(s.len()),
                Number(n) => self.take(n.to_string().len()),
                _ => true,
            }
    }
}
