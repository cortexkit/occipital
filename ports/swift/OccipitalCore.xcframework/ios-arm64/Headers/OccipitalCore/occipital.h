/*
 * occipital.h: the C ABI of occipital-ffi, the static library that embeds
 * the apps' rich UI language (occipital-lang) in an app.
 *
 * This header is written by hand. It is the contract; the Rust entry points
 * in crates/occipital-ffi/src follow it, not the other way round.
 *
 * CONVENTIONS
 *
 *   Every entry point returns an int32_t status (OCC_OK or one of the codes
 *   below), except the _free functions, which return void.
 *
 *   Strings passed in are NUL-terminated UTF-8 `const char *` unless a byte
 *   length is given next to them. Byte buffers (`const uint8_t *` plus a
 *   `size_t` length) are passed to the library as they are, without UTF-8
 *   validation; a NULL buffer is allowed when its length is 0.
 *
 *   Results come back through a final `out` parameter: either `char **out`,
 *   a Rust-owned NUL-terminated JSON string, or a handle pointer such as
 *   `OccAccount **out`. An entry given a non-NULL `out` sets *out to NULL
 *   before doing any other work, so *out is never left holding an old
 *   value.
 *
 * PAYLOAD RULES (what *out holds for each status)
 *
 *   OCC_OK                      *out holds the result.
 *   OCC_REFUSED, OCC_REJECTED   *out holds an error JSON object describing
 *                               the refusal or the rejecting diagnostics
 *                               (entries with a `char **out` only).
 *   every other status          *out is NULL.
 *
 *   Entries whose *out is a handle (occ_account_new, occ_reply_new) cannot
 *   carry error JSON there, so they leave *out NULL on every non-OK status.
 *
 *   Error JSON is {"error": "refused", "refused": {"kind": ...}} for
 *   OCC_REFUSED, where kind is "aggregate" (with the exhausted "scope":
 *   "interface", "reply", "chat" or "app"), "scope" (an account of the wrong
 *   scope: "handle" is the role it was passed for, "actual" its scope),
 *   "unit_fuel", "reservation" (with a "reason"), "arithmetic_overflow",
 *   "missing_retention" or "unsupported_mode"; and
 *   {"error": "rejected", "diagnostics": [diagnostic...]} for OCC_REJECTED.
 *
 * OUTPUT JSON
 *
 *   A reply revision is {"id", "provisional", "tree", "literals": [span...],
 *   "diagnostics": [diagnostic...], "plain_text"}; a source span is
 *   {"line", "column", "endLine", "endColumn"}.
 *
 *   A measurement is {"height", "key", "known_work_visits", "reservation",
 *   "boxes", "row_frame"}. A height is {"known": "<decimal>"} or
 *   "prose_deferred". "key" is the layout key {"instance_epoch", "revision",
 *   "state_hash", "width", "content_size_category", "catalog_digest",
 *   "layout_version", "network", "placement"}. "reservation" is "deferred"
 *   or {"abort": "<code>"}. Each box is {"path", "height", "width",
 *   "state"}, with state "ready", "loading", "held", "unavailable" or
 *   {"error": "<code>"}. "row_frame" is null or {"baseline", "height",
 *   "clipped"}.
 *
 * TREE JSON (the language spec's §14.3 tree ABI; this library is its
 * reference encoder)
 *
 *   The root is {"m": "root", "k": [block...]}. A component is
 *   {"c": name, "p": {props}, "k": [child...]}, with the props in the tree's
 *   order (source order, then bindings) and unique keys. An error cell is
 *   {"c": name, "e": code}, an inline error marker {"e": code}, text
 *   {"t": text} and a reference chip {"r": "7"}. A Markdown node is
 *   {"m": kind, ..., "k": [child...]}, with kind p, h (with "level"), hr,
 *   codeblock (with "lang" and "text"), code (with "text"), quote, list
 *   (with "ordered", and "start" when ordered), item, task (with
 *   "checked"), table, thead, row, cell, em, strong, del, link (with
 *   "href", the destination as written, "title" and "image"), softbreak or
 *   br.
 *
 *   A prop value is null, a boolean, a string or an array (a list) as
 *   itself, or an object with one member naming its kind:
 *   {"number": "<decimal>"}, {"record": {fields in order}}, {"node": node},
 *   {"action": [{"op": "set"|"reset"|"refresh"|"load_more"|"compose"|
 *   "open", "args": [value...]}...]}, {"snapshot": {"record_id", "state",
 *   "value"}} ("value" absent when nothing was parsed), {"handleRecord": id},
 *   {"site": {"name"}}, {"binding": {"state": "$name", "type"}} or
 *   {"error": code}.
 *
 *   Tree JSON passed in decodes closed like every envelope, and every text
 *   value in it (names, text, prop keys and string values) is sanitized
 *   again as display text. Nesting deeper than 128 JSON
 *   levels is OCC_INVALID_ARGUMENT.
 *
 * INPUT RULES
 *
 *   A NULL required pointer, a JSON parameter that is not valid UTF-8, a
 *   JSON parameter that does not parse, and a JSON envelope with a member
 *   it does not define or without a member it requires all return
 *   OCC_INVALID_ARGUMENT. Envelopes are
 *   closed: unknown members are an error, never ignored. Decimals in JSON
 *   are canonical decimal strings (such as "390" or "12.5"), byte strings
 *   are base64 strings, and enumerations are lower-case strings.
 *
 * OWNERSHIP
 *
 *   Every string returned through `char **out` is owned by the library and
 *   must be freed exactly once with occ_string_free. Every handle must be
 *   freed exactly once with its own _free function. Passing NULL to any
 *   _free function, including occ_string_free, does nothing; this is the
 *   only place a NULL pointer is accepted where an object is expected.
 *
 *   A reply session holds its own reference to each account it was created
 *   with, so an account handle may be freed while sessions created from it
 *   are still in use.
 *
 * THREAD RULES
 *
 *   A reply session (OccReply) has one caller at a time: calls on the same
 *   session must not overlap, although successive calls may come from
 *   different threads. Account handles (OccAccount) may be shared across
 *   threads and used by concurrent calls. A handle may only be freed after
 *   every call that uses it has returned. Every other entry point may be
 *   called from any thread, except from inside a metrics callback (see
 *   OccMetrics).
 *
 * PANICS
 *
 *   No panic crosses this boundary. Every entry point that returns a
 *   status catches a panic in the library and returns OCC_PANIC, with *out
 *   NULL where it has an `out`; the void _free functions catch a panic and
 *   discard it. Either way the process keeps running. After OCC_PANIC on a
 *   session, free the session rather than keep using it.
 */

