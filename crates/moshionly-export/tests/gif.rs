//! GIF export: decoded with the `gif` crate, the frames, timing and loop
//! are right and the colours are close to the rendered frames.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gif::{ColorOutput, DecodeOptions, DisposalMethod, Repeat};
use moshionly_core::{NodeId, io};
use moshionly_export::{Background, ExportJob, Format, FrameRate, Loop, Moment, Quality, Settings};
use moshionly_render::RenderOptions;

/// A decoded GIF: how it loops, and every frame as it is shown, in full,
/// with its delay.
struct Decoded {
    repeat: Repeat,
    width: usize,
    height: usize,
    /// Straight RGBA of the whole picture after each frame, and the delay.
    shown: Vec<(Vec<u8>, u16)>,
    stored: usize,
}

fn decode(path: &Path) -> Decoded {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::Indexed);
    let file = fs::read(path).unwrap();
    let mut decoder = options.read_info(file.as_slice()).unwrap();
    let (width, height) = (decoder.width() as usize, decoder.height() as usize);
    let global = decoder.global_palette().unwrap().to_vec();
    let mut canvas = vec![0u8; width * height * 4];
    let mut shown = Vec::new();
    let mut previous: Option<(DisposalMethod, (usize, usize, usize, usize))> = None;
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        if let Some((DisposalMethod::Background, (left, top, w, h))) = previous {
            for y in top..top + h {
                canvas[(y * width + left) * 4..(y * width + left + w) * 4].fill(0);
            }
        }
        let palette = frame.palette.as_deref().unwrap_or(&global);
        let (left, top) = (frame.left as usize, frame.top as usize);
        let (w, h) = (frame.width as usize, frame.height as usize);
        for y in 0..h {
            for x in 0..w {
                let index = frame.buffer[y * w + x];
                if Some(index) == frame.transparent {
                    continue;
                }
                let at = ((top + y) * width + left + x) * 4;
                let color = &palette[usize::from(index) * 3..usize::from(index) * 3 + 3];
                canvas[at..at + 3].copy_from_slice(color);
                canvas[at + 3] = 255;
            }
        }
        shown.push((canvas.clone(), frame.delay));
        previous = Some((frame.dispose, (left, top, w, h)));
    }
    Decoded {
        repeat: decoder.repeat(),
        width,
        height,
        stored: shown.len(),
        shown,
    }
}

impl Decoded {
    /// The picture showing at each multiple of `step` hundredths.
    fn timeline(&self, step: u32) -> Vec<&[u8]> {
        let mut slots = Vec::new();
        for (picture, delay) in &self.shown {
            assert_eq!(u32::from(*delay) % step, 0, "delays are whole frames");
            for _ in 0..u32::from(*delay) / step {
                slots.push(picture.as_slice());
            }
        }
        slots
    }

    fn total_delay(&self) -> u32 {
        self.shown.iter().map(|(_, delay)| u32::from(*delay)).sum()
    }
}

#[test]
fn every_frame_is_there_with_the_right_timing_and_colours() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.gif");
    let settings = Settings::default()
        .with_scale(0.25)
        .with_frame_rate(FrameRate::FPS_20);
    let (result, _) = common::run(&common::job(Format::Gif, &path, settings));
    let exported = result.unwrap();
    assert_eq!(
        (exported.width, exported.height, exported.frames),
        (270, 270, 80)
    );
    assert_eq!(exported.duration_ms, Some(4000.0));

    let gif = decode(&path);
    assert_eq!((gif.width, gif.height), (270, 270));
    assert_eq!(gif.repeat, Repeat::Infinite);
    assert_eq!(gif.total_delay(), 400, "four seconds in hundredths");
    // Frames that look the same are stored once, for longer.
    assert!(gif.stored < 80, "{} frames stored", gif.stored);
    let timeline = gif.timeline(5);
    assert_eq!(timeline.len(), 80);

    let options = RenderOptions::new().with_scale(0.25);
    let mut lowest = f64::INFINITY;
    for (index, picture) in timeline.iter().enumerate() {
        let time = index as f64 * 50.0;
        let expected = common::reference(Moment::At(time), &options).to_unpremultiplied();
        assert!(
            picture
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255),
            "opaque"
        );
        let psnr = common::psnr(&common::rgb_of(picture), &common::rgb_of(&expected));
        assert!(psnr > 30.0, "frame {index} at {time} ms: {psnr:.1} dB");
        lowest = lowest.min(psnr);
    }
    println!(
        "{} bytes, {} frames stored, lowest PSNR {lowest:.1} dB",
        exported.bytes, gif.stored
    );
}

