//! Video and sound in the editor: reading them with an installed `ffmpeg`.
//!
//! Moshionly links no video or sound decoders (see `moshionly_export::media`).
//! When `ffmpeg` is found at start, the renderer draws video frames with a
//! [`VideoDecoder`], playback reads sound with a `SoundDecoder`, and media
//! can be imported; without it, the import is disabled and says how to get
//! `ffmpeg`, and video layers draw a placeholder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moshionly_agent::{HostError, MediaProbe};
use moshionly_export::media::{self, MediaError, MediaTools, VideoDecoder};
use moshionly_render::AssetLoader;

/// What the editor says when media needs `ffmpeg` and it is missing.
pub const FFMPEG_NEEDED: &str = if cfg!(target_os = "macos") {
    "Video and sound need ffmpeg. Install it with `brew install ffmpeg`, then restart Moshionly."
} else {
    "Video and sound need ffmpeg. Install it with your package manager, then restart Moshionly."
};

/// An asset loader for a project saved as `project` (or never saved),
/// that draws video frames with `ffmpeg` when `tools` are given.
pub fn asset_loader(project: Option<PathBuf>, tools: Option<&MediaTools>) -> AssetLoader {
    let loader = AssetLoader::new(project);
    match tools {
        Some(tools) => loader.with_video(Arc::new(VideoDecoder::new(tools.clone()))),
        None => loader,
    }
}

/// How the agent tools read media files: with `ffmpeg`'s `ffprobe`.
#[derive(Clone, Debug, Default)]
pub struct FfmpegProbe {
    /// The tools to read with; `None` finds them each time, so installing
    /// `ffmpeg` while the app runs is enough for agents.
    tools: Option<MediaTools>,
}

impl FfmpegProbe {
    /// A probe that finds `ffmpeg` each time it is asked.
    pub fn located() -> Self {
        Self::default()
    }

    /// A probe that reads with `tools`.
    pub fn with_tools(tools: MediaTools) -> Self {
        Self { tools: Some(tools) }
    }
}

impl MediaProbe for FfmpegProbe {
    fn probe(&self, path: &Path) -> Result<moshionly_agent::MediaInfo, HostError> {
        let info = match &self.tools {
            Some(tools) => tools.probe(path),
            None => media::probe(path),
        }
        .map_err(host_error)?;
        Ok(moshionly_agent::MediaInfo {
            duration_ms: info.duration_ms,
            width: info.width,
            height: info.height,
            has_video: info.has_video,
            has_sound: info.has_sound,
        })
    }
}

/// Why a file could not be read, for the agent.
fn host_error(error: MediaError) -> HostError {
    match error {
        MediaError::ToolsMissing | MediaError::NoFfprobe(_) => {
            HostError::Unsupported(error.to_string())
        }
        other => HostError::Failed(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_ffmpeg_is_said_plainly() {
        assert!(FFMPEG_NEEDED.contains("ffmpeg"));
        assert!(FFMPEG_NEEDED.ends_with("restart Moshionly."));
        assert!(matches!(
            host_error(MediaError::ToolsMissing),
            HostError::Unsupported(_)
        ));
        let unreadable = MediaError::Unreadable {
            path: "a.mp4".into(),
            reason: "no streams".into(),
        };
        assert!(matches!(host_error(unreadable), HostError::Failed(_)));
    }

    #[test]
    fn video_is_drawn_only_with_ffmpeg() {
        assert!(asset_loader(None, None).video().is_none());
        if let Some(tools) = MediaTools::locate() {
            assert!(asset_loader(None, Some(&tools)).video().is_some());
        }
    }

    #[test]
    fn agents_are_told_what_a_file_is_not() {
        let missing = FfmpegProbe::located().probe(Path::new("/no/such/clip.mp4"));
        assert!(missing.is_err());
    }
}
