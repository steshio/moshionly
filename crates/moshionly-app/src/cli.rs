//! The `moshionly` command: open the window, serve MCP on standard input
//! and output for agents without a window, or export without a window.
//!
//! ```text
//! moshionly [file.moshi]
//! moshionly mcp [--save] [file.moshi]
//! moshionly export <file.moshi> --out <path> [--format ...] [--artboard ...] [--fps ...] [--scale ...]
//! ```
//!
//! [`parse`] reads the arguments into a [`Command`] and is tested on its
//! own; [`run_mcp`] and [`run_export`] carry the headless commands out and
//! return the process's exit code.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use moshionly_agent::mcp::{McpServer, stdio};
use moshionly_agent::{Session, SessionHandle, Tools};
use moshionly_core::{Document, NodeId};
use moshionly_export::{CancelToken, ExportJob, Format, FrameRate, Phase, Settings};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

use crate::export::file_size;

/// The command worked.
pub const EXIT_OK: i32 = 0;
/// The command was understood but failed, for example a file that does
/// not open.
pub const EXIT_FAILED: i32 = 1;
/// The arguments were wrong; the usage was printed.
pub const EXIT_USAGE: i32 = 2;

/// What `moshionly` is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Open the editor window, with this project if one is named.
    Window(Option<PathBuf>),
    /// Serve MCP on standard input and output.
    Mcp(McpArgs),
    /// Export an artboard without a window.
    Export(ExportArgs),
    /// Print this text and stop.
    Help(&'static str),
    /// Print the version and stop.
    Version,
}

/// The arguments of `moshionly mcp`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpArgs {
    /// The project to serve; a new, empty one when left out.
    pub file: Option<PathBuf>,
    /// Write every change back to the file.
    pub save: bool,
}

/// The arguments of `moshionly export`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportArgs {
    /// The project to export from.
    pub file: PathBuf,
    /// Where to write.
    pub out: PathBuf,
    /// The format; taken from the extension of `out` when left out.
    pub format: Option<Format>,
    /// The artboard's name or id; the first artboard when left out.
    pub artboard: Option<String>,
    /// Frames per second.
    pub fps: Option<f64>,
    /// The size, as a multiple of the artboard's size.
    pub scale: Option<f64>,
}

/// Arguments that do not make a command, with what is wrong.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct UsageError {
    /// What is wrong.
    pub message: String,
    /// The usage text of the command concerned.
    pub usage: &'static str,
}

/// The usage of `moshionly`.
pub const USAGE: &str = "\
Moshionly: motion design for the desktop.

Usage:
  moshionly [file.moshi]        Open the editor, with a project if one is named
  moshionly mcp [file.moshi]    Serve MCP on standard input and output, for agents
  moshionly export <file.moshi> --out <path> [options]
                                Export an artboard without a window

Run `moshionly mcp --help` or `moshionly export --help` for their options.

Options:
  -h, --help       Show this text
  -V, --version    Show the version
";

/// The usage of `moshionly mcp`.
pub const MCP_USAGE: &str = "\
Serve a Moshionly project to an AI agent over MCP on standard input and
output, without a window. The agent starts this command itself; for
example, for Claude Code:

  claude mcp add moshionly -- moshionly mcp --save /path/to/project.moshi

While the editor is open, agents can connect to it instead at
http://127.0.0.1:47821/mcp and work on the open project live.

Usage:
  moshionly mcp [--save] [file.moshi]

Arguments:
  file.moshi    The project to work on. Without one, a new, empty project
                that is not saved anywhere.

Options:
  --save        Write every change back to the file
  -h, --help    Show this text
";

/// The usage of `moshionly export`.
pub const EXPORT_USAGE: &str = "\
Export an artboard of a Moshionly project without a window.

Usage:
  moshionly export <file.moshi> --out <path> [options]

Options:
  -o, --out <path>         Where to write. A PNG sequence is a new folder.
  -f, --format <name>      png, png_sequence, gif, mp4, webm, prores or lottie.
                           Default: from the extension of --out.
  -a, --artboard <name>    The artboard's name or id. Default: the first.
      --fps <number>       Frames per second. Default: 30.
      --scale <number>     Size as a multiple of the artboard's. Default: 1.
  -h, --help               Show this text

WebM and ProRes need ffmpeg to be installed.
";

