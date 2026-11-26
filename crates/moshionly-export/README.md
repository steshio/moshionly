# moshionly-export

Writes a Moshionly artboard to an image, a video or a Lottie file. The
editor's Export dialog, the agents' `export` tool and the `moshionly export`
command all use it.

```rust
use std::sync::Arc;
use moshionly_core::io;
use moshionly_export::{CancelToken, ExportJob, Format, FrameRate, Settings};

let document = Arc::new(io::load("launch.moshi")?);
let artboard = document.artboards()[0].clone();
let settings = Settings::default()
    .with_scale(2.0)
    .with_frame_rate(FrameRate::FPS_60);
let job = ExportJob::new(document, artboard, Format::Mp4, "launch.mp4")
    .with_project_file("launch.moshi")
    .with_settings(settings);
let cancel = CancelToken::new(); // keep a clone to cancel from another thread
let exported = job.run(|progress| println!("{:.0}%", progress.fraction() * 100.0), &cancel)?;
```

`job.plan()` checks the settings without writing anything, so a dialog can
say what is wrong before the person starts.

The crate also reads video and sound files with `ffmpeg` (the [`media`
module](#video-and-sound)), for video layers in exports and in the editor,
and for sound.

## Formats

| Format | For | Transparent | Sound | Built in |
|---|---|---|---|---|
| PNG | One frame: a moment of the animation, or the design at rest | Yes | No | Yes |
| PNG sequence | Video editors that import numbered images: a folder of `frame_0000.png`, `frame_0001.png`, ... | Yes | No | Yes |
| GIF | Chat, email, documentation: plays everywhere | Pixels are fully see-through or fully solid | No | Yes |
| MP4 (H.264) | Social media, slides, websites | No | AAC, added by `ffmpeg` | Yes |
| WebM (VP9) | Websites, including transparent overlays | Yes | Opus | Needs `ffmpeg` |
| ProRes 4444 (MOV) | Video editing software, including transparent overlays | Yes | 24-bit PCM | Needs `ffmpeg` |
| Lottie (JSON) | Apps and websites: vector shapes and keyframes that play at any size | Yes | No | Yes |

`available_formats()` says which formats can be written right now, and
`availability()` says why one cannot (for example, "WebM needs ffmpeg, which
was not found; install ffmpeg from https://ffmpeg.org, or on a Mac with
Homebrew: brew install ffmpeg"). `ffmpeg` is looked for on `PATH` and in
`/opt/homebrew/bin`, `/usr/local/bin`, `/opt/local/bin`, `/usr/bin` and
`/snap/bin`, since an app started from the Finder does not see the shell's
`PATH`. The app can also pass a specific one with `ExportJob::with_ffmpeg`.

Lottie describes the animation as vector shapes and keyframes instead of
pixels; see [Lottie](#lottie) below. It runs through the same job, plan,
progress, cancellation and temporary-file-then-rename write as the other
formats.

## Settings

Every setting has a default, so `Settings::default()` exports the artboard at
its own size, 30 frames per second, on its own background.

- **Size**: a scale (`1` is the artboard's own size, `2` twice as sharp), a
  width or a height in pixels. Proportions are kept. MP4, WebM and ProRes
  need even sizes, so they are rounded down to even numbers, losing at most
  one pixel at the right or bottom edge. Each format has a limit: 4096 pixels
  a side for GIF; 3840 by 2160 (either way round) for MP4; 8192 a side and
  35.4 million pixels in all (8192 by 4320) for WebM and ProRes; 16384 a
  side and 134.2 million pixels in all (16384 by 8192) for PNG.
- **Frame rate**: an exact fraction, so 29.97 (30000/1001) and 23.976
  (24000/1001) keep their timing over long videos. `FrameRate::PRESETS` has
  the common rates; `FrameRate::from_fps(29.97)` understands the television
  rates written as decimals. GIF stops at 50 frames per second: its delays are
  whole hundredths of a second, and viewers slow down anything faster. MP4
  stops at 172, the most H.264 allows at the levels its encoder writes, and
  at fewer for large frames (64 at 3840 by 2160). The H.264 level written
  into the file is worked out from the true size and rate, so a player
  knows what the video needs before it tries to play it.
- **Background**: the artboard's own fill, transparent, or the artboard over
  a colour. Where the artboard's fill is missing or see-through, formats that
  can be transparent stay transparent and MP4 shows white. MP4 refuses a
  transparent background and says which formats can do it.
- **Quality**: draft, standard or high, for video and GIF.

  | | Draft | Standard | High |
  |---|---|---|---|
  | GIF | 128 colours, counted from small frames | 256 colours, from larger frames | 256 colours, from larger frames still |
  | MP4 | Fast, visible compression | Compression hard to see | Larger files for a master |
  | WebM | VP9 CRF 40 | CRF 30 | CRF 20, slower |
  | ProRes | 4444, coarser quantiser | 4444 | 4444 XQ |

- **Moments**: for a single PNG, the moment to capture or the design at rest;
  for everything else, an optional start and end. An export has at most a
  million frames (`MAX_FRAMES`, over nine hours at 30 fps); more is refused
  before anything is written, since it is almost certainly a mistake such
  as a duration with extra zeros.
- **GIF**: how many times it plays (forever by default, once, or a number of
  times) and whether it is dithered (on by default).
- **Lottie**: whether pictures go inside the file (the default) or into a
  folder beside it. The size sets the composition's size, which players use
  when nothing else decides; the content is scaled to it. Quality and the
  GIF settings do not apply.
- **Sound**: whether MP4, WebM and ProRes carry the artboard's sound (on by
  default). `Plan::sound` says whether the file will get a sound track:
  the format can hold one, the setting is on, and sound plays in the
  stretch exported.

### Which frames

Frame `i` shows the artboard at `start + i * 1000 / fps` milliseconds, for
every frame that starts before the end. The end itself is not a frame: each
frame stands for the time until the next, so 120 frames at 30 fps last
exactly 4 seconds, and a looping animation does not show its start and end
(the same picture) twice in a row. The last frame is one frame step before
the end.

## How it works

Frames are rendered on several worker threads, each a pure function of the
document and its moment, so the result does not depend on the number of
cores. They reach the encoder strictly in order, and workers may run only a
bounded number of frames ahead. Each frame's moment is worked out when it
is drawn, never listed in advance, so memory stays bounded however long the
export: the only thing kept for every frame is the MP4 index, four bytes a
frame. Per-frame work such as compressing PNGs or converting colours also
runs on the workers.

The file is written into a hidden temporary file beside the destination
(`.<name>.<random>.part`) and renamed into place only when complete, so a
failed or cancelled export leaves nothing behind. A PNG sequence is written
into a new folder, or an empty one; a folder with files in it is never
written into.

- **GIF**: one palette for the whole animation, chosen from the colours of
  every frame (each drawn small for counting, so a colour that shows for a
  single frame still gets in) by splitting the colour space where the
  colours are furthest apart and refining, with exact counts so that flat
  colours such as a brand colour stay exact. Dithering is an ordered pattern that mixes the two
  nearest palette colours in the right proportion; unlike error diffusion it
  does not crawl from frame to frame. Each frame stores only the rectangle
  that changed, with unchanged pixels see-through, and frames that look the
  same are merged into a longer delay. Delays carry their rounding, so a 30
  fps GIF lasts as long as the animation. Transparent GIFs clear each frame.
- **MP4**: frames are converted to BT.709 limited-range 4:2:0 and encoded
  with OpenH264 (High profile, a key frame every two seconds, no dropped
  frames). A small MP4 writer in this crate puts the index before the
  frames, so the video starts playing before it has fully downloaded, marks
  the colours as BT.709, and counts time in whole ticks of the frame rate.
- **WebM and ProRes**: raw frames are piped to `ffmpeg`, which converts them
  to BT.709 and encodes VP9 (`yuva420p` when transparent) or ProRes 4444
  (`yuva444p10le` when transparent). If `ffmpeg` fails, its own message is
  reported; on cancellation it is stopped at once, also while it is still
  finishing the file or not yet taking frames.

- **Sound**: the artboard's sound for the stretch exported (every `audio`
  item and every video with a `volume` above 0, from core's
  `eval::sounds`) is mixed into 48 kHz stereo and written beside the export
  as raw samples, which `ffmpeg` reads as a second input: Opus for WebM,
  24-bit PCM for ProRes. The MP4 is written in-process as usual and then
  copied once more by `ffmpeg`, video untouched, with AAC sound added.
  Without `ffmpeg`, or without the encoder the format's sound needs, the
  file is written without sound and `Exported::warnings` says so; GIF and
  PNG exports of an artboard with sound say that they hold none.
- **Video layers** are drawn with the renderer's frame provider, or with
  a `media::VideoDecoder` of the job's own when the renderer has none and
  the document has video. Without `ffmpeg` they draw a grey placeholder,
  with a warning.

Exporting the same thing twice gives the same bytes, in every format (for
WebM, ProRes and sound, with the same `ffmpeg`). That includes film grain,
which the renderer works out from the layer and the moment alone. Moving
grain changes every pixel it covers on every frame, so a GIF of it stores
whole frames and comes out larger.

## Video and sound

Moshionly links no video or sound decoders: the `media` module runs the
installed `ffmpeg` and `ffprobe`, like the WebM and ProRes exports, and
everything in it is optional at run time.

- `MediaTools::locate()` finds both programs; `probe(path)` (or
  `MediaTools::probe`) says how long a file is, the size of its picture as
  shown (in square pixels, turned upright), and whether it has video and
  sound. Cover art in a sound file is not video.
- `VideoDecoder` is the renderer's `VideoFrames` provider (install it with
  `AssetLoader::with_video`). For each file and frame size, one or two
  long-lived `ffmpeg` processes decode forward to raw RGBA; a request they
  have passed, or that is far ahead, starts one again a little before it.
  The frame shown at a moment is the last one whose own timestamp is at or
  before it: timestamps come from `ffmpeg`'s description of each frame, as
  written in the file, so the answer is the same in any order and from any
  thread, which export's parallel rendering needs. A seek can land after
  key frames that later frames depend on and skip a few frames; when the
  first frame read is past the moment, reading starts again further back.
  Frames read are kept within a memory budget (256 MB by default), and a
  small box gets frames halved in size as often as they still cover it.
- `SoundDecoder::decode` reads a stretch of a file's sound as 48 kHz stereo
  32-bit float samples (mono plays on both sides at full level), and
  `SoundDecoder::mix` mixes the sound of an artboard over a stretch of its
  timeline, honouring each clip's start, trim and volume, and leaving out
  what cannot be read with a warning. Each clip read is kept (512 MB by
  default), so mixing from another moment, as playback does, reads nothing
  again.

Reading frames needs `ffmpeg` 5.1 or newer (for `-fps_mode`).

## Lottie

A Lottie file is JSON that lottie-web, the Android and iOS players, Skottie,
ThorVG and others play. The export converts the scene model, not pixels:
each layer of the artboard becomes a Lottie layer, in the same order, with
the same transform (Lottie turns and scales a layer about its anchor, which
is set to the centre of the layer's box, as Moshionly does).

| Moshionly | Lottie |
|---|---|
| Artboard | The composition: its size, frame rate and length; its fill is a background layer |
| Rectangle, ellipse | Rectangle and ellipse shapes; different corners, arcs and rings are paths |
| Star, pen path | Paths |
| Fill, stroke | Fills and strokes, solid or linear and radial gradients; blend modes; opacity |
| Group that only moves its children | A null layer the children are parented to |
| Group that fades, blends, has effects or clips | A precomposition, sized to cover all of it that can be seen; a clip is a mask of its rounded box |
| Mask group | A track matte: the mask over a precomposition of the rest |
| Component | Like a group, where it is placed |
| Instance | Like a group holding copies of its component's layers and of their animations, later by its time offset (a negative offset cannot start them before 0 ms; see Warnings): the export detaches every instance on a copy of the document first |
| Text | Outlines of the letters, made with the renderer's own fonts, so it looks the same everywhere without font files |
| Text animations | One animated group per letter, word or line |
| Counter | Each value shown for its stretch of time, its letters placed from compositions drawn once per letter shape |
| Image | An image layer, with the picture inside the file as base64 PNG or JPEG, or in a folder beside it |
| Blur, drop shadow | Gaussian Blur and Drop Shadow effects |
| Glow | A Drop Shadow effect with no distance, before the shadow; its spread is left out |
| Directional blur | A Gaussian Blur effect that blurs across or down only, whichever is nearer its angle |
| Noise (film grain) | Left out, with a warning |
| Reveal | A mask that moves |
| Hidden layers | Left out |

### Keyframes

Moshionly has no keyframes: a property's value is worked out from its
animations at every moment. The export finds keyframes that reproduce it:

- A linear or Bezier-shaped easing (everything but elastic and bounce)
  becomes one keyframe whose Lottie handles are that easing. Where other
  animations cut it, each piece is the exact piece of the same curve, and
  neighbouring pieces join again when one curve covers them.
- Elastic and bouncing easings, and several animations on one property at
  once (moves that add up, a fade that interrupts another), are fitted with
  a few cubic pieces each, split at their turning points, until the fit
  keeps within a small tolerance: a tenth of a pixel, a hundredth of a
  degree, a fraction of a percent of opacity or scale, about one step of a
  colour channel.
- `show`, `hide` and values that jump get hold keyframes.

Every keyframe is checked against core's own evaluation eight times a frame
before it is kept, with its numbers rounded as they are written. Numbers
are rounded to what cannot be seen and values at their default are left
out, so files stay small; the demo is about 110 kB, most of it letters.

### Players

Effects follow lottie-web, the most used player: a shadow's or glow's
softness and a blur's blurriness are set so lottie-web draws them as
Moshionly does.
Skottie and the Android player read shadow softness as a blur radius and
draw shadows somewhat softer; lottie-web's canvas renderer draws no
effects, and fades the layers of a faded group one by one rather than
together. Players differ in whether a reveal also cuts a layer's shadow or
glow.

### Warnings

What Lottie cannot express exactly is drawn as closely as it can be, and
`Exported::warnings` says so in plain words, naming the layer. Video layers
and sound are left out, each named. For example: a stretched
radial gradient (drawn round), a paint that changes between linear and
radial, a stroke inside or outside the edge of anything but a rectangle or
ellipse (drawn centred), letters that blur as they come in, a gradient on a
counting text, characters drawn as pictures such as emoji, a picture that
cannot be read (left out), an instance resized out of proportion (its
content is stretched like a resized group, keeping stroke widths and font
sizes), an instance with a negative time offset whose component's
animations are still running when it starts (the motion before that is
squeezed into what is left of it), and of the effects: film grain (left
out), a glow's spread (left out, so the glow is thinner), a glow together
with a shadow (Lottie draws the shadow from the glow too), a glow or shadow
with a reveal (some players do not cut them with the reveal), and a
directional blur whose angle is not across or down (drawn across or down,
whichever is nearer). One layer never stops the export.

## The command-line example

```text
cargo run --release -p moshionly-export --example export -- demo.moshi demo.mp4
cargo run --release -p moshionly-export --example export -- demo.moshi demo.gif --fps 25 --width 600
cargo run --release -p moshionly-export --example export -- demo.moshi frames --scale 2
cargo run --release -p moshionly-export --example export -- demo.moshi clear.webm --transparent
cargo run --release -p moshionly-export --example export -- demo.moshi demo.json --fps 60
cargo run --release -p moshionly-export --example export -- demo.moshi demo.json --format lottie --images-beside
cargo run --release -p moshionly-export --example export -- clip.moshi clip.mp4 --no-sound
cargo run --release -p moshionly-export --example export -- --formats
```

The format follows the output's extension, or `--format`; run the example
without arguments for every option.

## Tests

`cargo test -p moshionly-export` checks PNG pixels against the renderer,
decodes GIFs and compares every frame, reads MP4 files back with the `mp4`
crate and decodes them with OpenH264's decoder, and checks WebM and ProRes
with `ffprobe`. For Lottie it checks the fitted keyframes against core's
`value_at` at every frame for every easing and for combined animations,
evaluating the written keyframes the way players do, and checks the
structure of shapes, groups, masks, text, images, effects and warnings. The media tests make their
video and sound with `ffmpeg` in a temporary folder and check probing,
that frames asked for out of order and from several threads equal frames
asked for in order, the frame shown at each moment, mixing with start,
trim and volume, and the sound tracks of MP4, WebM and ProRes exports with
`ffprobe`. These, the WebM and ProRes tests, and the MP4 check with
`ffprobe` print why they are skipped where `ffmpeg` is not installed.

## Patents

H.264 is covered by patents in some countries. Cisco pays the patent
licence for OpenH264 only for its own prebuilt binaries, which it
distributes itself. This crate builds OpenH264 from source, which does not
carry that licence. Whoever distributes builds of Moshionly should check
whether they need a licence where they distribute them. WebM (VP9) is
royalty-free, and ProRes and the other formats are written by `ffmpeg` as a
separate program. AAC, the sound of MP4 files, is patented in some
countries too; it is encoded by `ffmpeg` as a separate program.

## Licences

The code is MIT. Dependencies: `openh264` and `openh264-sys2` (BSD-2-Clause,
with OpenH264 itself BSD-2-Clause), `gif` (MIT or Apache-2.0), `image` (MIT
or Apache-2.0), `serde_json` (MIT or Apache-2.0), `tempfile` (MIT or
Apache-2.0), `thiserror` (MIT or Apache-2.0). `ffmpeg` and `ffprobe` are
never linked, only run, also for reading video and sound.
