//! What the host supplies: rendered frames, exports, fonts and text
//! measuring.
//!
//! This crate knows documents, not pixels. Whoever embeds it (the editor,
//! the `moshionly` command) owns the renderer and the exporter and hands
//! them in as trait objects in a [`Host`]. A tool that needs one that is
//! missing fails with a message saying it is not available in this host, so
//! the same tools work headless, with fewer abilities.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use moshionly_core::{Document, MonospaceMeasurer, NodeId, Rect, TextMeasurer};

/// Why the host could not do what a tool asked.
///
/// The message goes to the model, so say what to do instead where there is
/// something to do.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// The host cannot do this at all, for example write a format whose
    /// encoder is not installed.
    #[error("{0}")]
    Unsupported(String),
    /// The host tried and failed.
    #[error("{0}")]
    Failed(String),
    /// A file could not be read or written.
    #[error("could not access \"{}\": {source}", .path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
}

/// What to render.
#[derive(Debug, Clone, Copy)]
pub struct FrameRequest<'a> {
    /// The document to render from.
    pub document: &'a Document,
    /// The project file, for finding images stored next to it.
    pub project_file: Option<&'a Path>,
    /// The artboard to render.
    pub artboard: &'a NodeId,
    /// A layer of that artboard to frame. When set, the picture shows the
    /// part of the artboard this layer covers at that moment, with a little
    /// room for its stroke and shadow, instead of the whole artboard.
    pub layer: Option<&'a NodeId>,
    /// The moment to render, in milliseconds from the start of the
    /// artboard. `None` renders the design at rest, with no animation
    /// applied.
    pub time_ms: Option<f64>,
    /// A region of the artboard, in the artboard's coordinates, to render
    /// instead of all of it. Together with `layer`, the picture shows where
    /// the two overlap.
    pub crop: Option<Rect>,
    /// The longest the picture's longer side may be, in pixels. The region
    /// is scaled down to fit and never scaled up past this.
    pub max_size: u32,
}

/// A rendered picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The picture as a PNG file.
    pub png: Vec<u8>,
    /// Width of the picture in pixels.
    pub width: u32,
    /// Height of the picture in pixels.
    pub height: u32,
    /// The part of the artboard the picture shows, in the artboard's
    /// coordinates.
    pub region: Rect,
    /// When a layer was asked for: the box around that layer at that
    /// moment, in the artboard's coordinates. `None` when the layer is not
    /// drawn then.
    pub layer_box: Option<Rect>,
}

/// Renders frames of a document. The editor implements this with its
/// renderer; tests use a stand-in.
pub trait FrameSource: Send + Sync {
    /// Renders the picture described by `request`.
    fn render(&self, request: &FrameRequest<'_>) -> Result<Frame, HostError>;
}

/// A kind of file an artboard can be exported to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExportFormat {
    /// One frame as a PNG image.
    Png,
    /// Every frame as a numbered PNG image in a folder.
    PngSequence,
    /// An animated GIF.
    Gif,
    /// An MP4 video (H.264).
    Mp4,
    /// A WebM video, which can be transparent.
    Webm,
    /// A ProRes 4444 video, which can be transparent.
    Prores,
    /// A Lottie animation: vector JSON for apps and websites.
    Lottie,
}

impl ExportFormat {
    /// Every format this crate can name. A host may offer fewer.
    pub const ALL: [ExportFormat; 7] = [
        ExportFormat::Png,
        ExportFormat::PngSequence,
        ExportFormat::Gif,
        ExportFormat::Mp4,
        ExportFormat::Webm,
        ExportFormat::Prores,
        ExportFormat::Lottie,
    ];

    /// The name tools use for the format.
    pub fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::PngSequence => "png_sequence",
            ExportFormat::Gif => "gif",
            ExportFormat::Mp4 => "mp4",
            ExportFormat::Webm => "webm",
            ExportFormat::Prores => "prores",
            ExportFormat::Lottie => "lottie",
        }
    }

    /// Looks a format up by the name tools use.
    pub fn parse(name: &str) -> Option<ExportFormat> {
        Self::ALL.into_iter().find(|format| format.as_str() == name)
    }

    /// The extension of the exported file, without the dot. `None` for a
    /// PNG sequence, which is a folder.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            ExportFormat::Png => Some("png"),
            ExportFormat::PngSequence => None,
            ExportFormat::Gif => Some("gif"),
            ExportFormat::Mp4 => Some("mp4"),
            ExportFormat::Webm => Some("webm"),
            ExportFormat::Prores => Some("mov"),
            ExportFormat::Lottie => Some("json"),
        }
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Settings of an export. Each format uses the ones that apply to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportOptions {
    /// Size of the output as a multiple of the artboard's size.
    pub scale: f64,
    /// Frames per second, for everything that moves.
    pub fps: f64,
    /// The moment to capture for [`ExportFormat::Png`], in milliseconds.
    /// `None` captures the design at rest.
    pub time_ms: Option<f64>,
    /// Whether to leave the artboard's background out, for formats that can
    /// be transparent.
    pub transparent: bool,
}

impl Default for ExportOptions {
    /// Actual size, 30 frames per second, the design at rest, with its
    /// background.
    fn default() -> Self {
        Self {
            scale: 1.0,
            fps: 30.0,
            time_ms: None,
            transparent: false,
        }
    }
}

