//! Video and sound files, read with `ffmpeg`.
//!
//! Moshionly links no video or sound decoders. Like the WebM and ProRes
//! exports, media goes through an installed `ffmpeg` (and the `ffprobe`
//! beside it), run as separate programs. Without them media cannot be
//! added, and video layers in a file draw a placeholder.
//!
//! - [`MediaTools`] finds the programs; [`MediaTools::probe`] (or
//!   [`probe`]) says how long a file is, how large its picture is and
//!   whether it has video and sound.
//! - [`VideoDecoder`] is the renderer's [`VideoFrames`](moshionly_render::VideoFrames):
//!   it reads frames through long-lived `ffmpeg` processes and keeps
//!   recent ones.
//! - [`SoundDecoder`] reads sound as 48 kHz stereo samples and mixes the
//!   sound of an artboard ([`moshionly_core::eval::sounds`]) into one track.
//!
//! The editor uses all three; exports draw video with a [`VideoDecoder`]
//! and add the mix as a sound track.

mod cache;
mod frames;
mod probe;
mod sound;
mod stream;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use frames::{DEFAULT_FRAME_BUDGET, VideoDecoder};
pub use probe::MediaInfo;
pub use sound::{DEFAULT_SOUND_BUDGET, Mix, SoundDecoder};

use crate::ffmpeg::{self, Ffmpeg, INSTALL_HINT};

/// Samples per second of decoded and mixed sound.
pub const SAMPLE_RATE: u32 = 48_000;

/// Channels of decoded and mixed sound: left and right, interleaved.
pub const CHANNELS: usize = 2;

/// Why a media file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// `ffmpeg` or `ffprobe` is not installed.
    #[error("video and sound need ffmpeg, which was not found; {INSTALL_HINT}")]
    ToolsMissing,
    /// `ffmpeg` is there but `ffprobe`, which comes with it, is not.
    #[error(
        "ffprobe, which comes with ffmpeg, was not found next to {}; reinstall ffmpeg \
         (on a Mac with Homebrew: brew reinstall ffmpeg)",
        .0.display()
    )]
    NoFfprobe(PathBuf),
    /// There is no file at the path, or it is not a plain file.
    #[error("there is no file at {}", .0.display())]
    NoFile(PathBuf),
    /// The file is not a video or sound file `ffmpeg` can read.
    #[error("{} is not a video or sound file ffmpeg can read: {reason}", .path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// What `ffprobe` said.
        reason: String,
    },
    /// The file has no picture.
    #[error("{} has no video", .0.display())]
    NoVideo(PathBuf),
    /// `ffmpeg` failed while reading the file.
    #[error("{} could not be read: {message}", .path.display())]
    Failed {
        /// The file.
        path: PathBuf,
        /// What went wrong, in ffmpeg's words when it said anything.
        message: String,
    },
    /// The work was cancelled.
    #[error("cancelled")]
    Cancelled,
}

/// The installed `ffmpeg` and `ffprobe` that media is read with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl MediaTools {
    /// Finds `ffmpeg` and its `ffprobe`, on `PATH` or in the usual install
    /// places. `None` when either is missing.
    pub fn locate() -> Option<MediaTools> {
        MediaTools::with_ffmpeg(&Ffmpeg::locate()?).ok()
    }

    /// The tools of `ffmpeg`: its `ffprobe` is the one in the same folder,
    /// or else the first one found like `ffmpeg` is.
    pub fn with_ffmpeg(ffmpeg: &Ffmpeg) -> Result<MediaTools, MediaError> {
        let name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        let beside = ffmpeg.path().with_file_name(name);
        let ffprobe = if ffmpeg::is_program(&beside) {
            beside
        } else {
            ffmpeg::find_program("ffprobe")
                .ok_or_else(|| MediaError::NoFfprobe(ffmpeg.path().to_owned()))?
        };
        Ok(MediaTools {
            ffmpeg: ffmpeg.path().to_owned(),
            ffprobe,
        })
    }

    /// Where `ffmpeg` is.
    pub fn ffmpeg(&self) -> &Path {
        &self.ffmpeg
    }

    /// Where `ffprobe` is.
    pub fn ffprobe(&self) -> &Path {
        &self.ffprobe
    }

    /// How long the file at `path` is, how large its picture is, and
    /// whether it has video and sound.
    pub fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        probe::run(self, path).map(|probed| probed.info)
    }
}

/// [`MediaTools::probe`] with the tools found by [`MediaTools::locate`].
pub fn probe(path: &Path) -> Result<MediaInfo, MediaError> {
    MediaTools::locate()
        .ok_or(MediaError::ToolsMissing)?
        .probe(path)
}

/// When the file at `path` was last changed, so that what is kept of it
/// can be told apart from a newer version. Anything but a plain file is
/// refused.
fn modified(path: &Path) -> Result<SystemTime, MediaError> {
    let metadata = std::fs::metadata(path).map_err(|_| MediaError::NoFile(path.to_owned()))?;
    if !metadata.is_file() {
        return Err(MediaError::NoFile(path.to_owned()));
    }
    metadata.modified().map_err(|error| MediaError::Failed {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

/// A time in milliseconds as whole microseconds, the unit frames and seeks
/// are matched in, so that the same time always gives the same frame.
fn micros(time_ms: f64) -> i64 {
    if time_ms.is_finite() {
        (time_ms * 1000.0).round() as i64
    } else {
        0
    }
}

/// Seconds for an `ffmpeg` time option, from microseconds.
fn seconds(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let micros = micros.unsigned_abs();
    format!("{sign}{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_matched_in_whole_microseconds() {
        assert_eq!(micros(1250.0), 1_250_000);
        assert_eq!(micros(0.0004), 0);
        assert_eq!(micros(0.0006), 1);
        assert_eq!(micros(f64::NAN), 0);
        assert_eq!(seconds(1_250_000), "1.250000");
        assert_eq!(seconds(5), "0.000005");
        assert_eq!(seconds(-1_500_000), "-1.500000");
    }

    #[test]
    fn missing_files_are_said_to_be_missing() {
        assert_eq!(
            modified(Path::new("/no/such/clip.mp4")),
            Err(MediaError::NoFile("/no/such/clip.mp4".into()))
        );
        let folder = std::env::temp_dir();
        assert_eq!(modified(&folder), Err(MediaError::NoFile(folder.clone())));
    }

    #[test]
    fn messages_say_how_to_get_ffmpeg() {
        assert!(
            MediaError::ToolsMissing
                .to_string()
                .contains("brew install ffmpeg")
        );
        let tools = MediaTools::with_ffmpeg(&Ffmpeg::at("/nowhere/ffmpeg"));
        // Either an ffprobe elsewhere is found, or the message names the
        // ffmpeg it looked beside.
        if let Err(error) = tools {
            assert!(error.to_string().contains("/nowhere/ffmpeg"), "{error}");
        }
    }
}
