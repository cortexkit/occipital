//! Support for the spec's §16 examples and their golden traces, compiled for
//! tests and for the `traces` feature (the public golden-trace runner).
//!
//! `fixtures/section16/` holds the example sources, their golden traces and
//! hand-written host inputs in the provisional schema subset (§2.3,
//! §8.3–§8.6). The host inputs exist only so the examples can be checked and
//! driven in tests: a registry entry here grants no production
//! authorization, and only tests and the golden-trace runner read these
//! files. They are embedded at compile time; nothing reads the disk.

// This module runs only on the committed §16 golden fixtures (in tests and
// with the `traces` feature), never on program or reply input. A panic here
// is a failed trace, not a crash on untrusted input, so the library's
// no-panic lints do not apply.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::collections::BTreeMap;
use std::fmt::Write;

use serde_json::Value as Json;

use crate::eval::tree::{MarkdownKind, Tree, TreeNode, TreeValue};
use crate::{OperationRegistry, ToolInputs, Value};

#[cfg(test)]
mod coverage;
pub(crate) mod writes;

/// Read a fixture file at compile time, so a test run never touches the disk.
macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/section16/",
            $name
        ))
    };
}
pub(crate) use fixture;

/// The test-only operation registry the §16.3, §16.4 and §16.7 pages read.
pub(crate) fn registry() -> OperationRegistry {
    serde_json::from_str(fixture!("registry.json")).expect("the registry fixture parses")
}

/// The named `input`/`view` cases for one tool-view example, in name order.
pub(crate) fn hosts(example: &str) -> BTreeMap<String, ToolInputs> {
    let mut all: BTreeMap<String, BTreeMap<String, ToolInputs>> =
        serde_json::from_str(fixture!("hosts.json")).expect("the host fixture parses");
    all.remove(example).expect("the example has host cases")
}

/// A host projection as an evaluation value, converted as the public program
/// session converts `input` and `view`.
pub(crate) fn host_value(json: &Json) -> Value {
    crate::api::json_value(json).expect("fixture host values convert")
}

