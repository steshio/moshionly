//! WebM and ProRes export through `ffmpeg`, checked with `ffprobe`.
//!
//! These tests need `ffmpeg` and `ffprobe`. Where they are not installed
//! (the Ubuntu CI machines, for example) each test prints why and passes
//! without checking anything.

mod common;

use std::path::Path;
use std::process::Command;

use moshionly_export::{
    Background, CancelToken, ExportError, Ffmpeg, Format, FrameRate, Quality, Settings,
};

/// Whether ffmpeg and ffprobe are both here; prints why not.
fn tools_present(test: &str) -> bool {
    let ffprobe = Command::new("ffprobe").arg("-version").output().is_ok();
    let ffmpeg = Ffmpeg::locate().is_some();
    if !(ffmpeg && ffprobe) {
        println!("skipped {test}: ffmpeg or ffprobe is not installed");
    }
    ffmpeg && ffprobe
}

/// What ffprobe says about the video stream, as `key=value` lines.
fn probe(path: &Path, decoder: Option<&str>) -> String {
    let mut command = Command::new("ffprobe");
    command.args(["-v", "error"]);
    if let Some(decoder) = decoder {
        command.args(["-c:v", decoder]);
    }
    let output = command
        .args(["-count_frames", "-select_streams", "v:0", "-show_entries"])
        .arg(
            "stream=codec_name,profile,width,height,pix_fmt,nb_read_frames,avg_frame_rate,\
             color_space:stream_tags=alpha_mode:format=duration,format_name",
        )
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn has(report: &str, line: &str) -> bool {
    report.lines().any(|got| got.eq_ignore_ascii_case(line))
}

fn assert_has(report: &str, lines: &[&str]) {
    for line in lines {
        assert!(has(report, line), "{line} in\n{report}");
    }
}

#[test]
fn a_transparent_webm_keeps_its_alpha() {
    if !tools_present("a_transparent_webm_keeps_its_alpha") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clear.webm");
    let settings = Settings::default()
        .with_scale(0.2)
        .with_background(Background::Transparent)
        .with_quality(Quality::Draft)
        .with_range(None, Some(2000.0));
    let (result, _) = common::run(&common::job(Format::WebM, &path, settings));
    let exported = result.unwrap();
    assert_eq!(
        (exported.width, exported.height, exported.frames),
        (216, 216, 60)
    );
    assert!(common::names_in(dir.path()) == ["clear.webm"]);
    let report = probe(&path, None);
    assert_has(
        &report,
        &[
            "codec_name=vp9",
            "width=216",
            "height=216",
            "avg_frame_rate=30/1",
            "nb_read_frames=60",
            "color_space=bt709",
            "TAG:alpha_mode=1",
            "duration=2.000000",
        ],
    );
    // Decoded with libvpx, which reads the transparency layer too.
    assert_has(&probe(&path, Some("libvpx-vp9")), &["pix_fmt=yuva420p"]);
}

#[test]
fn an_opaque_webm_has_no_alpha() {
    if !tools_present("an_opaque_webm_has_no_alpha") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("solid.webm");
    let settings = Settings::default()
        .with_scale(0.1)
        .with_quality(Quality::Draft)
        .with_range(None, Some(500.0));
    common::run(&common::job(Format::WebM, &path, settings))
        .0
        .unwrap();
    let report = probe(&path, None);
    assert_has(
        &report,
        &["codec_name=vp9", "pix_fmt=yuv420p", "nb_read_frames=15"],
    );
    assert!(!has(&report, "TAG:alpha_mode=1"), "{report}");
}

#[test]
fn prores_4444_keeps_its_alpha_at_television_rates() {
    if !tools_present("prores_4444_keeps_its_alpha_at_television_rates") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clear.mov");
    let settings = Settings::default()
        .with_scale(0.2)
        .with_background(Background::Transparent)
        .with_frame_rate(FrameRate::FPS_29_97)
        .with_range(None, Some(1001.0));
    let (result, _) = common::run(&common::job(Format::ProRes, &path, settings));
    assert_eq!(result.unwrap().frames, 30);
    let report = probe(&path, None);
    assert_has(
        &report,
        &[
            "codec_name=prores",
            "profile=4444",
            "width=216",
            "height=216",
            "avg_frame_rate=30000/1001",
            "nb_read_frames=30",
            "duration=1.001000",
        ],
    );
    let pixel_format = report
        .lines()
        .find_map(|line| line.strip_prefix("pix_fmt="))
        .unwrap_or_default();
    assert!(pixel_format.starts_with("yuva444"), "{report}");
}

#[test]
fn cancelling_stops_ffmpeg_and_leaves_nothing() {
    if !tools_present("cancelling_stops_ffmpeg_and_leaves_nothing") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stopped.mov");
    let job = common::job(Format::ProRes, &path, Settings::default().with_scale(0.25));
    let cancel = CancelToken::new();
    let error = job
        .run(
            |progress| {
                if progress.frames_done == 5 {
                    cancel.cancel();
                }
            },
            &cancel,
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::Cancelled), "{error}");
    assert!(common::names_in(dir.path()).is_empty());
}

#[test]
fn a_missing_ffmpeg_is_explained_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let job = common::job(Format::WebM, dir.path().join("x.webm"), Settings::default())
        .with_ffmpeg(Ffmpeg::at(dir.path().join("no-ffmpeg-here")));
    let (result, reports) = common::run(&job);
    let error = result.unwrap_err();
    assert!(
        matches!(
            error,
            ExportError::FfmpegMissing {
                format: Format::WebM,
                ..
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("install ffmpeg"), "{error}");
    assert!(reports.is_empty());
    assert!(common::names_in(dir.path()).is_empty());
}
