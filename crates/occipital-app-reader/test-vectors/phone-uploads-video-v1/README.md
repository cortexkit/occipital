# phone-uploads-video-v1

A video sent from the phone's photo library, served through Occipital's own
frame loop on a paired phone's route. Both cases end in the session's project
folder. A video that goes into the model's prompt needs a runner whose model
takes video; those cases come in a later set.

`NN-*.request.json` is the request the app sends, and `NN-*.json` is the reply
body it receives. The project's folder is written as `/path/to/project`.
`IMG_0042.MOV` is the recorded file: a real one-second QuickTime movie, 32 by
32 pixels, 1,846 bytes. Its pieces are sent outside the set, as
`phone-upload-v1` samples `upload.chunk`.

| Files | What happens |
| --- | --- |
| `01-begin-video` | `upload.begin` with `prompt: true` and `video/quicktime`, for a model that takes images only: `destination: "folder"`. |
| `02-status` | `upload.status` after the file's one piece: `committed` is the whole file. |
| `03-finish` | `upload.finish`: the file's path under `.cortexkit/uploads/<date>/` and its size. The phone puts this path in the message text. |
| `04-begin-video-over-prompt-size` | `upload.begin` for a model that takes every type, with a 48 MiB video: `destination: "folder"`, since the prompt takes at most 32 MiB. |

A folder upload takes any type, up to 2 GiB, in 1 MiB pieces.