#[test]
fn the_same_export_gives_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::default()
        .with_scale(0.2)
        .with_frame_rate(FrameRate::FPS_25)
        .with_range(Some(0.0), Some(1000.0));
    let first = dir.path().join("a.gif");
    let second = dir.path().join("b.gif");
    common::run(&common::job(Format::Gif, &first, settings))
        .0
        .unwrap();
    common::run(&common::job(Format::Gif, &second, settings))
        .0
        .unwrap();
    assert!(fs::read(first).unwrap() == fs::read(second).unwrap());
}

#[test]
fn television_rates_keep_their_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ntsc.gif");
    let settings = Settings::default()
        .with_scale(0.1)
        .with_frame_rate(FrameRate::FPS_29_97)
        .with_quality(Quality::Draft);
    let (result, _) = common::run(&common::job(Format::Gif, &path, settings));
    assert_eq!(result.unwrap().frames, 120);
    // 120 frames at 29.97 fps last 4.004 seconds.
    assert_eq!(decode(&path).total_delay(), 400);
}

#[test]
fn a_transparent_gif_lets_the_background_through() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clear.gif");
    let settings = Settings::default()
        .with_scale(0.2)
        .with_frame_rate(FrameRate::new(10, 1).unwrap())
        .with_background(Background::Transparent)
        .with_gif_loop(Loop::Once);
    let (result, _) = common::run(&common::job(Format::Gif, &path, settings));
    result.unwrap();
    let gif = decode(&path);
    assert_eq!(gif.repeat, Repeat::Finite(0), "no loop: plays once");
    assert_eq!(gif.total_delay(), 400);
    let timeline = gif.timeline(10);
    assert_eq!(timeline.len(), 40);
    let options = RenderOptions::new()
        .with_scale(0.2)
        .with_background(Background::Transparent);
    for (index, picture) in timeline.iter().enumerate() {
        let expected = common::reference(Moment::At(index as f64 * 100.0), &options);
        let expected = expected.to_unpremultiplied();
        // Every pixel is see-through exactly where the frame is mostly
        // see-through.
        let (got, want) = (picture.as_chunks::<4>().0, expected.as_chunks::<4>().0);
        for (got, want) in got.iter().zip(want) {
            assert_eq!(got[3] == 255, want[3] >= 128, "frame {index}");
        }
    }
    // The corner, outside the card, is clear in the first frame.
    assert_eq!(timeline[0][3], 0);
}

#[test]
fn a_colour_seen_for_only_a_few_frames_keeps_its_exact_colour() {
    // A red square shows for three frames (1320 to 1400 ms at 25 fps) in a
    // five second animation of a blue square on white.
    let document = Arc::new(
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Flash","nodes":[
            {"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
             "width":200,"height":100,"duration":5000,"fill":"#FFFFFF"},
            {"id":"blue","parent":"board","index":"a0","type":"rect","name":"Blue",
             "x":10,"y":20,"width":60,"height":60,"fill":"#0D99FF"},
            {"id":"red","parent":"board","index":"a1","type":"rect","name":"Red",
             "x":120,"y":20,"width":60,"height":60,"fill":"#FF3B30"},
            {"id":"in","parent":"board","index":"a2","type":"fade_in","target":"red",
             "start":1300,"end":1301},
            {"id":"out","parent":"board","index":"a3","type":"fade_out","target":"red",
             "start":1400,"end":1401}
        ]}"##,
        )
        .unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flash.gif");
    let job = ExportJob::new(document, NodeId::from("board"), Format::Gif, &path)
        .with_settings(Settings::default().with_frame_rate(FrameRate::FPS_25))
        .with_renderer(common::renderer());
    let (result, _) = common::run(&job);
    assert_eq!(result.unwrap().frames, 125);

    let gif = decode(&path);
    let timeline = gif.timeline(4);
    assert_eq!(timeline.len(), 125);
    // The middle of the red square.
    let at = (50 * gif.width + 150) * 4;
    for (frame, expected) in [
        (32, [255, 255, 255]),
        (33, [255, 59, 48]),
        (34, [255, 59, 48]),
        (35, [255, 59, 48]),
        (36, [255, 255, 255]),
    ] {
        assert_eq!(timeline[frame][at..at + 3], expected, "frame {frame}");
    }
}
