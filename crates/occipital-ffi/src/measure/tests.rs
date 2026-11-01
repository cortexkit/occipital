use std::ffi::c_char;
use std::ptr;

use occipital_lang::{Decimal, Metrics, TextStyle};
use serde_json::Value as Json;

use super::*;
use crate::metrics::tests::{table, Script};
use crate::output::MeasurementJson;
use crate::reply::{occ_reply_finish, occ_reply_free, OccReply};
use crate::status::{OCC_INVALID_ARGUMENT, OCC_OK};
use crate::testing::{c, call, json, new_reply, BLOCKS, LAYOUT, RECORDS, TABLE, TAGGED};
use crate::tree::TreeJson;

/// The library-side twin of the stub callback table: 20 for `l` and `t`,
/// the UTF-8 byte length for `tw`.
struct Fixed;

impl Metrics for Fixed {
    fn l(&self, _: TextStyle) -> Decimal {
        Decimal::from_integer(20).unwrap()
    }
    fn t(&self, _: &str, _: TextStyle, _: Decimal, _: Option<u32>) -> Decimal {
        Decimal::from_integer(20).unwrap()
    }
    fn tw(&self, text: &str, _: TextStyle) -> Decimal {
        Decimal::from_integer(text.len() as i128).unwrap()
    }
}

fn layout() -> LayoutContext {
    serde_json::from_str::<LayoutEnvelope>(LAYOUT)
        .unwrap()
        .into_context()
        .unwrap()
}

/// Finishes `source` through the FFI and returns the tree JSON text from the
/// revision it returned, with the library's own tree for comparison.
fn finished(source: &str) -> (String, Tree) {
    let reply: *mut OccReply = new_reply();
    let records = c(RECORDS);
    // SAFETY: `reply` is live, the source valid for its length and
    // `records` a live C string.
    let (status, out) = call(|out| unsafe {
        occ_reply_finish(reply, source.as_ptr(), source.len(), records.as_ptr(), out)
    });
    assert_eq!(status, OCC_OK, "{out:?}");
    let out = out.unwrap();
    assert_eq!(json(&out)["diagnostics"], Json::Array(vec![]));
    // SAFETY: `reply` is live and no entry is running on it.
    let tree = (*unsafe { &*reply }.session.candidate.as_ref().unwrap().tree).clone();
    // SAFETY: freed exactly once.
    unsafe { occ_reply_free(reply) };
    // The revision JSON holds exactly this tree JSON. It is taken as text:
    // re-encoding a parsed `serde_json::Value` would sort the prop keys.
    let tree_json = serde_json::to_string(&TreeJson(&tree)).unwrap();
    assert!(out.contains(&format!(",\"tree\":{tree_json},")), "{out}");
    (tree_json, tree)
}

/// What the library measures for `tree` with [`Fixed`], as measurement JSON.
fn expected(tree: &Tree) -> Json {
    let revision = MeasureRevision {
        instance_epoch: 0,
        revision: 0,
        state_hash: "",
        tree,
        first: None,
    };
    let measured = measure(&revision, &layout(), &Fixed).unwrap();
    serde_json::to_value(MeasurementJson::new(&measured)).unwrap()
}

fn measure_tree(tree_json: &str, script: &mut Script) -> (i32, Option<String>) {
    let (tree, layout) = (c(tree_json), c(LAYOUT));
    let metrics = table(script);
    // SAFETY: every pointer is live for the call.
    call(|out| unsafe { occ_measure_tree(tree.as_ptr(), layout.as_ptr(), &metrics, out) })
}

fn measure_block(tree_json: &str, path_json: &str, script: &mut Script) -> (i32, Option<String>) {
    let (tree, path, layout) = (c(tree_json), c(path_json), c(LAYOUT));
    let metrics = table(script);
    // SAFETY: every pointer is live for the call.
    call(|out: *mut *mut c_char| unsafe {
        occ_measure_block(tree.as_ptr(), path.as_ptr(), layout.as_ptr(), &metrics, out)
    })
}

