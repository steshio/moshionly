//! Measures how long frames take: renders the core demo file across its
//! whole duration at 1080 pixels and prints the average per frame.
//!
//! ```text
//! cargo run --release -p moshionly-render --example benchmark [-- <file.moshi> [fps]]
//! cargo run --release -p moshionly-render --example benchmark -- effects
//! ```
//!
//! Evaluation (core) and drawing (this crate) are timed apart. The first
//! frame is left out of the averages, since it pays for decoding images
//! and laying out text once. Build with `--release`: a debug build is many
//! times slower and says little.
//!
//! `effects` times each layer effect instead, on one layer that covers a
//! whole 1920 by 1080 frame: the worst case, since effects cost in
//! proportion to the area they cover.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use moshionly_core::model::NodeData;
use moshionly_core::{evaluate, io};
use moshionly_render::{AssetLoader, FontLibrary, Frame, RenderOptions, Renderer};

fn run(arguments: &[String]) -> Result<(), String> {
    if arguments.first().map(String::as_str) == Some("effects") {
        return effects();
    }
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

/// The effects timed by `effects`, as the fields of the full-frame layer.
const EFFECTS: [(&str, &str); 9] = [
    ("none", ""),
    ("blur 20", r#","blur":20"#),
    ("shadow", r#","shadow":{"y":8,"blur":24}"#),
    ("glow", r#","glow":{"radius":24}"#),
    ("glow, spread 16", r#","glow":{"radius":24,"spread":16}"#),
    ("noise", r#","noise":{"amount":0.3}"#),
    (
        "directional blur 0°",
        r#","directional_blur":{"amount":24}"#,
    ),
    (
        "directional blur 30°",
        r#","directional_blur":{"amount":24,"angle":30}"#,
    ),
    (
        "all of them",
        r#","blur":4,"shadow":{"y":8,"blur":24},"glow":{"radius":24,"spread":4},"noise":{"amount":0.3},"directional_blur":{"amount":24,"angle":30}"#,
    ),
];

/// Times every effect in [`EFFECTS`] over 30 frames of moving grain.
fn effects() -> Result<(), String> {
    const FRAMES: u32 = 30;
    let fonts = FontLibrary::bundled();
    let renderer = Renderer::new(fonts.clone(), AssetLoader::default());
    let options = RenderOptions::new();
    println!("one 1920 by 1080 layer, {FRAMES} frames each:");
    for (name, fields) in EFFECTS {
        let file = format!(
            r##"{{"format":"moshionly","version":4,"name":"Effects","nodes":[
            {{"id":"board","parent":null,"index":"a0","type":"artboard","width":1920,"height":1080,"fill":"#202020"}},
            {{"id":"layer","parent":"board","index":"a0","type":"rect","width":1920,"height":1080,
              "fill":{{"type":"linear_gradient","stops":[{{"offset":0,"color":"#0D99FF"}},{{"offset":1,"color":"#FF5E3A"}}]}}{fields}}}
            ]}}"##
        );
        let document = io::parse(&file).map_err(|error| error.to_string())?;
        let artboard = document.artboards()[0].clone();
        let mut frame = Frame::new(1920, 1080).map_err(|error| error.to_string())?;
        let mut drawing = Duration::ZERO;
        for index in 0..=FRAMES {
            let time = f64::from(index) * 1000.0 / 30.0;
            let scene = evaluate(&document, &artboard, time, &fonts).map_err(|e| e.to_string())?;
            let started = Instant::now();
            renderer
                .render_to(&scene, &options, &mut frame)
                .map_err(|error| error.to_string())?;
            // The first frame compiles and caches; leave it out.
            if index > 0 {
                drawing += started.elapsed();
            }
        }
        let per_frame = drawing.as_secs_f64() * 1000.0 / f64::from(FRAMES);
        println!("  {name:<22} {per_frame:>7.2} ms per frame");
    }
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
