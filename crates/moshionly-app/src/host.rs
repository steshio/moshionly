//! What the app does for the agent tools: screenshots with the renderer,
//! the list of fonts from the font library, exports with
//! `moshionly-export`, and reading media files with `ffmpeg`.
//!
//! The agent crate knows documents, not pixels; it asks its host through
//! the traits [`FrameSource`], [`FontSource`] and [`Exporter`]. This module
//! implements them, and [`app_host`] puts them together for the window and
//! for the `moshionly` command.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use moshionly_agent::{
    ExportFormat, ExportOptions, ExportProgress, ExportRequest, Exported, Exporter, FontFamily,
    FontSource, Frame, FrameRequest, FrameSource, Host, HostError,
};
use moshionly_core::{Rect, evaluate, evaluate_at_rest};
use moshionly_export::{
    Background, CancelToken, ExportError, ExportJob, Ffmpeg, Format, FrameRate, Settings,
    availability_with,
};
use moshionly_render::{FontLibrary, RenderOptions, Renderer};

/// Room left around a layer in a screenshot of it, in artboard pixels, so
/// its stroke and shadow show.
pub const LAYER_MARGIN: f64 = 16.0;

/// What a tool hears when its work was cancelled, for example because the
/// person stopped the server agents reach the app through.
const CANCELLED: &str = "Moshionly stopped this work: agents were disconnected";

/// Everything the app can do for the tools, drawn with `renderer`: the
/// screenshots, the fonts, the exports and text measured with real fonts.
pub fn app_host(renderer: &Renderer) -> Host {
    cancellable_app_host(renderer, &CancelToken::new())
}

/// Like [`app_host`], with work that ends when `cancel` is cancelled: a
/// running export stops and leaves nothing behind, and no new screenshot
/// or export starts. The MCP server's tools use it, so stopping the server
/// stops what agents asked for.
pub fn cancellable_app_host(renderer: &Renderer, cancel: &CancelToken) -> Host {
    host_with(
        renderer,
        AppExporter::new().with_cancel(cancel.clone()),
        cancel,
    )
}

/// Like [`app_host`], with work that `work` stops and waits for: the
/// window's own tools, which the chat uses, so quitting leaves no
/// half-written export behind.
pub fn stoppable_app_host(renderer: &Renderer, work: &HostWork) -> Host {
    let exporter = AppExporter::new()
        .with_cancel(work.cancel.clone())
        .counting(Arc::clone(&work.exporting));
    host_with(renderer, exporter, &work.cancel)
}

fn host_with(renderer: &Renderer, exporter: AppExporter, cancel: &CancelToken) -> Host {
    let fonts = renderer.fonts().clone();
    Host::new()
        .with_frames(Arc::new(
            RendererFrames::new(renderer.clone()).with_cancel(cancel.clone()),
        ))
        .with_fonts(Arc::new(LibraryFonts::new(fonts.clone())))
        .with_exporter(Arc::new(exporter.with_renderer(renderer.clone())))
        .with_measurer(Arc::new(fonts))
        .with_media(Arc::new(crate::media::FfmpegProbe::located()))
}

/// Exports and screenshots done for tools, which can be stopped and
/// waited for: once cancelled, a running export stops and deletes its
/// unfinished file, and no new one starts.
#[derive(Clone, Default)]
pub struct HostWork {
    cancel: CancelToken,
    /// How many exports are running.
    exporting: Arc<AtomicUsize>,
}

impl HostWork {
    /// Work that runs until it is cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the work, without waiting for it.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether an export is running.
    pub fn is_exporting(&self) -> bool {
        self.exporting.load(Ordering::Acquire) > 0
    }

