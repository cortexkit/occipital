# Writing tool templates

A tool provider can give each of its tools a rich row in the apps'
chats, and a page the person opens by tapping the row. It does so with a
**view**: its answer to `tool.views` for the tool's pin, the
`tp1:<tool>:<schema digest>:<semantics>` string that names one version of
one tool's definition (the tool-provider contract, §2.1). A view holds the schema of the records the provider writes
for each call (`display.write`, the tool-display contract) and up to two
templates, small programs the apps run to draw the row and the page.

```json
{"record_schema": {"name": "aft.bash", "revision": 1, "fields": {"exitCode": "integer"}},
 "reads": [],
 "template": {"row": {...}, "page": {...}}}
```

Without templates (`"template": {}`), the call draws as the plain chip, and
its records are still kept. A template that fails the checks below is never
drawn either: that call falls back to the plain chip.

## The template document

Each template is an `occipital.template/1` document:

```json
{"schema": "occipital.template/1",
 "profile": "tool_row",
 "executionDialect": "1.0",
 "catalog": {"version": "1.1", "digest": "sha256:..."},
 "reads": [],
 "source": "..."}
```

- `profile` is `tool_row` for `template.row` and `tool_page` for `template.page`.
- `catalog` names the tool catalog. `occipital-check --catalog` prints the exact
  version and digest to put here.
- `reads` is empty: templates make no reads in this version.
- `source` is the program, at most 64 KiB of UTF-8.

To fix or improve a template later, serve the new one with a higher
`template_revision` in a new module version. The record schema and reads of a
pin never change.

## What a template sees

- **`input`:** the call's arguments, typed by the tool's own input schema. A row
  gets them shortened to 2 KiB, field by field. A call whose arguments are over
  1 MiB has no `input` and draws as the plain chip.
- **`view.call`:** `phase` (`running`, `answered` or `unanswered`), and
  `startedAtMs`, `finishedAtMs`, `isError`, `outputBytes` and `unanswered`, each
  of which may be null.
- **`view.record` and `view.completion`:** what the provider wrote for the call,
  or null until it arrives. Each holds the standard fields (`durationMs`,
  `outcome`, `reason`, `queuedMs`, `heldMs`, `location`) and the record
  schema's own fields, each of which may be absent.
- **`view.output` and `view.detail`,** on a page only: the call's whole output
  text, and the detail the provider wrote, either of which may be null.
- **`now` and `zone`:** the time in Unix milliseconds, and the person's time
  zone.

A template sees nothing outside its own call.

## The row rule: `Slot`

A row is measured once, before it is drawn, and never changes height. So
everything that can change while the row is on screen, which is all of `view`
and `now`, is read only inside a `Slot`, a box of fixed size:

- `width`: `short`, `medium` or `long` (12, 24 or 40 digit widths of the body
  text), or `fill` for what the row leaves.
- `lines`: 1 to 3 lines of body text, 1 by default.
- `needs`: the value the content waits for, such as `view.record`. While it is
  null the content isn't evaluated, and the app draws the box as loading, or as
  not kept once the call is long done.

Content that doesn't fit is clipped, and text ends with "…". A read of `view`
or `now` outside a Slot, directly or through a named value, is refused with
`E_UNSLOTTED_READ`. `input` and `zone` may be read anywhere. Pages are measured
each time they open, so they have no such rule.

## Components

- **Rows:** `Badge`, `Styled`, `Card`, `TextContent`, `InlineCode`, `Stat`,
  `Progress`, `Divider` and `Slot`, plus three parts the app draws itself:
  - `Elapsed {start, end?}`: how long the call ran, from Unix milliseconds,
    as `m:ss` or `h:mm:ss`. While `end` is absent the app keeps it ticking.
  - `Agent {id}`: the agent's creature and name, from the app's own list.
  - `Model {id}`: the model's name in words, such as "Opus 5.5 · Anthropic".

  Each has a fixed box and cuts what doesn't fit, and an id the app doesn't
  know shows as the raw id.
- **Pages:** the row components, plus `Callout`, `Table`, `CodeBlock`,
  `CardHeader`, `List`, `ListItem` and the other `tool_page` components, and
  `OpenItem {id, label}`: a link to an ask (`ask_`), an artifact (`art_`) or a
  work item (`wi_`) that opens the app's own card for it. Where the app can't
  open the item, the label shows as plain text.

Numbers read better formatted, with the language's built-ins, which work in
rows and pages alike:

- `FormatBytes(n, digits?)`: binary units, such as "7.6 MiB" or "512 B".
- `FormatDuration(ms, "compact")`: "920 ms", "2.4 s", "1m 5s" or "2h 3m";
  `"clock"` gives "m:ss" or "h:mm:ss".
- `FormatNumber(n, digits?, grouping?)`: thousands grouped with ",".

The tool catalog lists each component's props, types and defaults, in the
`cortexkit-occipital-lang` crate's `catalog/catalog-v1.json` and
`catalog/catalog-tool-v1.1.json`. A block component, such as `Card` or `Slot`,
stands on its own lines.

## Checking

```text
cargo install cortexkit-occipital-registry
occipital-check <file>...
```

Each file holds one view and the input schema of its tool, as
`{"input_schema": {...}, "view": {...}}`. The command prints one line per
template that passes, with its digest, and every problem or diagnostic
otherwise, with its line and column. Occipital runs the same checks when it
fetches a view.

Add sample calls to see each row's height, as
`"samples": [{"input": {...}, "record": {...}, "completion": {...}}]`, with
`record` and `completion` optional. For each sample the command measures the
row at a phone's width (320) and a desktop's (720), before anything is written
for the call and with the sample's record and completion. The heights use
fixed text metrics (every line 20 units, every character 8), so they compare a
row with itself, not with an app. A row whose two heights differ fails, and so
does a sample record the record schema refuses.

The command exits 0 when every template and sample passes, 1 when one
doesn't, and 2 when a file can't be read.

## An example: a shell command

A row with the command, its state and how long it took:

```occipital-row
<Card appearance="clear" direction="row" gap="s" align="center">
<InlineCode code={input.command} />
<Slot width="short">
{view.call.phase == "running" ? <Badge text="Running" tone="info" /> : view.call.isError == true ? <Badge text="Failed" tone="danger" /> : <Badge text="Done" tone="success" />}
</Slot>
<Slot width="short" needs={view.record}>
<TextContent text={FormatDuration(view.record.durationMs ?? 0)} size="small" maxLines={1} />
</Slot>
</Card>
```

And its page:

```occipital-page
<CardHeader title={input.description ?? "Shell command"} />

<CodeBlock language="bash" codeString={input.command} maxLines={12} />

<TextContent text={view.output ?? "No output was kept."} />
```