/// The path of the first component named `name`, searching blocks and
/// children in order.
fn path_of(tree: &Tree, name: &str) -> Vec<usize> {
    fn search(nodes: &[TreeNode], name: &str, path: &mut Vec<usize>) -> bool {
        for (index, node) in nodes.iter().enumerate() {
            path.push(index);
            let children = match node {
                TreeNode::Component {
                    name: found,
                    children,
                    ..
                } => {
                    if found.as_str() == name {
                        return true;
                    }
                    children
                }
                TreeNode::Markdown { children, .. } => children,
                _ => {
                    path.pop();
                    continue;
                }
            };
            if search(children, name, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = Vec::new();
    assert!(search(&tree.blocks, name, &mut path), "no {name}");
    path
}

#[test]
fn a_tree_round_tripped_through_tree_json_measures_like_the_original() {
    for source in [TAGGED, TABLE, BLOCKS] {
        let (tree_json, tree) = finished(source);
        // The tree JSON decodes to the very tree the session evaluated.
        assert_eq!(
            serde_json::from_str::<TreeDe>(&tree_json).unwrap().0,
            tree,
            "{tree_json}"
        );
        let measured = measure_tree(&tree_json, &mut Script::default());
        assert_eq!(measured.0, OCC_OK, "{measured:?}");
        let measured = json(&measured.1.unwrap());
        assert_eq!(measured, expected(&tree));
        if source == BLOCKS {
            // Only a tree without prose is measured in full.
            assert!(measured["height"]["known"].is_string(), "{measured}");
            assert!(!measured["boxes"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn a_block_measures_as_a_one_block_tree() {
    let (tagged_json, tagged) = finished(TAGGED);
    let (blocks_json, blocks) = finished(BLOCKS);
    let (table_json, tables) = finished(TABLE);
    for (tree_json, tree, name) in [
        // An inline Badge inside a paragraph, and a Callout with props only.
        (&tagged_json, &tagged, "Badge"),
        (&blocks_json, &blocks, "Callout"),
        (&table_json, &tables, "Table"),
    ] {
        let path = path_of(tree, name);
        let node = node_at(tree, &path).unwrap().clone();
        let mut script = Script::default();
        let measured = measure_block(
            tree_json,
            &serde_json::to_string(&path).unwrap(),
            &mut script,
        );
        assert!(!script.styles.is_empty(), "{name} used the callbacks");
        assert_eq!(measured.0, OCC_OK, "{name} at {path:?}: {measured:?}");
        let measured = json(&measured.1.unwrap());
        assert_eq!(measured, expected(&Tree { blocks: vec![node] }), "{name}");
        // The measured block is the component itself, at the root's block 0.
        assert!(
            measured["boxes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|measured| measured["path"] == "/0"),
            "{name}: {measured}"
        );
    }
}

#[test]
fn a_block_path_must_name_a_component() {
    let (tree_json, tree) = finished(TAGGED);
    let badge = path_of(&tree, "Badge");
    let text = {
        // The paragraph holding the Badges starts with text.
        let mut path = badge.clone();
        *path.last_mut().unwrap() = 0;
        assert!(matches!(node_at(&tree, &path), Some(TreeNode::Text(_))));
        path
    };
    let invalid = (OCC_INVALID_ARGUMENT, None);
    for path in [
        "[]".to_owned(),
        // The paragraph itself is Markdown, not a component.
        format!("[{}]", badge[0]),
        serde_json::to_string(&text).unwrap(),
        "[99]".to_owned(),
        format!("[{},99]", badge[0]),
        "[-1]".to_owned(),
        "[0.5]".to_owned(),
        "{}".to_owned(),
        "[\"0\"]".to_owned(),
    ] {
        assert_eq!(
            measure_block(&tree_json, &path, &mut Script::default()),
            invalid,
            "{path}"
        );
    }
}

#[test]
fn an_unusable_metric_is_a_measure_error_with_out_null() {
    let (tree_json, tree) = finished(BLOCKS);
    let callout = serde_json::to_string(&path_of(&tree, "Callout")).unwrap();
    for value in [
        f64::NAN,
        -1.0,
        -0.000_001,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e21,
    ] {
        let script = || Script {
            value: Some(value),
            ..Script::default()
        };
        assert_eq!(
            measure_tree(&tree_json, &mut script()),
            (OCC_MEASURE_ERROR, None),
            "{value:e}"
        );
        assert_eq!(
            measure_block(&tree_json, &callout, &mut script()),
            (OCC_MEASURE_ERROR, None),
            "{value:e}"
        );
    }
}

#[test]
fn bad_trees_and_arguments_are_invalid_with_out_null() {
    let invalid = (OCC_INVALID_ARGUMENT, None);
    for tree_json in [
        "",
        "null",
        "[]",
        r#"{"k":[]}"#,
        r#"{"m":"p","k":[]}"#,
        r#"{"m":"root","k":[],"extra":1}"#,
        r#"{"m":"root","k":[{"t":"a","r":"1"}]}"#,
    ] {
        assert_eq!(
            measure_tree(tree_json, &mut Script::default()),
            invalid,
            "{tree_json}"
        );
    }
    let empty = r#"{"m":"root","k":[]}"#;
    assert_eq!(measure_tree(empty, &mut Script::default()).0, OCC_OK);
    let (tree, layout) = (c(empty), c(LAYOUT));
    let bad_utf8 = std::ffi::CString::new(&b"{\"m\":\"ro\xFFot\",\"k\":[]}"[..]).unwrap();
    let mut script = Script::default();
    let metrics = table(&mut script);
    // SAFETY: every non-NULL pointer is live for the call.
    unsafe {
        assert_eq!(
            call(|out| occ_measure_tree(ptr::null(), layout.as_ptr(), &metrics, out)),
            invalid
        );
        assert_eq!(
            call(|out| occ_measure_tree(bad_utf8.as_ptr(), layout.as_ptr(), &metrics, out)),
            invalid
        );
        assert_eq!(
            call(|out| occ_measure_tree(tree.as_ptr(), ptr::null(), &metrics, out)),
            invalid
        );
        assert_eq!(
            call(|out| occ_measure_tree(tree.as_ptr(), layout.as_ptr(), ptr::null(), out)),
            invalid
        );
        assert_eq!(
            occ_measure_tree(tree.as_ptr(), layout.as_ptr(), &metrics, ptr::null_mut()),
            OCC_INVALID_ARGUMENT
        );
        assert_eq!(
            call(|out| occ_measure_block(
                tree.as_ptr(),
                ptr::null(),
                layout.as_ptr(),
                &metrics,
                out
            )),
            invalid
        );
    }
}
