use super::reply::{plain_text, Content, Replacement};
use super::*;

fn program(source: &str) -> FallbackText {
    program_fallback(source.as_bytes(), "Title", "Summary")
}

#[test]
fn program_header_limits_are_scalars_and_cost_includes_final_output() {
    let result = program_fallback(b"body", &"🦀".repeat(513), &"é".repeat(513));
    assert_eq!(
        result.text.as_str(),
        format!("{}\n{}\nbody", "🦀".repeat(512), "é".repeat(512))
    );
    assert_eq!(result.scanned_bytes, 4);
    assert_eq!(result.fuel, 4 + 512 + 1 + 512 + 1 + 4);
    assert!(!result.truncated);
    assert_eq!(program_fallback(b"", "", "").text.as_str(), "\n\n");
}

#[test]
fn declaration_termination_uses_depth_strings_operators_and_next_line() {
    let source = concat!(
        "export const a = (\n  [\"[\\\"\", {x: 1}]\n)\n",
        "export const b = 1 +\n 2\n",
        "export const c = true\n ? 1\n : 2\n",
        "export const d = base\n .field\n",
        "export const e = true\n & false\n | true\n",
        "  body\n",
        " export const indented = 3\n",
        "export const broken = {\n\n",
        "visible after blank\n",
        "export const string = \"open\ncontinued\n\n",
        "visible again"
    );
    assert_eq!(
        program(source).text.as_str(),
        "Title\nSummary\n  body\n export const indented = 3\nvisible after blank\nvisible again"
    );
    for operator in "+-*/%=<>&|?:,!".chars() {
        assert_eq!(
            program(&format!("export const x = 1 {operator}\nhidden\nvisible"))
                .text
                .as_str(),
            "Title\nSummary\nvisible",
            "operator {operator}"
        );
    }
}

#[test]
fn declaration_depth_saturates_at_64_and_does_not_track_tags() {
    let source = format!(
        "export const x = {}{}\nvisible",
        "(".repeat(70),
        ")".repeat(64)
    );
    assert_eq!(program(&source).text.as_str(), "Title\nSummary\nvisible");
    assert_eq!(
        program("export const x = <Card>\nhidden\nvisible")
            .text
            .as_str(),
        "Title\nSummary\nvisible"
    );
    assert_eq!(
        program("export const x = <Card\nvisible").text.as_str(),
        "Title\nSummary\nvisible"
    );
}

#[test]
fn declaration_depth_cap_is_exactly_64() {
    let source = format!(
        "export const x = {}{}\nhidden\n)\nvisible",
        "(".repeat(65),
        ")".repeat(63)
    );
    assert_eq!(program(&source).text.as_str(), "Title\nSummary\nvisible");
}

#[test]
fn removed_tags_keep_only_the_seven_literal_props_on_separate_lines() {
    let source = concat!(
        "before<Unknown title=\"Title {x}\" subtitle='Sub' label=\"L\" text='T' ",
        "description=\"D\" caption=\"C\" alt=\"A\" tone=\"danger\" ",
        "other={nested({title: \"not kept >\"})} title={\"computed\"} />after\n",
        "<Badge>children</Badge> <lower title=\"literal\">\n",
        "<Header title=\"escaped \\\"quote\\\" \\u{1F980}\\t!\" />"
    );
    assert_eq!(
        program(source).text.as_str(),
        "Title\nSummary\nbefore\nTitle {x}\nSub\nL\nT\nD\nC\nA\nafter\nchildren <lower title=\"literal\">\nescaped \"quote\" 🦀\t!\n"
    );
    assert_eq!(
        program("<Broken title='kept' missing").text.as_str(),
        "Title\nSummary\nkept\n"
    );
}

#[test]
fn interpolations_and_references_are_lexical_and_bad_candidates_are_literal() {
    assert_eq!(
        program("{a({x: \"}\\\"\"})} $$tc(0) $$tc(12) $$tc(01) $$tc(no)\n{unfinished")
            .text
            .as_str(),
        "Title\nSummary\n… [tool output] [tool output] $$tc(01) $$tc(no)\n…"
    );
    let result = program("é{1} $$tc(7)");
    assert_eq!(result.text.as_str(), "Title\nSummary\né… [tool output]");
    assert_eq!(result.scanned_bytes, 13);
    assert_eq!(result.fuel, 13 + 30);
}