/// Reads `moshionly`'s arguments, without the program's own name.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, UsageError> {
    let args: Vec<String> = args.into_iter().collect();
    match args.first().map(String::as_str) {
        Some("mcp") => parse_mcp(&args[1..]),
        Some("export") => parse_export(&args[1..]),
        Some("-h" | "--help" | "help") => Ok(Command::Help(USAGE)),
        Some("-V" | "--version") => Ok(Command::Version),
        // Anything else opens the window. Options are not files: macOS may
        // pass `-psn_…` to an app started from the Finder.
        _ => Ok(Command::Window(
            args.into_iter()
                .find(|arg| !arg.starts_with('-'))
                .map(PathBuf::from),
        )),
    }
}

fn usage_error(usage: &'static str, message: impl Into<String>) -> UsageError {
    UsageError {
        message: message.into(),
        usage,
    }
}

fn parse_mcp(args: &[String]) -> Result<Command, UsageError> {
    let mut parsed = McpArgs::default();
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help(MCP_USAGE)),
            "--save" => parsed.save = true,
            option if option.starts_with('-') => {
                return Err(usage_error(MCP_USAGE, format!("unknown option {option}")));
            }
            file if parsed.file.is_none() => parsed.file = Some(PathBuf::from(file)),
            extra => {
                return Err(usage_error(
                    MCP_USAGE,
                    format!("only one project can be served; {extra} is one too many"),
                ));
            }
        }
    }
    if parsed.save && parsed.file.is_none() {
        return Err(usage_error(MCP_USAGE, "--save needs a file to save to"));
    }
    Ok(Command::Mcp(parsed))
}

fn parse_export(args: &[String]) -> Result<Command, UsageError> {
    let fail = |message: String| usage_error(EXPORT_USAGE, message);
    let mut file = None;
    let mut out = None;
    let mut parsed = ExportArgs::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        // `--name=value` and `--name value` both work.
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |what: &str| {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .ok_or_else(|| fail(format!("{name} needs {what}")))
        };
        let number = |text: String, what: &str| {
            text.parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n > 0.0)
                .ok_or_else(|| fail(format!("{name} should be {what}, not {text}")))
        };
        match name {
            "-h" | "--help" => return Ok(Command::Help(EXPORT_USAGE)),
            "-o" | "--out" => out = Some(PathBuf::from(value("a path")?)),
            "-f" | "--format" => {
                let text = value("a format")?;
                parsed.format = Some(
                    Format::parse(&text)
                        .ok_or_else(|| fail(format!("{text} is not a format I can write")))?,
                );
            }
            "-a" | "--artboard" => parsed.artboard = Some(value("a name")?),
            "--fps" => parsed.fps = Some(number(value("a number")?, "a positive number")?),
            "--scale" => parsed.scale = Some(number(value("a number")?, "a positive number")?),
            option if option.starts_with('-') => {
                return Err(fail(format!("unknown option {option}")));
            }
            path if file.is_none() => file = Some(PathBuf::from(path)),
            extra => {
                return Err(fail(format!(
                    "only one project can be exported; {extra} is one too many"
                )));
            }
        }
    }
    parsed.file = file.ok_or_else(|| fail("name the project to export".to_owned()))?;
    parsed.out = out.ok_or_else(|| fail("say where to write with --out".to_owned()))?;
    if parsed.format.is_none() {
        let extension = parsed
            .out
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        parsed.format = match extension.as_deref() {
            Some(extension) => Format::parse(extension),
            // A path without an extension is a folder of PNG frames.
            None => Some(Format::PngSequence),
        };
        if parsed.format.is_none() {
            return Err(fail(format!(
                "the format cannot be told from {}; add --format",
                parsed.out.display()
            )));
        }
    }
    Ok(Command::Export(parsed))
}

/// The renderer for headless commands: bundled and installed fonts, and
/// images found next to `project`.
fn renderer(project: Option<PathBuf>) -> Renderer {
    Renderer::new(FontLibrary::with_system_fonts(), AssetLoader::new(project))
}

/// Serves MCP on standard input and output until the input ends. Returns
/// the exit code. Only protocol messages go to standard output;
/// everything else goes to standard error.
pub fn run_mcp(args: &McpArgs) -> i32 {
    let session = match &args.file {
        Some(file) => match SessionHandle::open(file) {
            Ok(session) => session,
            Err(error) => {
                eprintln!("moshionly: {} could not be opened: {error}", file.display());
                return EXIT_FAILED;
            }
        },
        None => SessionHandle::new(Session::new(crate::shell::new_document())),
    };
    let _saving = args.save.then(|| {
        let saver = session.clone();
        session.subscribe(move |_| {
            if let Err(error) = saver.write(Session::save) {
                eprintln!("moshionly: the change was not saved: {error}");
            }
        })
    });
    let tools = Tools::new(
        session.clone(),
        crate::host::app_host(&renderer(session.path())),
    );
    match stdio::serve(&McpServer::new(tools)) {
        Ok(()) => EXIT_OK,
        Err(error) => {
            eprintln!("moshionly: {error}");
            EXIT_FAILED
        }
    }
}

