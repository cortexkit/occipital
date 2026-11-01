//! The JSON envelopes entry points decode. Each mirrors a library struct
//! field for field, decodes closed (`deny_unknown_fields`), writes decimals as
//! canonical decimal strings and enumerations as lower-case strings.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use occipital_lang::{
    CatalogVersion, CheckContext, DataSnapshot, HandleRecord, HostValues, LayoutContext, Network,
    OperationRegistry, Placement, Profile, ReferenceRecord, ReplyEvent, ShowUiArguments,
    ToolInputs,
};
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::boundary::Failure;
use crate::{base64, decimal};

/// `layout_json`: a [`LayoutContext`].
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutEnvelope {
    /// The available width, as a canonical decimal string.
    pub(crate) width: String,
    /// The platform's content size category, such as `large`.
    pub(crate) content_size_category: String,
    /// The recorded network policy.
    pub(crate) network: NetworkName,
    /// Row or page placement.
    pub(crate) placement: PlacementName,
}

/// [`Network`] as `yes`, `no` or `unknown`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NetworkName {
    Yes,
    No,
    Unknown,
}

/// [`Placement`] as `row` or `page`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PlacementName {
    Row,
    Page,
}

impl LayoutEnvelope {
    /// The layout context, or `OCC_INVALID_ARGUMENT` when `width` is not a
    /// canonical decimal in range.
    pub(crate) fn into_context(self) -> Result<LayoutContext, Failure> {
        Ok(LayoutContext {
            width: decimal::parse(&self.width).ok_or_else(Failure::invalid)?,
            content_size_category: self.content_size_category,
            network: match self.network {
                NetworkName::Yes => Network::Yes,
                NetworkName::No => Network::No,
                NetworkName::Unknown => Network::Unknown,
            },
            placement: match self.placement {
                PlacementName::Row => Placement::Row,
                PlacementName::Page => Placement::Page,
            },
        })
    }

    /// The envelope of `layout`.
    // Only the layout test hook re-encodes a layout.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub(crate) fn from_context(layout: &LayoutContext) -> Self {
        Self {
            width: decimal::encode(layout.width),
            content_size_category: layout.content_size_category.clone(),
            network: match layout.network {
                Network::Yes => NetworkName::Yes,
                Network::No => NetworkName::No,
                Network::Unknown => NetworkName::Unknown,
            },
            placement: match layout.placement {
                Placement::Row => PlacementName::Row,
                Placement::Page => PlacementName::Page,
            },
        }
    }
}

/// `stamp_json`: the dialect the reply was taught, as `reply::Session::new`
/// takes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StampEnvelope {
    /// The execution dialect, such as `1.0`.
    pub(crate) dialect: String,
}

/// `records_json`: the records stamped at store time that
/// `reply::Session::finish` binds. Both members are required.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordsEnvelope {
    /// Reference records by N in `$$tc(N)`. Keys are canonical decimal
    /// integers (`"7"`, not `"07"`) and appear once each.
    pub(crate) references: Indexed<ReferenceRecord>,
    /// Handle records.
    pub(crate) handles: Vec<HandleRecord>,
}

/// A JSON object keyed by N in `$$tc(N)`: each key a canonical decimal
/// integer (`"7"`, not `"07"`) that appears once.
#[derive(Debug)]
pub(crate) struct Indexed<T>(pub(crate) BTreeMap<u64, T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Indexed<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IndexedVisitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for IndexedVisitor<T> {
            type Value = Indexed<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object keyed by N")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Indexed<T>, A::Error> {
                let mut entries = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    let n = canonical_index(&key)
                        .ok_or_else(|| de::Error::custom(format!("bad reference key {key:?}")))?;
                    let value = map.next_value::<T>()?;
                    if entries.insert(n, value).is_some() {
                        return Err(de::Error::custom(format!("duplicate reference {n}")));
                    }
                }
                Ok(Indexed(entries))
            }
        }
        deserializer.deserialize_map(IndexedVisitor(PhantomData))
    }
}

/// A non-negative integer written canonically: digits only, no leading zero
/// except `0` itself, and within `u64`.
fn canonical_index(text: &str) -> Option<u64> {
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if !digits || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}