#ifndef OCCIPITAL_H
#define OCCIPITAL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------------
 * Status codes
 * --------------------------------------------------------------------- */

/* Success. *out holds the result. */
#define OCC_OK 0

/* A required pointer was NULL, a JSON parameter was not valid UTF-8 or did
 * not parse, an envelope had an unknown or missing member, or a value was
 * out of its documented range (such as an unknown account scope).
 * *out is NULL. */
#define OCC_INVALID_ARGUMENT 1

/* An account or limit refused the work. *out holds the refusal as an
 * error JSON object (NULL for handle-returning entries). */
#define OCC_REFUSED 2

/* The library rejected the input with diagnostics. *out holds the
 * diagnostics as an error JSON object. */
#define OCC_REJECTED 3

/* A metrics callback returned NaN, a negative value, an infinity or a value
 * outside the decimal range, or the library could not measure the tree (a
 * tree that breaks a catalog shape, or a component the app measures
 * itself). The measurement was discarded and no session state changed.
 * *out is NULL. */
#define OCC_MEASURE_ERROR 4

/* The library panicked. The panic was caught and the process is still
 * running. *out is NULL. */
#define OCC_PANIC 5

/* The call is not allowed in the session's current state: finishing a
 * reply twice, sending an event before finishing, and the other session
 * state errors. *out is NULL. */
#define OCC_SESSION_STATE 6

/* An internal error in the library (a broken invariant reported as an
 * error instead of a crash). *out is NULL. */
#define OCC_INTERNAL 7

/* ------------------------------------------------------------------------
 * Account scopes for occ_account_new
 * --------------------------------------------------------------------- */

/* One account per chat window, shared by every reply in that window. */
#define OCC_SCOPE_CHAT 0

/* One account per app process, shared by all rich UI in the app. */
#define OCC_SCOPE_APP 1

/* ------------------------------------------------------------------------
 * Opaque types
 * --------------------------------------------------------------------- */

