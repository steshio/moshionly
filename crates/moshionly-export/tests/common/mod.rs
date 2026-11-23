//! Helpers shared by the export tests: the core demo file, a renderer that
//! uses only the bundled fonts (so pixels are the same on every machine),
//! and a way to run a job while recording its progress.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use moshionly_core::{Document, NodeId, evaluate, evaluate_at_rest, io};
use moshionly_export::{
    CancelToken, ExportError, ExportJob, Exported, Format, Moment, Progress, Settings,
};
use moshionly_render::{AssetLoader, FontLibrary, Frame, RenderOptions, Renderer};

/// The demo project that comes with core: one 1080 by 1080 artboard, four
/// seconds long, with text, a gradient, a shadow and several animations.
pub fn demo_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../moshionly-core/examples/demo.moshi")
}

/// The demo document.
pub fn demo() -> Arc<Document> {
    Arc::new(io::load(demo_path()).expect("the demo file loads"))
}

/// The demo's artboard.
pub fn artboard(document: &Document) -> NodeId {
    document.artboards()[0].clone()
}

/// A renderer with the bundled fonts only.
pub fn renderer() -> Renderer {
    Renderer::new(FontLibrary::bundled(), AssetLoader::new(Some(demo_path())))
}

/// A job exporting the demo as `format` to `destination`.
pub fn job(format: Format, destination: impl Into<PathBuf>, settings: Settings) -> ExportJob {
    let document = demo();
    let artboard = artboard(&document);
    ExportJob::new(document, artboard, format, destination)
        .with_project_file(demo_path())
        .with_settings(settings)
        .with_renderer(renderer())
}

/// Runs a job and returns what it reported along the way.
pub fn run(job: &ExportJob) -> (Result<Exported, ExportError>, Vec<Progress>) {
    let mut reports = Vec::new();
    let result = job.run(|progress| reports.push(progress), &CancelToken::new());
    (result, reports)
}

/// What the renderer draws for the demo at `moment` with `options`.
pub fn reference(moment: Moment, options: &RenderOptions) -> Frame {
    let document = demo();
    let artboard = artboard(&document);
    let renderer = renderer();
    let fonts = renderer.fonts();
    let scene = match moment {
        Moment::AtRest => evaluate_at_rest(&document, &artboard, fonts),
        Moment::At(time) => evaluate(&document, &artboard, time, fonts),
    }
    .expect("the demo evaluates");
    renderer.render(&scene, options).expect("the demo renders")
}

/// The names in a folder, sorted.
pub fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the folder can be read")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Decodes a PNG file into straight-alpha RGBA.
pub fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let image = image::load_from_memory(bytes)
        .expect("a PNG decodes")
        .into_rgba8();
    (image.width(), image.height(), image.into_raw())
}

/// Peak signal-to-noise ratio between two pictures of the same size, in
/// decibels: above 40 is hard to tell apart, above 30 is good video.
pub fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "pictures of the same size");
    let squared: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum();
    let mean = squared / a.len() as f64;
    if mean == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0 * 255.0 / mean).log10()
}

/// The RGB of straight-alpha RGBA pixels, dropping alpha.
pub fn rgb_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}
