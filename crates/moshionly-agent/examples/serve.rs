//! Serves a Moshionly file over MCP, for trying agents without the app.
//!
//! ```text
//! cargo run -p moshionly-agent --example serve -- [file.moshi] [--port N] [--save]
//! ```
//!
//! Without a file, a small demo project is served. The URL to give an
//! agent is printed, for example for Claude Code:
//!
//! ```text
//! claude mcp add --transport http moshionly http://127.0.0.1:47821/mcp
//! ```
//!
//! Every change is printed as it happens. With `--save` each change is also
//! written back to the file. Screenshots and exports are not available,
//! because the renderer belongs to the app; the other tools all work.
//! Designs sent from the Figma plugin are listed but not applied.

use std::path::PathBuf;
use std::process::ExitCode;

use moshionly_agent::import::ImportSink;
use moshionly_agent::mcp::McpServer;
use moshionly_agent::mcp::http::{DEFAULT_PORT, HttpConfig, HttpServer};
use moshionly_agent::{Host, Session, SessionHandle, Tools};
use moshionly_core::io;

/// A card that fades in, on one artboard.
const DEMO: &str = r##"{
  "format": "moshionly", "version": 1, "name": "Demo",
  "nodes": [
    {"id":"scene","parent":null,"index":"a0","type":"artboard","name":"Scene","width":1080,"height":1080,"fill":"#F2F3F5"},
    {"id":"card","parent":"scene","index":"a0","type":"rect","name":"Card","x":240,"y":340,"width":600,"height":400,"fill":"#FFFFFF","corner_radius":32},
    {"id":"title","parent":"scene","index":"a1","type":"text","name":"Title","x":300,"y":480,"content":"Hello","font_size":72,"fill":"#111111"},
    {"id":"cardfade","parent":"scene","index":"a0","type":"fade_in","target":"card","start":0,"end":500}
  ]
}"##;

struct Options {
    file: Option<PathBuf>,
    port: u16,
    save: bool,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        file: None,
        port: DEFAULT_PORT,
        save: false,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                let port = arguments.next().ok_or("--port needs a number")?;
                options.port = port
                    .parse()
                    .map_err(|_| format!("\"{port}\" is not a port number"))?;
            }
            "--save" => options.save = true,
            "-h" | "--help" => {
                return Err("usage: serve [file.moshi] [--port N] [--save]".to_owned());
            }
            file if options.file.is_none() && !file.starts_with('-') => {
                options.file = Some(PathBuf::from(file));
            }
            other => return Err(format!("unexpected argument \"{other}\"")),
        }
    }
    if options.save && options.file.is_none() {
        return Err("--save needs a file to save to".to_owned());
    }
    Ok(options)
}

fn run() -> Result<(), String> {
    let options = options()?;
    let session = match &options.file {
        Some(file) => SessionHandle::open(file).map_err(|error| error.to_string())?,
        None => SessionHandle::new(Session::new(
            io::parse(DEMO).map_err(|error| error.to_string())?,
        )),
    };
    let save = options.save;
    let subscription = session.subscribe({
        let session = session.clone();
        move |change| {
            let summary = session.read(|session| {
                session
                    .undo_steps()
                    .last()
                    .map(|step| step.summary.clone())
                    .unwrap_or_default()
            });
            eprintln!(
                "revision {}: {:?} by {} {summary}",
                change.revision, change.kind, change.origin
            );
            if save {
                match session.write(Session::save) {
                    Ok(path) => eprintln!("  saved {}", path.display()),
                    Err(error) => eprintln!("  not saved: {error}"),
                }
            }
        }
    });

    let (sink, inbox) = ImportSink::new();
    let tools = Tools::new(session.clone(), Host::new());
    let server = HttpServer::start_with_imports(
        McpServer::new(tools),
        HttpConfig {
            port: options.port,
            ..HttpConfig::default()
        },
        sink,
    )
    .map_err(|error| format!("cannot listen on port {}: {error}", options.port))?;

    let name = session.read(Session::file_name);
    println!("Serving {name} over MCP at {}", server.url());
    println!(
        "Claude Code: claude mcp add --transport http moshionly {}",
        server.url()
    );
    println!("Press Ctrl+C to stop.");
    // Designs from the Figma plugin are only listed: the app is what adds
    // them to the open project.
    for pending in inbox {
        println!("Received a design: {pending}. The app would add it.");
    }
    drop(subscription);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
