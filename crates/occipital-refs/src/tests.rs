//! The rules, case by case, and the `text-references-v2` vectors written from
//! the same cases. `text-references-v1` pinned rules version 1 and stays as
//! it was tagged; only its checksums are checked here.
//!
//! Each case's text marks every expected mention with `⟦` and `⟧`. The
//! expected ranges come from where the marks sit, never from [`references`],
//! so a case can't pass by agreeing with a wrong implementation. The vectors
//! are the cases with the marks taken out and the ranges written as byte
//! offsets, which is what the apps replay.

use super::*;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

const BLESS: &str = "OCCIPITAL_BLESS_VECTORS";
const OPEN: &str = "⟦";
const CLOSE: &str = "⟧";

const OCCIPITAL: Option<Repository<'static>> = Some(Repository {
    host: "github.com",
    owner: "cortexkit",
    name: "occipital",
});

/// A link into cortexkit/occipital.
fn home(number: u32) -> (&'static str, String) {
    (
        ISSUE,
        format!("https://github.com/cortexkit/occipital/issues/{number}"),
    )
}

/// An `@NAME`, naming `name`.
fn agent(name: &str) -> (&'static str, String) {
    (MENTION, name.to_owned())
}

/// A link into the repository an `owner/repo#N` names.
fn named(owner: &str, name: &str, number: u32) -> (&'static str, String) {
    (
        REPOSITORY_ISSUE,
        format!("https://github.com/{owner}/{name}/issues/{number}"),
    )
}

