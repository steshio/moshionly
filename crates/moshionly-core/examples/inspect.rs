//! Prints one frame of a project file as JSON.
//!
//! ```text
//! cargo run -p moshionly-core --example inspect -- <file.moshi> [time_ms] [artboard_id]
//! ```
//!
//! The frame is the [`Scene`](moshionly_core::Scene) that the renderer
//! draws: every layer that can be seen at that time, with its transform,
//! opacity, paints and text units resolved. Without a time the frame at 0
//! is printed; without an artboard id, the first artboard.
//!
//! Text is measured with the built-in estimate (every letter 0.6 of the font
//! size wide), so text boxes are close to, but not exactly, what the app
//! shows with real fonts.

use std::process::ExitCode;

use moshionly_core::{MonospaceMeasurer, NodeId, evaluate, io};

fn run(arguments: &[String]) -> Result<String, String> {
    let [path, rest @ ..] = arguments else {
        return Err("usage: inspect <file.moshi> [time_ms] [artboard_id]".into());
    };
    let time_ms = match rest.first() {
        None => 0.0,
        Some(text) => text
            .parse::<f64>()
            .map_err(|_| format!("\"{text}\" is not a time in milliseconds"))?,
    };
    let document = io::load(path).map_err(|error| error.to_string())?;
    let artboard = match rest.get(1) {
        Some(id) => NodeId::from(id.as_str()),
        None => document
            .artboards()
            .first()
            .cloned()
            .ok_or("the project has no artboards")?,
    };
    let scene = evaluate(&document, &artboard, time_ms, &MonospaceMeasurer::default())
        .map_err(|error| error.to_string())?;
    serde_json::to_string_pretty(&scene).map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(scene) => {
            println!("{scene}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
