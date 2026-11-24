# phone-reader-fallback-reason-v1

Replies another model wrote after the one asked for refused, as the apps
receive them (runner contract 0.9.6). Served through Occipital's real request
handling by `occipital-module`'s `broca::tests::contract_096_samples`, so the
reply is byte for byte what a phone receives. `01-newest-page.request.json` is
the request the app sends and `01-newest-page.json` the reply body.
`SHA256SUMS` lists every file but this README.

`modelFallback` names the model asked for (`requested`) and the one that
answered (`answered`), as before. When the runner recorded why, it now also
carries `by` (`client`: the sign-in plugin retried on its fallback model;
`provider`: the provider's own fallback answered), `category` (the provider's
refusal category, an open lowercase id) and `cause` (`refused_here`: this
reply's first attempt was refused; `round`: the reply ran on the fallback model
after an earlier refusal). The three travel together. The replies are invented
in the shape a runner serves them:

| Row | `by` | `category` | `cause` |
| --- | --- | --- | --- |
| 1 | `client` | `cyber` | `refused_here` |
| 2 | `provider` | `cyber` | `round` |
| 3 | none | none | none |

Row 3 is a fallback recorded without a reason: draw the note naming the two
models, with no reason.
