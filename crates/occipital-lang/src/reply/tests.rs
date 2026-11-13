use super::support::*;
use super::*;
use crate::geometry::{Height, RenderState};
use crate::{Decimal, DiagnosticCode as Code};

const EXAMPLES: [&str; 4] = [
    "The test run finished with one failure. <Badge tone=\"danger\">1 failed</Badge> <Badge tone=\"success\">17 passed</Badge>\n\n<Callout tone=\"warning\" title=\"testPaging failed\">\nThe second page came back empty after its cursor expired. The fix is in `PageLoader.swift`: the cursor is now refreshed before the request.\n</Callout>\n\nHow long each suite took:\n\n<Chart kind=\"bar\" data={Parse($$tc(7), \"csv\", {columns: {seconds: \"number\"}})} x=\"suite\" y=\"seconds\" aspect=\"16:9\" />",
    "## Two cafés near the station\n\n**Kahve Durağı** is the closest, two minutes on foot.<Citation source={$$tc(12)} /> It opens at 7 and has seats outside.\n\n<ImageGroup images={[Artifact(\"art_k1\"), Artifact(\"art_k2\")]} aspect=\"4:3\" caption=\"Kahve Durağı, inside and out\" />\n\n**Liman Kahvesi** is quieter, with a view of the ferry pier.<Citation source={$$tc(12)} />\n\n<Map places={Parse($$tc(14), \"json\")} aspect=\"16:9\" />\n\n<LinkCard href=\"https://limankahvesi.example/menu\" title=\"Liman Kahvesi menu\" />",
    "Your three largest folders are below. <Styled tone=\"muted\">Sizes come from a scan two minutes ago.</Styled>\n\n<Table data={Parse($$tc(22), \"json\", {columns: {path: \"text\", bytes: \"number\"}})} columns={[\"path\", \"bytes\"]} displayRows={6} />\n\n| Folder | Safe to clear? |\n|---|---|\n| `~/Library/Caches` | <Badge tone=\"success\">Yes</Badge> |\n| `~/Work` | <Badge tone=\"danger\">No</Badge> |",
    "Here is the summary. <Badge tone=\"info\">Draft</Badge>\n\nexport const total = Sum(data.rows.sales)\n\nThe total is {total}. <Gauge value={0.4} /> <Badge tone=\"info\" text={total} />",
];

#[test]
fn worked_replies_every_split_and_single_bytes_equal_one_shot_and_replay() {
    let records = || [7, 12, 14, 22].into_iter().map(|n| (n, record())).collect();
    for (example, source) in EXAMPLES.into_iter().enumerate() {
        let expected = session()
            .finish(source.as_bytes(), records(), vec![])
            .unwrap();
        if example < 3 {
            assert!(
                expected.diagnostics.is_empty(),
                "{:?}",
                expected.diagnostics
            );
        }
        let replay = session()
            .finish(source.as_bytes(), records(), vec![])
            .unwrap();
        same(&expected, &replay);
        for split in 0..=source.len() {
            let mut s = session();
            s.chunk(&source.as_bytes()[..split]).unwrap();
            same(
                &expected,
                &s.finish(&source.as_bytes()[split..], records(), vec![])
                    .unwrap(),
            );
        }
        let mut s = session();
        for byte in source.as_bytes() {
            s.chunk(std::slice::from_ref(byte)).unwrap();
        }
        same(&expected, &s.finish(&[], records(), vec![]).unwrap());
    }
}

#[test]
fn degradation_is_literal_and_has_exactly_the_two_specified_diagnostics() {
    let result = finish(EXAMPLES[3].as_bytes());
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|d| d.code())
            .collect::<Vec<_>>(),
        [Code::ReplyForbidden, Code::UnknownComponent]
    );
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code() == Code::UnknownComponent
            && serde_json::to_value(d).unwrap()["subject"] == "Gauge"));
    assert_eq!(result.plain_text.as_str(), "Here is the summary. [Draft]\n\nexport const total = Sum(data.rows.sales)\n\nThe total is {total}. <Gauge value={0.4} /> <Badge tone=\"info\" text={total} />");
    // The declaration and interpolation, as well as both failed tags, are opaque.
    assert_eq!(result.literals.len(), 4);
}