/// `event_json`: one `reply::Event` other than `MeasureComplete`, tagged by
/// `kind`:
///
/// - `{"kind":"snapshot","reference":7,"result":{"ok":"<base64>"}}`, or
///   `"result":"failed"` for a failed fetch;
/// - `{"kind":"handle_availability","record_id":"…","available":true}`;
/// - `{"kind":"account_release"}`;
/// - `{"kind":"layout","layout":{layout_json}}`.
///
/// `MeasureComplete` is not an event a host sends: `occ_reply_measure`
/// delivers it. Its kind is unknown here, so it is an invalid argument.
///
/// The members are decoded into one closed struct and then checked against
/// the kind. An internally tagged serde enum would buffer the members first,
/// and buffered numbers do not decode under serde_json's
/// `arbitrary_precision` feature, which occipital-lang enables.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventEnvelope {
    kind: EventKind,
    #[serde(default, deserialize_with = "present")]
    reference: Option<u64>,
    #[serde(default, deserialize_with = "present")]
    result: Option<SnapshotResult>,
    #[serde(default, deserialize_with = "present")]
    record_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    available: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    layout: Option<LayoutEnvelope>,
}

/// Deserializes a member that is present, so that with `#[serde(default)]`
/// an absent member is `None` and an explicit `null` is an error.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    Snapshot,
    HandleAvailability,
    AccountRelease,
    Layout,
}

/// A snapshot fetch result: `{"ok":"<base64 bytes>"}` or `"failed"`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SnapshotResult {
    Ok(String),
    Failed,
}

impl EventEnvelope {
    /// The library event, or `OCC_INVALID_ARGUMENT` when the members do not
    /// match the kind exactly or the bytes are not canonical base64.
    pub(crate) fn into_event(self) -> Result<ReplyEvent, Failure> {
        let members = [
            self.reference.is_some(),
            self.result.is_some(),
            self.record_id.is_some(),
            self.available.is_some(),
            self.layout.is_some(),
        ];
        let expected = match self.kind {
            EventKind::Snapshot => [true, true, false, false, false],
            EventKind::HandleAvailability => [false, false, true, true, false],
            EventKind::AccountRelease => [false; 5],
            EventKind::Layout => [false, false, false, false, true],
        };
        if members != expected {
            return Err(Failure::invalid());
        }
        Ok(match self.kind {
            EventKind::Snapshot => ReplyEvent::Snapshot {
                reference: self.reference.unwrap_or_default(),
                result: match self.result {
                    Some(SnapshotResult::Ok(text)) => {
                        Ok(base64::decode(&text).ok_or_else(Failure::invalid)?)
                    }
                    _ => Err(()),
                },
            },
            EventKind::HandleAvailability => ReplyEvent::HandleAvailability {
                record_id: self.record_id.unwrap_or_default(),
                available: self.available.unwrap_or_default(),
            },
            EventKind::AccountRelease => ReplyEvent::AccountRelease,
            EventKind::Layout => {
                ReplyEvent::Layout(self.layout.ok_or_else(Failure::invalid)?.into_context()?)
            }
        })
    }
}

/// `ctx_json` of `occ_program_check`: `{"context", "host"}`. `host` may be
/// left out, and then [`HostValues::default`] is used; an explicit `null`
/// is an error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckEnvelope {
    context: ContextEnvelope,
    #[serde(default, deserialize_with = "present")]
    host: Option<HostEnvelope>,
}

/// The `context` member: a [`CheckContext`] field for field. Every member is
/// required except `catalog`, which is absent for catalog 1.0; `tool` may be
/// `null`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextEnvelope {
    /// `reply`, `show_ui`, `tool_row` or `tool_page`.
    profile: Profile,
    /// `"1.0"` or `"1.1"`, the catalog a tool template names.
    #[serde(default)]
    catalog: CatalogVersion,
    execution_dialect: String,
    registry: OperationRegistry,
    #[serde(deserialize_with = "nullable")]
    tool: Option<ToolInputs>,
    show_ui: ShowUiArguments,
    references: Indexed<ReferenceRecord>,
    handles: Vec<HandleRecord>,
}

