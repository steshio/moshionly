//! What a media file holds, from `ffprobe`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use serde_json::Value;

use super::{MediaError, MediaTools, modified};
use crate::ffmpeg::last_words;

/// What a video or sound file holds.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    /// How long the file plays, in milliseconds, when it says.
    pub duration_ms: Option<f64>,
    /// Width of the picture in pixels as it is shown (turned upright); 0
    /// without video.
    pub width: u32,
    /// Height of the picture in pixels as it is shown; 0 without video.
    pub height: u32,
    /// Whether the file has a picture. Cover art in a sound file does not
    /// count.
    pub has_video: bool,
    /// Whether the file has sound.
    pub has_sound: bool,
    /// Frames per second of the picture, when known.
    pub frame_rate: Option<f64>,
}

/// [`MediaInfo`] and what reading the file needs besides.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Probed {
    /// What the file holds.
    pub info: MediaInfo,
    /// Where the file's timeline starts, in microseconds: time 0 of a clip
    /// is this timestamp.
    pub start_us: i64,
    /// Channels of the sound; 0 without sound.
    pub channels: u64,
}

/// What files hold, asked once for each version of a file.
#[derive(Default)]
pub(crate) struct Probes(Mutex<HashMap<PathBuf, (SystemTime, ProbeResult)>>);

type ProbeResult = Result<Arc<Probed>, MediaError>;

impl Probes {
    /// What the file at `path` holds, and when it was last changed.
    pub fn get(
        &self,
        tools: &MediaTools,
        path: &Path,
    ) -> Result<(SystemTime, Arc<Probed>), MediaError> {
        let modified = modified(path)?;
        let known = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned();
        if let Some((when, probed)) = known
            && when == modified
        {
            return probed.map(|probed| (modified, probed));
        }
        let probed = run(tools, path).map(Arc::new);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), (modified, probed.clone()));
        probed.map(|probed| (modified, probed))
    }
}

/// How much of what `ffprobe` prints on failure is kept for the message.
const MESSAGE_BYTES: usize = 400;

/// Asks `ffprobe` what the file at `path` holds.
pub(crate) fn run(tools: &MediaTools, path: &Path) -> Result<Probed, MediaError> {
    modified(path)?;
    let output = Command::new(tools.ffprobe())
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(input(path))
        .stdin(Stdio::null())
        .output()
        .map_err(|error| MediaError::Failed {
            path: path.to_owned(),
            message: format!(
                "{} could not be started ({error})",
                tools.ffprobe().display()
            ),
        })?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(MediaError::Unreadable {
            path: path.to_owned(),
            reason: match last_words(&said, MESSAGE_BYTES) {
                "" => format!("ffprobe stopped with {}", output.status),
                said => said.to_owned(),
            },
        });
    }
    let json: Value =
        serde_json::from_slice(&output.stdout).map_err(|error| MediaError::Unreadable {
            path: path.to_owned(),
            reason: format!("ffprobe's answer could not be read ({error})"),
        })?;
    let probed = parse(&json);
    if !probed.info.has_video && !probed.info.has_sound {
        return Err(MediaError::Unreadable {
            path: path.to_owned(),
            reason: "it has neither video nor sound".into(),
        });
    }
    Ok(probed)
}

/// The argument that names `path` as an input to `ffmpeg` or `ffprobe`:
/// with the `file:` protocol, so that a name with a colon in it is never
/// taken for another protocol.
pub(crate) fn input(path: &Path) -> std::ffi::OsString {
    let mut argument = std::ffi::OsString::from("file:");
    argument.push(path.as_os_str());
    argument
}

