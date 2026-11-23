//! Measures how long frames take: renders the core demo file across its
//! whole duration at 1080 pixels and prints the average per frame.
//!
//! ```text
//! cargo run --release -p moshionly-render --example benchmark [-- <file.moshi> [fps]]
//! ```
//!
//! Evaluation (core) and drawing (this crate) are timed apart. The first
//! frame is left out of the averages, since it pays for decoding images
//! and laying out text once. Build with `--release`: a debug build is many
//! times slower and says little.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use moshionly_core::model::NodeData;
use moshionly_core::{evaluate, io};
use moshionly_render::{AssetLoader, FontLibrary, Frame, RenderOptions, Renderer};

fn run(arguments: &[String]) -> Result<(), String> {
    let path = match arguments.first() {
        Some(path) => PathBuf::from(path),
        None => {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../moshionly-core/examples/demo.moshi")
        }
    };
    let fps: f64 = match arguments.get(1) {
        Some(text) => text
            .parse()
            .map_err(|_| format!("\"{text}\" is not a frame rate"))?,
        None => 60.0,
    };
    let document = io::load(&path).map_err(|error| error.to_string())?;
    let artboard = document
        .artboards()
        .first()
        .cloned()
        .ok_or("the project has no artboards")?;
    let Some(NodeData::Artboard(board)) = document.get(&artboard).map(|node| &node.data) else {
        return Err("the first root is not an artboard".into());
    };
    // 1080 pixels along the longer side.
    let scale = 1080.0 / board.layer.width.max(board.layer.height);
    let options = RenderOptions::new().with_scale(scale);

    let fonts = FontLibrary::bundled();
    let renderer = Renderer::new(fonts.clone(), AssetLoader::new(Some(path.clone())));
    let step = 1000.0 / fps;
    let count = (board.duration / step).floor() as usize + 1;
    let (mut evaluating, mut drawing) = (Duration::ZERO, Duration::ZERO);
    let mut frame: Option<Frame> = None;
    for index in 0..count {
        let time = index as f64 * step;
        let started = Instant::now();
        let scene = evaluate(&document, &artboard, time, &fonts).map_err(|e| e.to_string())?;
        let evaluated = Instant::now();
        let rendered = match frame.as_mut() {
            Some(frame) => renderer.render_to(&scene, &options, frame),
            None => renderer
                .render(&scene, &options)
                .map(|new| frame = Some(new)),
        };
        rendered.map_err(|error| error.to_string())?;
        if index > 0 {
            evaluating += evaluated - started;
            drawing += evaluated.elapsed();
        }
    }
    let frame = frame.ok_or("no frames")?;
    let measured = (count - 1).max(1) as f64;
    let per_frame = |total: Duration| total.as_secs_f64() * 1000.0 / measured;
    println!(
        "{}: {count} frames of {} by {} pixels",
        path.display(),
        frame.width(),
        frame.height()
    );
    println!("evaluate: {:.2} ms per frame", per_frame(evaluating));
    println!("render:   {:.2} ms per frame", per_frame(drawing));
    println!(
        "total:    {:.2} ms per frame",
        per_frame(evaluating + drawing)
    );
    Ok(())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
