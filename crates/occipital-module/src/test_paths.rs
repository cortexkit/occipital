//! Project folders for tests' stand-in Entorhinal answers, absolute on every
//! platform. A session's project folder must be absolute, and on Windows a
//! path without a drive, such as `/workspace/head`, isn't.

use serde_json::Value;

/// `unix_path`, which starts with `/`, as an absolute path here: unchanged on
/// Unix, and on the `C:` drive on Windows.
pub(crate) fn absolute(unix_path: &str) -> String {
    if cfg!(windows) {
        format!("C:{unix_path}")
    } else {
        unix_path.to_owned()
    }
}

/// Every `project_root` string in `value`, at any depth, made [`absolute`].
/// For recorded Entorhinal answers, whose files stay byte for byte as they
/// were recorded.
pub(crate) fn localize_roots(value: &mut Value) {
    match value {
        Value::Object(members) => {
            for (name, member) in members.iter_mut() {
                match member {
                    Value::String(root) if name == "project_root" && root.starts_with('/') => {
                        *root = absolute(root);
                    }
                    _ => localize_roots(member),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(localize_roots),
        _ => {}
    }
}