/* A shared chat or app account that every rich UI instance reserves work
 * from. Thread-safe. */
typedef struct OccAccount OccAccount;

/* One streaming reply session. One caller at a time. */
typedef struct OccReply OccReply;

/* ------------------------------------------------------------------------
 * Text metrics callbacks
 * --------------------------------------------------------------------- */

/*
 * The platform text measurements layout needs. `ctx` is passed back to
 * every callback unchanged. Lengths are in points.
 *
 * `style` is the text style ordinal:
 *   0 Body, 1 Subheadline, 2 Footnote, 3 Caption1, 4 Headline,
 *   5 Title2, 6 Title3, 7 Mono, 8 MonoFootnote.
 *
 * `text` is UTF-8 of `len` bytes and is not NUL-terminated.
 *
 * l   returns the height of one line in `style`.
 * t   returns the height of `text` wrapped to `width`, cut at `max_lines`
 *     lines; `max_lines` is -1 for no limit.
 * tw  returns the width of `text` set on one line.
 *
 * Each returned double is rounded up to 10^-6 pt, so measured text is
 * never shorter than drawn text. NaN, a negative value, an infinity or a
 * value outside the decimal range makes the measuring entry return
 * OCC_MEASURE_ERROR. Callbacks must never call back into this library.
 * A NULL table or a NULL callback is OCC_INVALID_ARGUMENT.
 */
typedef struct OccMetrics {
    void *ctx;
    double (*l)(void *ctx, int32_t style);
    double (*t)(void *ctx, const char *text, size_t len, int32_t style,
                double width, int32_t max_lines);
    double (*tw)(void *ctx, const char *text, size_t len, int32_t style);
} OccMetrics;

/* ------------------------------------------------------------------------
 * Version
 * --------------------------------------------------------------------- */

/*
 * The version descriptor of what this library implements, as JSON:
 *   {"executionDialect": "1.0",
 *    "catalog": {"version": "...", "digest": "<hex>", "layout": "...",
 *                "aliases": ["<hex>", ...]},
 *    "toolCatalog": {"version": "1.1", "digest": "<hex>", "layout": "...",
 *                    "aliases": ["<hex>", ...]}}
 * `catalog` is the embedded catalog: `digest` is its digest and `layout`
 * its layout version. `aliases` are earlier digests of the same catalog,
 * from before a change to its descriptive text only: a stored reply whose
 * rich UI stamp names `digest` or any alias draws with this catalog.
 * `toolCatalog` is the catalog tool templates name: a template whose
 * `catalog.digest` is "sha256:" followed by its `digest` or one of its
 * `aliases` is for this library; one naming any other digest is not. Its
 * aliases are earlier digests of the tool catalog, which it only grew from
 * (the narrow Slot width), so a template naming one draws as it always did.
 */
int32_t occ_version(char **out);

/* ------------------------------------------------------------------------
 * Accounts
 * --------------------------------------------------------------------- */

/*
 * A fresh account: `scope` is OCC_SCOPE_CHAT (0) or OCC_SCOPE_APP (1). Any
 * other scope returns OCC_INVALID_ARGUMENT. Free with occ_account_free.
 */
int32_t occ_account_new(int32_t scope, OccAccount **out);

/* Frees an account handle. NULL does nothing. */
void occ_account_free(OccAccount *account);

/* ------------------------------------------------------------------------
 * Reply sessions
 * --------------------------------------------------------------------- */

/*
 * A session for one reply under the reply profile and the embedded
 * catalog. `stamp_json` is {"dialect": "..."}; `layout_json` is
 * {"width", "content_size_category", "network": "yes"|"no"|"unknown",
 *  "placement": "row"|"page"}. `chat` and `app` are the shared accounts
 * the reply reserves from; the session keeps its own references to them.
 * An account that refuses the session (exhausted, or `chat` and `app` not
 * a chat and an app account) is OCC_REFUSED, with *out NULL. Free with
 * occ_reply_free.
 */
int32_t occ_reply_new(const char *stamp_json, const char *layout_json,
                      const OccAccount *chat, const OccAccount *app,
                      OccReply **out);

/*
 * Feeds streamed reply bytes, passed to the library unvalidated: a UTF-8
 * sequence may be split across calls and the finish tail. *out is the
 * provisional revision JSON, or JSON `null` when nothing visible changed.
 * After the reply is finished this is OCC_SESSION_STATE.
 */
