use std::collections::BTreeMap;

use crate::{Decimal, Operation, OperationRegistry, Value, ValueKind};

/// Broker-validated provenance. These are explicit inputs, never inferred from
/// an operation name or used as a substitute for broker authorization (§8.2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Authority {
    /// The computer that serves the read.
    pub computer: String,
    /// The calling session.
    pub session: String,
    /// The scope epoch; an ended scope refuses reads.
    pub scope_epoch: String,
    /// The serving provider.
    pub provider: String,
    /// The validated entity the read is about.
    pub entity: String,
    /// The computer the request goes to.
    pub serving_computer: String,
    /// The verified-holder identity; it must equal `serving_computer` before a
    /// read is dispatched.
    pub verified_holder: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// An operation's and its schema's versions, part of every read identity.
pub struct ReadVersion {
    /// The operation version.
    pub operation: String,
    /// The schema version.
    pub schema: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// The full identity of one read (§8.2): equal identities share one request.
pub struct Identity {
    /// The provenance.
    pub authority: Authority,
    /// The operation name.
    pub operation: String,
    /// The versions.
    pub version: ReadVersion,
    /// `query`, `watch` or `paged`.
    pub kind: &'static str,
    /// The arguments as canonical JSON.
    pub arguments: String,
    /// The SHA-256 of the default value.
    pub default_digest: [u8; 32],
    /// A query's refresh interval in seconds.
    pub refresh: Option<Decimal>,
    /// A paged list's page size.
    pub page_size: Option<u16>,
}

#[derive(Clone, Debug)]
pub(crate) struct Reads {
    pub authority: Authority,
    pub versions: BTreeMap<String, ReadVersion>,
    pub registry: OperationRegistry,
}

impl Reads {
    pub(super) fn identity(&self, arguments: &[Value]) -> Result<Identity, &'static str> {
        self.identity_for("Query", arguments)
    }

    pub(super) fn identity_for(
        &self,
        constructor: &str,
        arguments: &[Value],
    ) -> Result<Identity, &'static str> {
        if constructor != "Query" {
            return self.stream_identity(constructor, arguments);
        }
        let [op, args, default, rest @ ..] = arguments else {
            return Err("E_SITE_ARGS");
        };
        let ValueKind::String(op) = op.kind() else {
            return Err("E_SITE_ARGS");
        };
        let Some(Operation::Query {
            arguments: schema, ..
        }) = self.registry.0.get(op.as_str())
        else {
            return Err("E_SITE_ARGS");
        };
        if !matches!(args.kind(), ValueKind::Record(_)) || !data(args) || !data(default) {
            return Err("E_SITE_ARGS");
        }
        let canonical = args.canonical_json().map_err(|_| "E_SITE_ARGS")?;
        if canonical.len() > 8192
            || !schema.accepts(&serde_json::from_str(&canonical).map_err(|_| "E_SITE_ARGS")?)
        {
            return Err("E_SITE_ARGS");
        }
        let refresh = match rest {
            [] => None,
            [value] => match value.kind() {
                ValueKind::Number(n) => {
                    Some((*n).max(Decimal::from_i64(2)).min(Decimal::from_i64(3600)))
                }
                _ => return Err("E_SITE_ARGS"),
            },
            _ => return Err("E_SITE_ARGS"),
        };
        Ok(Identity {
            authority: self.authority.clone(),
            operation: op.as_str().into(),
            version: self.versions.get(op.as_str()).ok_or("E_SITE_ARGS")?.clone(),
            kind: "query",
            arguments: canonical,
            default_digest: hmac_sha256::Hash::hash(
                default
                    .canonical_json()
                    .map_err(|_| "E_SITE_ARGS")?
                    .as_bytes(),
            ),
            refresh,
            page_size: None,
        })
    }

    fn stream_identity(
        &self,
        constructor: &str,
        values: &[Value],
    ) -> Result<Identity, &'static str> {
        let [op, args, rest @ ..] = values else {
            return Err("E_SITE_ARGS");
        };
        let ValueKind::String(op) = op.kind() else {
            return Err("E_SITE_ARGS");
        };
        let (kind, schema) = match (constructor, self.registry.0.get(op.as_str())) {
            ("Watch", Some(Operation::Watch { arguments, .. })) if rest.is_empty() => {
                ("watch", arguments)
            }
            ("PagedList", Some(Operation::Paged { arguments, .. })) => ("paged", arguments),
            _ => return Err("E_SITE_ARGS"),
        };
        if !matches!(args.kind(), ValueKind::Record(_)) || !data(args) {
            return Err("E_SITE_ARGS");
        }
        let arguments = args.canonical_json().map_err(|_| "E_SITE_ARGS")?;
        if arguments.len() > 8192
            || !schema.accepts(&serde_json::from_str(&arguments).map_err(|_| "E_SITE_ARGS")?)
        {
            return Err("E_SITE_ARGS");
        }
        // Page size is a sharing policy, not part of the producer's arguments (§8.8).
        let page_size = if kind == "paged" {
            let size = match rest {
                [] => Decimal::from_i64(50),
                [v] => match v.kind() {
                    ValueKind::Number(n) => *n,
                    _ => return Err("E_SITE_ARGS"),
                },
                _ => return Err("E_SITE_ARGS"),
            };
            if size < Decimal::from_i64(1)
                || size > Decimal::from_i64(200)
                || size.to_string().contains('.')
            {
                return Err("E_SITE_ARGS");
            }
            Some(size.to_string().parse().map_err(|_| "E_SITE_ARGS")?)
        } else {
            None
        };
        Ok(Identity {
            authority: self.authority.clone(),
            operation: op.as_str().into(),
            version: self.versions.get(op.as_str()).ok_or("E_SITE_ARGS")?.clone(),
            kind,
            arguments,
            default_digest: hmac_sha256::Hash::hash(b"null"),
            refresh: None,
            page_size,
        })
    }
}

pub(super) fn data(value: &Value) -> bool {
    match value.kind() {
        ValueKind::Number(_) | ValueKind::String(_) | ValueKind::Boolean(_) | ValueKind::Null => {
            true
        }
        ValueKind::List(values) => values.iter().all(data),
        ValueKind::Record(record) => record.values().all(data),
        _ => false,
    }
}
