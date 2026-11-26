//! Reading video and sound with `ffmpeg`: probing, exact frames in any
//! order, and mixing sound.
//!
//! The media is made by `ffmpeg` itself in a temporary folder. These tests
//! need `ffmpeg` and `ffprobe`; where they are not installed each test
//! prints why and passes without checking anything.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use moshionly_core::{Document, Edit, Node, NodeId};
use moshionly_export::media::{MediaError, MediaTools, SoundDecoder, VideoDecoder};
use moshionly_export::{CancelToken, Ffmpeg};
use moshionly_render::skia_safe as sk;
use serde_json::{Value, json};

/// The tools, or `None` (after saying so) when they are not installed.
fn tools(test: &str) -> Option<MediaTools> {
    let tools = MediaTools::locate();
    if tools.is_none() {
        println!("skipped {test}: ffmpeg or ffprobe is not installed");
    }
    tools
}

/// Makes `name` in `dir` with `ffmpeg` and these arguments.
fn make(tools: &MediaTools, dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let path = dir.join(name);
    let output = Command::new(tools.ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

/// Three seconds of 32 by 16 grey frames at 10 per second, frame `n` a
/// little lighter than the one before (see [`frame_number`]). Encoded with
/// key frames 0.7 s apart and frames that depend on later ones, so
/// seeking skips frames and has to start further back.
fn counting_video(tools: &MediaTools, dir: &Path) -> PathBuf {
    make(
        tools,
        dir,
        "counting.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x16:r=10:d=3,geq=lum='16+N*7':cb=128:cr=128",
            "-c:v",
            "mpeg4",
            "-q:v",
            "1",
            "-g",
            "7",
            "-bf",
            "2",
        ],
    )
}

/// Two seconds of stereo sound whose value is half the time in seconds:
/// 0.15 at 300 ms.
fn ramp(tools: &MediaTools, dir: &Path) -> PathBuf {
    make(
        tools,
        dir,
        "ramp.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.5*t|0.5*t:s=48000:d=2",
            "-c:a",
            "pcm_f32le",
        ],
    )
}

/// Tools whose `ffmpeg` is a shell script in `dir` that runs `script` and
/// then the real `ffmpeg`, beside the real `ffprobe`.
#[cfg(unix)]
fn wrapped(tools: &MediaTools, dir: &Path, script: &str) -> MediaTools {
    use std::os::unix::fs::PermissionsExt;
    let ffmpeg = dir.join("ffmpeg");
    let real = tools.ffmpeg().display();
    std::fs::write(
        &ffmpeg,
        format!("#!/bin/sh\n{script}\nexec '{real}' \"$@\"\n"),
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(tools.ffprobe(), dir.join("ffprobe")).unwrap();
    // On Linux a script just written can briefly be "busy" while another
    // test starts a program.
    for _ in 0..50 {
        match Command::new(&ffmpeg).arg("-version").output() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => break,
        }
    }
    MediaTools::with_ffmpeg(&Ffmpeg::at(ffmpeg)).unwrap()
}

/// Which frame of [`counting_video`] a picture is.
fn frame_number(image: &sk::Image) -> i64 {
    let red = pixels(image)[0];
    (f64::from(red) / (7.0 * 255.0 / 219.0)).round() as i64
}

fn pixels(image: &sk::Image) -> Vec<u8> {
    let (width, height) = (image.width(), image.height());
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let info = sk::ImageInfo::new(
        (width, height),
        sk::ColorType::RGBA8888,
        sk::AlphaType::Premul,
        None,
    );
    assert!(image.read_pixels(
        &info,
        &mut pixels,
        width as usize * 4,
        (0, 0),
        sk::image::CachingHint::Allow
    ));
    pixels
}

#[test]
fn probing_says_what_a_file_holds() {
    let Some(tools) = tools("probing_says_what_a_file_holds") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let both = make(
        &tools,
        dir.path(),
        "both.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=10:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let info = tools.probe(&both).unwrap();
    assert_eq!((info.width, info.height), (64, 48));
    assert!(info.has_video && info.has_sound);
    assert_eq!(info.frame_rate, Some(10.0));
    let duration = info.duration_ms.unwrap();
    assert!((duration - 2000.0).abs() < 100.0, "{duration}");

    let sound = tools.probe(&ramp(&tools, dir.path())).unwrap();
    assert!(!sound.has_video && sound.has_sound);
    assert_eq!((sound.width, sound.height), (0, 0));

    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "not a video").unwrap();
    assert!(matches!(
        tools.probe(&text),
        Err(MediaError::Unreadable { .. })
    ));
    let missing = dir.path().join("missing.mp4");
    assert_eq!(tools.probe(&missing), Err(MediaError::NoFile(missing)));
}