/// Compare a trace with its committed golden file. Setting
/// `OCCIPITAL_BLESS_GOLDEN` rewrites the file instead; a rewritten golden is
/// a new recording and has to be reviewed before it is committed.
#[cfg(test)]
pub(crate) fn golden(name: &str, expected: &str, actual: &str) {
    if std::env::var_os("OCCIPITAL_BLESS_GOLDEN").is_some() {
        let path = format!(
            "{}/fixtures/section16/golden/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::write(path, actual).expect("the golden directory is writable");
        return;
    }
    assert!(
        actual == expected,
        "golden trace {name} differs from the committed file; actual trace:\n{actual}"
    );
}

/// The SHA-256 of a tree's debug form. Traces print each distinct tree once
/// and then refer to it by this digest.
pub(crate) fn tree_digest(tree: &Tree) -> String {
    hmac_sha256::Hash::hash(format!("{tree:?}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Append a tree to a trace: its full definition the first time a digest
/// appears, then the digest and its visible lines (see `visible`).
pub(crate) fn write_tree(trace: &mut String, tree: &Tree) {
    let digest = tree_digest(tree);
    if !trace.contains(&format!("tree-definition={digest} ")) {
        writeln!(trace, "tree-definition={digest} {tree:?}").unwrap();
    }
    writeln!(trace, "tree-sha256={digest}").unwrap();
    for line in visible(tree) {
        writeln!(trace, "  | {line}").unwrap();
    }
}

/// What a reader of the tree sees, as text: one line per block, with tables
/// one line per row and components written as tags with their props. This is
/// a test rendering for assertions, not a §11 fallback text.
pub(crate) fn visible(tree: &Tree) -> Vec<String> {
    let mut lines = vec![];
    for node in &tree.blocks {
        block(node, &mut lines);
    }
    lines
}

/// The visible lines joined, for substring assertions.
pub(crate) fn visible_text(tree: &Tree) -> String {
    visible(tree).join("\n")
}

fn block(node: &TreeNode, lines: &mut Vec<String>) {
    match node {
        TreeNode::Markdown {
            kind: MarkdownKind::Table,
            children,
        } => rows(children, lines),
        TreeNode::Markdown {
            kind: MarkdownKind::Heading(level),
            children,
        } => lines.push(format!(
            "{} {}",
            "#".repeat(usize::from(*level)),
            inline(children)
        )),
        TreeNode::Markdown {
            kind: MarkdownKind::List { .. } | MarkdownKind::Quote,
            children,
        } => {
            for child in children {
                block(child, lines);
            }
        }
        other => lines.push(inline(std::slice::from_ref(other))),
    }
}

fn rows(children: &[TreeNode], lines: &mut Vec<String>) {
    for child in children {
        match child {
            TreeNode::Markdown {
                kind: MarkdownKind::TableRow,
                children,
            } => lines.push(row(children)),
            TreeNode::Markdown {
                kind: MarkdownKind::TableHead,
                children,
            } if children.iter().all(|cell| {
                matches!(
                    cell,
                    TreeNode::Markdown {
                        kind: MarkdownKind::TableCell,
                        ..
                    }
                )
            }) =>
            {
                lines.push(row(children))
            }
            TreeNode::Markdown { children, .. } => rows(children, lines),
            other => lines.push(inline(std::slice::from_ref(other))),
        }
    }
}

fn row(cells: &[TreeNode]) -> String {
    let cells: Vec<String> = cells
        .iter()
        .map(|cell| match cell {
            TreeNode::Markdown { children, .. } => inline(children),
            other => inline(std::slice::from_ref(other)),
        })
        .collect();
    format!("| {} |", cells.join(" | "))
}

fn inline(nodes: &[TreeNode]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            TreeNode::Text(text) => out.push_str(text.as_str()),
            TreeNode::Reference(text) => write!(out, "[ref {}]", text.as_str()).unwrap(),
            TreeNode::ErrorMarker(code) => write!(out, "[!{code}]").unwrap(),
            TreeNode::ErrorCell { name, code } => {
                write!(out, "<{} !{code}>", name.as_str()).unwrap()
            }
            TreeNode::Component {
                name,
                props,
                children,
            } => {
                write!(out, "<{}", name.as_str()).unwrap();
                for (key, value) in props {
                    write!(out, " {}={}", key.as_str(), prop(value)).unwrap();
                }
                if children.is_empty() {
                    out.push_str(" />");
                } else {
                    write!(out, ">{}</{}>", inline(children), name.as_str()).unwrap();
                }
            }
            TreeNode::Markdown { kind, children } => {
                let inner = inline(children);
                match kind {
                    MarkdownKind::Code(text) => write!(out, "`{}`", text.as_str()).unwrap(),
                    MarkdownKind::CodeBlock { language, text } => {
                        write!(out, "```{} {}```", language.as_str(), text.as_str()).unwrap()
                    }
                    MarkdownKind::Strong => write!(out, "**{inner}**").unwrap(),
                    MarkdownKind::Emphasis => write!(out, "*{inner}*").unwrap(),
                    MarkdownKind::Strikethrough => write!(out, "~~{inner}~~").unwrap(),
                    MarkdownKind::Link {
                        destination, image, ..
                    } => write!(
                        out,
                        "{}[{inner}]({})",
                        if *image { "!" } else { "" },
                        destination.as_str()
                    )
                    .unwrap(),
                    MarkdownKind::SoftBreak => out.push(' '),
                    MarkdownKind::HardBreak => out.push_str("\\n"),
                    MarkdownKind::ThematicBreak => out.push_str("---"),
                    _ => out.push_str(&inner),
                }
            }
        }
    }
    out
}

fn prop(value: &TreeValue) -> String {
    match value {
        TreeValue::Null => "null".into(),
        TreeValue::Number(number) => number.to_string(),
        TreeValue::String(text) => format!("{:?}", text.as_str()),
        TreeValue::Boolean(value) => value.to_string(),
        TreeValue::List(items) => format!(
            "[{}]",
            items.iter().map(prop).collect::<Vec<_>>().join(", ")
        ),
        TreeValue::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!("{}: {}", key.as_str(), prop(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TreeValue::Node(node) => inline(std::slice::from_ref(node)),
        TreeValue::Action(steps) => format!(
            "action({})",
            steps
                .iter()
                .map(|step| format!("{:?}", step.operation))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TreeValue::Snapshot {
            record_id, state, ..
        } => format!("snapshot({}, {})", record_id.as_str(), state.as_str()),
        TreeValue::Handle(id) => format!("handle({})", id.as_str()),
        TreeValue::Site(name) => format!("site({})", name.as_str()),
        TreeValue::Binding { state, kind } => format!("${}:{kind}", state.as_str()),
        TreeValue::Error(code) => format!("!{code}"),
    }
}