struct Case {
    name: &'static str,
    /// The project's repository; `None` is a project with none, or text with
    /// no project at all, such as a room post by the person.
    repository: Option<Repository<'static>>,
    marked: &'static str,
    expected: Vec<(&'static str, String)>,
}

fn case(
    name: &'static str,
    repository: Option<Repository<'static>>,
    marked: &'static str,
    expected: Vec<(&'static str, String)>,
) -> Case {
    Case {
        name,
        repository,
        marked,
        expected,
    }
}

fn cases() -> Vec<Case> {
    vec![
        case(
            "01-keywords",
            OCCIPITAL,
            "See ⟦Issue 322⟧, ⟦PR 422⟧, ⟦PR #499⟧ and ⟦issue #911⟧.",
            vec![home(322), home(422), home(499), home(911)],
        ),
        case(
            "02-pull-request-and-case",
            OCCIPITAL,
            "Merged ⟦pull request 12⟧, ⟦pr 7⟧ and ⟦PULL REQUEST 8⟧; ⟦Pr #9⟧ too.",
            vec![home(12), home(7), home(8), home(9)],
        ),
        case(
            "03-bare-hash-is-plain",
            OCCIPITAL,
            "Room post #3543 and #12 stay plain, as do PR#5 and Issue#6.",
            vec![],
        ),
        case(
            "04-named-repository",
            OCCIPITAL,
            "Moved to ⟦cortexkit/aft#422⟧, after ⟦PR 3⟧ here.",
            vec![named("cortexkit", "aft", 422), home(3)],
        ),
        case(
            "05-no-repository",
            None,
            "Issue 322 and PR 4 name nothing, but ⟦cortexkit/aft#12⟧ does.",
            vec![named("cortexkit", "aft", 12)],
        ),
        case(
            "06-code-spans",
            OCCIPITAL,
            "Run `PR 12`, then ⟦PR 13⟧; ``a `PR 1` b`` stays code, a lone ` is not, so ⟦PR 2⟧ links.",
            vec![home(13), home(2)],
        ),
        case(
            "07-fenced-blocks",
            OCCIPITAL,
            "Before ⟦PR 1⟧\n```text\nPR 2\n```\nBetween ⟦PR 3⟧\n~~~~\nIssue 4\n~~~~\nAfter ⟦Issue 5⟧\n",
            vec![home(1), home(3), home(5)],
        ),
        case(
            "08-fence-in-a-list",
            OCCIPITAL,
            "- step\n  ```\n  PR 4\n  ```\n- ⟦PR 5⟧\n",
            vec![home(5)],
        ),
        case(
            "09-unclosed-fence",
            OCCIPITAL,
            "⟦PR 1⟧ before\n```\nPR 2\nand PR 3 never close",
            vec![home(1)],
        ),
        case(
            "10-markdown-links",
            OCCIPITAL,
            "[PR 12](https://github.com/cortexkit/occipital/pull/12) and ![Issue 3](shot.png) keep theirs; ⟦PR 13⟧ links, and so does [⟦PR 14⟧] without a destination.",
            vec![home(13), home(14)],
        ),
        case(
            "11-urls-and-tags",
            OCCIPITAL,
            "https://github.com/cortexkit/aft#12, https://example.com/a/b#3 and <https://x.dev/PR> stay; <Badge text=\"PR 12\" /> too; then ⟦PR 9⟧.",
            vec![home(9)],
        ),
        case(
            "12-number-boundaries",
            OCCIPITAL,
            "PR 3.5, Issue 1,000, PR 12abc, PR 12_x, PR 0, PR 07, PR 1234567890, xPR 1, issues 4 and PR\t5 are not mentions.",
            vec![],
        ),
        case(
            "13-punctuation-around",
            OCCIPITAL,
            "(⟦PR 12⟧) **⟦Issue 5⟧** ⟦PR 6⟧. ⟦PR 7⟧, ⟦PR 8⟧; ⟦PR 9⟧: ⟦PR  10⟧!",
            vec![home(12), home(5), home(6), home(7), home(8), home(9), home(10)],
        ),
        case(
            "14-unicode-around",
            OCCIPITAL,
            "Résumé: ⟦PR 12⟧’s fix — ⟦Issue 4⟧… but PR 12é and éPR 3 aren't.",
            vec![home(12), home(4)],
        ),
        case(
            "15-escaped-backticks",
            OCCIPITAL,
            "\\`⟦PR 1⟧\\` is not code.",
            vec![home(1)],
        ),
        case(
            "16-named-boundaries",
            OCCIPITAL,
            "path/cortexkit/aft#1, ⟦@me⟧/repo#2, a.b/c#3 and x-y/z#0 aren't; ⟦b-c/d_e.f#4⟧ is.",
            vec![agent("me"), named("b-c", "d_e.f", 4)],
        ),
        case(
            "17-unknown-host",
            Some(Repository {
                host: "gitlab.com",
                owner: "cortexkit",
                name: "occipital",
            }),
            "Issue 3 and cortexkit/aft#4 get no links on a host the rules don't know.",
            vec![],
        ),
        case(
            "18-repository-not-linkable",
            Some(Repository {
                host: "github.com",
                owner: "cortex kit",
                name: "occipital",
            }),
            "Issue 3 gets no link into an owner GitHub can't have, but ⟦cortexkit/aft#4⟧ does.",
            vec![named("cortexkit", "aft", 4)],
        ),
        case(
            "20-reference-links",
            OCCIPITAL,
            "See [PR 14][1], [PR 15][] and [PR 16]; [⟦PR 17⟧] has no definition.\n\n[1]: https://github.com/cortexkit/occipital/pull/14 \"PR 14\"\n  [pr 15]: https://x.dev/15\n[PR  16]: https://x.dev/16\n",
            vec![home(17)],
        ),
        case(
            "21-keyword-spacing-and-escapes",
            OCCIPITAL,
            "pull  request 3, pull\nrequest 4, PR\n12 and PR \\#12 aren't mentions; ⟦Pull Request 5⟧ is.",
            vec![home(5)],
        ),
        case(
            "19-code-span-stops-at-a-blank-line",
            OCCIPITAL,
            "A ` opens nothing here\n\n⟦PR 1⟧ after ` it.",
            vec![home(1)],
        ),
        case(
            "22-mentions",
            None,
            "⟦@CKDESK⟧ and ⟦@ckios⟧, can you check? ⟦@TRIALPROBE-B⟧ too.",
            vec![agent("CKDESK"), agent("ckios"), agent("TRIALPROBE-B")],
        ),
        case(
            "23-mention-punctuation",
            None,
            "Thanks ⟦@ALF⟧. Ask ⟦@SUBC⟧, then ⟦@MC⟧'s room (⟦@AFT⟧); ⟦@ENTO⟧: yes! ⟦@CEREB⟧?",
            vec![
                agent("ALF"),
                agent("SUBC"),
                agent("MC"),
                agent("AFT"),
                agent("ENTO"),
                agent("CEREB"),
            ],
        ),
        case(
            "24-mention-all",
            None,
            "⟦@all⟧ the window moves to 22:00Z; ⟦@ALL⟧ means the same.",
            vec![agent("all"), agent("ALL")],
        ),
        case(
            "25-mention-whole-run",
            None,
            "⟦@ALF-style⟧ and ⟦@ALF_2⟧ are whole names, not ALF; ⟦@ALF-⟧ keeps its dash.",
            vec![agent("ALF-style"), agent("ALF_2"), agent("ALF-")],
        ),
        case(
            "26-mention-not-in-a-word",
            None,
            "Mail ufuk@example.com or a_@B; a lone @ and @. name nobody, nor \\@ESCAPED. ⟦@ÇAĞRI⟧ does.",
            vec![agent("ÇAĞRI")],
        ),
        case(
            "27-mention-not-in-code-or-links",
            OCCIPITAL,
            "`@CKDESK` in a span, [@CKIOS](https://x.dev/@CKIOS), <b>, https://x.dev/@SUBC\n\n```\n@MC wakes nobody\n```\n⟦@AFT⟧ does, next to ⟦PR 3⟧.",
            vec![agent("AFT"), home(3)],
        ),
        case(
            "28-mention-emphasis",
            None,
            "**⟦@CKDESK⟧** and *⟦@CKIOS⟧* and >⟦@ALF⟧ quote; <⟦@ENTO⟧> is text, not a tag; _@AFT_ is not, as `_` is part of a word.",
            vec![agent("CKDESK"), agent("CKIOS"), agent("ALF"), agent("ENTO")],
        ),
    ]
}

/// The text without its marks, and the byte ranges the marks held.
fn unmark(marked: &str) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut ranges = Vec::new();
    let mut open = None;
    let mut rest = marked;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix(OPEN) {
            assert!(open.is_none(), "nested mark in {marked:?}");
            open = Some(text.len());
            rest = after;
        } else if let Some(after) = rest.strip_prefix(CLOSE) {
            ranges.push((open.take().expect("close without open"), text.len()));
            rest = after;
        } else {
            let next = rest.chars().next().unwrap();
            text.push(next);
            rest = &rest[next.len_utf8()..];
        }
    }
    assert!(open.is_none(), "unclosed mark in {marked:?}");
    (text, ranges)
}

