use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical;

const CATALOG_BYTES: &[u8] = include_bytes!("../catalog/catalog-v1.json");

/// What tool templates add to catalog 1.0 (catalog-tool-v1.1.json): new
/// catalog metadata, new components and their profile lists. It never changes
/// or removes anything 1.0 defines.
const TOOL_OVERLAY_BYTES: &[u8] = include_bytes!("../catalog/catalog-tool-v1.1.json");

/// Component catalog v1, embedded in the crate (catalog-v1.json). Checking,
/// evaluation and geometry all use this one catalog; a host never supplies
/// its own. Hosts use it to name the catalog a reply session runs with and
/// to read the catalog fields of the §13 version descriptors.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Catalog {
    #[serde(rename = "$schema")]
    schema: String,
    catalog: Metadata,
    pub(crate) vocabulary: Vocabulary,
    profiles: Vec<String>,
    capabilities: BTreeMap<String, String>,
    types: BTreeMap<String, TypeDescription>,
    text_styles: BTreeMap<String, String>,
    layouts: BTreeMap<String, Value>,
    geometry: BTreeMap<String, Value>,
    states: BTreeMap<String, String>,
    accounts: BTreeMap<String, Value>,
    teaching: BTreeMap<String, Value>,
    pub(crate) components: BTreeMap<String, Component>,
    host: BTreeMap<String, Value>,
    profiles_allow: BTreeMap<String, Value>,
    open: Vec<OpenItem>,
    deferred: Vec<String>,
    admission: BTreeMap<String, Value>,
}

/// Catalog 1.0's earlier digests, oldest first. Its header and a few prose
/// lines once named the team that drafted it and the apps' product name;
/// they were rewritten without changing what any component, prop, rule or
/// layout means, so these digests name the same catalog. The first is the
/// digest replies are stamped with ([`crate::reply_stamp_digest`]).
pub(crate) const V1_ALIASES: &[&str] =
    &["beb0904e8b402916d84d7609a8b06db30ac308f692dcc048db2dc1138cd0c986"];

