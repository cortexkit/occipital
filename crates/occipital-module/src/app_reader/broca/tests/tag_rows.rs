//! Broca's `session.read` entries now carry a `tags` member: one row per part of
//! the entry's `content`, in stored order, each naming its part with the runner's
//! key (`"m3#0"`) and carrying the tag only when that part was numbered. Samples
//! recorded from Broca's own serve path replay here through every handler that
//! decodes an entry: the newest page, a claim, a watch's first notice and a full
//! message read, with `session.details` alongside them for the session facts the
//! apps show. The rows the phone sees must not change.
//!
//! The samples also cover parts that are never numbered and sessions with no tags
//! to show at all, which now still send `tags`: a reader that refused a row without
//! `tag`, or an entry without `tags`, would decode neither.
//!
//! Neither recorded session has a `session.head` sample, so the tests build a
//! contract-valid head around the entries: the ordinal range they hold, their
//! lineage and history counter, and no send grant.

use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::{
    resolve::Entorhinal, AppReader, DETAILS, TRANSCRIPT_PAGE, TRANSCRIPT_WATCH,
};
use occipital_app_reader::broca_entries::{PART_INDEX, PART_KEY};
use std::path::{Path, PathBuf};

const GOLDENS: &str = "tests/data/broca-tag-rows";
/// Every sample Broca recorded from its serve path. The first three are read replies
/// on their own; the last two Broca wrapped in `{"result": …}` and stamped with a
/// lineage, as some of its read routes serve them. A sample is tested byte for byte
/// as Broca sent it, so the wrapper stays in the file and is unwrapped on the way in.
const FILES: &[&str] = &[
    "session_read.counted.json",
    "session_read.tagged_open_tool_turn.json",
    "session_read.two_episode.descriptors.json",
    "session_read.two_episode.json",
    "session_read.written_at.json",
];
const LINEAGE: &str = "00000000-0000-4000-8000-0000000000a1";
const INCARNATION: &str = "00000000-0000-4000-8000-0000000000a2";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDENS)
}

#[test]
fn broca_tag_row_checksums_match_the_supplied_bytes() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    let mut listed = Vec::new();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        assert!(FILES.contains(&name), "unexpected checksum entry: {name}");
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        listed.push(name);
    }
    listed.sort_unstable();
    let mut expected = FILES.to_vec();
    expected.sort_unstable();
    assert_eq!(listed, expected, "every sample has one checksum");
    let mut on_disk: Vec<_> = std::fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".json"))
        .collect();
    on_disk.sort();
    assert_eq!(on_disk, expected, "every JSON file is tested");
}

/// The read reply a sample holds, unwrapped from the `{"result": …}` envelope when
/// Broca sent one.
fn sample(name: &str) -> Value {
    let value: Value = serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap();
    match value {
        Value::Object(object) if object.contains_key("result") => object["result"].clone(),
        value => value,
    }
}

/// The sample's entries in ordinal order.
fn entries(name: &str) -> Vec<Value> {
    sample(name)["messages"].as_array().unwrap().clone()
}

/// The row keys of one entry's `tags`, member by member and in order, read against
/// the stored block each key names. Fails when a part has no row, a row names no
/// part of the entry, or a key is not the runner's `mid#index`.
fn tag_rows(entry: &Value) -> Vec<String> {
    let mid = entry["mid"].as_str().unwrap();
    let blocks = entry["message"]["content"].as_array().unwrap();
    let rows = entry["tags"]
        .as_array()
        .unwrap_or_else(|| panic!("{mid} has no tags member: {entry}"));
    let mut keys = Vec::new();
    for row in rows {
        let part = row["part"]
            .as_str()
            .unwrap_or_else(|| panic!("{mid} has a row without a part: {row}"));
        let (row_mid, index) = part
            .split_once('#')
            .unwrap_or_else(|| panic!("{mid} row names no part: {part}"));
        assert_eq!(row_mid, mid, "row names another entry's part: {part}");
        let index: usize = index
            .parse()
            .unwrap_or_else(|_| panic!("{mid} row has a bad index: {part}"));
        assert!(
            index < blocks.len(),
            "{mid} row {part} names no stored part of {}",
            blocks.len()
        );
        if let Some(tag) = row.get("tag") {
            let tag = tag
                .as_str()
                .unwrap_or_else(|| panic!("{mid} row {part} has a non-string tag: {row}"));
            assert!(
                tag.len() > 2 && tag.starts_with('§') && tag.ends_with('§'),
                "{mid} row {part} has a malformed tag: {tag}"
            );
        }
        keys.push(part.to_string());
    }
    keys
}