#[test]
fn every_case_finds_exactly_its_marked_mentions() {
    for case in cases() {
        let (text, ranges) = unmark(case.marked);
        assert_eq!(
            ranges.len(),
            case.expected.len(),
            "{}: marks and expectations",
            case.name
        );
        let expected: Vec<Reference> = ranges
            .iter()
            .zip(&case.expected)
            .map(|(&(start, end), (kind, target))| Reference {
                start,
                end,
                kind,
                target: target.clone(),
            })
            .collect();
        let project = Project {
            repository: case.repository,
        };
        assert_eq!(
            references(&text, &project),
            expected,
            "{}: {text:?}",
            case.name
        );
    }
}

/// Text full of brackets, backticks and fences that never close still takes
/// time in proportion to its length, not its square.
#[test]
fn unclosed_markup_stays_linear() {
    let project = Project {
        repository: OCCIPITAL,
    };
    for unit in ["[", "` ", "[x](", "<a ", "PR 1 "] {
        let text = unit.repeat(200_000);
        let started = std::time::Instant::now();
        let _ = references(&text, &project);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{unit:?} took {:?}",
            started.elapsed()
        );
    }
}

fn json_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

const README: &str = "# text-references-v2

Test vectors for `cortexkit-occipital-refs` rules version 2. They are
generated by `crates/occipital-refs/src/tests.rs`; do not edit by hand.
`SHA256SUMS` lists the SHA-256 of every other file here
(`sha256sum -c SHA256SUMS`). Run the crate's tests with
`OCCIPITAL_BLESS_VECTORS=1` to regenerate them; without it a test fails if
regenerating would change any byte.