/// The `host` member: a [`HostValues`] field for field. Every member is
/// required; `data` may be `null`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostEnvelope {
    network: bool,
    /// Milliseconds since the Unix epoch, as a canonical decimal string.
    now: String,
    zone: String,
    #[serde(deserialize_with = "nullable")]
    data: Option<DataEnvelope>,
    /// Fetched snapshot bytes as base64, keyed by N in `$$tc(N)`.
    snapshots: Indexed<String>,
}

/// A [`DataSnapshot`]: `bytes` is base64 or `null` when not fetched, and is
/// required like every other member.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataEnvelope {
    record_id: String,
    record: ReferenceRecord,
    #[serde(deserialize_with = "nullable")]
    bytes: Option<String>,
}

/// Deserializes an `Option` member that must be present but may be `null`.
/// Without it serde treats a missing `Option` member as `None`.
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

impl CheckEnvelope {
    /// The check context and host values, or `OCC_INVALID_ARGUMENT` when
    /// `now` is not a canonical decimal or some bytes are not canonical
    /// base64.
    pub(crate) fn into_parts(self) -> Result<(CheckContext, HostValues), Failure> {
        let context = self.context;
        let context = CheckContext {
            profile: context.profile,
            catalog: context.catalog,
            execution_dialect: context.execution_dialect,
            registry: context.registry,
            tool: context.tool,
            show_ui: context.show_ui,
            references: context.references.0,
            handles: context.handles,
        };
        let host = match self.host {
            None => HostValues::default(),
            Some(host) => HostValues {
                network: host.network,
                now: decimal::parse(&host.now).ok_or_else(Failure::invalid)?,
                zone: host.zone,
                data: host
                    .data
                    .map(|data| {
                        Ok::<_, Failure>(DataSnapshot {
                            record_id: data.record_id,
                            record: data.record,
                            bytes: data.bytes.as_deref().map(bytes).transpose()?,
                        })
                    })
                    .transpose()?,
                snapshots: host
                    .snapshots
                    .0
                    .into_iter()
                    .map(|(n, text)| Ok((n, bytes(&text)?)))
                    .collect::<Result<_, Failure>>()?,
            },
        };
        Ok((context, host))
    }
}

