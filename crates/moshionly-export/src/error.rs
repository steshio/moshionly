//! What can stop an export. Every message says what to do about it.

use std::io;
use std::path::PathBuf;

use moshionly_core::EvalError;
use moshionly_render::RenderError;
use thiserror::Error;

use crate::format::Format;
use crate::settings::SettingsError;

/// Why an export did not finish. When an export fails or is cancelled,
/// nothing is left at the destination: not a half-written file, and not a
/// half-filled folder.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The settings cannot be used for this artboard and format.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// The export was cancelled before it finished.
    #[error("the export was cancelled")]
    Cancelled,
    /// The artboard could not be evaluated.
    #[error("the artboard could not be prepared: {0}")]
    Evaluate(#[from] EvalError),
    /// A frame could not be rendered.
    #[error("frame {frame} could not be rendered: {source}")]
    Render {
        /// The frame, counting from 0.
        frame: u64,
        /// Why.
        #[source]
        source: RenderError,
    },
    /// The folder the export goes into does not exist.
    #[error("the folder {} does not exist; create it or choose another place", .0.display())]
    NoSuchFolder(PathBuf),
    /// A file export was pointed at a folder.
    #[error("{} is a folder; give the file a name, for example {}", .0.display(), .0.join("export.mp4").display())]
    DestinationIsFolder(PathBuf),
    /// A PNG sequence was pointed at a file, or at a folder with files in it.
    #[error(
        "{} already exists and is not an empty folder; choose a new folder name so that \
         nothing is overwritten",
        .0.display()
    )]
    FolderNotEmpty(PathBuf),
    /// Reading or writing a file failed.
    #[error(
        "{} could not be {action}: {source}. Check that there is free space and that you may \
         write there",
        path.display()
    )]
    Io {
        /// What was being done: "written", "created", "moved into place".
        action: &'static str,
        /// The file or folder.
        path: PathBuf,
        /// What the system said.
        #[source]
        source: io::Error,
    },
    /// The format needs `ffmpeg`, and it is not installed or cannot write
    /// the format.
    #[error("{format} export needs ffmpeg: {reason}")]
    FfmpegMissing {
        /// The format.
        format: Format,
        /// What is missing and how to get it.
        reason: String,
    },
    /// `ffmpeg` stopped with an error.
    #[error("ffmpeg could not write the {format} video: {message}")]
    Ffmpeg {
        /// The format.
        format: Format,
        /// What ffmpeg said, or how it stopped.
        message: String,
    },
    /// The H.264 encoder failed.
    #[error("the MP4 video could not be encoded: {0}")]
    H264(String),
    /// The GIF encoder failed.
    #[error("the GIF could not be encoded: {0}")]
    Gif(String),
}

impl ExportError {
    /// A failed read or write of `path`.
    pub(crate) fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        ExportError::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_say_what_to_do() {
        let error = ExportError::NoSuchFolder(PathBuf::from("/nowhere"));
        assert!(error.to_string().contains("create it"), "{error}");
        let error = ExportError::FolderNotEmpty(PathBuf::from("/frames"));
        assert!(error.to_string().contains("new folder name"), "{error}");
        let error = ExportError::io(
            "written",
            "/out.gif",
            io::Error::new(io::ErrorKind::StorageFull, "disk full"),
        );
        let text = error.to_string();
        assert!(
            text.contains("/out.gif") && text.contains("disk full"),
            "{text}"
        );
        let error = ExportError::DestinationIsFolder(PathBuf::from("/videos"));
        assert!(error.to_string().contains("/videos/export.mp4"), "{error}");
    }
}
