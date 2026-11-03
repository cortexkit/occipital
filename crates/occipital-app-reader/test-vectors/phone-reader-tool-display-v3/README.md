# phone-reader-tool-display-v3

How an app reads what tool providers wrote for a chat's calls, so it can draw
each call's row and page from the provider's templates.

This set replaces `phone-reader-tool-display-v2`, which was served from an OpenCode 1
chat. Thalamus's runner now hosts OpenCode 2 chats (harness `opencode2`), so this set
is served from an OpenCode 2 chat. The operations, the views, the records and the
templates are the same; only the chat's own bytes differ: message ordinals count from
1, and a part's `partKey` is the runner's key for its slot (`"0"`, `"1"`, …) rather than
an OpenCode 1 part id. Its templates still name the tool catalog digest v2 named, which
stays accepted.

## The page

A `type: "tool"` part may carry four more members. Each is present only when the
runner serves the call's identity (`call_identity`):

- `callKey`: the key the provider's records for the call are filed under.
- `schemaPin`: the tool version whose view draws the call.
- `input`: the call's arguments as an object, for the row template, kept within
  2,048 bytes of compact JSON. When they don't fit, every string longer than
  one length cap, the same cap for all of the call's strings, is cut on a
  character boundary and ends with `…`. The cap is the largest one that makes
  the whole fit. An exact read (`messageOrdinal`) carries the whole arguments.
- `inputTruncatedFields`: the JSON Pointer of each string cut in `input`, in
  member order. Empty when nothing was cut.

A call whose arguments aren't an object, or can't fit even with every string
cut, has no `input` and draws as the plain chip. Without `schemaPin` the call
always draws as the plain chip. Display data never goes inside a page, so a
provider's write never changes a page's `version`.

## The operations

Every operation is a read, admitted like `session.transcript_page`. Each reply
is `{"result": …}`; a refusal is `{code, message, detail?}`.

- `display.views {pins, held?}`: at most 64 pins. Each pin's answer is one of:
  - a view: `{provider, recordSchema, reads, templateRevision, row?, page?}`.
    `row` and `page` are template digests, present only for templates that
    pass Occipital's checks. Keep a view by pin and `templateRevision`.
  - `"unknown"`: the provider has no view for this pin, or more than one
    module claims it. Draw the plain chip.
  - `"unavailable"`: not fetched yet. Draw the plain chip, keep nothing, and
    ask again on the chat's next fresh draw.

  `held` maps a pin to the revision the app holds. A pin held at the kept
  revision or a higher one is left out of the answer.
- `display.template {digest}`: the template document in its canonical form.
  Hash its canonical JSON and compare with the digest before use. Keep it by
  digest for good. A digest no kept view has is refused `not_found`.
- `session.tool_records {session, harness?, callKeys}`: at most 100 call keys.
  `calls` maps each call that has display data to
  `{record?, completion?, detail}`, where `detail` says whether a detail can be
  read. A call with nothing is left out. Records and completions never change
  once served, so only an absent call needs reading again.
  `loadingWindowMs`: show an absent call as loading while
  `now - finishedAtMs` is under it, and as not kept after that.
- `session.tool_detail {session, harness?, callKey}`: `{detail}`, or `{}` when
  the call has none. Read it when the person opens the call.
- `session.tool_records_watch {session, harness?}`: a watch. Its notices carry
  no data:
  - the first notice, `{"callKeys": []}`, says the watch is listening. Read
    the shown calls' records after it, and nothing written meanwhile is
    missed;
  - `{"callKeys": [...]}` names calls of the session that got new data. Read
    those calls again;
  - `{"pins": [...]}` names pins with a newer template revision. Ask
    `display.views` for them before the chat's next fresh draw.

  When the watch ends, reopen it and read the shown calls again.

A write joins a call only through a view its own provider serves for the pin
the runner stored for the call. A call two providers could join shows neither.

## Where the files come from

Occipital's app reader served these replies at the commit that adds this
folder, through the same serving path a phone's route takes. The session is a
test session on Thalamus's OpenCode 2 runner (harness `opencode2`) with four calls,
served with their keys and pins. The display store holds the views and writes a
provider would have sent, so the bytes are fixed. The pins' digests are a
repeated hex digit. Each `.json` file is the whole response body or watch
notice, and each `.request.json` file is the request's `{method, params}`. The
test `app_reader::broca::tests::tool_display_samples` in `occipital-module`
serves every request again and fails if a reply's bytes change, so a reader
change that alters them needs a new sample set, never an edit to this one.

## Cases

| Files | What it shows |
| --- | --- |
| `01-page-with-call-keys` | The page: four calls, each with `callKey` and `schemaPin`. The `bash` command is too long for a row, so `input.command` is cut and `inputTruncatedFields` is `["/command"]`. The other inputs are whole. |
| `02-views` | `bash`: a view at revision 1 with a row and a page. `edit`: a view at revision 0. `read`: `"unknown"`, since its provider has no view. `new`: `"unavailable"`, not fetched yet. `twice`: `"unknown"`, since two modules claim it. |
| `03-views-held` | `bash` held at revision 1 is left out; `edit` is answered. |
| `04-template` | The `bash` row template by the digest `02` gave. |
| `05-records` | `ck_ls`: a `held` record and its completion. `ck_edit`: a record, with a detail to read. `ck_read` (no view), `ck_two` (two providers) and `ck_missing` (unknown call) are left out. Another module's write for `ck_ls`, from a module with no view of its pin, is ignored. |
| `06-detail` | The `edit` call's detail: a diff. |
| `07-records-watch-open` | The watch's first notice. |
| `08-records-watch-hint` | A provider wrote for `ck_read`, so the watch names it. Writes for a call this session's pages never carried, and for the same key under another runner, aren't named. |
| `09-records-watch-pins` | A newer template revision was kept for the `bash` pin. |
| `10-views-too-many-pins` | 65 pins: refused `invalid_params`, naming `pins`. |
| `11-template-not-found` | A digest no kept view has: refused `not_found`. |
