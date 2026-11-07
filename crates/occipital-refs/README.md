# cortexkit-occipital-refs

Finds mentions of issues and pull requests in an agent's text, so the
CortexKit apps can draw them as links. "PR #499" in a reply becomes a link to
pull request 499 of the repository the agent works in.

The text is never changed. An app passes the Markdown source it is about to
draw and the project the text belongs to, and gets back byte ranges, each
with a kind and a URL. Drawing a link over a range changes colour, not words,
so copies, quotes and row heights stay as they were.

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

## Rules, version 1

`RULES_VERSION` is 1. Test vectors pinning these rules are in
`vectors/text-references-v1/` in the repository, tagged
`text-references-v1`.

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

| kind | text | links to |
|---|---|---|
| `issue` | `Issue N`, `issue #N`, `PR N`, `PR #N`, `pull request N`, keywords in any letter case | issue or pull request N in the project's repository; nothing without one |
| `repository_issue` | `owner/repo#N` | issue or pull request N in `owner/repo`, whatever the project |

The range covers the whole mention, from the first letter of the keyword or
of `owner` to the last digit.

- **Keywords** are `issue`, `pr` and `pull request`, matched without regard
  to ASCII case. One or more spaces (U+0020) follow the keyword, then an
  optional `#`, then N. A tab or no space at all (`PR#5`) is not a mention.
  `issues 4` is not one either.
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
- **Markdown inline links and images,** `[label](destination)` and
  `![alt](destination)`, label and destination both: a mention the agent
  linked already keeps the agent's link. Brackets with no destination
  (`[PR 14]`) are searched.
- **HTML tags and autolinks,** from a `<` followed by a letter, `/`, `!` or
  `?` to the next `>` on the same line. This covers the apps' rich UI tags.
- **Bare URLs,** from `http://` or `https://` (any case) at the start of a
  word to the next blank.

A paragraph ends at a line holding only blanks; no code span or link crosses
one.

## Later versions

New kinds come as new rules reading new optional members of the project
record, such as another tracker's ticket ids once a project names its
tracker. Each changes `RULES_VERSION` and ships new tagged vectors. An app
that meets a kind it doesn't know draws it as a plain link to its `target`.