    /// Waits up to `limit` for running exports to end, and says whether
    /// they did.
    pub fn wait(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while self.is_exporting() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

/// Counts one running export while it lives.
struct Counted(Arc<AtomicUsize>);

impl Counted {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(count))
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Screenshots for agents, drawn with the editor's renderer.
///
/// The renderer finds images next to the project file its asset loader was
/// given; in the window that is the open project, which is also the one the
/// tools work on.
#[derive(Clone)]
pub struct RendererFrames {
    renderer: Renderer,
    cancel: CancelToken,
}

impl RendererFrames {
    /// Screenshots drawn with `renderer`.
    pub fn new(renderer: Renderer) -> Self {
        Self {
            renderer,
            cancel: CancelToken::new(),
        }
    }

    /// The same screenshots, refused once `cancel` is cancelled. A tool
    /// that takes several stops between them.
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

impl FrameSource for RendererFrames {
    fn render(&self, request: &FrameRequest<'_>) -> Result<Frame, HostError> {
        if self.cancel.is_cancelled() {
            return Err(HostError::Failed(CANCELLED.into()));
        }
        let fonts = self.renderer.fonts();
        let scene = match request.time_ms {
            Some(time) => evaluate(request.document, request.artboard, time, fonts),
            None => evaluate_at_rest(request.document, request.artboard, fonts),
        }
        .map_err(|error| HostError::Failed(error.to_string()))?;
        let artboard = Rect::new(0.0, 0.0, scene.width, scene.height);

        // The layer's box, and the part of the artboard it draws on.
        let bounds = request
            .layer
            .and_then(|layer| self.renderer.bounds_of(&scene, layer));
        let layer_box = bounds.and_then(|bounds| Rect::bounding(bounds.corners));
        let drawn = bounds.and_then(|bounds| bounds.visual.or(layer_box));

        let mut region = request.crop.unwrap_or(artboard);
        if let Some(drawn) = drawn {
            let around = intersection(&padded(&drawn, LAYER_MARGIN), &artboard)
                .ok_or_else(|| HostError::Failed("the layer is outside its artboard".into()))?;
            region = intersection(&region, &around).ok_or_else(|| {
                HostError::Failed("the crop does not overlap the layer; crop elsewhere".into())
            })?;
        }

        let scale = screenshot_scale(&region, request.max_size)
            .ok_or_else(|| HostError::Failed("there is nothing to show in that region".into()))?;
        let options = RenderOptions::new().with_scale(scale).with_region(region);
        let frame = self
            .renderer
            .render(&scene, &options)
            .map_err(|error| HostError::Failed(error.to_string()))?;
        let png = frame
            .to_png()
            .map_err(|error| HostError::Failed(error.to_string()))?;
        Ok(Frame {
            png,
            width: frame.width(),
            height: frame.height(),
            region,
            layer_box,
        })
    }
}

/// The scale that makes the longer side of `region` `max_size` pixels long.
/// `None` for an empty region.
pub fn screenshot_scale(region: &Rect, max_size: u32) -> Option<f64> {
    let longest = region.width.max(region.height);
    (longest.is_finite() && longest > 0.0 && max_size > 0).then(|| f64::from(max_size) / longest)
}

/// `rect` grown by `margin` on every side.
fn padded(rect: &Rect, margin: f64) -> Rect {
    Rect::new(
        rect.x - margin,
        rect.y - margin,
        rect.width + 2.0 * margin,
        rect.height + 2.0 * margin,
    )
}

/// Where two rectangles overlap, or `None` when they do not.
fn intersection(a: &Rect, b: &Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

/// The fonts agents may use: the font library's families.
#[derive(Clone)]
pub struct LibraryFonts {
    fonts: FontLibrary,
}

impl LibraryFonts {
    /// The families of `fonts`.
    pub fn new(fonts: FontLibrary) -> Self {
        Self { fonts }
    }
}

impl FontSource for LibraryFonts {
    fn families(&self) -> Vec<FontFamily> {
        self.fonts
            .families()
            .into_iter()
            .map(|family| FontFamily {
                family: family.name,
                weights: family.weights,
                italic: family.italic,
            })
            .collect()
    }
}

/// Exports for agents, written by `moshionly-export`.
#[derive(Clone, Default)]
pub struct AppExporter {
    renderer: Option<Renderer>,
    ffmpeg: Option<Ffmpeg>,
    cancel: CancelToken,
    /// Counts the exports running, when someone waits for them.
    exporting: Option<Arc<AtomicUsize>>,
}

impl AppExporter {
    /// An exporter that draws with the bundled and installed fonts and
    /// finds `ffmpeg` where it is usually installed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same exporter, drawing with `renderer`: the editor passes its
    /// own, with the fonts it registered and the images it decoded.
    pub fn with_renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// The same exporter, writing WebM and ProRes with `ffmpeg`.
    pub fn with_ffmpeg(mut self, ffmpeg: Ffmpeg) -> Self {
        self.ffmpeg = Some(ffmpeg);
        self
    }

    /// The same exporter, whose exports stop, leaving nothing behind, once
    /// `cancel` is cancelled.
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// The same exporter, counting its running exports in `exporting`.
    fn counting(mut self, exporting: Arc<AtomicUsize>) -> Self {
        self.exporting = Some(exporting);
        self
    }
}

impl Exporter for AppExporter {
    fn formats(&self) -> Vec<ExportFormat> {
        let located = self.ffmpeg.clone().or_else(Ffmpeg::locate);
        availability_with(located.as_ref())
            .into_iter()
            .filter(|availability| availability.is_available())
            .map(|availability| agent_format(availability.format))
            .collect()
    }

    fn export(
        &self,
        request: &ExportRequest<'_>,
        progress: &mut dyn FnMut(ExportProgress),
    ) -> Result<Exported, HostError> {
        // Counted until the unfinished file is gone, as the job deletes it
        // before it returns.
        let _running = self.exporting.as_ref().map(Counted::new);
        let format = export_format(request.format).ok_or_else(|| {
            HostError::Unsupported(format!("{} cannot be exported here", request.format))
        })?;
        let settings = settings_for(&request.options)?;
        let mut job = ExportJob::new(
            Arc::new(request.document.clone()),
            request.artboard.clone(),
            format,
            request.path,
        )
        .with_settings(settings);
        if let Some(file) = request.project_file {
            job = job.with_project_file(file);
        }
        if let Some(renderer) = &self.renderer {
            job = job.with_renderer(renderer.clone());
        }
        if let Some(ffmpeg) = &self.ffmpeg {
            job = job.with_ffmpeg(ffmpeg.clone());
        }
        let exported = job
            .run(
                |done| {
                    progress(ExportProgress {
                        frames_done: saturate(done.frames_done),
                        frames_total: saturate(done.frames_total),
                    });
                },
                &self.cancel,
            )
            .map_err(host_error)?;
        Ok(Exported {
            path: exported.path,
            bytes: exported.bytes,
            frames: saturate(exported.frames),
            warnings: exported.warnings,
        })
    }
}

/// A count that fits in a `u32`.
fn saturate(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The export settings for what a tool asked for.
pub fn settings_for(options: &ExportOptions) -> Result<Settings, HostError> {
    let rate = FrameRate::from_fps(options.fps)
        .map_err(|error| HostError::Unsupported(error.to_string()))?;
    let mut settings = Settings::default()
        .with_scale(options.scale)
        .with_frame_rate(rate)
        .with_still(options.time_ms)
        .with_sound(options.sound);
    if options.transparent {
        settings = settings.with_background(Background::Transparent);
    }
    Ok(settings)
}

/// Why an export failed, for the model.
fn host_error(error: ExportError) -> HostError {
    match error {
        ExportError::FfmpegMissing { .. } | ExportError::Settings(_) => {
            HostError::Unsupported(error.to_string())
        }
        ExportError::Cancelled => HostError::Failed(CANCELLED.into()),
        other => HostError::Failed(other.to_string()),
    }
}

/// The exporter's format for a format the tools name.
pub fn export_format(format: ExportFormat) -> Option<Format> {
    Some(match format {
        ExportFormat::Png => Format::Png,
        ExportFormat::PngSequence => Format::PngSequence,
        ExportFormat::Gif => Format::Gif,
        ExportFormat::Mp4 => Format::Mp4,
        ExportFormat::Webm => Format::WebM,
        ExportFormat::Prores => Format::ProRes,
        ExportFormat::Lottie => Format::Lottie,
        _ => return None,
    })
}

/// The tools' name for one of the exporter's formats.
pub fn agent_format(format: Format) -> ExportFormat {
    match format {
        Format::Png => ExportFormat::Png,
        Format::PngSequence => ExportFormat::PngSequence,
        Format::Gif => ExportFormat::Gif,
        Format::Mp4 => ExportFormat::Mp4,
        Format::WebM => ExportFormat::Webm,
        Format::ProRes => ExportFormat::Prores,
        Format::Lottie => ExportFormat::Lottie,
    }
}

#[cfg(test)]
mod tests {
    use moshionly_agent::{SessionHandle, Tools};
    use moshionly_core::{Document, NodeId, io};
    use moshionly_render::AssetLoader;
    use serde_json::{Value, json};

    use super::*;

    fn renderer() -> Renderer {
        Renderer::new(FontLibrary::bundled(), AssetLoader::default())
    }

    fn demo() -> Document {
        io::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moshionly-core/examples/demo.moshi"
        ))
        .expect("the demo opens")
    }

    fn request<'a>(document: &'a Document, artboard: &'a NodeId) -> FrameRequest<'a> {
        FrameRequest {
            document,
            project_file: None,
            artboard,
            layer: None,
            time_ms: None,
            crop: None,
            max_size: 256,
        }
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        let image = image::load_from_memory(png).expect("a real PNG");
        (image.width(), image.height())
    }

    #[test]
    fn a_screenshot_is_a_png_of_the_whole_artboard_scaled_to_fit() {
        let document = demo();
        let artboard = NodeId::from("scene");
        let frame = RendererFrames::new(renderer())
            .render(&request(&document, &artboard))
            .unwrap();
        let board = document.world_bounds(&artboard).unwrap();
        assert_eq!(frame.width.max(frame.height), 256);
        assert_eq!(png_size(&frame.png), (frame.width, frame.height));
        assert_eq!(frame.region, Rect::new(0.0, 0.0, board.width, board.height));
        assert_eq!(frame.layer_box, None);
    }

    #[test]
    fn a_screenshot_of_a_layer_frames_it_with_room_around() {
        let document = demo();
        let artboard = NodeId::from("scene");
        let layer = NodeId::from("card");
        let frame = RendererFrames::new(renderer())
            .render(&FrameRequest {
                layer: Some(&layer),
                ..request(&document, &artboard)
            })
            .unwrap();
        let layer_box = frame.layer_box.expect("the card is drawn");
        assert!(frame.region.x <= layer_box.x && frame.region.y <= layer_box.y);
        assert!(frame.region.right() >= layer_box.right());
        assert!(frame.region.bottom() >= layer_box.bottom());
        let board = document.world_bounds(&artboard).unwrap();
        assert!(frame.region.width < board.width, "only the card's part");
        assert_eq!(png_size(&frame.png), (frame.width, frame.height));
    }

    #[test]
    fn a_crop_that_misses_the_layer_is_refused() {
        let document = demo();
        let artboard = NodeId::from("scene");
        let layer = NodeId::from("card");
        let result = RendererFrames::new(renderer()).render(&FrameRequest {
            layer: Some(&layer),
            crop: Some(Rect::new(0.0, 0.0, 4.0, 4.0)),
            ..request(&document, &artboard)
        });
        assert!(matches!(result, Err(HostError::Failed(_))));
        assert_eq!(screenshot_scale(&Rect::new(0.0, 0.0, 0.0, 0.0), 64), None);
        assert_eq!(
            screenshot_scale(&Rect::new(0.0, 0.0, 512.0, 128.0), 256),
            Some(0.5)
        );
    }

    #[test]
    fn fonts_are_the_libraries_families() {
        let families = LibraryFonts::new(FontLibrary::bundled()).families();
        let inter = families
            .iter()
            .find(|family| family.family == "Inter")
            .expect("Inter is bundled");
        assert!(!inter.weights.is_empty());
    }

    #[test]
    fn formats_have_the_same_names_on_both_sides() {
        for format in Format::ALL {
            assert_eq!(export_format(agent_format(format)), Some(format));
            assert_eq!(agent_format(format).as_str(), format.as_str());
        }
        let formats = AppExporter::new().formats();
        for built_in in [ExportFormat::Png, ExportFormat::Gif, ExportFormat::Mp4] {
            assert!(formats.contains(&built_in), "{built_in}");
        }
    }

    #[test]
    fn tool_options_become_export_settings() {
        let settings = settings_for(&ExportOptions {
            scale: 2.0,
            fps: 24.0,
            time_ms: Some(500.0),
            transparent: true,
            sound: true,
        })
        .unwrap();
        assert_eq!(settings.size, moshionly_export::Size::Scale(2.0));
        assert_eq!(settings.frame_rate, FrameRate::FPS_24);
        assert_eq!(settings.still_ms, Some(500.0));
        assert_eq!(settings.background, Background::Transparent);
        assert!(settings.sound);
        let quiet = settings_for(&ExportOptions {
            sound: false,
            ..ExportOptions::default()
        })
        .unwrap();
        assert!(!quiet.sound, "an agent can leave the sound out");
        let bad = settings_for(&ExportOptions {
            fps: -1.0,
            ..ExportOptions::default()
        });
        assert!(matches!(bad, Err(HostError::Unsupported(_))));
    }

    #[test]
    fn agents_screenshot_list_fonts_and_export_through_the_app() {
        let dir = std::env::temp_dir().join(format!(
            "moshionly-app-host-{}-{}",
            std::process::id(),
            NodeId::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let session = SessionHandle::from_document(demo());
        session.write(|s| s.set_path(Some(dir.join("demo.moshi"))));
        let tools = Tools::new(session, app_host(&renderer()));

        let shot = tools.call(
            "screenshot",
            json!({ "artboard": "scene", "max_size": 128 }),
        );
        assert!(!shot.is_error, "{}", shot.joined_text());

        let fonts = tools.call("list_fonts", json!({}));
        assert!(fonts.joined_text().contains("Inter"));

        let output = tools.call(
            "export",
            json!({ "artboard": "scene", "format": "png", "scale": 0.25 }),
        );
        assert!(!output.is_error, "{}", output.joined_text());
        let answer: Value = serde_json::from_str(&output.joined_text()).unwrap();
        let path = answer["path"].as_str().expect("the export says where");
        assert!(std::fs::read(path).unwrap().starts_with(b"\x89PNG"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn once_cancelled_the_host_starts_no_export_or_screenshot() {
        let dir = std::env::temp_dir().join(format!(
            "moshionly-app-host-cancel-{}-{}",
            std::process::id(),
            NodeId::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let session = SessionHandle::from_document(demo());
        session.write(|s| s.set_path(Some(dir.join("demo.moshi"))));
        let cancel = CancelToken::new();
        let tools = Tools::new(session, cancellable_app_host(&renderer(), &cancel));
        cancel.cancel();

        let shot = tools.call("screenshot", json!({ "artboard": "scene" }));
        assert!(shot.is_error);
        assert!(
            shot.joined_text().contains("stopped"),
            "{}",
            shot.joined_text()
        );

        let output = tools.call(
            "export",
            json!({ "artboard": "scene", "format": "mp4", "scale": 2 }),
        );
        assert!(output.is_error);
        assert!(
            output.joined_text().contains("stopped"),
            "{}",
            output.joined_text()
        );
        let left: Vec<_> = std::fs::read_dir(dir.join("exports"))
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "nothing is left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
