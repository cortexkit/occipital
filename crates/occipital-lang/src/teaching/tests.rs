use std::collections::BTreeMap;

use super::*;
use crate::geometry::{LayoutContext, Network, Placement};
use crate::{Decimal, Profile, ReplySession, SharedAccount};

fn reduced() -> ReplyTeaching {
    build(Catalog::shared(), Runner::default())
}

/// A list of strings from the embedded catalog document, for comparing the
/// generator with lists the catalog states for review.
fn catalog_list(pointer: &str) -> Vec<String> {
    let document = serde_json::to_value(Catalog::shared()).unwrap();
    serde_json::from_value(document.pointer(pointer).unwrap().clone()).unwrap()
}

/// Check one reply source the way an app finishes a stored reply.
fn check_reply(source: &str) -> crate::ReplyRevision {
    let layout = LayoutContext {
        width: Decimal::from_integer(320).unwrap(),
        content_size_category: "large".into(),
        network: Network::Unknown,
        placement: Placement::Page,
    };
    ReplySession::new(
        Profile::Reply,
        crate::host::EXECUTION_DIALECT.into(),
        Catalog::shared(),
        layout,
        SharedAccount::chat(),
        SharedAccount::app(),
    )
    .unwrap()
    .finish(source.as_bytes(), BTreeMap::new(), vec![])
    .unwrap()
}

#[test]
fn the_text_is_pinned_byte_for_byte() {
    let text = &reply_teaching().text;
    let digest: String = hmac_sha256::Hash::hash(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // A change here changes every session's prompt and the item_digest the
    // module serves with it; update this pin only together with a reviewed
    // text.
    assert_eq!(
        digest, "e4fad0395f9c1ef22e475b10f3a278040dd833d441f96de79845f6316f15dc2b",
        "{text}"
    );
    // Every session pays for this text; growth should be a visible choice.
    assert_eq!(text.len(), 1874);
}

#[test]
fn the_same_catalog_gives_the_same_bytes() {
    let first = reduced();
    let second = build(&Catalog::load().unwrap(), Runner::default());
    assert_eq!(first, second);
    assert_eq!(&first, reply_teaching());
}

#[test]
fn every_example_is_in_the_text_and_checks_clean_under_the_reply_profile() {
    let teaching = reply_teaching();
    assert!(!teaching.examples.is_empty());
    for example in &teaching.examples {
        assert!(teaching.text.contains(example.as_str()), "{example}");
        let revision = check_reply(example);
        assert!(
            revision.diagnostics.is_empty(),
            "{example}: {:?}",
            revision.diagnostics
        );
        assert!(
            revision.literals.is_empty(),
            "{example}: drawn as literal text"
        );
    }
}

#[test]
fn the_check_used_for_examples_does_reject_a_broken_tag() {
    // Guards the test above: an example that breaks a rule must not pass.
    let revision = check_reply("<Badge tone=\"loud\">x</Badge>");
    assert!(!revision.diagnostics.is_empty());
}

#[test]
fn it_teaches_exactly_the_reduced_reply_set_the_catalog_lists() {
    assert_eq!(
        reply_teaching().tags,
        catalog_list("/profilesAllow/replyReduced/tags")
    );
}

#[test]
fn it_names_no_reference_handle_or_map_tag() {
    let text = &reply_teaching().text;
    for absent in [
        "<Map",
        "<Citation",
        "<Image",
        "<Chart",
        "$$tc",
        "Artifact(",
        "File(",
        "snapshot",
        "handle<",
        "node<",
        "more?:",
    ] {
        assert!(!text.contains(absent), "{absent} in:\n{text}");
    }
    // Table is taught only in its written-rows form.
    assert!(text.contains("<Table data: rows, columns?: list<string|record>"));
}

#[test]
fn a_runner_with_references_and_handles_gets_the_full_taught_set() {
    let full = build(
        Catalog::shared(),
        Runner {
            refs: true,
            handles: true,
            maps: false,
        },
    );
    assert_eq!(full.tags, catalog_list("/profilesAllow/replyTaught/tags"));
    assert!(full.text.contains("$$tc(N)"));
    let with_maps = build(
        Catalog::shared(),
        Runner {
            refs: true,
            handles: true,
            maps: true,
        },
    );
    assert!(with_maps.tags.iter().any(|tag| tag == "Map"));
}

#[test]
fn alternatives_split_only_at_the_top_level() {
    assert_eq!(
        alternatives("list<string|record>|list<node<Col>>"),
        ["list<string|record>", "list<node<Col>>"]
    );
    assert_eq!(alternatives("string"), ["string"]);
}