/// Every sample's rows key each stored part exactly once, in order. This test decodes
/// nothing else, so refusing a row without `tag` turns it red.
#[test]
fn every_sample_names_each_stored_part_once_and_in_order() {
    for name in FILES {
        for entry in entries(name) {
            let mid = entry["mid"].as_str().unwrap();
            let blocks = entry["message"]["content"].as_array().unwrap().len();
            let expected: Vec<String> = (0..blocks).map(|at| format!("{mid}#{at}")).collect();
            assert_eq!(tag_rows(&entry), expected, "{name} {mid}");
        }
    }
}

/// A route that answers every `session.read` with `entries`, as Broca serves the
/// whole session up to its head, and builds the head from them.
struct TagRoute {
    entries: Vec<Value>,
    describe: Value,
}

struct QuietEvents;

#[async_trait]
impl EventStream for QuietEvents {
    async fn next(&mut self) -> Result<Option<Value>, ReadError> {
        std::future::pending().await
    }
}

#[async_trait]
impl BrocaRoute for TagRoute {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let request: Value = serde_json::from_slice(&body(op, params)).unwrap();
        let method = request["method"].as_str().unwrap();
        let reply = match method {
            "session.head" => json!({"result":{
                "first_ordinal":0,
                "last_ordinal":self.entries.len() - 1,
                "history_version":0,
                "lineage_id":LINEAGE,
                "incarnation":INCARNATION,
                "updated_at":506,
                "last_run_state":{"run_id":"run-latest", "state":"completed"},
                "input":{"attachments":"no", "mime_types":[], "modalities":[]}}}),
            "session.read" => json!({"result":{
                "messages":self.entries,
                "history_version":0,
                "lineage_id":LINEAGE}}),
            "session.history" => raw_history(
                &json!({"messages":self.entries, "history_version":0, "lineage_id":LINEAGE}),
                &request["params"],
            )?,
            "role.describe" => self.describe.clone(),
            _ => panic!("a tag-row read unexpectedly called {method}"),
        };
        // Match DaemonRoute::read's JSON decoding and result-envelope handling.
        result(serde_json::from_slice(&serde_json::to_vec(&reply).unwrap()).unwrap())
    }

    async fn subscribe(&self, _live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        Ok(Box::new(QuietEvents))
    }
}

struct TagRoutes(Arc<TagRoute>);

#[async_trait]
impl BrocaRoutes for TagRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// A lookup with no network history, so no row's network comes from a write time.
struct TagLookup;

#[async_trait]
impl Entorhinal for TagLookup {
    async fn request(&self, _: &str, _: Value) -> Result<Value, ReadError> {
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref", "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner":"broca", "session":"alfonso:s1", "harness":"broca",
            "project_root":"/workspace/project", "network":"no", "network_history":[]}}),
        )
    }
}

/// A reader over `entries` with a `role.describe` that answers and declares the
/// `head_sender` group, so a head without `sender` reads as no grant rather than as
/// unknown.
fn reader(entries: &[Value]) -> Arc<AppReader> {
    let describe = serde_json::to_value(
        RoleDescribe::new(vec![], "tag-rows")
            .with_capabilities(vec![
                cortexkit_role_llm_runner::capabilities::HEAD_SENDER.to_string()
            ])
            .with_session_capabilities_from("baseline"),
    )
    .unwrap();
    let route = Arc::new(TagRoute {
        entries: entries.to_vec(),
        describe,
    });
    Arc::new(AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(TagLookup))),
        Arc::new(TagRoutes(route)),
    ))))
}

fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