int32_t occ_reply_chunk(OccReply *reply, const uint8_t *bytes, size_t len,
                        char **out);

/*
 * Feeds the final bytes and finishes the reply with the records stamped at
 * store time. `records_json` is
 * {"references": {"N": ReferenceRecord}, "handles": [HandleRecord]}, both
 * required; N is written canonically ("7", not "07"). *out is the
 * authoritative revision JSON. Finishing twice is OCC_SESSION_STATE.
 */
int32_t occ_reply_finish(OccReply *reply, const uint8_t *tail, size_t len,
                         const char *records_json, char **out);

/*
 * Delivers a late host event to a finished reply. `event_json` is one
 * event, tagged by "kind":
 *   {"kind": "snapshot", "reference": N, "result": {"ok": "<base64>"}}
 *     or with "result": "failed" for a failed fetch;
 *   {"kind": "handle_availability", "record_id": "...", "available": bool};
 *   {"kind": "account_release"};
 *   {"kind": "layout", "layout": layout_json}.
 * There is no measure_complete event: occ_reply_measure delivers
 * measurements. *out is the resulting revision JSON, or JSON `null` when
 * nothing changed. An event other than "layout" before the reply is
 * finished is OCC_SESSION_STATE.
 */
int32_t occ_reply_event(OccReply *reply, const char *event_json, char **out);

/*
 * Measures the session's current candidate revision under `layout_json`
 * with `metrics`. Only a successful measurement is delivered to the
 * session, which commits it when `layout_json` is the session's current
 * layout. *out is the measurement JSON. A session with no revision yet is
 * OCC_SESSION_STATE.
 */
int32_t occ_reply_measure(OccReply *reply, const char *layout_json,
                          const OccMetrics *metrics, char **out);

/* Frees a reply session. NULL does nothing. */
void occ_reply_free(OccReply *reply);

/* ------------------------------------------------------------------------
 * Measurement
 * --------------------------------------------------------------------- */

/*
 * Measures one component node of `tree_json` (tree JSON) as a one-block
 * tree. `path_json` is a non-empty JSON array of child indices: the first
 * picks one of the root's blocks, each later one a child ("k") of the node
 * before. A path that names no node, or a node that is not a component, is
 * OCC_INVALID_ARGUMENT. *out is the measurement JSON. A tree measured
 * outside a session has layout key instance epoch 0, revision 0 and an
 * empty state hash.
 */
int32_t occ_measure_block(const char *tree_json, const char *path_json,
                          const char *layout_json, const OccMetrics *metrics,
                          char **out);

/* Measures a whole tree (tree JSON), as occ_measure_block measures one
 * block. *out is the measurement JSON. */
int32_t occ_measure_tree(const char *tree_json, const char *layout_json,
                         const OccMetrics *metrics, char **out);

/* ------------------------------------------------------------------------
 * Programs
 * --------------------------------------------------------------------- */

/*
 * Checks program source bytes and runs the trial evaluation (language spec
 * §12.2). Invalid UTF-8 is repaired and reported as a W_ENCODING
 * diagnostic, not rejected. A NULL `src` is allowed when `len` is 0.
 *
 * `ctx_json` is {"context": context, "host": host}; "host" may be left
 * out, and then the default host values are used (network true, now "0",
 * zone "UTC", no data and no snapshots).
 *
 *   context is {"profile": "reply"|"show_ui"|"tool_row"|"tool_page",
 *     "execution_dialect", "catalog": "1.0"|"1.1", "registry": {name:
 *     operation}, "tool": null or {"input", "view", "truncated",
 *     "truncated_fields"}, "show_ui": {"title", "summary", "data_format",
 *     "data_columns"}, "references": {"N": ReferenceRecord}, "handles":
 *     [HandleRecord]}, every member required but "catalog". "catalog" is the
 *     catalog version the program names: "1.1", the tool catalog, for a tool
 *     template, and "1.0" when left out.
 *   host is {"network": bool,
 *     "now": "<canonical decimal ms since the Unix epoch>", "zone",
 *     "data": null or {"record_id", "record": ReferenceRecord,
 *     "bytes": null or "<canonical base64>"},
 *     "snapshots": {"N": "<canonical base64>"}}, every member required.
 *
 * A completed check returns OCC_OK even when it found error diagnostics;
 * *out is the check result JSON {"ok", "language", "executionDialect",
 * "approvalChecked", "requires", "diagnostics", "omitted", "trial":
 * {"fuel", "nodes", "outcome"}}. A check refused by an account is
 * OCC_REFUSED, and a host value that cannot become a language value is
 * OCC_INVALID_ARGUMENT.
 */