#[test]
fn each_moment_shows_the_last_frame_that_starts_at_or_before_it() {
    let Some(tools) = tools("each_moment_shows_the_last_frame_that_starts_at_or_before_it") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = counting_video(&tools, dir.path());
    let decoder = VideoDecoder::new(tools.clone());
    let shown =
        |time_ms: f64| frame_number(&decoder.frame_at(&video, time_ms, 32.0, 16.0).unwrap());
    // Backwards first, so every one starts reading again.
    for (time, frame) in [
        (5000.0, 29),
        (2999.0, 29),
        (1234.0, 12),
        (1199.9, 11),
        (1100.0, 11),
        (1050.0, 10),
        (150.0, 1),
        (100.0, 1),
        (99.0, 0),
        (0.0, 0),
    ] {
        assert_eq!(shown(time), frame, "at {time} ms");
    }
    assert!(decoder.memory_used() > 0);
    // A small box gets a smaller frame of the same moment.
    let small = decoder.frame_at(&video, 1234.0, 8.0, 4.0).unwrap();
    assert_eq!((small.width(), small.height()), (8, 4));
    assert_eq!(frame_number(&small), 12);
    // A sound file has no frames.
    let sound = ramp(&tools, dir.path());
    assert!(matches!(
        decoder.frame_at(&sound, 0.0, 10.0, 10.0),
        Err(MediaError::NoVideo(_))
    ));
}

#[test]
fn frames_asked_for_out_of_order_equal_frames_asked_for_in_order() {
    let Some(tools) = tools("frames_asked_for_out_of_order_equal_frames_asked_for_in_order") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = counting_video(&tools, dir.path());
    let times: Vec<f64> = (0..95).map(|step| f64::from(step) * 33.3).collect();

    let in_order = VideoDecoder::new(tools.clone());
    let expected: Vec<Vec<u8>> = times
        .iter()
        .map(|time| pixels(&in_order.frame_at(&video, *time, 32.0, 16.0).unwrap()))
        .collect();
    // Frame by frame, each the last to start at or before its moment.
    for (time, frame) in times.iter().zip(&expected) {
        let red = f64::from(frame[0]);
        let number = (red / (7.0 * 255.0 / 219.0)).round() as i64;
        // Past the end of the video, its last frame stays.
        let wanted = ((*time / 100.0).floor() as i64).min(29);
        assert_eq!(number, wanted, "at {time} ms");
    }

    // Four threads at once, each jumping around, with room for few frames
    // so that frames are read again.
    let shuffled = Arc::new(VideoDecoder::new(tools).with_budget(32 * 16 * 4 * 6));
    std::thread::scope(|scope| {
        for thread in 0..4 {
            let (shuffled, times, expected, video) = (&shuffled, &times, &expected, &video);
            scope.spawn(move || {
                for step in 0..times.len() {
                    let at = (step * 37 + thread * 11) % times.len();
                    let image = shuffled.frame_at(video, times[at], 32.0, 16.0).unwrap();
                    assert_eq!(pixels(&image), expected[at], "at {} ms", times[at]);
                }
            });
        }
    });
}

#[test]
#[cfg(unix)]
fn an_ffmpeg_too_old_to_read_frames_is_said_to_be() {
    let Some(tools) = tools("an_ffmpeg_too_old_to_read_frames_is_said_to_be") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = counting_video(&tools, dir.path());
    // What ffmpeg 5.0 and older answer to -fps_mode.
    let script = r#"for arg in "$@"; do
  if [ "$arg" = "-fps_mode" ]; then
    echo "Unrecognized option 'fps_mode'." >&2
    echo "Error splitting the argument list: Option not found" >&2
    exit 1
  fi
done"#;
    let bin = tempfile::tempdir().unwrap();
    let decoder = VideoDecoder::new(wrapped(&tools, bin.path(), script));
    let error = decoder.frame_at(&video, 0.0, 32.0, 16.0).unwrap_err();
    assert!(
        error.to_string().contains("needs ffmpeg 5.1 or newer"),
        "{error}"
    );
}

/// A document with one artboard "ab" holding `nodes`.
fn document(nodes: Value) -> Document {
    let mut document = Document::new("Sound");
    let mut edits = vec![Edit::add(
        Node::from_value(
            json!({ "id": "ab", "index": "a0", "type": "artboard", "duration": 2000 }),
        )
        .unwrap(),
    )];
    edits.extend(
        nodes
            .as_array()
            .unwrap()
            .iter()
            .map(|node| Edit::add(Node::from_value(node.clone()).unwrap())),
    );
    document.apply(&edits).unwrap();
    document
}

