# moshionly-export

Writes a Moshionly artboard to an image or a video file. The editor's Export
dialog, the agents' `export` tool and the `moshionly export` command all use
it.

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

## Formats

| Format | For | Transparent | Built in |
|---|---|---|---|
| PNG | One frame: a moment of the animation, or the design at rest | Yes | Yes |
| PNG sequence | Video editors that import numbered images: a folder of `frame_0000.png`, `frame_0001.png`, ... | Yes | Yes |
| GIF | Chat, email, documentation: plays everywhere | Pixels are fully see-through or fully solid | Yes |
| MP4 (H.264) | Social media, slides, websites | No | Yes |
| WebM (VP9) | Websites, including transparent overlays | Yes | Needs `ffmpeg` |
| ProRes 4444 (MOV) | Video editing software, including transparent overlays | Yes | Needs `ffmpeg` |

`available_formats()` says which formats can be written right now, and
`availability()` says why one cannot (for example, "WebM needs ffmpeg, which
was not found; install ffmpeg from https://ffmpeg.org, or on a Mac with
Homebrew: brew install ffmpeg"). `ffmpeg` is looked for on `PATH` and in
`/opt/homebrew/bin`, `/usr/local/bin`, `/opt/local/bin`, `/usr/bin` and
`/snap/bin`, since an app started from the Finder does not see the shell's
`PATH`. The app can also pass a specific one with `ExportJob::with_ffmpeg`.

Lottie export, which describes the animation as vectors instead of pixels,
will be added in `src/lottie/` as another `Format`; it plugs into the same
job, plan and progress.

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

Exporting the same thing twice gives the same bytes, in every format (for
WebM and ProRes, with the same `ffmpeg`).

## The command-line example

```text
cargo run --release -p moshionly-export --example export -- demo.moshi demo.mp4
cargo run --release -p moshionly-export --example export -- demo.moshi demo.gif --fps 25 --width 600
cargo run --release -p moshionly-export --example export -- demo.moshi frames --scale 2
cargo run --release -p moshionly-export --example export -- demo.moshi clear.webm --transparent
cargo run --release -p moshionly-export --example export -- --formats
```

The format follows the output's extension, or `--format`; run the example
without arguments for every option.

## Tests

`cargo test -p moshionly-export` checks PNG pixels against the renderer,
decodes GIFs and compares every frame, reads MP4 files back with the `mp4`
crate and decodes them with OpenH264's decoder, and checks WebM and ProRes
with `ffprobe`. The WebM and ProRes tests, and the MP4 check with
`ffprobe`, print why they are skipped where `ffmpeg` is not installed.

## Patents

H.264 is covered by patents in some countries. Cisco pays the patent
licence for OpenH264 only for its own prebuilt binaries, which it
distributes itself. This crate builds OpenH264 from source, which does not
carry that licence. Whoever distributes builds of Moshionly should check
whether they need a licence where they distribute them. WebM (VP9) is
royalty-free, and ProRes and the other formats are written by `ffmpeg` as a
separate program.

## Licences

The code is MIT. Dependencies: `openh264` and `openh264-sys2` (BSD-2-Clause,
with OpenH264 itself BSD-2-Clause), `gif` (MIT or Apache-2.0), `tempfile` (MIT
or Apache-2.0), `thiserror` (MIT or Apache-2.0). `ffmpeg` is never linked,
only run.
