# phone-attachments-v2

How many files one message may carry, as a phone sees it. Each case is one
request on a paired phone's route and the reply the phone receives.

The replies are the same bytes as `phone-attachments-v1`. What changed is the
requests: each names the session's `harness`, as every session request must.
A request without one is read as a Broca session, so for any other harness
(`opencode2`, `pi`, `claude-code`) it is refused `session_unknown`. v1's
requests named none, and an app that copied them lost every other harness's
limits.

`session.details` reports the limits as `attachments` when the request asks
with `attachmentLimits: true` (so the details other apps read keep their
bytes), and a send is held to exactly those limits whether or not the app
asked:

- `maxImages`: the most files whose mime starts with `image/`. It is the model
  catalog's `limit.images_per_request` for the session's model, else 4.
- `maxFiles`: the most other files, counted separately from images. Always 4,
  since the catalog states no count for them.
- `imageBytes` and `imageLongEdgePx`: the largest image file in bytes, and the
  longest side of one image in pixels. The catalog's edge is the smallest of
  `limit.image_long_edge_px` and every `limit.image_long_edge_px.above_images.<n>`
  (a smaller edge once a request holds more than n images, history included,
  which an app can't count). Each bound is the stricter of the catalog's and
  the runner's own (the runner's are the ones `upload.begin` reports
  as `max_image_bytes` and `max_image_dimension`). Each is present only when one
  of the two states it.

`maxImages` and `maxFiles` are always present. The catalog is read with the
model names and kept for 30 minutes; while it can't be read and none is held,
the fallbacks apply.

A send is counted over the files its `parts` name, all of them finished
uploads, before anything is copied to the runner. An upload that failed isn't a
part of the send, so it isn't counted. Files placed in the project are named in
the message text, not in `parts`, so they aren't counted either. A send naming
more images than `maxImages`, or more other files than `maxFiles`, is refused
`too_many_files`; nothing is sent, and its `detail` carries `maxImages`,
`maxFiles` and the send's own `images` and `files`.

The cases, in the order they were served:

1. `01-details`: a model whose catalog row states its image count and bounds
   (`anthropic/claude-opus-5-5`: 600 images, 7,500,000 bytes, an 8,000-pixel
   edge and 3,000 pixels above 20 images, so 3,000).
   The runner bounds images at 5,242,880 bytes and 8,000 pixels, so the details
   report 5,242,880 bytes and 3,000 pixels.
2. `02-send-twelve-photos`: twelve photos in one message, more than the 10
   Occipital allowed before this set, accepted.
3. `refusal-too_many_files-files`: five PDFs, refused, since other files are
   held to 4 whatever the model takes.
4. `03-details-without-catalog-limits`: a gateway model whose catalog row states
   no bounds. The fallbacks apply, with the runner's own image bounds. The
   twelve-photo send of case 2 is still listed in `queuedSends`, because the test
   runner never reports it stored.
5. `refusal-too_many_files-images`: five photos to that model, refused.

The uploads themselves are made outside the set, as `phone-uploads-prompt-v2`
samples them.
