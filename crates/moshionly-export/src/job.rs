//! An export job: what to export, where, and the call that does it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moshionly_core::{Document, NodeId};
use moshionly_render::{AssetLoader, FontLibrary, RenderWarning, Renderer};

use crate::error::ExportError;
use crate::format::Format;
use crate::frames::FrameSource;
use crate::gif;
use crate::mp4;
use crate::output::Output;
use crate::png;
use crate::progress::{CancelToken, Phase, Progress};
use crate::settings::{Plan, Settings};

/// One export: an artboard of a document, written to a file in a format.
///
/// The document is shared and never changed, so the editor can keep
/// working on its own copy while an export runs on another thread.
///
/// ```no_run
/// use std::sync::Arc;
/// use moshionly_core::io;
/// use moshionly_export::{CancelToken, ExportJob, Format, Settings};
///
/// let document = Arc::new(io::load("launch.moshi")?);
/// let artboard = document.artboards()[0].clone();
/// let job = ExportJob::new(document, artboard, Format::Mp4, "launch.mp4")
///     .with_project_file("launch.moshi")
///     .with_settings(Settings::default().with_scale(2.0));
/// let exported = job.run(
///     |progress| println!("{:.0}%", progress.fraction() * 100.0),
///     &CancelToken::new(),
/// )?;
/// println!("{} bytes", exported.bytes);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct ExportJob {
    /// The document to export from.
    pub document: Arc<Document>,
    /// The artboard to export.
    pub artboard: NodeId,
    /// The project file, for finding images stored next to it.
    pub project_file: Option<PathBuf>,
    /// The kind of file to write.
    pub format: Format,
    /// What the person chose.
    pub settings: Settings,
    /// Where to write: a file, or for a PNG sequence a folder that does not
    /// exist yet or is empty.
    pub destination: PathBuf,
    /// What draws the frames. `None` draws with the bundled and installed
    /// fonts, and finds images next to the project file.
    pub renderer: Option<Renderer>,
}

/// A finished export.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    /// The file or folder that was written.
    pub path: PathBuf,
    /// The kind of file.
    pub format: Format,
    /// Size of what was written, in bytes.
    pub bytes: u64,
    /// Number of frames in it.
    pub frames: u64,
    /// Width of the frames in pixels.
    pub width: u32,
    /// Height of the frames in pixels.
    pub height: u32,
    /// How long it plays for, in milliseconds. `None` for a still.
    pub duration_ms: Option<f64>,
    /// What was drawn differently from the document, such as a missing
    /// image or font, each said once.
    pub warnings: Vec<String>,
}

impl ExportJob {
    /// Exports `artboard` of `document` as `format` to `destination`, with
    /// default settings.
    pub fn new(
        document: Arc<Document>,
        artboard: NodeId,
        format: Format,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self {
            document,
            artboard,
            project_file: None,
            format,
            settings: Settings::default(),
            destination: destination.into(),
            renderer: None,
        }
    }

    /// The same job with other settings.
    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// The same job, finding images next to `project_file`.
    pub fn with_project_file(mut self, project_file: impl Into<PathBuf>) -> Self {
        self.project_file = Some(project_file.into());
        self
    }

    /// The same job, drawn by `renderer`. Pass the editor's renderer to
    /// export with the fonts it has registered and the images it has
    /// already decoded.
    pub fn with_renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// Checks the settings and works out exactly what will be written,
    /// without writing anything. Call it to show a person what is wrong
    /// before they start.
    pub fn plan(&self) -> Result<Plan, ExportError> {
        Ok(self
            .settings
            .plan(self.format, &self.document, &self.artboard)?)
    }

    /// Runs the export, calling `progress` as frames are written, and
    /// stopping early when `cancel` is cancelled.
    ///
    /// Frames are rendered on several threads and handed to the encoder in
    /// order. The file is written beside the destination and moved into
    /// place only when it is complete; a failed or cancelled export leaves
    /// nothing behind.
    pub fn run(
        &self,
        mut progress: impl FnMut(Progress),
        cancel: &CancelToken,
    ) -> Result<Exported, ExportError> {
        let plan = self.plan()?;
        let renderer = self.renderer.clone().unwrap_or_else(|| {
            Renderer::new(
                FontLibrary::with_system_fonts(),
                AssetLoader::new(self.project_file.clone()),
            )
        });
        let output = Output::create(&self.destination, plan.format.writes_folder())?;
        let mut cx = Context {
            plan: &plan,
            source: FrameSource {
                document: &self.document,
                artboard: &self.artboard,
                renderer: &renderer,
            },
            reporter: Reporter::new(&mut progress, cancel, plan.frame_count()),
        };
        cx.reporter.report(Phase::Preparing, 0);
        write(&mut cx, output.path())?;
        cx.reporter.check()?;
        cx.reporter.report(Phase::Finishing, plan.frame_count());
        let warnings = std::mem::take(&mut cx.reporter.warnings);
        let path = output.destination().to_owned();
        let bytes = output.commit()?;
        Ok(Exported {
            path,
            format: plan.format,
            bytes,
            frames: plan.frame_count(),
            width: plan.width,
            height: plan.height,
            duration_ms: plan.duration_ms(),
            warnings,
        })
    }
}

/// Writes the export to `path`, the temporary file or folder.
///
/// Every format plugs in here. A raster format renders frames from
/// `cx.source` and encodes them; a format that converts the scene model
/// instead of pixels (Lottie, to come in `src/lottie/`) evaluates scenes
/// at the plan's moments and writes its file, reporting progress the same
/// way.
fn write(cx: &mut Context<'_>, path: &Path) -> Result<(), ExportError> {
    match cx.plan.format {
        Format::Png => png::write_still(cx, path),
        Format::PngSequence => png::write_sequence(cx, path),
        Format::Gif => gif::write(cx, path),
        Format::Mp4 => mp4::write(cx, path),
        format @ (Format::WebM | Format::ProRes) => Err(ExportError::NotWritten(format)),
    }
}

/// What a format's writer works with.
pub(crate) struct Context<'a> {
    /// What to write.
    pub plan: &'a Plan,
    /// Where frames come from.
    pub source: FrameSource<'a>,
    /// Where progress, warnings and cancellation go.
    pub reporter: Reporter<'a>,
}

/// Reports progress, gathers warnings and watches for cancellation.
pub(crate) struct Reporter<'a> {
    progress: &'a mut dyn FnMut(Progress),
    cancel: &'a CancelToken,
    total: u64,
    warnings: Vec<String>,
}

impl<'a> Reporter<'a> {
    pub fn new(progress: &'a mut dyn FnMut(Progress), cancel: &'a CancelToken, total: u64) -> Self {
        Self {
            progress,
            cancel,
            total,
            warnings: Vec::new(),
        }
    }

    /// Says what the export is doing and how many frames are done.
    pub fn report(&mut self, phase: Phase, frames_done: u64) {
        (self.progress)(Progress {
            phase,
            frames_done,
            frames_total: self.total,
        });
    }

    /// The cancellation token, for work that runs on other threads.
    pub fn cancel_token(&self) -> &CancelToken {
        self.cancel
    }

    /// Fails with [`ExportError::Cancelled`] once the export is cancelled.
    pub fn check(&self) -> Result<(), ExportError> {
        if self.cancel.is_cancelled() {
            Err(ExportError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Notes what a frame drew differently from the document. Each warning
    /// is kept once, however many frames have it.
    pub fn warn(&mut self, warnings: &[RenderWarning]) {
        for warning in warnings {
            let text = warning.to_string();
            if !self.warnings.contains(&text) {
                self.warnings.push(text);
            }
        }
    }
}