/// What to export.
#[derive(Debug, Clone, Copy)]
pub struct ExportRequest<'a> {
    /// The document to export from.
    pub document: &'a Document,
    /// The project file, for finding images stored next to it.
    pub project_file: Option<&'a Path>,
    /// The artboard to export.
    pub artboard: &'a NodeId,
    /// Where to write: a file, or for a PNG sequence a folder. The tool has
    /// already checked that this is a place it may write to.
    pub path: &'a Path,
    /// The kind of file to write.
    pub format: ExportFormat,
    /// The settings.
    pub options: ExportOptions,
}

/// How far an export has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    /// Frames written so far.
    pub frames_done: u32,
    /// Frames there will be in all.
    pub frames_total: u32,
}

/// A finished export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The file or folder that was written.
    pub path: PathBuf,
    /// Size of what was written, in bytes.
    pub bytes: u64,
    /// Number of frames in it.
    pub frames: u32,
}

/// Writes artboards to files. The editor implements this with its exporter.
pub trait Exporter: Send + Sync {
    /// The formats this host can write right now. A format that needs a
    /// program that is not installed is left out.
    fn formats(&self) -> Vec<ExportFormat>;

    /// Exports the artboard described by `request`, calling `progress` as
    /// frames are written.
    fn export(
        &self,
        request: &ExportRequest<'_>,
        progress: &mut dyn FnMut(ExportProgress),
    ) -> Result<Exported, HostError>;
}

/// A font family the host can draw text with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamily {
    /// The family name, as it goes in a text layer's `font.family`.
    pub family: String,
    /// The weights the family comes in, for example `[400, 700]`. Empty
    /// when the host does not know.
    pub weights: Vec<u16>,
    /// Whether the family has italic faces.
    pub italic: bool,
}

/// Lists the fonts the host can draw text with: the ones it bundles and the
/// ones installed on the machine.
pub trait FontSource: Send + Sync {
    /// Every family, in the order the host wants them shown.
    fn families(&self) -> Vec<FontFamily>;
}

/// A text measurer that can be shared between threads.
pub type SharedMeasurer = Arc<dyn TextMeasurer + Send + Sync>;

/// The abilities the host hands to the tools. Everything is optional.
///
/// ```
/// use std::sync::Arc;
/// use moshionly_agent::{Frame, FrameRequest, FrameSource, Host, HostError};
///
/// struct Renderer;
///
/// impl FrameSource for Renderer {
///     fn render(&self, _request: &FrameRequest<'_>) -> Result<Frame, HostError> {
///         Err(HostError::Failed("this example draws nothing".into()))
///     }
/// }
///
/// let host = Host::new().with_frames(Arc::new(Renderer));
/// assert!(host.frames.is_some());
/// assert!(host.exporter.is_none());
/// ```
#[derive(Clone, Default)]
pub struct Host {
    /// Renders screenshots.
    pub frames: Option<Arc<dyn FrameSource>>,
    /// Writes exports.
    pub exporter: Option<Arc<dyn Exporter>>,
    /// Lists the fonts that can be used.
    pub fonts: Option<Arc<dyn FontSource>>,
    /// Measures text with real fonts. Without one, sizes of text boxes that
    /// follow their text are estimated.
    pub measurer: Option<SharedMeasurer>,
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Host")
            .field("frames", &self.frames.is_some())
            .field("exporter", &self.exporter.is_some())
            .field("fonts", &self.fonts.is_some())
            .field("measurer", &self.measurer.is_some())
            .finish()
    }
}

impl Host {
    /// A host with no abilities: the tools that read and edit documents
    /// work, screenshots and exports do not.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same host, able to render screenshots.
    pub fn with_frames(mut self, frames: Arc<dyn FrameSource>) -> Self {
        self.frames = Some(frames);
        self
    }

    /// The same host, able to export.
    pub fn with_exporter(mut self, exporter: Arc<dyn Exporter>) -> Self {
        self.exporter = Some(exporter);
        self
    }

    /// The same host, able to list fonts.
    pub fn with_fonts(mut self, fonts: Arc<dyn FontSource>) -> Self {
        self.fonts = Some(fonts);
        self
    }

    /// The same host, measuring text with real fonts.
    pub fn with_measurer(mut self, measurer: SharedMeasurer) -> Self {
        self.measurer = Some(measurer);
        self
    }

    /// Runs `measure` with the host's text measurer, or with an estimate
    /// that needs no fonts when the host has none. Returns the result and
    /// whether real fonts were used.
    pub fn with_text_measurer<R>(&self, measure: impl FnOnce(&dyn TextMeasurer) -> R) -> (R, bool) {
        match &self.measurer {
            Some(measurer) => (measure(measurer.as_ref()), true),
            None => (measure(&MonospaceMeasurer::default()), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_found_by_name() {
        for format in ExportFormat::ALL {
            assert_eq!(ExportFormat::parse(format.as_str()), Some(format));
        }
        assert_eq!(ExportFormat::parse("avi"), None);
        assert_eq!(ExportFormat::Prores.extension(), Some("mov"));
        assert_eq!(ExportFormat::PngSequence.extension(), None);
        assert_eq!(ExportFormat::Mp4.to_string(), "mp4");
    }

    #[test]
    fn a_host_without_a_measurer_estimates() {
        let (_, real) = Host::new().with_text_measurer(|_| ());
        assert!(!real);
        let host = Host::new().with_measurer(Arc::new(MonospaceMeasurer::default()));
        let (_, real) = host.with_text_measurer(|_| ());
        assert!(real);
        assert_eq!(
            format!("{host:?}"),
            "Host { frames: false, exporter: false, fonts: false, measurer: true }"
        );
    }
}
