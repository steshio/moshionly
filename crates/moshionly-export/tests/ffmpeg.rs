//! WebM and ProRes export through `ffmpeg`, checked with `ffprobe`.
//!
//! These tests need `ffmpeg` and `ffprobe`. Where they are not installed
//! (the Ubuntu CI machines, for example) each test prints why and passes
//! without checking anything.

mod common;

use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use moshionly_export::{
    Background, CancelToken, ExportError, Ffmpeg, Format, FrameRate, Phase, Quality, Settings,
    availability_with,
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
             color_space,color_primaries,color_transfer:stream_tags=alpha_mode:\
             format=duration,format_name",
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
            "color_primaries=bt709",
            "color_transfer=bt709",
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
fn the_same_export_gives_the_same_bytes() {
    if !tools_present("the_same_export_gives_the_same_bytes") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::default()
        .with_scale(0.1)
        .with_quality(Quality::Draft)
        .with_range(None, Some(500.0));
    for format in [Format::WebM, Format::ProRes] {
        let first = dir
            .path()
            .join(format!("a.{}", format.extension().unwrap()));
        let second = dir
            .path()
            .join(format!("b.{}", format.extension().unwrap()));
        common::run(&common::job(format, &first, settings))
            .0
            .unwrap();
        common::run(&common::job(format, &second, settings))
            .0
            .unwrap();
        assert!(
            std::fs::read(first).unwrap() == std::fs::read(second).unwrap(),
            "{format}"
        );
    }
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

/// A shell script standing in for ffmpeg, in `dir`. Waits until the system
/// lets it run: on Linux a script just written can briefly be "busy" while
/// another test starts a program.
#[cfg(unix)]
fn stand_in(dir: &Path, script: &str) -> Ffmpeg {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("ffmpeg");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..50 {
        match Command::new(&path)
            .args(["-hide_banner", "-encoders"])
            .output()
        {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => break,
        }
    }
    Ffmpeg::at(path)
}

#[test]
#[cfg(unix)]
fn an_ffmpeg_that_cannot_run_is_not_called_incomplete() {
    // What Homebrew's ffmpeg does after an upgrade removed a library.
    let dir = tempfile::tempdir().unwrap();
    let broken = stand_in(
        dir.path(),
        "echo 'dyld[4242]: Library not loaded: /opt/homebrew/opt/x265/lib/libx265.215.dylib' >&2\n\
         exit 134",
    );
    let reason = broken.check(Format::WebM).unwrap_err();
    assert!(!reason.contains("built without"), "{reason}");
    assert!(reason.contains("failed to run"), "{reason}");
    assert!(reason.contains("134"), "{reason}");
    assert!(reason.contains("Library not loaded"), "{reason}");
    assert!(reason.contains("reinstall"), "{reason}");
    let entries = availability_with(Some(&broken));
    let prores = entries
        .iter()
        .find(|entry| entry.format == Format::ProRes)
        .unwrap();
    let missing = prores.missing.as_deref().unwrap();
    assert!(missing.contains("Library not loaded"), "{missing}");
}

/// The start of a stand-in that lists the encoders when asked, like a full
/// ffmpeg, and otherwise goes on to the rest of the script.
#[cfg(unix)]
const LISTS_ENCODERS: &str = r#"for arg in "$@"; do
  if [ "$arg" = "-encoders" ]; then
    echo " V....D libvpx-vp9           libvpx VP9"
    echo " V....D prores_ks            Apple ProRes"
    exit 0
  fi
done"#;

#[test]
#[cfg(unix)]
fn cancelling_while_ffmpeg_finishes_the_file_stops_it_at_once() {
    let tools = tempfile::tempdir().unwrap();
    // Reads every frame, then takes 30 seconds to finish the file.
    let slow = stand_in(
        tools.path(),
        &format!("{LISTS_ENCODERS}\ncat > /dev/null\nexec sleep 30"),
    );
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::default()
        .with_scale(0.1)
        .with_range(None, Some(200.0));
    let job = common::job(Format::WebM, dir.path().join("slow.webm"), settings).with_ffmpeg(slow);
    let cancel = CancelToken::new();
    let started = Instant::now();
    let error = job
        .run(
            |progress| {
                if progress.phase == Phase::Encoding {
                    cancel.cancel();
                }
            },
            &cancel,
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::Cancelled), "{error}");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert!(common::names_in(dir.path()).is_empty());
}

#[test]
#[cfg(unix)]
fn cancelling_while_ffmpeg_takes_no_frames_stops_it_at_once() {
    let tools = tempfile::tempdir().unwrap();
    // Never reads its input, so the first frame fills the pipe and writing
    // it waits.
    let stuck = stand_in(tools.path(), &format!("{LISTS_ENCODERS}\nexec sleep 30"));
    let dir = tempfile::tempdir().unwrap();
    let job = common::job(
        Format::ProRes,
        dir.path().join("stuck.mov"),
        Settings::default().with_scale(0.25),
    )
    .with_ffmpeg(stuck);
    let cancel = CancelToken::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let error = job.run(|_| {}, &cancel).unwrap_err();
    canceller.join().unwrap();
    assert!(matches!(error, ExportError::Cancelled), "{error}");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert!(common::names_in(dir.path()).is_empty());
}
