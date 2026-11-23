//! MP4 export: read back with the `mp4` crate, decoded with OpenH264's
//! decoder and compared with the rendered frames, and checked by `ffprobe`
//! when it is installed.

mod common;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use moshionly_export::{
    Background, ExportError, Format, FrameRate, Moment, Settings, SettingsError,
};
use moshionly_render::RenderOptions;
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

/// The top-level boxes of an MP4 file, in order.
fn top_level_boxes(file: &[u8]) -> Vec<String> {
    let mut kinds = Vec::new();
    let mut at = 0;
    while at + 8 <= file.len() {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        kinds.push(String::from_utf8_lossy(&file[at + 4..at + 8]).into_owned());
        at += size.max(8);
    }
    kinds
}

/// Converts limited-range BT.709 planes back to RGB.
fn rgb_from_yuv(picture: &impl YUVSource) -> Vec<u8> {
    let (width, height) = picture.dimensions();
    let (ys, us, vs) = picture.strides();
    let (kr, kb) = (0.2126, 0.0722);
    let kg = 1.0 - kr - kb;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let l = (f64::from(picture.y()[y * ys + x]) - 16.0) / 219.0;
            let pb = (f64::from(picture.u()[y / 2 * us + x / 2]) - 128.0) / 224.0;
            let pr = (f64::from(picture.v()[y / 2 * vs + x / 2]) - 128.0) / 224.0;
            let r = l + 2.0 * (1.0 - kr) * pr;
            let b = l + 2.0 * (1.0 - kb) * pb;
            let g = (l - kr * r - kb * b) / kg;
            rgb.extend([r, g, b].map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8));
        }
    }
    rgb
}

/// A length-prefixed MP4 sample as an Annex B stream.
fn annex_b(sample: &[u8]) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut at = 0;
    while at + 4 <= sample.len() {
        let length = u32::from_be_bytes(sample[at..at + 4].try_into().unwrap()) as usize;
        stream.extend([0, 0, 0, 1]);
        stream.extend_from_slice(&sample[at + 4..at + 4 + length]);
        at += 4 + length;
    }
    stream
}

#[test]
fn the_video_plays_back_as_the_rendered_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.mp4");
    let settings = Settings::default().with_scale(0.5);
    let (result, _) = common::run(&common::job(Format::Mp4, &path, settings));
    let exported = result.unwrap();
    assert_eq!(
        (exported.width, exported.height, exported.frames),
        (540, 540, 120)
    );
    assert_eq!(exported.duration_ms, Some(4000.0));
    let file = fs::read(&path).unwrap();
    assert_eq!(exported.bytes, file.len() as u64);
    // The header comes first, so the video starts before it has all loaded.
    assert_eq!(top_level_boxes(&file), ["ftyp", "moov", "mdat"]);

    let mut reader = mp4::Mp4Reader::read_header(Cursor::new(&file), file.len() as u64).unwrap();
    assert_eq!(reader.duration().as_millis(), 4000);
    let track = reader.tracks().get(&1).unwrap();
    assert_eq!(track.media_type().unwrap(), mp4::MediaType::H264);
    assert_eq!((track.width(), track.height()), (540, 540));
    assert_eq!(track.sample_count(), 120);
    assert_eq!(track.timescale(), 30_000);
    assert_eq!(track.frame_rate(), 30.0);
    let avcc = &track.trak.mdia.minf.stbl.stsd.avc1.as_ref().unwrap().avcc;
    let mut parameters = Vec::new();
    for unit in avcc
        .sequence_parameter_sets
        .iter()
        .chain(&avcc.picture_parameter_sets)
    {
        parameters.extend([0, 0, 0, 1]);
        parameters.extend_from_slice(&unit.bytes);
    }

    let mut decoder = Decoder::new().unwrap();
    let options = RenderOptions::new().with_scale(0.5);
    let mut checked = 0;
    for index in 0..120u32 {
        let sample = reader.read_sample(1, index + 1).unwrap().unwrap();
        assert_eq!(sample.start_time, u64::from(index) * 1000);
        assert_eq!(
            sample.is_sync,
            index % 60 == 0,
            "a key frame every two seconds"
        );
        let mut packet = if index == 0 {
            parameters.clone()
        } else {
            Vec::new()
        };
        packet.extend(annex_b(&sample.bytes));
        let Some(picture) = decoder.decode(&packet).unwrap() else {
            continue;
        };
        if index % 20 != 0 && index != 119 {
            continue;
        }
        assert_eq!(picture.dimensions(), (540, 540));
        let decoded = rgb_from_yuv(&picture);
        let time = f64::from(index) * 1000.0 / 30.0;
        let expected = common::reference(Moment::At(time), &options).to_unpremultiplied();
        let psnr = common::psnr(&decoded, &common::rgb_of(&expected));
        assert!(psnr > 34.0, "frame {index}: {psnr:.1} dB");
        checked += 1;
    }
    assert!(checked >= 6, "{checked} frames compared");
}

