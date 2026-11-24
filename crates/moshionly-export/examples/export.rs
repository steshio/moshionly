//! Exports an artboard of a project file, printing progress and the result.
//!
//! ```text
//! cargo run --release -p moshionly-export --example export -- <file.moshi> <output> [options]
//!
//! --format <name>      png, png_sequence, gif, mp4, webm, prores or lottie; by
//!                      default from the output's extension (.png .gif .mp4
//!                      .webm .mov .json), and a PNG sequence when it has none
//! --fps <rate>         frames per second, such as 30 or 29.97 (default 30)
//! --scale <factor>     size as a multiple of the artboard (default 1)
//! --width <pixels>     size as a width instead, keeping proportions
//! --artboard <name>    the artboard to export, by name or id (default the first)
//! --quality <name>     draft, standard or high (default standard)
//! --transparent        leave the background out (not for MP4)
//! --background <#hex>  put a colour behind the artboard
//! --at <ms>            the moment a single PNG shows (default: at rest)
//! --start <ms>         where an animation starts (default 0)
//! --end <ms>           where it ends (default: the end of the artboard)
//! --once               a GIF that plays once instead of looping
//! --no-dither          a GIF without dithering
//! --images-beside      a Lottie file's images as files in a folder beside it
//! --formats            list the formats that can be written here, and stop
//! ```
//!
//! Text is drawn with the bundled fonts and the fonts installed on this
//! machine, and images are found next to the project file.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use moshionly_core::{Color, NodeId, io};
use moshionly_export::{
    Background, CancelToken, ExportJob, Format, FrameRate, Loop, LottieImages, Phase, Quality,
    Settings, Size, availability,
};

const USAGE: &str = "usage: export <file.moshi> <output> \
     [--format png|png_sequence|gif|mp4|webm|prores|lottie] \
     [--fps 30] [--scale 1 | --width 1080] [--artboard <name>] [--quality draft|standard|high] \
     [--transparent | --background #RRGGBB] [--at <ms>] [--start <ms>] [--end <ms>] [--once] \
     [--no-dither] [--images-beside] | --formats";

fn number(flag: &str, value: Option<&String>) -> Result<f64, String> {
    let value = value.ok_or(format!("{flag} needs a value\n{USAGE}"))?;
    value
        .parse()
        .map_err(|_| format!("{flag} takes a number, and \"{value}\" is not one"))
}

fn run(arguments: &[String]) -> Result<(), String> {
    if arguments.iter().any(|argument| argument == "--formats") {
        for entry in availability() {
            match entry.missing {
                None => println!("{:<14} available", entry.format.as_str()),
                Some(reason) => println!("{:<14} missing: {reason}", entry.format.as_str()),
            }
        }
        return Ok(());
    }
    let [input, output, flags @ ..] = arguments else {
        return Err(USAGE.into());
    };
    let mut settings = Settings::default();
    let mut format = None;
    let mut artboard_name = None;
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--format" => {
                let name = flags.next().ok_or(USAGE)?;
                format = Some(Format::parse(name).ok_or(format!("unknown format \"{name}\""))?);
            }
            "--fps" => {
                let fps = number(flag, flags.next())?;
                settings.frame_rate = FrameRate::from_fps(fps).map_err(|e| e.to_string())?;
            }
            "--scale" => settings.size = Size::Scale(number(flag, flags.next())?),
            "--width" => settings.size = Size::Width(number(flag, flags.next())? as u32),
            "--artboard" => artboard_name = Some(flags.next().ok_or(USAGE)?.clone()),
            "--quality" => {
                let name = flags.next().ok_or(USAGE)?;
                settings.quality =
                    Quality::parse(name).ok_or(format!("unknown quality \"{name}\""))?;
            }
            "--transparent" => settings.background = Background::Transparent,
            "--background" => {
                let text = flags.next().ok_or(USAGE)?;
                let color = Color::parse(text).map_err(|_| format!("\"{text}\" is not #RRGGBB"))?;
                settings.background = Background::Over(color);
            }
            "--at" => settings.still_ms = Some(number(flag, flags.next())?),
            "--start" => settings.start_ms = Some(number(flag, flags.next())?),
            "--end" => settings.end_ms = Some(number(flag, flags.next())?),
            "--once" => settings.gif_loop = Loop::Once,
            "--no-dither" => settings.dither = false,
            "--images-beside" => settings.lottie_images = LottieImages::Files,
            other => return Err(format!("unknown option \"{other}\"\n{USAGE}")),
        }
    }
    let format = match format {
        Some(format) => format,
        None => match Path::new(output).extension().and_then(|e| e.to_str()) {
            None => Format::PngSequence,
            Some(extension) => Format::parse(extension)
                .ok_or(format!("choose a --format: \".{extension}\" is not one"))?,
        },
    };

    let document = Arc::new(io::load(input).map_err(|error| error.to_string())?);
    let artboard = match &artboard_name {
        None => document.artboards().first().cloned(),
        Some(name) => document
            .artboards()
            .iter()
            .find(|id| {
                id.as_str() == name
                    || document
                        .get(id)
                        .is_some_and(|node| node.name().eq_ignore_ascii_case(name))
            })
            .cloned(),
    }
    .ok_or(match artboard_name {
        Some(name) => format!("the project has no artboard named \"{name}\""),
        None => "the project has no artboards".to_owned(),
    })?;
    let job = ExportJob::new(document, NodeId::from(&artboard), format, output)
        .with_project_file(input)
        .with_settings(settings);
    let plan = job.plan().map_err(|error| error.to_string())?;
    println!(
        "{format}: {} by {} pixels, {} frame{}",
        plan.width,
        plan.height,
        plan.frame_count(),
        if plan.frame_count() == 1 { "" } else { "s" }
    );

    let started = Instant::now();
    let mut last = None;
    let exported = job
        .run(
            |progress| {
                let step = (
                    progress.phase,
                    progress.frames_done * 20 / progress.frames_total.max(1),
                );
                if last != Some(step) || progress.phase != Phase::Rendering {
                    last = Some(step);
                    println!(
                        "  {:?}: {} of {} frames",
                        progress.phase, progress.frames_done, progress.frames_total
                    );
                }
            },
            &CancelToken::new(),
        )
        .map_err(|error| error.to_string())?;
    let seconds = started.elapsed().as_secs_f64();
    for warning in &exported.warnings {
        eprintln!("warning: {warning}");
    }
    println!(
        "{}: {:.1} kB, {} frames, {} by {} pixels{} in {seconds:.2} s ({:.1} frames a second)",
        exported.path.display(),
        exported.bytes as f64 / 1000.0,
        exported.frames,
        exported.width,
        exported.height,
        exported
            .duration_ms
            .map(|ms| format!(", {:.3} s long,", ms / 1000.0))
            .unwrap_or_default(),
        exported.frames as f64 / seconds
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