`cases.json` is a list of cases:

    {\"name\", \"project\", \"text\", \"references\": [{\"start\", \"end\", \"kind\", \"target\"}]}

- `project` is the project record as core serves it beside an agent, or
  `null` for text with no project (a room post by the person). Only its
  `repository` member matters to the rules.
- `start` and `end` are UTF-8 byte offsets into `text`; the mention is
  `text[start..end]`.
- Replaying a case means calling the rules with `text` and `project` and
  getting exactly `references`, in order.

The rules are in `crates/occipital-refs/README.md`.
";

fn generate() -> BTreeMap<String, Vec<u8>> {
    let mut out = String::from("[\n");
    let all = cases();
    for (index, case) in all.iter().enumerate() {
        let (text, _) = unmark(case.marked);
        let project = Project {
            repository: case.repository,
        };
        out.push_str("  {\"name\": ");
        json_string(&mut out, case.name);
        out.push_str(", \"project\": ");
        match case.repository {
            None => out.push_str("null"),
            Some(repository) => {
                out.push_str("{\"id\": \"pj-test\", \"repository\": {\"host\": ");
                json_string(&mut out, repository.host);
                out.push_str(", \"owner\": ");
                json_string(&mut out, repository.owner);
                out.push_str(", \"name\": ");
                json_string(&mut out, repository.name);
                out.push_str("}}");
            }
        }
        out.push_str(",\n   \"text\": ");
        json_string(&mut out, &text);
        out.push_str(",\n   \"references\": [");
        for (at, reference) in references(&text, &project).iter().enumerate() {
            if at > 0 {
                out.push_str(", ");
            }
            let _ = write!(
                out,
                "{{\"start\": {}, \"end\": {}, \"kind\": ",
                reference.start, reference.end
            );
            json_string(&mut out, reference.kind);
            out.push_str(", \"target\": ");
            json_string(&mut out, &reference.target);
            out.push('}');
        }
        out.push_str("]}");
        out.push_str(if index + 1 < all.len() { ",\n" } else { "\n" });
    }
    out.push_str("]\n");
    let mut files = BTreeMap::new();
    files.insert("cases.json".to_owned(), out.into_bytes());
    files.insert("README.md".to_owned(), README.as_bytes().to_vec());
    let mut sums = String::new();
    for (name, bytes) in &files {
        let digest = hmac_sha256::Hash::hash(bytes);
        for byte in digest {
            let _ = write!(sums, "{byte:02x}");
        }
        let _ = writeln!(sums, "  {name}");
    }
    files.insert("SHA256SUMS".to_owned(), sums.into_bytes());
    files
}

/// The tagged version 1 set is never rewritten: its files still match its own
/// checksums. Version 2 adds `@NAME`, so replaying it under today's rules
/// would differ wherever its text has an `@`.
#[test]
fn the_version_1_vectors_are_unchanged() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/text-references-v1");
    let sums = fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 2);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let mut digest = String::new();
        for byte in hmac_sha256::Hash::hash(&fs::read(dir.join(name)).unwrap()) {
            let _ = write!(digest, "{byte:02x}");
        }
        assert_eq!(digest, expected, "{name}");
    }
}

#[test]
fn the_vectors_are_current() {
    let files = generate();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/text-references-v2");
    if std::env::var_os(BLESS).is_some() {
        fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in &files {
            fs::write(dir.join(name), bytes).unwrap();
        }
        return;
    }
    for (name, bytes) in &files {
        let on_disk = fs::read(dir.join(name))
            .unwrap_or_else(|error| panic!("{name}: {error}; run the tests with {BLESS}=1"));
        assert!(
            on_disk == *bytes,
            "{name} differs from a regeneration; run the tests with {BLESS}=1 and review the result"
        );
    }
}