/// Exports as `args` says, printing progress to standard error and the
/// result to standard output. Returns the exit code.
pub fn run_export(args: &ExportArgs) -> i32 {
    match export(args) {
        Ok(message) => {
            println!("{message}");
            EXIT_OK
        }
        Err(message) => {
            eprintln!("moshionly: {message}");
            EXIT_FAILED
        }
    }
}

/// The export job `args` describe, for the document `document`.
pub fn export_job(args: &ExportArgs, document: Arc<Document>) -> Result<ExportJob, String> {
    let artboard = find_artboard(&document, args.artboard.as_deref())?;
    let format = args.format.unwrap_or(Format::Mp4);
    let mut settings = Settings::default();
    if let Some(fps) = args.fps {
        settings =
            settings.with_frame_rate(FrameRate::from_fps(fps).map_err(|error| error.to_string())?);
    }
    if let Some(scale) = args.scale {
        settings = settings.with_scale(scale);
    }
    Ok(ExportJob::new(document, artboard, format, args.out.clone())
        .with_settings(settings)
        .with_project_file(std::path::absolute(&args.file).unwrap_or(args.file.clone())))
}

/// The artboard named `wanted` (by name or id), or the first one.
fn find_artboard(document: &Document, wanted: Option<&str>) -> Result<NodeId, String> {
    let artboards = document.artboards();
    let Some(wanted) = wanted else {
        return artboards
            .first()
            .cloned()
            .ok_or_else(|| "the project has no artboards to export".to_owned());
    };
    artboards
        .iter()
        .find(|id| id.as_str() == wanted)
        .or_else(|| {
            artboards
                .iter()
                .find(|id| document.get(id).is_some_and(|node| node.name() == wanted))
        })
        .cloned()
        .ok_or_else(|| {
            let names: Vec<String> = artboards
                .iter()
                .filter_map(|id| document.get(id).map(|node| format!("\"{}\"", node.name())))
                .collect();
            format!(
                "there is no artboard \"{wanted}\"; the project has {}",
                names.join(", ")
            )
        })
}