#[test]
fn adversarial_contexts_combined_chunkings_and_provisional_views_are_split_invariant() {
    let cases = [
        "`unmatched <Badge>hidden</Badge>",
        "[label <Badge>hidden</Badge>](https://example.test)",
        "| a | b |\n|---|---|\n| <Badge>yes</Badge> | no |\n",
        "text\n<Badge>block or inline</Badge> suffix\n\nnext",
        "\\$$tc(1) \\<Badge>escaped</Badge> `$$tc(2)`",
        "<Badge tone=\"info\" broken > malformed $$tc(4)",
    ];
    for source in cases {
        let expected = finish(source.as_bytes());
        for split in 0..=source.len() {
            let mut a = session();
            a.chunk(&source.as_bytes()[..split]).unwrap();
            let mut b = session();
            for byte in &source.as_bytes()[..split] {
                b.chunk(std::slice::from_ref(byte)).unwrap();
            }
            assert_eq!(
                a.candidate.as_ref().map(|r| &r.tree),
                b.candidate.as_ref().map(|r| &r.tree)
            );
            for chunk in source.as_bytes()[split..].chunks(3) {
                a.chunk(chunk).unwrap();
            }
            same(&expected, &a.finish(&[], BTreeMap::new(), vec![]).unwrap());
        }
    }
}

