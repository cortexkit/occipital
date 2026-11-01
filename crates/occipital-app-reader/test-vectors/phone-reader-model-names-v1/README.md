# phone-reader-model-names-v1

`display.model_names`: each model id as the apps show it, so both apps name a
model the same way and the phone needs no route of its own to the model
catalog.

## The operation

`display.model_names {ids}`: 1 to 64 model ids, each 1 to 256 bytes; repeats
are answered once. It needs no session and is admitted like the transcript
reads.

The reply is `{"models": {id: answer}}`, one member per id asked:

- `{name, provider?, variant?}`: `name` is the model's name and `provider` its
  provider's name, as the model catalog writes them. `provider` is left out when
  the catalog has no provider name. `variant` is the effort the id ends with,
  such as `xhigh` in `openai/gpt-6.1-sol:xhigh`, as written; show it as you like.
- `{}`: the catalog can't name this id. Show the id itself. This covers a bare id
  without its provider (the same model id exists under many providers), a
  retired model, and an id no model matches.

Ids may name the provider a credential is filed under, such as
`anthropic-platform`; Occipital maps it to the catalog's provider the way
routing does. Names can change when the catalog changes, so keep them no longer
than the chat is open, or about 30 minutes. A refusal `provider_unavailable`
with `retryable: true` means the catalog couldn't be read yet; show the ids
and ask again later.

## Where the files come from

Occipital's app reader served these replies at the commit that adds this folder,
through the same serving path a phone's route takes. The names come from
fusiform's own sample replies and routing's tagged sample
(`crates/occipital-module/src/display/model_names/fixtures/README.md` lists
them), so they are real catalog rows. Each `.json` file is the whole response
body, and each `.request.json` file is the request's `{method, params}`. The
test `app_reader::broca::tests::model_names_samples` in `occipital-module`
serves every request again and fails if a reply's bytes change, so a change
that alters them needs a new sample set.

## Cases

| Files | What it shows |
| --- | --- |
| `01-names` | A whole id; an id with an effort; a credential provider with an effort; a model id that itself holds `/` and has no provider name; a bare id; a retired model. |
| `02-too-many-ids` | 65 ids: refused `invalid_params`, naming `ids`. |
