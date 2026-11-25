# phone-reader-head-rich-v4

A reply stamped with a reply catalog digest no reader knows, as the apps
receive it. Served through Occipital's real request handling by
`occipital-module`'s `broca::tests::rich_ui`, so the reply is byte for byte
what a phone receives. `01-newest-page.request.json` is the request the app
sends and `01-newest-page.json` the reply body. `SHA256SUMS` lists every file
but this README.

Every app meets such a reply: one written after a catalog change the app
hasn't taken yet, and, after it has, one written before that change. Occipital
serves the stamp as the runner stored it; whether a digest is known is the
app's own question.

Row 1 is the person's question. Row 2 is the reply, with `richUi`
`{"dialect": "1.0", "catalogDigest": "0123456789abcdef…"}` (a digest no
catalog has). Its `text` is Markdown with one display tag in it:

- bold, a bulleted list, inline code and a link;
- a Markdown table;
- a fenced code block whose lines hold a tab;
- a `Badge` after the first sentence.

Draw it as an app draws a reply stamped with a digest it doesn't hold: the
language library's readable text, drawn as Markdown, with the `Badge` as its
label in words. Nothing in it may show as typed: no `**`, no list dashes, no
`[label](url)`, no pipes, no tag. The code block keeps its lines and tabs as
written.