/// A page handler's reply, unwrapped from the route's `{"result":…}` envelope the
/// way a device read receives it.
async fn page(app: &Arc<AppReader>, params: &Value) -> Value {
    let mut reply = parse(&serve(app, TRANSCRIPT_PAGE, params).await);
    reply.as_object_mut().unwrap().remove("result").unwrap()
}

async fn details(app: &AppReader) -> Value {
    app.read(DETAILS, json!({"session":"alfonso:s1", "harness":"broca"}))
        .await
        .unwrap()
}

fn params() -> Value {
    json!({"session":"alfonso:s1", "harness":"broca", "limit":100})
}

/// A claim as a page two messages behind the newest sends: its floor and since ids
/// are messages the newest page already holds, so it matches and carries `newTurns`.
fn claim(base: &Value, held: Value, entries: &[Value]) -> Value {
    let mut request = base.clone();
    request["claim"] = json!({"heldFloorMessageId":entries[0]["mid"],
        "sinceMessageId":entries[1]["mid"], "heldVersion":held});
    request
}

/// The served rows of a reply: a page's `messages`, a claim's `newTurns` or a watch
/// notice's `newTurns`.
fn rows(reply: &Value) -> Vec<Value> {
    let body = reply.get("result").unwrap_or(reply);
    body["messages"]
        .as_array()
        .or_else(|| body["newTurns"].as_array())
        .unwrap_or_else(|| panic!("no rows in {reply}"))
        .clone()
}

/// Drop every sample's `tags` member, keeping every other member and its order.
fn without_tags(entries: &[Value]) -> Vec<Value> {
    let mut entries = entries.to_vec();
    for entry in &mut entries {
        entry.as_object_mut().unwrap().remove("tags");
    }
    entries
}

/// Every row keeps `partKey` and `partIndex` at the stored block's position, and
/// that position is one the sample's own rows name.
fn assert_rows_match_tags(surface: &str, rows: &[Value], entries: &[Value]) {
    for row in rows {
        let mid = row["messageId"].as_str().unwrap();
        let Some(entry) = entries.iter().find(|entry| entry["mid"] == mid) else {
            continue;
        };
        let keys = tag_rows(entry);
        for part in row["parts"].as_array().unwrap() {
            let key = part[PART_KEY]
                .as_str()
                .unwrap_or_else(|| panic!("{surface} {mid} part without a key: {part}"));
            let index = part[PART_INDEX]
                .as_u64()
                .unwrap_or_else(|| panic!("{surface} {mid} part without an index: {part}"));
            assert_eq!(key, index.to_string(), "{surface} {mid}: {part}");
            assert!(
                keys.contains(&format!("{mid}#{index}")),
                "{surface} {mid}: part {key} names no row of {}",
                keys.len()
            );
        }
    }
}

/// The samples by file name, each with its entries.
fn sessions() -> Vec<(&'static str, Vec<Value>)> {
    FILES.iter().map(|name| (*name, entries(name))).collect()
}

/// A full read of an entry whose parts must decode in the full form: the tag-rendering
/// sample's assistant message holds a text part and an open call, and its last message
/// is a lone open call.
fn full_of(entries: &[Value]) -> Value {
    let ordinal = entries
        .iter()
        .find(|entry| entry["message"]["role"] == "assistant")
        .and_then(|entry| entry["ordinal"].as_u64())
        .expect("the sample holds an assistant message");
    json!({"session":"alfonso:s1", "harness":"broca", "messageOrdinal":ordinal})
}

