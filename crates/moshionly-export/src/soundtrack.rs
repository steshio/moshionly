//! The sound track of an export: the artboard's sound, mixed for the
//! stretch exported and handed to `ffmpeg`.
//!
//! MP4, WebM and ProRes carry sound when the artboard has some and
//! [`Settings::sound`](crate::Settings::sound) is on. The mix is written
//! beside the export as raw 48 kHz stereo float samples, which `ffmpeg`
//! reads as a second input: WebM gets Opus, ProRes 24-bit PCM, and an MP4
//! (written in-process without sound) is copied once more with AAC sound
//! added. Without `ffmpeg` the file is written without sound, with a
//! warning; GIF, PNG and PNG sequences have no sound, and say so.

use std::ffi::OsString;
use std::io::{BufWriter, Write};
use std::path::Path;

use moshionly_core::eval::{duration_of, sounds};
use tempfile::TempPath;

use crate::error::ExportError;
use crate::ffmpeg::{Ffmpeg, INSTALL_HINT, Running};
use crate::format::Format;
use crate::job::ExportJob;
use crate::media::{CHANNELS, MediaError, MediaTools, SAMPLE_RATE, SoundDecoder};
use crate::progress::CancelToken;
use crate::settings::Plan;

/// The mixed sound of an export, in a temporary file beside it that goes
/// when this is dropped.
pub(crate) struct SoundTrack {
    file: TempPath,
}

impl SoundTrack {
    /// The `ffmpeg` arguments that read the track as an input.
    pub fn input(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = ["-f", "f32le", "-ar", &SAMPLE_RATE.to_string(), "-ac"]
            .iter()
            .map(OsString::from)
            .collect();
        args.push(CHANNELS.to_string().into());
        args.push("-i".into());
        let mut file = OsString::from("file:");
        file.push(self.file.as_os_str());
        args.push(file);
        args
    }
}

/// The sound of `job`'s export, mixed into a file in `dir`, with what
/// to tell the person about it. `None` when the export gets no sound:
/// there is none, it is turned off, the format cannot hold it, or it
/// cannot be read.
pub(crate) fn prepare(
    job: &ExportJob,
    plan: &Plan,
    dir: &Path,
    cancel: &CancelToken,
) -> Result<(Option<SoundTrack>, Vec<String>), ExportError> {
    let mut notes = Vec::new();
    let (from_ms, to_ms) = match plan.moments.frames() {
        Some(times) => (times.start_ms(), times.start_ms() + times.duration_ms()),
        None => (
            0.0,
            duration_of(&job.document, &job.artboard).unwrap_or(0.0),
        ),
    };
    let has_sound = !sounds(&job.document, &job.artboard, from_ms, to_ms).is_empty();
    if !has_sound || !job.settings.sound {
        return Ok((None, notes));
    }
    if !plan.format.supports_sound() {
        // Lottie names each sound and video it leaves out itself.
        if plan.format != Format::Lottie {
            notes.push(format!(
                "{} files hold no sound, so the artboard's sound was left out; export MP4, \
                 WebM or ProRes to keep it",
                plan.format
            ));
        }
        return Ok((None, notes));
    }
    let Some(ffmpeg) = job.ffmpeg.clone().or_else(Ffmpeg::locate) else {
        notes.push(format!(
            "the sound was left out: adding sound needs ffmpeg, which was not found; \
             {INSTALL_HINT}"
        ));
        return Ok((None, notes));
    };
    if let Err(reason) = ffmpeg.check_sound(plan.format) {
        notes.push(format!("the sound was left out: {reason}"));
        return Ok((None, notes));
    }
    let tools = match MediaTools::with_ffmpeg(&ffmpeg) {
        Ok(tools) => tools,
        Err(error) => {
            notes.push(format!("the sound was left out: {error}"));
            return Ok((None, notes));
        }
    };
    let mix = SoundDecoder::new(tools)
        .mix(
            &job.document,
            &job.artboard,
            job.project_file.as_deref(),
            from_ms,
            to_ms,
            cancel,
        )
        .map_err(|error| match error {
            MediaError::Cancelled => ExportError::Cancelled,
            other => ExportError::Internal {
                frame: 0,
                message: other.to_string(),
            },
        })?;
    notes.extend(mix.warnings);
    if mix.sounds == 0 {
        return Ok((None, notes));
    }
    let file = tempfile::Builder::new()
        .prefix(".sound.")
        .suffix(".part")
        .tempfile_in(dir)
        .map_err(|error| ExportError::io("created", dir, error))?;
    let path = file.path().to_owned();
    let written = (|| {
        let mut writer = BufWriter::new(file.as_file());
        for sample in &mix.samples {
            writer.write_all(&sample.to_le_bytes())?;
        }
        writer.flush()
    })();
    written.map_err(|error| ExportError::io("written", &path, error))?;
    Ok((
        Some(SoundTrack {
            file: file.into_temp_path(),
        }),
        notes,
    ))
}

/// Adds `sound` to the MP4 at `path` as AAC with `ffmpeg`, copying the
/// video as it is. The result replaces the file at `path`.
pub(crate) fn add_to_mp4(
    path: &Path,
    sound: &SoundTrack,
    ffmpeg: &Ffmpeg,
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let muxed: TempPath = tempfile::Builder::new()
        .prefix(".with-sound.")
        .suffix(".part")
        .tempfile_in(dir)
        .map_err(|error| ExportError::io("created", dir, error))?
        .into_temp_path();
    let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-nostats", "-y"]
        .iter()
        .map(OsString::from)
        .collect();
    let mut video = OsString::from("file:");
    video.push(path.as_os_str());
    args.extend(["-i".into(), video]);
    args.extend(sound.input());
    args.extend(
        [
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-fflags",
            "+bitexact",
            "-flags:a",
            "+bitexact",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .iter()
        .map(OsString::from),
    );
    args.push(muxed.as_os_str().to_owned());
    Running::start(ffmpeg, &args, Format::Mp4, cancel.clone())?.finish()?;
    muxed
        .persist(path)
        .map_err(|error| ExportError::io("written", path, error.error))?;
    Ok(())
}

#[cfg(test)]
impl SoundTrack {
    /// A track read from `path`, for tests.
    pub fn at(path: std::path::PathBuf) -> Self {
        Self {
            file: TempPath::try_from_path(path).expect("an absolute path"),
        }
    }
}
