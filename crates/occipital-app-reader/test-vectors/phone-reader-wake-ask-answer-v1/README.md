# phone-reader-wake-ask-answer-v1

A wake whose record holds the person's answers to asks (tool-display 0.1.11,
kind `ask_answer`), so the apps draw a "You answered" card instead of the
prompt text. It's the chat of `phone-reader-wake-records-v1`, with one record.

## The item

`{kind: "ask_answer", refs: {ask, question, answer, by, round?}, at, excerpt}`

- `ask`: the ask's id (`ask_…`), which opens the ask's own page.
- `question`: the question, or a form's title, at most 200 characters.
- `answer`: exactly one of:
  - `option`: the chosen option's label;
  - `text`: the person's own words;
  - `form`: `{answered, total}` for a form.
- `by`: `person`, or `default` when nobody answered and the ask's default
  applied.
- `round`: a form's follow-up round, 2 or more, when it is one.

Text that was cut ends with `…`. Prefrontal wrote ask answers as `notice`
items before this kind existed, so an older wake holds a `notice` item for an
answer, and a newer one an `ask_answer` item.

## Files

- `01-page`: the newest page. Reading it is how Occipital learns which send
  each row came from.
- `02-wake-records`: `msg_1`'s record, holding the contract's four
  `ask_answer` items:
  - a default that applied;
  - the person's own words;
  - a form answered 1 of 2;
  - a form's second round, answered 2 of 2.

The asks and their text are invented.