#[tokio::test]
async fn every_sample_decodes_on_every_handler() {
    for (name, entries) in sessions() {
        let app = reader(&entries);
        let newest = page(&app, &params()).await;
        assert!(
            newest.get("error").is_none(),
            "{name}: newest page refused: {newest}"
        );
        assert_rows_match_tags(&format!("{name} newest"), &rows(&newest), &entries);

        let behind = claim(&params(), newest["version"].clone(), &entries);
        let claimed = page(&app, &behind).await;
        assert_eq!(claimed["claim"], "matched", "{name}: {claimed}");
        assert_rows_match_tags(&format!("{name} claim"), &rows(&claimed), &entries);

        let notice = parse(&serve(&app, TRANSCRIPT_WATCH, &behind).await);
        assert_rows_match_tags(&format!("{name} watch"), &rows(&notice), &entries);

        let exact = page(&app, &full_of(&entries)).await;
        assert!(
            exact.get("error").is_none(),
            "{name}: full read refused: {exact}"
        );
        assert_rows_match_tags(&format!("{name} full"), &rows(&exact), &entries);

        // Details reads the same head the page did, and no entry. The described
        // session declares `head_sender` and its head holds no `sender`, so the app
        // gets no grant: a reply with neither `sender` nor `senderUnknown`.
        let facts = details(&app).await;
        assert_eq!(facts["incarnation"], INCARNATION, "{name}: {facts}");
        assert!(facts.get("sender").is_none(), "{name}: {facts}");
        assert!(facts.get("senderUnknown").is_none(), "{name}: {facts}");
    }
}

/// The phone's rows, claim answers, watch notices and details are the same whether or
/// not the entries carry `tags`: nothing the readers serve derives a member from
/// them. Only the upstream `session.read` calls see the member. Every recorded entry
/// carries it, so each sample is compared with the same entries minus that member.
#[tokio::test]
async fn the_phone_sees_the_same_session_with_or_without_tag_rows() {
    for (name, entries) in sessions() {
        assert!(
            entries.iter().all(|entry| entry.get("tags").is_some()),
            "{name}: every recorded entry carries `tags`"
        );
        let other = without_tags(&entries);
        let tagged = reader(&entries);
        let untagged = reader(&other);

        let newest = page(&tagged, &params()).await;
        let plain = page(&untagged, &params()).await;
        // Both are digests of the head, which only `session.read` saw the tags through.
        assert_eq!(newest["version"], plain["version"], "{name}");
        assert_eq!(newest["activityToken"], plain["activityToken"], "{name}");
        assert_eq!(newest["messages"], plain["messages"], "{name} newest");

        let behind = claim(&params(), newest["version"].clone(), &entries);
        assert_eq!(
            page(&tagged, &behind).await,
            page(&untagged, &behind).await,
            "{name} claim"
        );
        assert_eq!(
            parse(&serve(&tagged, TRANSCRIPT_WATCH, &behind).await),
            parse(&serve(&untagged, TRANSCRIPT_WATCH, &behind).await),
            "{name} watch"
        );
        assert_eq!(
            page(&tagged, &full_of(&entries)).await,
            page(&untagged, &full_of(&entries)).await,
            "{name} full"
        );
        assert_eq!(
            details(&tagged).await,
            details(&untagged).await,
            "{name} details"
        );
    }
}

/// The adapter's own decode refuses nothing a `session.read` entry carries beyond
/// the display form, the per-part tag rows included: an entry must never fall
/// through to the placeholder row, which would hide a readable message.
#[test]
fn every_sample_decodes_through_the_adapter_to_a_readable_row() {
    for (name, entries) in sessions() {
        let rows = occipital_app_reader::broca_entries::rows(&entries, false)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(rows.len(), entries.len(), "{name}");
        for row in &rows {
            assert!(row.display.get("unreadable").is_none(), "{name}: {row:?}");
        }
        // The full form decodes the same entries with no placeholder either.
        let full = occipital_app_reader::broca_entries::rows(&entries, true)
            .unwrap_or_else(|error| panic!("{name} full: {error:?}"));
        assert_eq!(full.len(), entries.len(), "{name} full");
        for row in &full {
            assert!(
                row.display.get("unreadable").is_none(),
                "{name} full: {row:?}"
            );
        }
    }
}

/// The adapter's own decode, given the entries unwrapped from the route's reply
/// envelope, folds no tag into the display and keeps every part's key.
#[test]
fn the_adapter_decodes_every_sample_without_a_tag_dependent_member() {
    for (name, entries) in sessions() {
        let rows = occipital_app_reader::broca_entries::rows(&entries, false)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        for row in &rows {
            assert!(row.display.get("tags").is_none(), "{name}: {row:?}");
            for part in row.display["parts"].as_array().unwrap() {
                assert!(part[PART_KEY].is_string(), "{name}: {part}");
            }
        }
    }
}
