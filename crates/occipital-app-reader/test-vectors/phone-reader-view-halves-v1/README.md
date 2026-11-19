# phone-reader-view-halves-v1

Tool views that pair their row with a wake record for the same event, so the
apps show and hide the two together (tool-provider 0.15.13, `half_of`).

## The member

`display.views` gives each view `halfOf: {kind, family?}` when its provider
declares it:

- `kind`: the wake record kind the row is the other half of, as tool-display
  names them (`peer_message`, `ask_answer`, `notice` and the rest).
- `family`: for `notice`, the notice family the row pairs with.

A view without it has no `halfOf` and is an ordinary row. Occipital passes the
member on exactly as the provider wrote it. When `kind` is one the app doesn't
know, it treats the row as an ordinary one. When the app gets a `family` on a
kind other than `notice`, which a provider shouldn't send, it ignores it.

## Files

- `01-views`: six pins. The first five are the contract's own
  `view-halves.json` cases that a reader keeps:
  - `peer`: a sent peer message, paired with its received-message card.
  - `ask`: an ask row, paired with its "You answered" card.
  - `noted`: a row paired with notices of family `example_family`.
  - `future`: an unfamiliar kind with a family, passed on.
  - `family`: `peer_message` with a family, passed on.
  - `plain`: a view without the member, so no `halfOf`.

The contract's cases carry no template, so no pin here has a `row` or `page`
digest, and each draws as a plain row. The pins are invented.
