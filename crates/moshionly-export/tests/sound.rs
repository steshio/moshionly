//! Exports of artboards with video and sound: the frames show the video,
//! and MP4, WebM and ProRes carry the sound, checked with `ffprobe`.
//!
//! The media is made by `ffmpeg` itself in a temporary folder. These tests
//! need `ffmpeg` and `ffprobe`; where they are not installed each test
//! prints why and passes without checking anything.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use moshionly_core::{Document, NodeId, io};
use moshionly_export::media::MediaTools;
use moshionly_export::{CancelToken, ExportJob, Exported, Format, Settings};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};
use serde_json::Value;

/// The tools, or `None` (after saying so) when they are not installed.
fn tools(test: &str) -> Option<MediaTools> {
    let tools = MediaTools::locate();
    if tools.is_none() {
        println!("skipped {test}: ffmpeg or ffprobe is not installed");
    }
    tools
}

/// Makes `name` in `dir` with `ffmpeg` and these arguments.
fn make(tools: &MediaTools, dir: &Path, name: &str, args: &[&str]) {
    let output = Command::new(tools.ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(dir.join(name))
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A project in `dir`: a 64 by 36 artboard, one second long, with a video
/// of grey frames getting lighter (frame `n` shows from `n` × 100 ms)
/// filling it, and a tone from 200 to 800 ms. Returns the project file.
fn project(tools: &MediaTools, dir: &Path) -> (PathBuf, Arc<Document>) {
    make(
        tools,
        dir,
        "grey.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x18:r=10:d=2,geq=lum='16+N*7':cb=128:cr=128",
            "-c:v",
            "mpeg4",
            "-q:v",
            "1",
        ],
    );
    make(
        tools,
        dir,
        "tone.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
        ],
    );
    let file = dir.join("project.moshi");
    let document = io::parse(
        r##"{"format":"moshionly","version":3,"name":"Media","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":64,"height":36,
             "duration":1000,"fill":"#FFFFFF"},
            {"id":"film","parent":"ab","index":"a0","type":"video","source":"grey.mp4",
             "width":64,"height":36,"fit":"stretch","start":0,"end":1000,"volume":0},
            {"id":"tone","parent":"ab","index":"a0","type":"audio","name":"Tone",
             "source":"tone.wav","start":200,"end":800}
        ]}"##,
    )
    .unwrap();
    (file, Arc::new(document))
}

fn export(
    file: &Path,
    document: &Arc<Document>,
    format: Format,
    out: &Path,
    settings: Settings,
) -> Exported {
    ExportJob::new(Arc::clone(document), NodeId::from("ab"), format, out)
        .with_project_file(file)
        .with_settings(settings)
        .with_renderer(Renderer::new(
            FontLibrary::bundled(),
            AssetLoader::new(Some(file.to_owned())),
        ))
        .run(|_| {}, &CancelToken::new())
        .unwrap()
}

/// The streams of a file: (kind, codec, duration in seconds).
fn streams(path: &Path) -> Vec<(String, String, f64)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,duration",
        ])
        .args(["-show_entries", "format=duration", "-of", "json"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let format_duration = json["format"]["duration"]
        .as_str()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0.0);
    json["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stream| {
            (
                stream["codec_type"].as_str().unwrap_or_default().to_owned(),
                stream["codec_name"].as_str().unwrap_or_default().to_owned(),
                stream["duration"]
                    .as_str()
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(format_duration),
            )
        })
        .collect()
}

#[test]
fn mp4_webm_and_prores_carry_the_sound() {
    let Some(tools) = tools("mp4_webm_and_prores_carry_the_sound") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (file, document) = project(&tools, dir.path());
    for (format, name, codec) in [
        (Format::Mp4, "out.mp4", "aac"),
        (Format::WebM, "out.webm", "opus"),
        (Format::ProRes, "out.mov", "pcm_s24le"),
    ] {
        let out = dir.path().join(name);
        let exported = export(&file, &document, format, &out, Settings::default());
        assert!(
            exported.warnings.is_empty(),
            "{format}: {:?}",
            exported.warnings
        );
        let found = streams(&out);
        let sound: Vec<_> = found.iter().filter(|stream| stream.0 == "audio").collect();
        assert_eq!(sound.len(), 1, "{format}: {found:?}");
        assert_eq!(sound[0].1, codec, "{format}");
        // The whole second, with the tone in the middle of it.
        assert!((sound[0].2 - 1.0).abs() < 0.08, "{format}: {found:?}");
        assert!(found.iter().any(|stream| stream.0 == "video"), "{format}");

        // Without sound, there is no sound track.
        let quiet = dir.path().join(format!("quiet-{name}"));
        export(
            &file,
            &document,
            format,
            &quiet,
            Settings::default().with_sound(false),
        );
        let found = streams(&quiet);
        assert!(
            found.iter().all(|stream| stream.0 != "audio"),
            "{format}: {found:?}"
        );
    }
    // Only the exported file and its silent twin are left: no temporary
    // sound files.
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    left.sort();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn video_frames_are_drawn_in_exports() {
    let Some(tools) = tools("video_frames_are_drawn_in_exports") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (file, document) = project(&tools, dir.path());
    let out = dir.path().join("still.png");
    let exported = export(
        &file,
        &document,
        Format::Png,
        &out,
        Settings::default().with_still(Some(450.0)),
    );
    // A still has no sound, and says so.
    assert_eq!(exported.warnings.len(), 1, "{:?}", exported.warnings);
    assert!(
        exported.warnings[0].contains("hold no sound"),
        "{:?}",
        exported.warnings
    );
    let image = image::open(&out).unwrap().into_rgba8();
    // Frame 4 of the video: grey 16 + 4 × 7 in video levels.
    let red = f64::from(image.get_pixel(32, 18)[0]);
    let frame = (red / (7.0 * 255.0 / 219.0)).round();
    assert_eq!(frame, 4.0, "red {red}");
}

#[test]
fn formats_without_sound_say_it_was_left_out() {
    let Some(tools) = tools("formats_without_sound_say_it_was_left_out") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (file, document) = project(&tools, dir.path());
    let gif = export(
        &file,
        &document,
        Format::Gif,
        &dir.path().join("out.gif"),
        Settings::default(),
    );
    assert!(
        gif.warnings
            .iter()
            .any(|warning| warning.contains("GIF files hold no sound")),
        "{:?}",
        gif.warnings
    );
    // Turned off, nothing is said.
    let quiet = export(
        &file,
        &document,
        Format::Gif,
        &dir.path().join("quiet.gif"),
        Settings::default().with_sound(false),
    );
    assert!(quiet.warnings.is_empty(), "{:?}", quiet.warnings);

    // Lottie names the video and the sound it leaves out.
    let lottie = export(
        &file,
        &document,
        Format::Lottie,
        &dir.path().join("out.json"),
        Settings::default(),
    );
    assert!(
        lottie
            .warnings
            .iter()
            .any(|warning| warning.contains("is a video")),
        "{:?}",
        lottie.warnings
    );
    assert!(
        lottie
            .warnings
            .iter()
            .any(|warning| warning.starts_with("\"Tone\" is sound")),
        "{:?}",
        lottie.warnings
    );
}