#[test]
fn fenced_code_is_literal_with_matching_marker_and_length() {
    let source = concat!(
        "  ````rust\nexport const keep = 1\n<Tag title='keep' /> {keep} $$tc(7)\n",
        "```\n~~~~\n````not a close\n  ```` \n",
        "<Badge>drawn</Badge>\n",
        "~~~\n{literal}\n~~~\n{changed}"
    );
    assert_eq!(
        program(source).text.as_str(),
        concat!(
            "Title\nSummary\n  ````rust\nexport const keep = 1\n<Tag title='keep' /> {keep} $$tc(7)\n",
            "```\n~~~~\n````not a close\n  ```` \ndrawn\n~~~\n{literal}\n~~~\n…"
        )
    );
    assert_eq!(
        program("```\n{unclosed} $$tc(1)").text.as_str(),
        "Title\nSummary\n```\n{unclosed} $$tc(1)"
    );
}

#[test]
fn blank_lines_collapse_and_unparsed_bytes_are_sanitized() {
    assert_eq!(
        program("a\n\n \t\n\n b").text.as_str(),
        "Title\nSummary\na\n\n b"
    );
    let result = program_fallback(
        b"\xef\xbb\xbfA\r\nB\rC\x00\x1b\xc2\x85\xff",
        "\u{7f}",
        "\u{1b}",
    );
    assert_eq!(result.text.as_str(), "�\n�\nA\nB\nC����");
    assert_eq!(result.scanned_bytes, 14);
    assert_eq!(result.fuel, 14 + 13);
}

#[test]
fn program_scan_cap_does_not_parse_or_inspect_the_tail() {
    let mut source = b"export ".to_vec();
    source.resize(PROGRAM_BYTES - 1, b'x');
    source.extend_from_slice(b"\nsecret tail");
    let result = program_fallback(&source, "T", "S");
    assert_eq!(result.text.as_str(), "T\nS\n…");
    assert_eq!(result.scanned_bytes, 65_536);
    assert_eq!(result.fuel, 65_541);
    assert!(result.truncated);
    let result = program_fallback(&source[..PROGRAM_BYTES], "T", "S");
    assert_eq!(result.text.as_str(), "T\nS\n");
    assert!(!result.truncated);
}

#[test]
fn program_output_cap_stops_scanning_and_includes_the_ellipsis() {
    let result = program(&"🦀".repeat(5000));
    assert_eq!(
        result.text.as_str(),
        format!("Title\nSummary\n{}…", "🦀".repeat(4081))
    );
    assert_eq!(result.text.as_str().chars().count(), 4096);
    assert_eq!(result.scanned_bytes, 4082 * 4);
    assert_eq!(result.fuel, (4082 * 4 + 4096) as u64);
    assert!(result.truncated);
    let exact = program(&"x".repeat(4082));
    assert!(!exact.truncated);
    assert!(exact.text.as_str().ends_with('x'));
}

#[test]
fn byte_cap_does_not_complete_a_scalar_from_the_tail() {
    let mut source = b"export ".to_vec();
    source.resize(PROGRAM_BYTES - 2, b'x');
    source.push(b'\n');
    source.extend_from_slice("🦀".as_bytes());
    let result = program_fallback(&source, "T", "S");
    assert_eq!(result.text.as_str(), "T\nS\n…");
    assert_eq!(result.scanned_bytes, PROGRAM_BYTES);
    assert_eq!(
        program_fallback(b"\xf0\x9f", "T", "S").text.as_str(),
        "T\nS\n�"
    );
}