/// Canonical base64 text as bytes, or `OCC_INVALID_ARGUMENT`.
fn bytes(text: &str) -> Result<Vec<u8>, Failure> {
    base64::decode(text).ok_or_else(Failure::invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use occipital_lang::{Decimal, ReferenceState};

    fn decode(json: &str) -> Result<LayoutContext, Failure> {
        serde_json::from_str::<LayoutEnvelope>(json)
            .map_err(|_| Failure::invalid())
            .and_then(LayoutEnvelope::into_context)
    }

    #[test]
    fn decodes_every_field() {
        let layout = decode(
            r#"{"width":"390.5","content_size_category":"large","network":"unknown","placement":"page"}"#,
        )
        .unwrap();
        assert_eq!(
            layout,
            LayoutContext {
                width: Decimal::from_coefficient(390_500_000_000_000_000_000).unwrap(),
                content_size_category: "large".to_owned(),
                network: Network::Unknown,
                placement: Placement::Page,
            }
        );
        let json = serde_json::to_string(&LayoutEnvelope::from_context(&layout)).unwrap();
        assert_eq!(
            json,
            r#"{"width":"390.5","content_size_category":"large","network":"unknown","placement":"page"}"#
        );
    }

    #[test]
    fn refuses_bad_members() {
        for json in [
            // An unknown member.
            r#"{"width":"1","content_size_category":"large","network":"yes","placement":"row","extra":1}"#,
            // A missing member.
            r#"{"width":"1","content_size_category":"large","network":"yes"}"#,
            // A number where a decimal string belongs.
            r#"{"width":1,"content_size_category":"large","network":"yes","placement":"row"}"#,
            // A decimal string that is not canonical.
            r#"{"width":"1.0","content_size_category":"large","network":"yes","placement":"row"}"#,
            // An enumeration that is not lower case.
            r#"{"width":"1","content_size_category":"large","network":"Yes","placement":"row"}"#,
            // A duplicated member.
            r#"{"width":"1","width":"2","content_size_category":"large","network":"yes","placement":"row"}"#,
        ] {
            assert_eq!(decode(json), Err(Failure::invalid()), "{json}");
        }
    }

    fn event(json: &str) -> Result<ReplyEvent, Failure> {
        serde_json::from_str::<EventEnvelope>(json)
            .map_err(|_| Failure::invalid())
            .and_then(EventEnvelope::into_event)
    }

    #[test]
    fn decodes_each_event_kind() {
        let Ok(ReplyEvent::Snapshot { reference, result }) =
            event(r#"{"kind":"snapshot","reference":7,"result":{"ok":"aGk="}}"#)
        else {
            panic!("a snapshot event");
        };
        assert_eq!((reference, result), (7, Ok(b"hi".to_vec())));
        let Ok(ReplyEvent::Snapshot { reference, result }) =
            event(r#"{"result":"failed","kind":"snapshot","reference":0}"#)
        else {
            panic!("a failed snapshot event");
        };
        assert_eq!((reference, result), (0, Err(())));
        let Ok(ReplyEvent::HandleAvailability {
            record_id,
            available,
        }) = event(r#"{"kind":"handle_availability","record_id":"h1","available":false}"#)
        else {
            panic!("a handle event");
        };
        assert_eq!((record_id.as_str(), available), ("h1", false));
        assert!(matches!(
            event(r#"{"kind":"account_release"}"#),
            Ok(ReplyEvent::AccountRelease)
        ));
        let Ok(ReplyEvent::Layout(layout)) = event(
            r#"{"kind":"layout","layout":{"width":"200","content_size_category":"large","network":"no","placement":"row"}}"#,
        ) else {
            panic!("a layout event");
        };
        assert_eq!(layout.width, Decimal::from_integer(200).unwrap());
        assert_eq!(layout.network, Network::No);
    }

    #[test]
    fn refuses_bad_events() {
        for json in [
            // Measurements are delivered by occ_reply_measure, not sent.
            r#"{"kind":"measure_complete"}"#,
            r#"{"kind":"MeasureComplete"}"#,
            r#"{"kind":"unknown"}"#,
            r#"{"reference":7}"#,
            // Members of another kind, missing members and explicit nulls.
            r#"{"kind":"account_release","reference":7}"#,
            r#"{"kind":"snapshot","reference":7}"#,
            r#"{"kind":"snapshot","reference":null,"result":"failed"}"#,
            r#"{"kind":"handle_availability","record_id":"h1"}"#,
            r#"{"kind":"layout"}"#,
            // An unknown member and a duplicated one.
            r#"{"kind":"account_release","extra":1}"#,
            r#"{"kind":"account_release","kind":"account_release"}"#,
            // Bytes that are not canonical base64, a negative or fractional
            // reference, and an unknown result.
            r#"{"kind":"snapshot","reference":7,"result":{"ok":"aGk"}}"#,
            r#"{"kind":"snapshot","reference":-1,"result":"failed"}"#,
            r#"{"kind":"snapshot","reference":1.5,"result":"failed"}"#,
            r#"{"kind":"snapshot","reference":7,"result":"ok"}"#,
        ] {
            assert!(event(json).is_err(), "{json}");
        }
    }

    #[test]
    fn decodes_records() {
        let records: RecordsEnvelope = serde_json::from_str(
            r#"{"references":{"7":{"state":"resolved","reason":null,"tool":"bash","bytes":20,"call_key":"c"},"12":{"state":"held"}},"handles":[]}"#,
        )
        .unwrap();
        assert_eq!(records.references.0.len(), 2);
        assert_eq!(records.references.0[&7].state, ReferenceState::Resolved);
        assert_eq!(records.references.0[&7].bytes, Some(20));
        assert_eq!(records.references.0[&12].state, ReferenceState::Held);
        for json in [
            r#"{"references":{}}"#,
            r#"{"handles":[]}"#,
            r#"{"references":{},"handles":[],"extra":1}"#,
            r#"{"references":{"07":{"state":"held"}},"handles":[]}"#,
            r#"{"references":{"-1":{"state":"held"}},"handles":[]}"#,
            r#"{"references":{"x":{"state":"held"}},"handles":[]}"#,
            r#"{"references":{"7":{"state":"held"},"7":{"state":"held"}},"handles":[]}"#,
            r#"{"references":{"7":{"state":"held","extra":1}},"handles":[]}"#,
        ] {
            assert!(
                serde_json::from_str::<RecordsEnvelope>(json).is_err(),
                "{json}"
            );
        }
    }
}
