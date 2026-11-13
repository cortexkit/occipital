# cortexkit-occipital-refs

Finds mentions of issues, pull requests and agents in text, so the CortexKit
apps can draw them as links and chips. "PR #499" in a reply becomes a link to
pull request 499 of the repository the agent works in, and `@CKDESK` names the
agent CKDESK.

The text is never changed. An app passes the Markdown source it is about to
draw and the project the text belongs to, and gets back byte ranges, each
with a kind and a target: a URL, or a name. Drawing over a range changes how
it looks, not its words, so copies and quotes stay as they were.

```rust
use occipital_refs::{references, Project, Repository, ISSUE};

let project = Project {
    repository: Some(Repository { host: "github.com", owner: "cortexkit", name: "occipital" }),
};
let text = "Fixed in PR #499; see `PR 12` for the old attempt.";
let found = references(text, &project);
assert_eq!(found.len(), 1);
assert_eq!(&text[found[0].start..found[0].end], "PR #499");
assert_eq!(found[0].kind, ISSUE);
assert_eq!(found[0].target, "https://github.com/cortexkit/occipital/issues/499");
```

The crate has no dependencies, so depending on it never changes the features
of an app's other crates.

## Rules, version 2

`RULES_VERSION` is 2. Test vectors pinning these rules are in
`vectors/text-references-v2/` in the repository, tagged
`text-references-v2`. Version 1 had no `mention` kind; its vectors stay in
`vectors/text-references-v1/`, tagged `text-references-v1`.

### The project

`Project` holds what the text's project tells the rules: its `repository`,
`{host, owner, name}`, from the project record core serves beside each agent.
Text with no project (a room post by the person) passes a project with no
repository. A room post by an agent passes its author's project.

Version 1 builds URLs for `github.com` only:
`https://github.com/<owner>/<name>/issues/<N>`. GitHub sends an issue URL on
to the pull request when N is one, so one form serves both. A repository on
another host gets no links of either kind until a later version knows the
host. A repository whose owner or name GitHub wouldn't allow (see below) gets
no `issue` links.

### Mentions

| kind | text | target |
|---|---|---|
| `issue` | `Issue N`, `issue #N`, `PR N`, `PR #N`, `pull request N`, keywords in any letter case | the URL of issue or pull request N in the project's repository; nothing without one |
| `repository_issue` | `owner/repo#N` | the URL of issue or pull request N in `owner/repo`, whatever the project |
| `mention` | `@NAME` | the name, without the `@`, as typed |

The range covers the whole mention, from the first letter of the keyword or
of `owner` to the last digit, and from the `@` to the name's last character.

- **Keywords** are `issue`, `pr` and `pull request`, matched without regard
  to ASCII case. In `pull request` exactly one space (U+0020) sits between
  the two words. One or more spaces (U+0020) follow the keyword, then an
  optional `#`, then N. Anything else between them is not a mention: a tab,
  a line break (`PR` at the end of a soft-wrapped line and `12` on the next),
  no space at all (`PR#5`), or a backslash (`PR \#12`, which Markdown draws
  as "PR #12"). `issues 4` is not one either.
- **N** is one to nine ASCII digits, not starting with `0`.
- **A mention starts** at the start of the text or after a character that is
  not a letter, digit or `_` (any script). For `owner/repo#N`, the character
  before may also not be `-`, `.`, `/`, `@`, `:` or `#`, so paths and handles
  (`path/cortexkit/aft#1`, `@me/repo#2`) are not mentions.
- **A mention ends** after N, where the next character is not a letter,
  digit or `_` (any script), and not a `.` or `,` followed by a digit. So
  `PR 3.5`, `Issue 1,000`, `PR 12abc` and `PR 12é` are not mentions, while
  `PR 12.`, `PR 12,` and `PR 12’s` are.
- **`owner`** is letters, digits and `-`, at most 39, not starting with `-`.
  **`repo`** is letters, digits, `-`, `_` and `.`, at most 100, and not `.`
  or `..`. Letters and digits are ASCII.
- **A bare `#N` is never a mention.** In agents' text it is usually a room
  post number.
- **`@NAME`** starts at an `@` that comes at the start of the text or after a
  character that is not a letter, digit or `_` (any script), so neither
  `ufuk@example.com` nor `_@AFT_` (Markdown's underscore emphasis) is one;
  `*@AFT*` is. The name is the longest run after it of
  letters, digits (any script), `_` and `-`; an `@` with none after it is not
  a mention. So `@CKDESK.`, `@CKDESK,` and `@CKDESK's` name `CKDESK`, while
  `@ALF-style` names `ALF-style`, not ALF. `\@NAME` (escaped) is not a
  mention.
- **Names are looked up by the app.** The rules don't know which agents
  exist. An app draws a chip when the name matches, without regard to case,
  an agent it knows, the person's own name, or `all` (`@all` addresses every
  member of a room); any other name stays plain text.

### What is never searched

- **Fenced code blocks.** A line whose first non-blank characters are three
  or more backticks or tildes opens one, at any indentation, so a fence
  inside a list item counts. A backtick fence's info string has no backtick.
  The block ends at a line of the same character, at least as long, with only
  blanks after it; an unclosed block runs to the end of the text. Indented
  code blocks (four spaces, no fence) are searched like prose.
- **Code spans.** A run of backticks opens one, closed by the next run of the
  same length in the same paragraph. A run that nothing closes is plain text.
  A backtick after a backslash opens nothing.
- **Markdown links and images,** label and destination both, so a mention
  the agent linked already keeps the agent's link: inline links
  `[label](destination)` and `![alt](destination)`; full and collapsed
  references `[label][ref]` and `[label][]`; and a shortcut reference
  `[label]` whose label the text defines. Labels match as CommonMark matches
  them: blanks collapsed, ASCII case ignored. Brackets that are none of these
  (`[PR 14]` with no definition) are searched.
- **Link reference definitions,** a line whose first non-blank characters
  are `[label]:`, to the end of that line.
- **HTML tags and autolinks,** from a `<` followed by a letter, `/`, `!` or
  `?` to the next `>` on the same line. This covers the apps' rich UI tags.
- **Bare URLs,** from `http://` or `https://` (any case) at the start of a
  word to the next blank.

A paragraph ends at a line holding only blanks; no code span or link crosses
one.

## Later versions

New kinds come as new rules, some reading new optional members of the
project record, such as another tracker's ticket ids once a project names its
tracker. Each changes `RULES_VERSION` and ships new tagged vectors. An app
that meets a kind it doesn't know draws the text as it is.