#[test]
fn television_rates_keep_exact_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ntsc.mp4");
    let settings = Settings::default()
        .with_scale(0.25)
        .with_frame_rate(FrameRate::FPS_29_97)
        .with_range(Some(1000.0), Some(2001.0));
    let (result, _) = common::run(&common::job(Format::Mp4, &path, settings));
    assert_eq!(result.unwrap().frames, 30);
    let file = fs::read(&path).unwrap();
    let mut reader = mp4::Mp4Reader::read_header(Cursor::new(&file), file.len() as u64).unwrap();
    let track = reader.tracks().get(&1).unwrap();
    assert_eq!(track.timescale(), 30_000);
    assert_eq!(track.duration().as_micros(), 1_001_000);
    let last = reader.read_sample(1, 30).unwrap().unwrap();
    assert_eq!((last.start_time, last.duration), (29 * 1001, 1001));
}

#[test]
fn mp4_cannot_be_transparent() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::default().with_background(Background::Transparent);
    let job = common::job(Format::Mp4, dir.path().join("clear.mp4"), settings);
    let (result, _) = common::run(&job);
    assert!(matches!(
        result,
        Err(ExportError::Settings(SettingsError::CannotBeTransparent(
            Format::Mp4
        )))
    ));
    assert!(common::names_in(dir.path()).is_empty());
}

/// What `ffprobe` says about the first video stream, as `key=value` lines.
fn probe(path: &Path) -> Option<String> {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=codec_name,profile,width,height,pix_fmt,nb_read_frames,color_space,color_primaries,color_transfer,color_range,avg_frame_rate:format=duration")
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn ffprobe_agrees_when_it_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("probe.mp4");
    let settings = Settings::default()
        .with_scale(0.2)
        .with_range(None, Some(1000.0));
    common::run(&common::job(Format::Mp4, &path, settings))
        .0
        .unwrap();
    let Some(report) = probe(&path) else {
        println!("skipped: ffprobe is not installed");
        return;
    };
    for line in [
        "codec_name=h264",
        "width=216",
        "height=216",
        "pix_fmt=yuv420p",
        "nb_read_frames=30",
        "color_space=bt709",
        "color_primaries=bt709",
        "color_transfer=bt709",
        "color_range=tv",
        "avg_frame_rate=30/1",
        "duration=1.000000",
    ] {
        assert!(report.lines().any(|got| got == line), "{line} in\n{report}");
    }
}

#[test]
fn cancelling_an_mp4_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let job = common::job(
        Format::Mp4,
        dir.path().join("stopped.mp4"),
        Settings::default(),
    );
    let cancel = moshionly_export::CancelToken::new();
    let error = job
        .run(
            |progress| {
                if progress.frames_done == 10 {
                    cancel.cancel();
                }
            },
            &cancel,
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::Cancelled), "{error}");
    assert!(common::names_in(dir.path()).is_empty());
}