#[test]
fn reply_forms_cover_every_reply_catalog_component() {
    type TextFormCase<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);
    let cases: &[TextFormCase<'_>] = &[
        ("Badge", &[("label", "First choice")], "[First choice]"),
        ("Styled", &[("children", "**kept**")], "**kept**"),
        (
            "Callout",
            &[("title", "Note"), ("body", "Body")],
            "Note: Body",
        ),
        ("Table", &[("N", "12")], "[table: 12 rows]"),
        (
            "LinkCard",
            &[
                ("title", "Full URL"),
                ("href", "https://example.com/path?q=x"),
            ],
            "Full URL (https://example.com/path?q=x)",
        ),
        ("Citation", &[("index", "2")], "[2]"),
        ("Image", &[("alt", "A cat")], "[image: A cat]"),
        (
            "ImageGroup",
            &[("n", "3"), ("caption", "Cats")],
            "[3 images: Cats]",
        ),
        ("Map", &[("n", "4")], "[map: 4 places]"),
        (
            "Chart",
            &[("kind", "bar"), ("n", "7")],
            "[chart: bar, 7 points]",
        ),
    ];
    for (name, fields, expected) in cases {
        let result = plain_text(
            b"<tag />",
            &[Replacement {
                start: 0,
                end: 7,
                content: Content::Tag { name, fields },
            }],
            false,
        );
        assert_eq!(result.text.as_str(), *expected, "{name}");
        assert_eq!(result.fuel, (7 + expected.chars().count()) as u64);
    }
    let catalog = crate::catalog::Catalog::shared();
    let names = catalog
        .components
        .iter()
        .filter(|(_, c)| c.profiles.iter().any(|p| p == "reply"))
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), cases.len());
    assert!(names
        .iter()
        .all(|name| cases.iter().any(|case| case.0 == *name)));
}

#[test]
fn reply_replaces_only_drawn_spans_and_recorded_references() {
    let source = "**prose** <Badge>OK</Badge> <Broken/> $$tc(1) $$tc(2)\n```\n$$tc(1)\n```";
    let replacements = [
        Replacement {
            start: 10,
            end: 27,
            content: Content::Tag {
                name: "Badge",
                fields: &[("label", "OK")],
            },
        },
        Replacement {
            start: 38,
            end: 45,
            content: Content::Reference {
                tool: Some("bash\u{1b}"),
            },
        },
        Replacement {
            start: 46,
            end: 53,
            content: Content::Reference { tool: None },
        },
    ];
    let result = plain_text(source.as_bytes(), &replacements, false);
    assert_eq!(
        result.text.as_str(),
        "**prose** [OK] <Broken/> [output of bash�] [tool output]\n```\n$$tc(1)\n```"
    );
    assert_eq!(result.scanned_bytes, source.len());
    assert_eq!(result.fuel, (source.len() + 72) as u64);
    assert_eq!(
        plain_text(
            b"<Unknown/>",
            &[Replacement {
                start: 0,
                end: 10,
                content: Content::Tag {
                    name: "Unknown",
                    fields: &[]
                }
            }],
            false
        )
        .text
        .as_str(),
        "<Unknown/>"
    );
}

#[test]
fn reply_source_and_output_caps_are_independent_and_fuel_is_bounded() {
    let source = vec![b'x'; REPLY_BYTES + 1];
    let result = plain_text(
        &source,
        &[Replacement {
            start: 0,
            end: REPLY_BYTES,
            content: Content::Tag {
                name: "Styled",
                fields: &[("children", "short")],
            },
        }],
        false,
    );
    assert_eq!(result.text.as_str(), "short…");
    assert_eq!(result.scanned_bytes, 262_144);
    assert_eq!(result.fuel, 262_150);
    let result = plain_text(&source, &[], false);
    assert_eq!(result.text.as_str(), format!("{}…", "x".repeat(65_536)));
    assert_eq!(result.scanned_bytes, 65_536);
    assert_eq!(result.fuel, 131_073);
    let huge = "🦀".repeat(70_000);
    let result = plain_text(
        &source,
        &[Replacement {
            start: 0,
            end: REPLY_BYTES,
            content: Content::Tag {
                name: "Styled",
                fields: &[("children", &huge)],
            },
        }],
        false,
    );
    assert_eq!(result.text.as_str().chars().count(), 65_537);
    assert_eq!(result.fuel, 327_681);
    assert!(result.fuel <= 400_000);
    let exact = plain_text(&source[..65_536], &[], false);
    assert!(!exact.truncated);
    assert_eq!(exact.text.as_str().chars().count(), 65_536);
}

#[test]
fn reply_literals_repair_encoding_and_keep_source_line_breaks() {
    let result = plain_text(b"\xef\xbb\xbfA\r\nB\r\n\rC\x1b\xff", &[], false);
    assert_eq!(result.text.as_str(), "A\nB\n\nC��");
    assert_eq!(result.scanned_bytes, 13);
    assert_eq!(result.fuel, 21);
    assert!(!result.truncated);
}

#[test]
fn tool_views_request_the_app_owned_generic_row() {
    assert_eq!(tool_view_fallback(), ToolViewFallback::UseGenericRow);
}