impl Catalog {
    /// The embedded component catalog v1, parsed once per process.
    pub fn v1() -> &'static Catalog {
        Self::shared()
    }

    /// The catalog version, `1.0` for catalog v1 (§13.3 `catalogs[].version`).
    pub fn version(&self) -> &str {
        &self.catalog.version
    }

    /// The geometry layout version every cached height is keyed by
    /// (§13.3 `catalogs[].layout`, catalog `admission.layoutVersion`).
    pub fn layout_version(&self) -> &str {
        &self.catalog.layout_version
    }

    /// The catalog digest by its `digestRule`: the SHA-256 of the catalog's
    /// §14.3 canonical JSON with `catalog.digest` set to null, in lowercase
    /// hex. It is not the file hash.
    ///
    /// `None` when the catalog has no canonical JSON, so no digest: a
    /// catalog deserialized from other JSON can hold a number outside the
    /// §4.2 range. The embedded catalog always has one.
    pub fn publication_digest(&self) -> Option<String> {
        if std::ptr::eq(self, Self::shared()) {
            return Some(Self::embedded_digest().to_owned());
        }
        self.digest().ok()
    }

    /// Whether this is the embedded catalog v1, compared by digest so an
    /// equal copy is accepted and a modified one is not.
    pub(crate) fn is_embedded(&self) -> bool {
        std::ptr::eq(self, Self::shared())
            || self.digest().ok().as_deref() == Some(Self::embedded_digest())
    }

    /// The part of the catalog's `teaching` section with this name, such as
    /// `reply`, or `None` when the catalog has no such part.
    pub(crate) fn teaching(&self, part: &str) -> Option<&Value> {
        self.teaching.get(part)
    }

    /// The capabilities a value of the named prop type needs, from the
    /// catalog's `types` table: `snapshot` needs `refs`, `handle<K>` needs
    /// `handles`. A type the table describes in prose only needs nothing.
    pub(crate) fn type_needs(&self, name: &str) -> &[String] {
        match self.types.get(name) {
            Some(TypeDescription::Detailed { needs, .. }) => needs,
            _ => &[],
        }
    }

    /// The layout constant at this dotted path, such as `gap.m`, or `None`
    /// when the catalog defines no such number.
    pub(crate) fn layout_constant(&self, name: &str) -> Option<crate::Decimal> {
        let mut value = self.layouts.get(&self.catalog.layout_version)?.get("k")?;
        for part in name.split('.') {
            value = value.get(part)?;
        }
        value
            .is_number()
            .then(|| crate::Decimal::from_literal(&value.to_string()).ok())
            .flatten()
    }

    /// The clip limit with this name, or `None` when the catalog defines none.
    pub(crate) fn clip_limit(&self, name: &str) -> Option<usize> {
        self.layouts
            .get(&self.catalog.layout_version)?
            .get("clip")?
            .get(name)?
            .as_u64()
            .map(|limit| limit as usize)
    }

    pub(crate) fn load() -> Result<Self, serde_json::Error> {
        serde_json::from_slice(CATALOG_BYTES)
    }

    /// The embedded catalog, parsed once per process. Evaluation consults it
    /// for every component it builds, so it must not reparse the JSON each time.
    pub(crate) fn shared() -> &'static Self {
        &Self::embedded().0
    }

    /// The embedded catalog's publication digest, computed once per process.
    pub(crate) fn embedded_digest() -> &'static str {
        &Self::embedded().1
    }

    /// The embedded catalog and its digest, built once per process.
    // The catalog bytes are compiled into the crate, and the catalog tests
    // parse and digest exactly these bytes, so neither step can fail at run
    // time; a failure would be a broken build, caught before release. This
    // and `tool_embedded` are the places a catalog defect would stop the
    // library, and they return a plain reference every caller can use
    // without a check.
    #[allow(clippy::expect_used)]
    fn embedded() -> &'static (Catalog, String) {
        static EMBEDDED: std::sync::OnceLock<(Catalog, String)> = std::sync::OnceLock::new();
        EMBEDDED.get_or_init(|| {
            let catalog = Self::load().expect("the embedded catalog is pinned and tested");
            let digest = catalog
                .digest()
                .expect("the embedded catalog has canonical JSON");
            (catalog, digest)
        })
    }

    /// The tool catalog, 1.1: catalog 1.0 with the components only tool rows
    /// and pages use, such as `Slot`. A tool template names this catalog.
    /// Replies and `show_ui` stay on 1.0 and keep its digest, so an app that
    /// predates 1.1 still draws them. Every 1.0 component is in it unchanged,
    /// so evaluation and geometry look components up here for every profile.
    pub fn tool_templates() -> &'static Catalog {
        &Self::tool_embedded().0
    }

    /// The tool catalog's publication digest, by the same `digestRule`.
    pub fn tool_templates_digest() -> &'static str {
        &Self::tool_embedded().1
    }

    /// Catalog 1.0 with the tool overlay applied.
    pub(crate) fn load_tool() -> Result<Self, serde_json::Error> {
        let mut document: Value = serde_json::from_slice(CATALOG_BYTES)?;
        let overlay: Value = serde_json::from_slice(TOOL_OVERLAY_BYTES)?;
        overlaid(&mut document, overlay).map_err(serde::de::Error::custom)?;
        serde_json::from_value(document)
    }

    #[allow(clippy::expect_used)]
    fn tool_embedded() -> &'static (Catalog, String) {
        static EMBEDDED: std::sync::OnceLock<(Catalog, String)> = std::sync::OnceLock::new();
        EMBEDDED.get_or_init(|| {
            let catalog = Self::load_tool().expect("the tool catalog is pinned and tested");
            let digest = catalog
                .digest()
                .expect("the tool catalog has canonical JSON");
            (catalog, digest)
        })
    }

    /// Lowercase hex publication digest, not the file hash.
    /// Used by layout keys and stored program/app catalog descriptors (§13).
    pub(crate) fn digest(&self) -> Result<String, serde_json::Error> {
        let mut document = serde_json::to_value(self)?;
        // digestRule excludes the published digest itself, even after publication.
        document["catalog"]["digest"] = Value::Null;
        let bytes = canonical::json(&document)?;
        let digest = hmac_sha256::Hash::hash(bytes.as_bytes());
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

/// Apply a catalog overlay: its `catalog` members replace the document's, its
/// `components` are added and never replace one, and its `profilesAllow` lists
/// extend the document's. Anything else is refused, so an overlay can't
/// change what catalog 1.0 defines. `tools/catalog_digest.py --overlay`
/// implements the same rules independently, for the pinned digest.
fn overlaid(document: &mut Value, overlay: Value) -> Result<(), String> {
    let Value::Object(overlay) = overlay else {
        return Err("the overlay is not an object".into());
    };
    for (key, value) in overlay {
        match (key.as_str(), value) {
            ("catalog", Value::Object(members)) => {
                for (member, value) in members {
                    document["catalog"][member] = value;
                }
            }
            ("components", Value::Object(components)) => {
                for (name, component) in components {
                    if document["components"].get(&name).is_some() {
                        return Err(format!("the overlay replaces component {name}"));
                    }
                    document["components"][name] = component;
                }
            }
            ("profilesAllow", Value::Object(lists)) => {
                for (profile, names) in lists {
                    let Some(listed) = document["profilesAllow"]
                        .get_mut(&profile)
                        .and_then(Value::as_array_mut)
                    else {
                        return Err(format!("catalog 1.0 has no profilesAllow list {profile}"));
                    };
                    for name in names.as_array().into_iter().flatten() {
                        if listed.contains(name) {
                            return Err(format!("{name} is already allowed in {profile}"));
                        }
                        listed.push(name.clone());
                    }
                }
            }
            (key, _) => return Err(format!("unknown overlay member {key}")),
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Metadata {
    id: String,
    version: String,
    layout_version: String,
    dialect: String,
    status: String,
    date: String,
    owner: String,
    companion: String,
    digest: Option<String>,
    digest_rule: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Vocabulary {
    pub(crate) tone: Vec<String>,
    pub(crate) tone_text: Vec<String>,
    appearance: BTreeMap<String, Vec<String>>,
    pub(crate) aspect: BTreeMap<String, serde_json::Number>,
    pub(crate) gap: BTreeMap<String, u32>,
    rule: String,
    retired: BTreeMap<String, String>,
    retired_use: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum TypeDescription {
    Text(String),
    Detailed {
        means: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        needs: Vec<String>,
        late: bool,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Component {
    pub(crate) sum: String,
    #[serde(rename = "in")]
    pub(crate) profiles: Vec<String>,
    pub(crate) flow: String,
    pub(crate) kids: Children,
    pub(crate) props: BTreeMap<String, Prop>,
    // Formula and component-specific rules remain data until their evaluators are added.
    #[serde(flatten)]
    pub(crate) rules: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Children {
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) only: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) items: Option<[u32; 2]>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Prop {
    pub(crate) t: String,
    // Defaults and constraints have different shapes for different prop types.
    #[serde(flatten)]
    pub(crate) rules: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OpenItem {
    item: String,
    owner: String,
}

#[cfg(test)]
mod tests {
    use hmac_sha256::Hash;

    use super::{Catalog, CATALOG_BYTES};

    #[test]
    fn committed_catalog_file_sha256_is_pinned() {
        // Pin the exact file bytes, not the publication digest of canonical JSON.
        let digest = Hash::hash(CATALOG_BYTES);
        let hex_digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex_digest,
            "f36ec15e7fc7ed69bbcedbddbdee057acb30d006cf2e0cac1c19011bdc07a3ea"
        );
    }

    #[test]
    fn catalog_digest_matches_independent_literal() {
        let catalog = Catalog::load().unwrap();
        // Generated by the Python tool, never by this crate; see tools/README.md.
        let expected = include_str!("../catalog/catalog-v1.digest").trim_end();
        assert_eq!(catalog.digest().unwrap(), expected);
    }

    #[test]
    fn catalog_digest_ignores_published_digest() {
        let mut catalog = Catalog::load().unwrap();
        let unpublished = catalog.digest().unwrap();
        catalog.catalog.digest = Some("a previously published digest".into());
        assert_eq!(catalog.digest().unwrap(), unpublished);
    }

    #[test]
    fn catalog_digest_covers_document_content() {
        let mut catalog = Catalog::load().unwrap();
        let original = catalog.digest().unwrap();
        catalog.catalog.layout_version = "2".into();
        assert_ne!(catalog.digest().unwrap(), original);
        catalog = Catalog::load().unwrap();
        catalog.components.get_mut("Badge").unwrap().sum.push('!');
        assert_ne!(catalog.digest().unwrap(), original);
    }

    #[test]
    fn publication_digest_is_none_for_a_catalog_without_canonical_json() {
        let expected = include_str!("../catalog/catalog-v1.digest").trim_end();
        assert_eq!(
            Catalog::v1().publication_digest().as_deref(),
            Some(expected)
        );
        let copy = Catalog::load().unwrap();
        assert_eq!(copy.publication_digest().as_deref(), Some(expected));

        // A host can deserialize its own catalog JSON. A number outside the
        // §4.2 range has no canonical form, so there is no digest, and asking
        // for one must not panic.
        let mut json: serde_json::Value = serde_json::from_slice(CATALOG_BYTES).unwrap();
        json["layouts"]["1"]["huge"] = serde_json::from_str("1e400").unwrap();
        let catalog = serde_json::from_value::<Catalog>(json).unwrap();
        assert_eq!(catalog.publication_digest(), None);
        assert!(!catalog.is_embedded());
    }

    #[test]
    fn committed_catalog_loads_with_v1_identity() {
        let catalog = Catalog::load().expect("the committed catalog must deserialize");
        assert_eq!(catalog.schema, "occipital.catalog/1");
        assert_eq!(catalog.catalog.id, "occipital.catalog");
        assert_eq!(catalog.catalog.version, "1.0");
        assert_eq!(catalog.catalog.layout_version, "1");
        assert_eq!(catalog.catalog.dialect, "1.0");
        assert_eq!(catalog.catalog.digest, None);
        assert_eq!(
            catalog.profiles,
            ["reply", "show_ui", "tool_row", "tool_page"]
        );
        assert_eq!(catalog.components["Badge"].props["tone"].t, "tone");
    }

    #[test]
    fn typed_catalog_preserves_the_complete_document() {
        let catalog = Catalog::load().expect("the committed catalog must deserialize");
        let original: serde_json::Value = serde_json::from_slice(CATALOG_BYTES).unwrap();
        assert_eq!(serde_json::to_value(catalog).unwrap(), original);
    }

    #[test]
    fn tool_catalog_digest_matches_independent_literal() {
        // Generated by the Python tool with --overlay; see tools/README.md.
        let expected = include_str!("../catalog/catalog-tool-v1.1.digest").trim_end();
        assert_eq!(Catalog::load_tool().unwrap().digest().unwrap(), expected);
        assert_eq!(Catalog::tool_templates_digest(), expected);
        assert_ne!(expected, Catalog::embedded_digest());
    }

    /// The tool catalog only adds: every 1.0 component, layout and rule is in
    /// it unchanged, so a reply or show_ui tree means the same under either.
    #[test]
    fn tool_catalog_is_catalog_1_0_plus_its_own_components() {
        let base = serde_json::to_value(Catalog::v1()).unwrap();
        let tool = serde_json::to_value(Catalog::tool_templates()).unwrap();
        assert_eq!(Catalog::tool_templates().version(), "1.1");
        for (key, value) in base.as_object().unwrap() {
            match key.as_str() {
                "catalog" => {
                    assert_eq!(tool[key]["layoutVersion"], value["layoutVersion"]);
                    assert_eq!(tool[key]["dialect"], value["dialect"]);
                }
                "components" => {
                    for (name, component) in value.as_object().unwrap() {
                        assert_eq!(&tool[key][name], component, "{name}");
                    }
                }
                "profilesAllow" => {
                    for (profile, listed) in value.as_object().unwrap() {
                        let extended = &tool[key][profile];
                        match listed.as_array() {
                            Some(names) => assert!(names
                                .iter()
                                .all(|name| extended.as_array().unwrap().contains(name))),
                            None => assert_eq!(extended, listed, "{profile}"),
                        }
                    }
                }
                _ => assert_eq!(&tool[key], value, "{key}"),
            }
        }
        // Each tool component is allowed exactly where its own `in` says.
        let added: Vec<_> = Catalog::tool_templates()
            .components
            .iter()
            .filter(|(name, _)| !Catalog::v1().components.contains_key(*name))
            .collect();
        assert!(!added.is_empty());
        for (name, component) in added {
            for profile in ["reply", "show_ui", "tool_row", "tool_page"] {
                let allowed = tool["profilesAllow"][profile]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!(name));
                assert_eq!(
                    allowed,
                    component.profiles.iter().any(|p| p == profile),
                    "{name} {profile}"
                );
            }
        }
    }

    #[test]
    fn an_overlay_never_replaces_what_catalog_1_0_defines() {
        for overlay in [
            serde_json::json!({"components": {"Badge": {}}}),
            serde_json::json!({"profilesAllow": {"tool_row": ["Badge"]}}),
            serde_json::json!({"types": {}}),
        ] {
            let mut document: serde_json::Value = serde_json::from_slice(CATALOG_BYTES).unwrap();
            assert!(
                super::overlaid(&mut document, overlay.clone()).is_err(),
                "{overlay}"
            );
        }
    }

    #[test]
    fn typed_catalog_rejects_wrong_field_types() {
        let original: serde_json::Value = serde_json::from_slice(CATALOG_BYTES).unwrap();
        serde_json::from_value::<Catalog>(original.clone())
            .expect("the unmodified catalog must be valid before testing malformed fields");
        for pointer in [
            "/catalog/version",
            "/profiles",
            "/vocabulary/tone",
            "/components/Badge/props/tone/t",
        ] {
            let mut malformed = original.clone();
            *malformed.pointer_mut(pointer).unwrap() = serde_json::json!(42);
            assert!(
                serde_json::from_value::<Catalog>(malformed).is_err(),
                "the catalog field at {pointer} must retain its declared type"
            );
        }
    }
}
