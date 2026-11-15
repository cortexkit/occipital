# phone-reader-host-notes-v3

The three notes OpenCode 2 writes into a chat to make the model go on, as the
apps receive them. The page is an OpenCode 2 chat holding Thalamus's recorded
continuation rows (its `host-rows-v2` recording, made on real OpenCode 2.2.3),
each unchanged except that the chat is numbered from ordinal 1 in the order
they were recorded. It is read through Occipital's real request handling, so
the reply is byte for byte what a phone receives. `phone-reader-host-notes-v1`
and `-v2` stay as they were.

OpenCode 2 writes these notes itself, as `synthetic` messages, never the
person. Occipital serves each as a `user` row with empty `text` and one
`injected` part with `host: true`. The runner names the note's wording in the
read's `host_parts` fact, and the part carries it as `hostCode`.

`01-page-host-notes.request.json` is the request the app sends, and
`01-page-host-notes.json` is the reply body it receives. Its three rows:

| Row | `hostCode` | When OpenCode 2 writes it |
| --- | --- | --- |
| 1 | `continued_after_interruption` | The model's reply stream was cut before it finished, and OpenCode asks it to continue. |
| 2 | `continued_after_restart` | The OpenCode server restarted while the model was working. |
| 3 | `continued_after_output_limit` | The model's reply stopped at its output token limit. |

Draw each as a quiet note worded from its code, never as the person's message
and never as a "sent a file" placeholder. The part's `text` is the exact
sentence OpenCode sent the model; it is there for reference, not for drawing.
A host part with a code an app doesn't know, or with none, is drawn as nothing,
as before.

`SHA256SUMS` covers every file except this README. A test in
`occipital-module` (`broca::tests::host_parts`) serves the set again and fails
if any byte changes.