fn export(args: &ExportArgs) -> Result<String, String> {
    let document = moshionly_core::io::load(&args.file)
        .map_err(|error| format!("{} could not be opened: {error}", args.file.display()))?;
    let project = std::path::absolute(&args.file).unwrap_or(args.file.clone());
    let job = export_job(args, Arc::new(document))?.with_renderer(renderer(Some(project)));
    let plan = job.plan().map_err(|error| error.to_string())?;
    let mut last = None;
    let exported = job
        .run(
            |progress| {
                let percent = (progress.fraction() * 100.0).floor() as u32;
                let step = (progress.phase, percent / 10);
                if last != Some(step) {
                    last = Some(step);
                    let what = match progress.phase {
                        Phase::Preparing => "preparing".to_owned(),
                        Phase::Rendering => format!("{percent}%"),
                        Phase::Encoding | Phase::Finishing => "finishing".to_owned(),
                    };
                    eprint!("\rexporting {}: {what}   ", plan.format.label());
                    let _ = std::io::stderr().flush();
                }
            },
            &CancelToken::new(),
        )
        .map_err(|error| {
            eprintln!();
            error.to_string()
        })?;
    eprintln!();
    for warning in &exported.warnings {
        eprintln!("note: {warning}");
    }
    let mut message = format!(
        "{} ({}, {} × {} px",
        exported.path.display(),
        file_size(exported.bytes),
        exported.width,
        exported.height
    );
    if let Some(duration) = exported.duration_ms {
        message.push_str(&format!(
            ", {} frames, {:.2} s",
            exported.frames,
            duration / 1000.0
        ));
    }
    message.push(')');
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &str) -> Result<Command, UsageError> {
        parse(words.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn a_file_or_nothing_opens_the_window() {
        assert_eq!(parse_words(""), Ok(Command::Window(None)));
        assert_eq!(
            parse_words("teaser.moshi"),
            Ok(Command::Window(Some(PathBuf::from("teaser.moshi"))))
        );
        assert_eq!(parse_words("-psn_0_1234"), Ok(Command::Window(None)));
        assert_eq!(parse_words("--help"), Ok(Command::Help(USAGE)));
        assert_eq!(parse_words("-V"), Ok(Command::Version));
    }

    #[test]
    fn mcp_takes_a_file_and_save() {
        assert_eq!(parse_words("mcp"), Ok(Command::Mcp(McpArgs::default())));
        assert_eq!(
            parse_words("mcp --save a.moshi"),
            Ok(Command::Mcp(McpArgs {
                file: Some(PathBuf::from("a.moshi")),
                save: true
            }))
        );
        assert_eq!(parse_words("mcp --help"), Ok(Command::Help(MCP_USAGE)));
        let error = parse_words("mcp --save").unwrap_err();
        assert_eq!(error.usage, MCP_USAGE);
        assert!(error.message.contains("--save"));
        assert!(parse_words("mcp a.moshi b.moshi").is_err());
        assert!(parse_words("mcp --port 3").is_err());
    }

    #[test]
    fn export_reads_its_options_in_either_form() {
        let Ok(Command::Export(args)) =
            parse_words("export demo.moshi --out demo.gif --artboard Intro --fps=24 --scale 0.5")
        else {
            panic!("an export");
        };
        assert_eq!(args.file, PathBuf::from("demo.moshi"));
        assert_eq!(args.out, PathBuf::from("demo.gif"));
        assert_eq!(args.format, Some(Format::Gif), "from the extension");
        assert_eq!(args.artboard.as_deref(), Some("Intro"));
        assert_eq!((args.fps, args.scale), (Some(24.0), Some(0.5)));

        let Ok(Command::Export(args)) = parse_words("export d.moshi -o clip.mov") else {
            panic!("an export");
        };
        assert_eq!(args.format, Some(Format::ProRes));
        let Ok(Command::Export(args)) = parse_words("export d.moshi -o frames") else {
            panic!("an export");
        };
        assert_eq!(args.format, Some(Format::PngSequence));
        let Ok(Command::Export(args)) = parse_words("export d.moshi -o x.json -f mp4") else {
            panic!("an export");
        };
        assert_eq!(args.format, Some(Format::Mp4), "--format wins");
    }

    #[test]
    fn export_says_what_is_missing_or_wrong() {
        let cases = [
            ("export --out a.mp4", "name the project"),
            ("export a.moshi", "--out"),
            ("export a.moshi --out a.avi", "--format"),
            ("export a.moshi --out a.mp4 --fps fast", "--fps"),
            ("export a.moshi --out a.mp4 --scale -1", "--scale"),
            ("export a.moshi --out", "--out needs"),
            ("export a.moshi --out a.mp4 --format avi", "avi"),
            ("export a.moshi --out a.mp4 --loud", "--loud"),
        ];
        for (words, expected) in cases {
            let error = parse_words(words).unwrap_err();
            assert_eq!(error.usage, EXPORT_USAGE, "{words}");
            assert!(
                error.message.contains(expected),
                "{words}: {}",
                error.message
            );
        }
        assert_eq!(parse_words("export -h"), Ok(Command::Help(EXPORT_USAGE)));
    }

    #[test]
    fn the_export_job_follows_the_arguments() {
        let document = Arc::new(
            moshionly_core::io::load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moshionly-core/examples/demo.moshi"
            ))
            .unwrap(),
        );
        let name = document
            .get(&document.artboards()[0])
            .unwrap()
            .name()
            .to_owned();
        let args = ExportArgs {
            file: PathBuf::from("demo.moshi"),
            out: PathBuf::from("demo.gif"),
            format: Some(Format::Gif),
            artboard: Some(name),
            fps: Some(25.0),
            scale: Some(0.5),
        };
        let job = export_job(&args, Arc::clone(&document)).unwrap();
        assert_eq!(job.format, Format::Gif);
        assert_eq!(job.artboard, NodeId::from("scene"));
        assert_eq!(job.settings.frame_rate, FrameRate::FPS_25);
        assert_eq!(job.settings.size, moshionly_export::Size::Scale(0.5));
        let missing = ExportArgs {
            artboard: Some("Nope".into()),
            ..args
        };
        let error = export_job(&missing, document).unwrap_err();
        assert!(error.contains("Nope"), "{error}");
    }
}