int32_t occ_program_check(const uint8_t *src, size_t len, const char *ctx_json,
                          char **out);

/*
 * Checks a tool template (profile "tool_row" or "tool_page", catalog
 * "1.1") as occ_program_check does, then evaluates it once with the
 * context's "tool" input and view and the host's now and zone. A tool
 * template declares no sites, so one evaluation is all a row or page needs;
 * evaluate again when the view changes.
 *
 * *out is {"tree": tree JSON or null, "diagnostics", "outcome"}, where
 * "outcome" is "ok" or the abort that stopped the evaluation, as in the
 * check result's trial; an aborted evaluation has a null tree and is still
 * OCC_OK. A template with an error diagnostic is OCC_REJECTED with its
 * diagnostics, and any other profile is OCC_INTERNAL.
 */
int32_t occ_template_evaluate(const uint8_t *src, size_t len,
                              const char *ctx_json, char **out);

/* ------------------------------------------------------------------------
 * Text references
 * --------------------------------------------------------------------- */

/*
 * Finds the issue and pull request mentions in `text` (`len` bytes of UTF-8,
 * the Markdown source the app is about to draw), so the app can draw them as
 * links without changing the text. The rules are those of the
 * cortexkit-occipital-refs crate (its README); "rules" in the answer is
 * their version.
 *
 * `project_json` is the project record core serves beside the text's agent,
 * passed as it arrived (members other than "repository" are ignored), or
 * JSON null for text with no project, such as a room post by the person.
 *
 * *out is {"rules": 1, "references": [{"start", "end", "kind", "target"}]},
 * in text order, none overlapping. "start" and "end" are UTF-8 byte offsets
 * into `text`, "kind" is "issue" or "repository_issue" (draw a kind you
 * don't know as a plain link), and "target" is the URL. Text that is not
 * UTF-8 is OCC_INVALID_ARGUMENT.
 */
int32_t occ_text_references(const uint8_t *text, size_t len,
                            const char *project_json, char **out);

/* ------------------------------------------------------------------------
 * Facts
 * --------------------------------------------------------------------- */

/*
 * Words one fact a module reports, such as a fact in a command's preview,
 * so both apps draw it alike. `fact_json` is the fact as the module sent it:
 * {"label", "value", "unit"?, "noun"?, "approx"?, "tone"?} for a number,
 * or {"label", "state", "progress"?} for a state. The rules are those of
 * the cortexkit-occipital-lang crate's format_fact.
 *
 * *out is {"label", "text", "tone"}, such as {"label": "Removes", "text":
 * "about 18k tokens", "tone": "neutral"}, where "tone" is one of
 * neutral, info, success, warning, danger. A fact the rules refuse answers
 * {"error": "fact_malformed" | "fact_conflict" | "fact_range"}, and the
 * app shows it as missing. Text that is not JSON is OCC_INVALID_ARGUMENT.
 */
int32_t occ_format_fact(const char *fact_json, char **out);

/* ------------------------------------------------------------------------
 * Strings
 * --------------------------------------------------------------------- */

/* Frees a string returned through `char **out`. NULL does nothing. */
void occ_string_free(char *s);

/* ------------------------------------------------------------------------
 * Test hooks
 *
 * Present only in builds with the crate's non-default `test-hooks` feature,
 * never in the shipped archive. Define OCC_TEST_HOOKS to declare them.
 * --------------------------------------------------------------------- */

#ifdef OCC_TEST_HOOKS

/* Panics inside the library and returns OCC_PANIC with the process alive. */
int32_t occ_test_force_panic(void);

/*
 * Decodes `layout_json` exactly as the layout parameters of the entries
 * above do, and returns the decoded layout re-encoded as canonical JSON.
 */
int32_t occ_test_decode_layout(const char *layout_json, char **out);

#endif /* OCC_TEST_HOOKS */

#ifdef __cplusplus
}
#endif

#endif /* OCCIPITAL_H */