/// Reads `ffprobe -show_format -show_streams -of json`.
fn parse(json: &Value) -> Probed {
    let streams: &[Value] = json["streams"].as_array().map_or(&[], Vec::as_slice);
    let kind = |stream: &&Value, wanted: &str| stream["codec_type"] == wanted;
    // Cover art of a sound file is a still picture, not video.
    let video = streams
        .iter()
        .find(|stream| kind(stream, "video") && stream["disposition"]["attached_pic"] != 1);
    let sound = streams.iter().find(|stream| kind(stream, "audio"));
    let (mut width, mut height) = video.map_or((0, 0), |video| {
        let side = |name: &str| video[name].as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        (side("width"), side("height"))
    });
    if video.is_some_and(|video| rotation(video).rem_euclid(180) == 90) {
        (width, height) = (height, width);
    }
    let frame_rate = video.and_then(|video| {
        ratio(&video["avg_frame_rate"]).or_else(|| ratio(&video["r_frame_rate"]))
    });
    let duration_ms = number(&json["format"]["duration"])
        .or_else(|| {
            streams
                .iter()
                .filter_map(|stream| number(&stream["duration"]))
                .reduce(f64::max)
        })
        .map(|seconds| seconds * 1000.0);
    let start_us = number(&json["format"]["start_time"])
        .map_or(0, |seconds| (seconds * 1_000_000.0).round() as i64);
    Probed {
        info: MediaInfo {
            duration_ms,
            width,
            height,
            has_video: video.is_some() && width > 0 && height > 0,
            has_sound: sound.is_some(),
            frame_rate,
        },
        start_us,
        channels: sound
            .and_then(|sound| sound["channels"].as_u64())
            .unwrap_or(0),
    }
}

/// How far the picture is turned to be shown, in whole degrees: from the
/// display matrix, or the older `rotate` tag.
fn rotation(video: &Value) -> i64 {
    let from_matrix = video["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|side| number(&side["rotation"]));
    let from_tag = || number(&video["tags"]["rotate"]);
    from_matrix
        .or_else(from_tag)
        .map_or(0, |degrees| degrees.round() as i64)
}

/// A number that `ffprobe` writes as a number or as a string.
fn number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }?;
    number.is_finite().then_some(number)
}

/// A rate written as `"30000/1001"`; `None` for `"0/0"` and the like.
fn ratio(value: &Value) -> Option<f64> {
    let (top, bottom) = value.as_str()?.split_once('/')?;
    let rate = top.trim().parse::<f64>().ok()? / bottom.trim().parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_video_with_sound_is_read() {
        let probed = parse(&json!({
            "streams": [
                { "codec_type": "video", "width": 1920, "height": 1080,
                  "avg_frame_rate": "30000/1001", "r_frame_rate": "30/1",
                  "disposition": { "attached_pic": 0 } },
                { "codec_type": "audio", "channels": 1 }
            ],
            "format": { "duration": "12.500000", "start_time": "0.021000" }
        }));
        assert_eq!(
            probed.info,
            MediaInfo {
                duration_ms: Some(12_500.0),
                width: 1920,
                height: 1080,
                has_video: true,
                has_sound: true,
                frame_rate: Some(30_000.0 / 1001.0),
            }
        );
        assert_eq!((probed.start_us, probed.channels), (21_000, 1));
    }

    #[test]
    fn a_phone_video_turned_upright_swaps_its_sides() {
        let matrix = parse(&json!({ "streams": [
            { "codec_type": "video", "width": 1920, "height": 1080,
              "side_data_list": [{ "side_data_type": "Display Matrix", "rotation": -90 }] }
        ], "format": {} }));
        assert_eq!((matrix.info.width, matrix.info.height), (1080, 1920));
        let tag = parse(&json!({ "streams": [
            { "codec_type": "video", "width": 640, "height": 480, "tags": { "rotate": "180" } }
        ], "format": {} }));
        assert_eq!((tag.info.width, tag.info.height), (640, 480));
        assert_eq!(tag.info.duration_ms, None);
        assert_eq!(tag.start_us, 0);
    }

    #[test]
    fn cover_art_is_not_video() {
        let song = parse(&json!({
            "streams": [
                { "codec_type": "audio", "channels": 2, "duration": "3.5" },
                { "codec_type": "video", "width": 600, "height": 600,
                  "disposition": { "attached_pic": 1 } }
            ],
            "format": {}
        }));
        assert!(!song.info.has_video && song.info.has_sound);
        assert_eq!((song.info.width, song.info.height), (0, 0));
        assert_eq!(song.info.duration_ms, Some(3500.0));
        assert_eq!(song.info.frame_rate, None);
        assert_eq!(ratio(&json!("0/0")), None);
    }

    #[test]
    fn inputs_are_named_as_files() {
        assert_eq!(input(Path::new("/clips/a:b.mp4")), "file:/clips/a:b.mp4");
    }
}
