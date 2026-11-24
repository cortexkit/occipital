# phone-reader-tool-versions-v2

This set replaces `phone-reader-tool-versions-v1`. The tool catalog changed in place when
replies stopped holding `Table`, so its digest is now
`sha256:f144092fdccf4813f84f84f034356532bc6770e7ae63dce3bf0fc2f7600f20d3`,
and v1's (`sha256:6e323928…`) is no longer accepted. The templates name
the new digest, which changes the template digests in `02-views` and `04-views-grown`.
Every other file is v1's bytes.

A call whose tool carries a version (tool-provider 0.15.17), as the apps read
it. The runner pins the call with a `tp2` pin,
`tp2:<tool>:<schema digest>:<major>`. The pin names the tool's major version
and never its minor or patch, so a compatible update keeps the pin. The apps
keep `schemaPin` as an opaque string and send the same bytes back to
`display.views`, as they do for a `tp1` pin.

## What it shows

- **A `tp2` pin:** the page names it as the part's `schemaPin`, and
  `display.views` serves the view for it in the same shape as for a `tp1` pin.
  Occipital checks that the view's tool recomputes to the pin before keeping
  it; the apps get no new member.
- **A field ahead of its view:** the provider writes the command's last output
  lines (`tail`) before its view declares them. Occipital drops a field its
  view's record schema doesn't declare before serving a record or completion
  (tool-display 0.1.14, §3.1), so the apps never see it.
- **A view that grows under its pin:** the provider's next minor version
  declares `tail`. Its view keeps every field it had and adds the new one, at a
  higher record schema revision and a higher `templateRevision`. An app that
  holds revision 1 (`held`) gets the grown view, and the same stored completion
  then carries the lines.

## Files

- `01-page`: the newest page. The call's `schemaPin` is a `tp2` pin.
- `02-views`: the view for that pin, at `templateRevision` 1, with
  `exitCode` in its record schema.
- `03-records`: the completion, without `tail`.
- `04-views-grown`: the view read again with `held` at revision 1: revision 2,
  with `tail` among its `completion_fields`.
- `05-records-grown`: the same completion, now with `tail`.

The command, its output and the digest's input schema are invented.