#[test]
fn sound_is_read_from_its_moment() {
    let Some(tools) = tools("sound_is_read_from_its_moment") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let ramp = ramp(&tools, dir.path());
    let decoder = SoundDecoder::new(tools);
    let samples = decoder.decode(&ramp, 300.0, 500.0).unwrap();
    assert_eq!(samples.len(), 24_000 * 2);
    let near = |got: f32, wanted: f32| (got - wanted).abs() < 1e-3;
    assert!(
        near(samples[0], 0.15) && near(samples[1], 0.15),
        "{:?}",
        &samples[..2]
    );
    assert!(near(samples[2 * 12_000], 0.275), "{}", samples[2 * 12_000]);
    // Past the end of the file it is silent.
    let tail = decoder.decode(&ramp, 1900.0, 500.0).unwrap();
    assert!(near(tail[0], 0.95));
    assert!(tail[2 * 4800..].iter().all(|sample| *sample == 0.0));
    // Read once, then kept.
    let used = decoder.memory_used();
    assert!(used >= 24_000 * 2 * 4);
    decoder.decode(&ramp, 300.0, 500.0).unwrap();
    assert_eq!(decoder.memory_used(), used);
}

#[test]
fn mixing_honours_start_trim_and_volume() {
    let Some(tools) = tools("mixing_honours_start_trim_and_volume") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    ramp(&tools, dir.path());
    counting_video(&tools, dir.path());
    let project = dir.path().join("project.moshi");
    let document = document(json!([
        // From 500 ms, 300 ms into the ramp, at half volume.
        { "id": "a", "parent": "ab", "index": "a0", "type": "audio", "source": "ramp.wav",
          "start": 500, "end": 1000, "trim_start": 300, "volume": 0.5 },
        // From 800 ms, from the start of the ramp.
        { "id": "b", "parent": "ab", "index": "a1", "type": "audio", "source": "ramp.wav",
          "start": 800, "end": 1200 },
        // A video without sound adds nothing, and a missing file is said.
        { "id": "v", "parent": "ab", "index": "a0", "type": "video", "source": "counting.mp4",
          "start": 0, "end": 2000 },
        { "id": "gone", "parent": "ab", "index": "a2", "type": "audio", "source": "gone.wav",
          "start": 0, "end": 100 },
    ]));
    let decoder = SoundDecoder::new(tools);
    let artboard = NodeId::from("ab");
    let cancel = CancelToken::new();
    let mix = decoder
        .mix(&document, &artboard, Some(&project), 0.0, 1500.0, &cancel)
        .unwrap();
    assert_eq!(mix.frames(), 72_000);
    assert_eq!(mix.sounds, 2);
    assert_eq!(mix.warnings.len(), 1, "{:?}", mix.warnings);
    assert!(mix.warnings[0].contains("gone.wav"), "{:?}", mix.warnings);
    let at = |mix: &moshionly_export::media::Mix, from_ms: f64, time_ms: f64| {
        let frame = ((time_ms - from_ms) * 48.0).round() as usize;
        (mix.samples[frame * 2], mix.samples[frame * 2 + 1])
    };
    let near = |(left, right): (f32, f32), wanted: f32| {
        (left - wanted).abs() < 1e-3 && (right - wanted).abs() < 1e-3
    };
    assert!(near(at(&mix, 0.0, 400.0), 0.0));
    // a: half of the ramp at 300 + 100 ms.
    assert!(
        near(at(&mix, 0.0, 600.0), 0.5 * 0.2),
        "{:?}",
        at(&mix, 0.0, 600.0)
    );
    // a and b together.
    assert!(
        near(at(&mix, 0.0, 900.0), 0.5 * 0.35 + 0.05),
        "{:?}",
        at(&mix, 0.0, 900.0)
    );
    // b alone, then nothing.
    assert!(near(at(&mix, 0.0, 1100.0), 0.15));
    assert!(near(at(&mix, 0.0, 1300.0), 0.0));

    // Mixing from another moment gives the same samples from there on.
    let later = decoder
        .mix(&document, &artboard, Some(&project), 700.0, 1500.0, &cancel)
        .unwrap();
    assert_eq!(later.samples[..], mix.samples[700 * 48 * 2..]);

    // A stretch with no sound in it is silent.
    let quiet = decoder
        .mix(
            &document,
            &artboard,
            Some(&project),
            1300.0,
            1500.0,
            &cancel,
        )
        .unwrap();
    assert_eq!(quiet.sounds, 0);
    assert!(quiet.samples.iter().all(|sample| *sample == 0.0));
    cancel.cancel();
    assert_eq!(
        decoder.mix(&document, &artboard, Some(&project), 0.0, 1500.0, &cancel),
        Err(MediaError::Cancelled)
    );
}