#[test]
fn delivered_snapshot_moves_loading_to_ready_at_the_final_height() {
    let mut s = session();
    let r = s
        .finish(
            b"<Chart kind=\"bar\" data={Parse($$tc(7), \"json\")} x=\"label\" y=\"seconds\" />",
            [(7, record())].into(),
            vec![],
        )
        .unwrap();
    let first = measure(&r);
    assert_eq!(first.boxes[0].state, RenderState::Loading);
    // No caption or axis labels: the fixed 16:9 viewport is 320 × 9 / 16.
    assert_eq!(
        first.height,
        Height::Known(Decimal::from_integer(180).unwrap())
    );
    let ready = s
        .event(Event::Snapshot {
            reference: 7,
            result: Ok(br#"[{"label":"a","seconds":2}]"#.to_vec()),
        })
        .unwrap()
        .unwrap();
    let measured = measure(&ready);
    assert_eq!(measured.boxes[0].state, RenderState::Ready);
    assert_eq!(first.height, measured.height);
    assert_eq!(ready.plain_text.as_str(), "[chart: bar, 1 points]");
    s.event(Event::MeasureComplete(Box::new(first))).unwrap();
    assert!(s.committed.is_none());
    s.event(Event::MeasureComplete(Box::new(measured))).unwrap();
    assert_eq!(s.committed.as_ref().unwrap().0.id, ready.id);
}

#[test]
fn fetch_failure_is_unavailable_but_malformed_fetched_data_is_an_error() {
    for (format, bad, code) in [
        ("json", "{", Code::JsonSyntax),
        ("csv", "a\n\"unterminated", Code::CsvSyntax),
    ] {
        let source = format!("<Table data={{Parse($$tc(1), \"{format}\")}} displayRows={{6}} />");
        let mut s = session();
        let initial = s
            .finish(source.as_bytes(), [(1, record())].into(), vec![])
            .unwrap();
        let unavailable = s
            .event(Event::Snapshot {
                reference: 1,
                result: Err(()),
            })
            .unwrap()
            .unwrap();
        assert!(unavailable.diagnostics.is_empty());
        assert_eq!(
            measure(&unavailable).boxes[0].state,
            RenderState::Unavailable
        );
        let error = s
            .event(Event::Snapshot {
                reference: 1,
                result: Ok(bad.as_bytes().to_vec()),
            })
            .unwrap()
            .unwrap();
        assert!(
            error.diagnostics.iter().any(|d| d.code() == code),
            "{:?}",
            error.diagnostics
        );
        assert_eq!(measure(&error).boxes[0].state, RenderState::Error(code));
        assert_eq!(measure(&initial).height, measure(&error).height);
    }
}

#[test]
fn buffer_thresholds_emit_sanitized_provisional_literals_without_changing_final_limits() {
    for source in [
        format!("`\u{1}{}", "a".repeat(32 * 1024)),
        format!("<Badge text=\"\u{1}{}", "a".repeat(4 * 1024)),
        format!("$$tc({}\u{1}", "0".repeat(24)),
    ] {
        let mut s = session();
        let r = s
            .chunk(source.as_bytes())
            .unwrap()
            .expect("crossing a buffering threshold emits a frame");
        assert!(r.provisional);
        let display = format!("{:?}", r.tree);
        assert!(!display.contains("\\u{1}"));
        assert!(display.contains('�'), "{display}");
        same(
            &finish(source.as_bytes()),
            &s.finish(&[], BTreeMap::new(), vec![]).unwrap(),
        );
    }
}

#[test]
fn provisional_exhaustion_freezes_the_frame_and_protects_final_reply_and_chat_charges() {
    let mut s = session();
    let chat = s.accounts.chat_handle();
    assert_eq!(chat.usage().reserved_fuel, 600_000);
    let first = s.chunk(b"visible\n\n").unwrap().unwrap();
    let spent = chat.usage().spent_fuel;
    s.work
        .as_mut()
        .unwrap()
        .provisional(1_400_000 - spent)
        .unwrap();
    assert!(s.chunk(b"<Badge>late</Badge>").unwrap().is_none());
    assert!(s.frozen);
    assert_eq!(s.candidate.as_ref().unwrap().tree, first.tree);
    assert_eq!(chat.usage().reserved_fuel, 600_000);
    let final_result = s.finish(&[], BTreeMap::new(), vec![]).unwrap();
    same(&finish(b"visible\n\n<Badge>late</Badge>"), &final_result);
    assert_eq!(chat.usage().reserved_fuel, 0);
    assert_eq!(
        chat.usage().spent_fuel,
        1_400_000 + 2 * b"visible\n\n<Badge>late</Badge>".len() as u64
    );
}

#[test]
fn handle_changes_layout_keys_account_release_and_stale_completions() {
    let source = b"<Image image={Artifact(\"photo\")} alt=\"scene\" />";
    let handle = crate::HandleRecord {
        record_id: "stored-photo".into(),
        computer: "computer".into(),
        session: "session".into(),
        scope: "scope".into(),
        kind: crate::HandleKind::Artifact,
        span: SourceSpan {
            line: 1,
            column: 15,
            end_line: 1,
            end_column: 32,
        },
        resource_id: Some("photo".into()),
        version: Some("1".into()),
        content_hash: None,
        mime: Some("image/png".into()),
        bytes: Some(10),
        available: false,
    };
    let mut s = session();
    let first = s.finish(source, BTreeMap::new(), vec![handle]).unwrap();
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    assert_eq!(measure(&first).boxes[0].state, RenderState::Unavailable);
    let ready = s
        .event(Event::HandleAvailability {
            record_id: "stored-photo".into(),
            available: true,
        })
        .unwrap()
        .unwrap();
    assert_eq!(measure(&ready).boxes[0].state, RenderState::Ready);
    assert_eq!(measure(&first).height, measure(&ready).height);
    let old = measure(&ready);
    let mut narrow = layout();
    narrow.width = Decimal::from_integer(200).unwrap();
    s.event(Event::Layout(narrow.clone())).unwrap();
    s.event(Event::MeasureComplete(Box::new(old))).unwrap();
    assert!(s.committed.is_none());
    let current = geometry::measure(&ready.measure_revision(), &narrow, &Fixed).unwrap();
    s.event(Event::MeasureComplete(Box::new(current))).unwrap();
    assert_eq!(s.committed.as_ref().unwrap().0.id, ready.id);
    let retry = s.event(Event::AccountRelease).unwrap().unwrap();
    assert_eq!(retry.tree, ready.tree);
}

#[test]
fn provisional_pictures_at_every_prefix_do_not_depend_on_transport_chunking() {
    for source in [EXAMPLES[0], EXAMPLES[2], "é <Badge>ok</Badge> `later`\n\n"] {
        let mut single = session();
        for (end, byte) in source.as_bytes().iter().enumerate() {
            single.chunk(std::slice::from_ref(byte)).unwrap();
            let mut combined = session();
            combined.chunk(&source.as_bytes()[..=end]).unwrap();
            assert_eq!(
                single.candidate.as_ref().map(|r| &r.tree),
                combined.candidate.as_ref().map(|r| &r.tree)
            );
        }
    }
}

#[test]
fn records_survive_replay_and_parse_preparation_is_cached_and_charged() {
    let source = b"<Table data={Parse($$tc(1), \"json\")} />";
    let mut s = session();
    let chat = s.accounts.chat_handle();
    let initial = s.finish(source, [(1, record())].into(), vec![]).unwrap();
    let mut replay = session();
    same(
        &initial,
        &replay
            .finish(source, [(1, record())].into(), vec![])
            .unwrap(),
    );
    let spent = chat.usage().spent_fuel;
    let ready = s
        .event(Event::Snapshot {
            reference: 1,
            result: Ok(b"[]".to_vec()),
        })
        .unwrap()
        .unwrap();
    assert_eq!(chat.usage().spent_fuel - spent, 4); // two bytes and one JSON value at two fuel.
    let spent = chat.usage().spent_fuel;
    s.event(Event::AccountRelease).unwrap();
    assert_eq!(chat.usage().spent_fuel, spent);
    assert_eq!(ready.plain_text.as_str(), "[table: 0 rows]");
}

#[test]
fn repaired_byte_offsets_and_failed_reference_chips_remain_in_their_fixed_boxes() {
    fn chip(revision: &Revision) -> Measurement {
        // Prose sizing is deferred; measure the fixed-size child independently.
        let crate::eval::tree::TreeNode::Markdown { children, .. } = &revision.tree.blocks[0]
        else {
            panic!("reference paragraph")
        };
        let tree = Tree {
            blocks: children.clone(),
        };
        let input = revision.measure_revision();
        geometry::measure(
            &MeasureRevision {
                tree: &tree,
                ..input
            },
            &layout(),
            &Fixed,
        )
        .unwrap()
    }
    let result = finish(b"\xff <Badge>ok</Badge> <Gauge />");
    assert_eq!(result.plain_text.as_str(), "� [ok] <Gauge />");
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|d| d.code())
            .collect::<Vec<_>>(),
        [Code::Encoding, Code::UnknownComponent]
    );
    let mut s = session();
    let initial = s
        .finish(b"$$tc(1)", [(1, record())].into(), vec![])
        .unwrap();
    let first = chip(&initial);
    assert_eq!(first.boxes[0].state, RenderState::Loading);
    let failed = s
        .event(Event::Snapshot {
            reference: 1,
            result: Err(()),
        })
        .unwrap()
        .unwrap();
    assert!(failed.diagnostics.is_empty());
    let measured = chip(&failed);
    assert_eq!(measured.boxes[0].state, RenderState::Unavailable);
    assert_eq!(measured.height, first.height);
    assert_eq!(failed.plain_text.as_str(), "[output of bash]");
}

