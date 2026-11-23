//! Renders one frame of a project file to a PNG.
//!
//! ```text
//! cargo run -p moshionly-render --example render -- <file.moshi> <time_ms> <out.png> [scale]
//! ```
//!
//! The first artboard is rendered at `time_ms`, `scale` times its size
//! (default 1), with the bundled fonts and the fonts installed on this
//! machine. Images are found next to the project file. Anything drawn
//! differently from what the file asks for, such as a missing image, is
//! printed as a warning.

use std::process::ExitCode;

use moshionly_core::{evaluate, io};
use moshionly_render::{AssetLoader, FontLibrary, RenderOptions, Renderer};

const USAGE: &str = "usage: render <file.moshi> <time_ms> <out.png> [scale]";

fn run(arguments: &[String]) -> Result<(), String> {
    let [path, time, out, rest @ ..] = arguments else {
        return Err(USAGE.into());
    };
    let time_ms: f64 = time
        .parse()
        .map_err(|_| format!("\"{time}\" is not a time in milliseconds\n{USAGE}"))?;
    let scale: f64 = match rest.first() {
        None => 1.0,
        Some(text) => text
            .parse()
            .map_err(|_| format!("\"{text}\" is not a scale such as 1 or 0.5\n{USAGE}"))?,
    };
    let document = io::load(path).map_err(|error| error.to_string())?;
    let artboard = document
        .artboards()
        .first()
        .cloned()
        .ok_or("the project has no artboards")?;

    let fonts = FontLibrary::with_system_fonts();
    let renderer = Renderer::new(fonts.clone(), AssetLoader::new(Some(path.into())));
    let scene = evaluate(&document, &artboard, time_ms, &fonts).map_err(|e| e.to_string())?;
    let frame = renderer
        .render(&scene, &RenderOptions::new().with_scale(scale))
        .map_err(|error| error.to_string())?;
    for warning in frame.warnings() {
        eprintln!("warning: {warning}");
    }
    let png = frame.to_png().map_err(|error| error.to_string())?;
    std::fs::write(out, png).map_err(|error| format!("{out} could not be written: {error}"))?;
    println!("{out}: {} by {} pixels", frame.width(), frame.height());
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
