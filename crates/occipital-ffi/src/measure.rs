//! Measuring trees outside a session: `occ_measure_tree` and
//! `occ_measure_block`.

use std::ffi::c_char;

use occipital_lang::{measure, LayoutContext, MeasureRevision, Tree, TreeNode};

use crate::boundary::{json_arg, string_entry, Failure};
use crate::envelope::LayoutEnvelope;
use crate::metrics::{Adapter, OccMetrics};
use crate::output::{to_json, MeasurementJson};
use crate::status::OCC_MEASURE_ERROR;
use crate::tree::{node_at, TreeDe};

/// Measures `tree` and encodes the measurement. A tree measured outside a
/// session has no instance, revision or evaluated state, so its layout key
/// carries instance epoch 0, revision 0 and an empty state hash.
fn measure_json(tree: &Tree, layout: &LayoutContext, metrics: &Adapter) -> Result<String, Failure> {
    let revision = MeasureRevision {
        instance_epoch: 0,
        revision: 0,
        state_hash: "",
        tree,
        first: None,
    };
    match measure(&revision, layout, metrics) {
        Ok(measured) if !metrics.failed() => to_json(&MeasurementJson::new(&measured)),
        _ => Err(Failure::bare(OCC_MEASURE_ERROR)),
    }
}

/// Measures the tree in `tree_json` (tree JSON) under `layout_json` with
/// `metrics`. `*out` is the measurement JSON. A metrics callback returning
/// an unusable value, or a tree the library cannot measure, is
/// `OCC_MEASURE_ERROR`.
///
/// # Safety
///
/// `tree_json` and `layout_json` must be NULL or NUL-terminated strings,
/// `metrics` NULL or a valid table whose callbacks never call back into this
/// library, and `out` NULL or valid for writing one pointer, all for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn occ_measure_tree(
    tree_json: *const c_char,
    layout_json: *const c_char,
    metrics: *const OccMetrics,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `json_arg` and
        // `Adapter::new`.
        let (tree, layout, metrics) = unsafe {
            (
                json_arg::<TreeDe>(tree_json)?,
                json_arg::<LayoutEnvelope>(layout_json)?,
                Adapter::new(metrics)?,
            )
        };
        measure_json(&tree.0, &layout.into_context()?, &metrics)
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

/// Measures one component node of `tree_json` as a tree whose only block is
/// that node. `path_json` is a non-empty JSON array of child indices: the
/// first picks one of the root's blocks, each later one a child of the node
/// before. A path that names no node, or names a node that is not a
/// component, is `OCC_INVALID_ARGUMENT`. Otherwise as [`occ_measure_tree`].
///
/// # Safety
///
/// As [`occ_measure_tree`], and `path_json` must be NULL or a
/// NUL-terminated string valid for the call.
#[no_mangle]
pub unsafe extern "C" fn occ_measure_block(
    tree_json: *const c_char,
    path_json: *const c_char,
    layout_json: *const c_char,
    metrics: *const OccMetrics,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `json_arg` and
        // `Adapter::new`.
        let (tree, path, layout, metrics) = unsafe {
            (
                json_arg::<TreeDe>(tree_json)?,
                json_arg::<Vec<usize>>(path_json)?,
                json_arg::<LayoutEnvelope>(layout_json)?,
                Adapter::new(metrics)?,
            )
        };
        let node = match node_at(&tree.0, &path) {
            Some(node @ TreeNode::Component { .. }) => node.clone(),
            _ => return Err(Failure::invalid()),
        };
        let block = Tree { blocks: vec![node] };
        measure_json(&block, &layout.into_context()?, &metrics)
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

#[cfg(test)]
mod tests;