#[test]
fn provisional_frames_commit_only_after_current_measurement() {
    let mut s = session();
    let r = s.chunk(b"<Badge>ok</Badge>\n\n").unwrap().unwrap();
    assert!(s.committed.is_none());
    let measured = measure(&r);
    s.event(Event::MeasureComplete(Box::new(measured))).unwrap();
    assert_eq!(s.committed.as_ref().unwrap().0.id, r.id);
    assert!(s.committed.as_ref().unwrap().0.provisional);
    let final_revision = s.finish(&[], BTreeMap::new(), vec![]).unwrap();
    assert!(!final_revision.provisional);
    assert!(matches!(s.chunk(b"no"), Err(Error::Finished)));
}

#[test]
fn late_render_limit_is_local_to_its_tag_and_keeps_its_final_height() {
    let mut s = session();
    let source = b"<Chart kind=\"bar\" data={Parse($$tc(1), \"json\")} x=\"x\" y=\"y\" />\n\n<Badge>keep</Badge>";
    let initial = s.finish(source, [(1, record())].into(), vec![]).unwrap();
    let before = measure(&initial).boxes[0].height;
    let bytes = format!("[{}]", vec![r#"{"x":"a","y":1}"#; 2049].join(",")).into_bytes();
    let limited = s
        .event(Event::Snapshot {
            reference: 1,
            result: Ok(bytes),
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        limited
            .diagnostics
            .iter()
            .map(|d| d.code())
            .collect::<Vec<_>>(),
        [Code::RenderLimit]
    );
    let measured = measure(&limited);
    assert_eq!(
        measured.boxes[0].state,
        RenderState::Error(Code::RenderLimit)
    );
    assert_eq!(measured.boxes[0].height, before);
    assert!(limited.plain_text.as_str().ends_with("\n\n[keep]"));
}

#[test]
fn independently_created_replies_do_not_accept_each_others_measurements() {
    let mut a = session();
    let mut b = session();
    let ar = a
        .finish(
            b"<Chart kind=\"line\" data={$$tc(1)} />",
            BTreeMap::new(),
            vec![],
        )
        .unwrap();
    let br = b
        .finish(
            b"<Image image={Artifact(\"photo\")} />",
            BTreeMap::new(),
            vec![],
        )
        .unwrap();
    assert_eq!(ar.id, br.id);
    a.event(Event::MeasureComplete(Box::new(measure(&br))))
        .unwrap();
    assert!(a.committed.is_none());
    a.event(Event::MeasureComplete(Box::new(measure(&ar))))
        .unwrap();
    assert!(a.committed.is_some());
}

#[test]
fn an_evaluated_prop_failure_is_literal_without_consuming_the_next_tags_text_form() {
    let source = b"<LinkCard href=\"javascript:alert(1)\" title=\"bad\" />\n\n<Badge>keep</Badge>";
    let mut s = session();
    let result = s.finish(source, BTreeMap::new(), vec![]).unwrap();
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|d| d.code())
            .collect::<Vec<_>>(),
        [Code::Scheme]
    );
    assert_eq!(result.literals.len(), 1);
    assert_eq!(
        result.plain_text.as_str(),
        "<LinkCard href=\"javascript:alert(1)\" title=\"bad\" />\n\n[keep]"
    );
    let repeated = s.event(Event::AccountRelease).unwrap().unwrap();
    same(&result, &repeated);
}

#[test]
fn missing_parse_key_has_no_source_span() {
    let program = crate::prepare::prepare(
        b"<Badge>ok</Badge>",
        Mode::Program,
        &crate::prepare::types::TypeContext::default(),
    )
    .unwrap();
    let key = snapshot::ParseKey {
        source: snapshot::SnapshotSource::Reference("404".into()),
        format: crate::data::Format::Json,
        options: crate::data::Options::default(),
    };
    assert_eq!(render::parse_span(&program, &key), None);
}

#[test]
fn parse_span_skips_malformed_calls_before_a_valid_key() {
    let program = crate::prepare::prepare(
        b"{Parse()}\n\n<Table data={Parse($$tc(1), \"json\")} />",
        Mode::Program,
        &crate::prepare::types::TypeContext::default(),
    )
    .unwrap();
    let keys = snapshot::parse_keys(&program);
    assert_eq!(keys.len(), 1);
    let span = render::parse_span(&program, &keys[0]).unwrap();
    assert_eq!((span.line, span.column), (3, 14));
}

#[test]
fn nested_parse_call_argument_reaches_reply_refresh_with_its_source_span() {
    let source = b"<Badge text={FormatNumber(Count(Parse($$tc(1), \"json\").rows), 0)} />";
    let mut s = session();
    let ordinary = s.finish(source, [(1, record())].into(), vec![]).unwrap();
    assert_eq!(
        ordinary
            .diagnostics
            .iter()
            .map(|d| d.code())
            .collect::<Vec<_>>(),
        [Code::ReplyForbidden]
    );
    assert!(snapshot::parse_keys(s.program.as_ref().unwrap()).is_empty());

    // Computed reply values are forbidden today. Exercise refresh with a resolved
    // expression tree so traversal remains safe independently of that policy.
    let program = crate::prepare::prepare(
        source,
        Mode::Program,
        &crate::prepare::types::TypeContext::default(),
    )
    .unwrap();
    let keys = snapshot::parse_keys(&program);
    assert_eq!(keys.len(), 1);
    let span = render::parse_span(&program, &keys[0]).unwrap();
    assert_eq!((span.line, span.column), (1, 33));
    s.program = Some(program);
    let ready = s
        .event(Event::Snapshot {
            reference: 1,
            result: Ok(b"[]".to_vec()),
        })
        .unwrap()
        .unwrap();
    assert!(ready.diagnostics.is_empty(), "{:?}", ready.diagnostics);
    assert_eq!(s.prepared.len(), 1);
    assert_eq!(ready.plain_text.as_str(), "[0]");
}

#[test]
fn unknown_snapshot_does_not_retain_or_revise_and_real_snapshot_still_applies() {
    let mut s = session();
    let chat = s.accounts.chat_handle();
    let initial = s
        .finish(
            b"<Table data={Parse($$tc(1), \"json\")} />",
            [(1, record())].into(),
            vec![],
        )
        .unwrap();
    let reply_retained = s.accounts.local_usage_for_tests().retained;
    let chat_retained = chat.usage().retained;
    assert!(reply_retained > 0);
    let ignored = s
        .event(Event::Snapshot {
            reference: 404,
            result: Ok(br#"[{"unexpected":"ignored"}]"#.to_vec()),
        })
        .unwrap();
    assert_eq!(
        (
            ignored.is_none(),
            s.accounts.local_usage_for_tests().retained,
            chat.usage().retained
        ),
        (true, reply_retained, chat_retained),
    );
    let ready = s
        .event(Event::Snapshot {
            reference: 1,
            result: Ok(br#"[{"value":7}]"#.to_vec()),
        })
        .unwrap()
        .unwrap();
    assert_eq!(ready.id, initial.id + 1);
    assert!(ready.diagnostics.is_empty());
    assert_eq!(ready.plain_text.as_str(), "[table: 1 rows]");
    assert_eq!(measure(&ready).boxes[0].state, RenderState::Ready);
    assert!(s.accounts.local_usage_for_tests().retained > reply_retained);
}

#[test]
fn a_reply_session_accepts_an_equal_catalog_copy_and_refuses_a_modified_one() {
    let open = |catalog: &Catalog| {
        Session::new(
            Profile::Reply,
            "1.0".into(),
            catalog,
            layout(),
            crate::SharedAccount::chat(),
            crate::SharedAccount::app(),
        )
    };
    // A separately loaded copy is not the shared instance, but its digest is
    // the embedded catalog's, so it is the same catalog v1.
    let copy = Catalog::load().unwrap();
    assert!(!std::ptr::eq(&copy, Catalog::shared()));
    assert!(open(&copy).is_ok());
    // Any change to the catalog changes its digest, and a session must not
    // claim to run under a catalog it does not evaluate with.
    let mut modified = Catalog::load().unwrap();
    modified.vocabulary.tone.push("loud".into());
    assert!(matches!(open(&modified), Err(Error::Catalog)));
}

/// A reply drawn without its catalog reads as its readable text: a table as
/// its rows (header first, cells tab-separated, in the named column order)
/// and a badge as its bare label, where the plain text keeps the catalog's
/// `[table: N rows]` and `[label]`.
#[test]
fn readable_text_gives_a_table_its_rows_and_a_badge_its_label() {
    let source = "Tests finished. <Badge tone=\"success\">17 passed</Badge>\n\n<Table data={[{state: \"up\", service: \"api\"}, {state: \"down\", service: \"db\"}]} columns={[\"service\", {key: \"state\", label: \"State\"}]} />\n";
    let done = finish(source.as_bytes());
    assert_eq!(
        done.plain_text.as_str(),
        "Tests finished. [17 passed]\n\n[table: 2 rows]\n"
    );
    assert_eq!(
        done.readable_text.as_str(),
        "Tests finished. 17 passed\n\nservice\tState\napi\tup\ndb\tdown\n"
    );
}

/// With no columns named, the first row's keys are the columns; a cell's
/// tabs and line breaks become spaces; a long table stops at 200 rows.
#[test]
fn readable_text_bounds_a_long_table() {
    let rows: Vec<String> = (0..203)
        .map(|n| format!("{{n: {n}, note: \"a\\tb\\nc\"}}"))
        .collect();
    let source = format!("<Table data={{[{}]}} />", rows.join(", "));
    let done = finish(source.as_bytes());
    let lines: Vec<&str> = done.readable_text.as_str().lines().collect();
    assert_eq!(lines[0], "n\tnote");
    assert_eq!(lines[1], "0\ta b c");
    assert_eq!(lines.len(), 1 + 200 + 1);
    assert_eq!(lines[201], "…and 3 more rows");
}

/// While streaming, the readable text is the words so far, so an app that
/// doesn't hold the catalog has something to draw before the reply finishes.
#[test]
fn a_provisional_revision_carries_the_words_so_far() {
    let mut s = session();
    let source = b"First paragraph.\n\nSecond one, still ";
    let latest = source
        .iter()
        .filter_map(|byte| s.chunk(std::slice::from_ref(byte)).unwrap())
        .last()
        .unwrap();
    assert!(latest.provisional);
    assert_eq!(latest.plain_text.as_str(), "");
    assert!(latest
        .readable_text
        .as_str()
        .starts_with("First paragraph.\nSecond one"));
}
