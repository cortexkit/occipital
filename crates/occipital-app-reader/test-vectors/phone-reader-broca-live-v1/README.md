# Phone reader live-text samples from a real Broca worker (live-v1)

These are the bytes the production harness module served to a phone-class caller
while a real Broca worker was writing its reply. CKIOS's probe recorded them
through Callosum, as the iOS simulator's device profile, on 2026-10-24 between
10:36:36Z and 10:37:31Z. The module build was `a785c70e4` and Broca was 0.3.253.
The session, `alfonso:bg_0efce9ddd8162c1f`, was a throwaway worker told to write
a long essay as plain reply text.

Every `NN-*.request.json` holds the params the probe sent. Every `.reply.json`
holds a read's whole response body, and every `.notice-N.json` holds one watch
notice. Nothing was edited after recording. `SHA256SUMS` is the file CKIOS
published with the recording; its own SHA-256 starts with `0052f385e07c6b1f`.

Verify:

```sh
(cd crates/thalamus-app-reader/test-vectors/phone-reader-broca-live-v1 && shasum -a 256 -c SHA256SUMS)
```

| Files | What the probe did |
| --- | --- |
| `01-read-before` | `session.display_read` before any watch. The answer is `{"absent":"no_ring_attached"}`, because a read never starts a Broca subscription. |
| `02-read-mid-turn` | The same read after the worker started streaming, with the same answer for the same reason. |
| `03-watch-attach` | `session.display_watch`, 44 notices (seq 1 to 44). `read-during` is a read made while this watch held the session's display ring. |
| `04-watch-reattach` | The probe cancelled the first watch and opened a new one 2 seconds later: 37 notices (seq 49 to 85), and a read during it. |
| `05-read-after` | A read after the second watch ended. |

## Notice shapes

- A notice with one `frame` and no `frames` is a **baseline**. It carries the
  ring's current frame: the live text of the current message from its start, or a
  `display_gap` frame when no text is known. A baseline replaces whatever the
  phone shows. Every watch starts with one, and one also follows mid-watch when
  the ring has dropped frames the watch had not sent yet.
- A notice with `frames` carries the frames after the previous notice, in `seq`
  order. Its own `seq` is the last frame's.

The first watch starts with a `display_gap` baseline, because the ring had just
been created and held no text yet. Its 44 notices then carry two gaps, 7
reasoning deltas and 35 text deltas, all on message `m2`. The reattach came while
the ring was still held for its short grace after the first watch ended. So no new
Broca subscription opened, and its first notice is the baseline of the text so
far: 2,086 characters from the start of `m2`. Seqs 45 to 48 arrived between the
two watches and are inside that baseline.

## Text

Before publication, the chat text in these files was replaced with invented
words: message text, live text, tool output text, and the string values in tool
arguments and inputs. Each word became an invented word of the same length, the
same way throughout the set, so a send still matches its stored row and live text
still joins into its message. Ids, versions, tokens, times, codes, file names and
JSON keys are as recorded, and every string keeps its exact byte length, so sizes
and the 2,048-byte argument cap still match. `SHA256SUMS` lists the files as
published here.
