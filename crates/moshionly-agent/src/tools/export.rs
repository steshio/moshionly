//! `export`: render an artboard to a file through the host's exporter.

use std::path::{Path, PathBuf};

use moshionly_core::{Document, NodeId};
use serde_json::json;

use super::args::Args;
use super::common::{self, number};
use super::files::{free_path, safe_name, split_name};
use super::{Context, ToolDef, input_schema};
use crate::host::{ExportFormat, ExportOptions, ExportRequest, HostError};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static EXPORT: ToolDef = ToolDef {
    name: "export",
    title: "Export",
    description: "Renders an artboard to a file: an mp4, webm or prores video, an animated gif, \
        a png of one moment, a png_sequence folder of frames, or lottie JSON. Files go in the \
        project's exports folder and never replace an existing file; the result gives the path. \
        get_overview lists the formats this host can write. Export only when the person asks for \
        a file.",
    read_only: false,
    destructive: false,
    schema: || {
        let formats: Vec<&str> = ExportFormat::ALL
            .iter()
            .map(|format| format.as_str())
            .collect();
        input_schema(
            json!({
                "artboard": {
                    "type": "string",
                    "description": "The artboard to export. May be left out when the file has one."
                },
                "format": {
                    "type": "string",
                    "enum": formats,
                    "description": "The kind of file."
                },
                "file_name": {
                    "type": "string",
                    "description": "A name for the file, such as \"teaser\". Default: the artboard's name."
                },
                "path": {
                    "type": "string",
                    "description": "An absolute path to write to instead of the exports folder. Only accepted when the host allows it; give file_name otherwise."
                },
                "scale": {
                    "type": "number",
                    "minimum": MIN_SCALE,
                    "maximum": MAX_SCALE,
                    "description": "Output size as a multiple of the artboard's size. Default 1."
                },
                "fps": {
                    "type": "number",
                    "minimum": 1,
                    "maximum": MAX_FPS,
                    "description": "Frames per second for everything that moves. Default 30."
                },
                "time": {
                    "type": "number",
                    "minimum": 0,
                    "description": "png only: the moment to capture, in milliseconds. Default: the design at rest."
                },
                "transparent": {
                    "type": "boolean",
                    "description": "Leave the artboard's background out. Works for png, png_sequence, webm, prores and lottie. Default false."
                }
            }),
            &["format"],
        )
    },
    run,
};

const MIN_SCALE: f64 = 0.1;
const MAX_SCALE: f64 = 8.0;
const MAX_FPS: f64 = 120.0;
/// The folder next to the project file that exports go into.
const EXPORTS_FOLDER: &str = "exports";

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let Some(exporter) = context.host.exporter.as_ref() else {
        return Err(ToolError::NotAvailable {
            what: "Exports are",
            why: "this host has no exporter attached.",
            instead: "Tell the person they can export from the app.",
        });
    };
    let available = exporter.formats();
    let names: Vec<&str> = ExportFormat::ALL
        .iter()
        .map(|format| format.as_str())
        .collect();
    let format = args
        .choice("format", &names)?
        .and_then(ExportFormat::parse)
        .ok_or_else(|| args.missing("format", "the kind of file, such as \"mp4\" or \"gif\""))?;
    if !available.contains(&format) {
        let offered: Vec<&str> = available.iter().map(|format| format.as_str()).collect();
        return Err(ToolError::argument(
            "format",
            format!(
                "This host cannot write {format} files{}. It can write: {}.",
                if format == ExportFormat::Prores || format == ExportFormat::Webm {
                    " (they need ffmpeg, which is not installed)"
                } else {
                    ""
                },
                if offered.is_empty() {
                    "nothing".to_owned()
                } else {
                    offered.join(", ")
                }
            ),
        ));
    }

    let (document, project_file) = context
        .session
        .read(|session| (session.snapshot(), session.path().map(ToOwned::to_owned)));
    let artboard = artboard(&document, args)?;
    let options = options(args, format)?;
    let path = destination(
        context,
        args,
        &document,
        &artboard,
        format,
        project_file.as_deref(),
    )?;

    let exported = exporter.export(
        &ExportRequest {
            document: &document,
            project_file: project_file.as_deref(),
            artboard: &artboard,
            path: &path,
            format,
            options,
        },
        // The host shows progress itself; a tool call only has its result.
        &mut |_| {},
    )?;
    Ok(ToolOutput::json(&json!({
        "path": exported.path,
        "format": format.as_str(),
        "bytes": exported.bytes,
        "frames": exported.frames,
        "artboard": artboard,
        "scale": number(options.scale),
    })))
}

/// The artboard named, or the only one there is.
fn artboard(document: &Document, args: &Args) -> Result<NodeId, ToolError> {
    if let Some(id) = args.string("artboard")? {
        return Ok(common::artboard(document, &id, "artboard")?.id.clone());
    }
    match document.artboards() {
        [only] => Ok(only.clone()),
        [] => Err(ToolError::Rejected(
            "The file has no artboards, so there is nothing to export.".to_owned(),
        )),
        several => {
            let list: Vec<String> = several
                .iter()
                .filter_map(|id| document.get(id))
                .map(|node| node.label())
                .collect();
            Err(args.missing(
                "artboard",
                &format!("the artboard to export; the file has {}", list.join(", ")),
            ))
        }
    }
}

/// The export settings, checked against the format.
fn options(args: &Args, format: ExportFormat) -> Result<ExportOptions, ToolError> {
    let defaults = ExportOptions::default();
    let scale = args.number("scale")?.unwrap_or(defaults.scale);
    if !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return Err(args.wrong(
            "scale",
            &format!("a number from {MIN_SCALE} to {MAX_SCALE}"),
        ));
    }
    let fps = args.number("fps")?.unwrap_or(defaults.fps);
    if !(1.0..=MAX_FPS).contains(&fps) {
        return Err(args.wrong("fps", &format!("a number from 1 to {MAX_FPS}")));
    }
    let time_ms = args.number("time")?;
    if let Some(time) = time_ms {
        if format != ExportFormat::Png {
            return Err(ToolError::argument(
                "time",
                format!(
                    "\"time\" picks the moment for a png; a {format} file holds the whole \
                     timeline. Leave \"time\" out."
                ),
            ));
        }
        if time < 0.0 {
            return Err(args.wrong("time", "0 or more milliseconds"));
        }
    }
    let transparent = args.boolean("transparent")?.unwrap_or(false);
    if transparent && matches!(format, ExportFormat::Mp4 | ExportFormat::Gif) {
        return Err(ToolError::argument(
            "transparent",
            format!(
                "A {format} cannot be transparent. Use webm or prores for a transparent video, \
                 or png_sequence for frames."
            ),
        ));
    }
    Ok(ExportOptions {
        scale,
        fps,
        time_ms,
        transparent,
    })
}

/// Where the export goes: a free name in the exports folder, or the path
/// given when the host allows that.
fn destination(
    context: &Context<'_>,
    args: &Args,
    document: &Document,
    artboard: &NodeId,
    format: ExportFormat,
    project_file: Option<&Path>,
) -> Result<PathBuf, ToolError> {
    let extension = format.extension();
    if let Some(path) = args.string("path")? {
        if !context.config.export_anywhere {
            return Err(ToolError::argument(
                "path",
                "This host keeps exports in the project's exports folder. Leave \"path\" out \
                 and give \"file_name\" to name the file.",
            ));
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(ToolError::argument(
                "path",
                "\"path\" should be an absolute path, such as /Users/me/Desktop/teaser.mp4.",
            ));
        }
        if path.exists() {
            return Err(ToolError::argument(
                "path",
                format!(
                    "\"{}\" already exists, and exports never replace a file. Choose another \
                     name.",
                    path.display()
                ),
            ));
        }
        return Ok(path);
    }
    let folder = match (&context.config.export_dir, project_file) {
        (Some(folder), _) => folder.clone(),
        (None, Some(project)) => project
            .parent()
            .unwrap_or(Path::new("."))
            .join(EXPORTS_FOLDER),
        (None, None) => {
            return Err(ToolError::Rejected(
                "The project has not been saved yet, so there is no exports folder. Ask the \
                 person to save the project first."
                    .to_owned(),
            ));
        }
    };
    let name = match args.string("file_name")? {
        Some(name) => name,
        None => document
            .get(artboard)
            .map(|node| node.name().to_owned())
            .unwrap_or_default(),
    };
    let name = safe_name(&name, "Export");
    let stem = split_name(&name).0;
    std::fs::create_dir_all(&folder).map_err(|source| {
        ToolError::Host(HostError::Io {
            path: folder.clone(),
            source,
        })
    })?;
    Ok(free_path(&folder, stem, extension))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use crate::host::{ExportFormat, ExportProgress, ExportRequest, Exported, Exporter, HostError};
    use crate::tools::testing::{TempDir, call, call_error, demo_document, demo_tools};
    use crate::{Host, Session, SessionHandle, Tools, ToolsConfig};

    /// Writes a few bytes where it is told, and remembers the settings.
    #[derive(Default)]
    struct StubExporter {
        seen: Mutex<Vec<(ExportFormat, crate::ExportOptions)>>,
    }

    impl Exporter for StubExporter {
        fn formats(&self) -> Vec<ExportFormat> {
            vec![ExportFormat::Png, ExportFormat::Gif, ExportFormat::Mp4]
        }

        fn export(
            &self,
            request: &ExportRequest<'_>,
            progress: &mut dyn FnMut(ExportProgress),
        ) -> Result<Exported, HostError> {
            self.seen
                .lock()
                .unwrap()
                .push((request.format, request.options));
            progress(ExportProgress {
                frames_done: 1,
                frames_total: 1,
            });
            std::fs::write(request.path, b"frames").map_err(|source| HostError::Io {
                path: request.path.to_owned(),
                source,
            })?;
            Ok(Exported {
                path: request.path.to_owned(),
                bytes: 6,
                frames: 120,
            })
        }
    }

    fn tools(dir: Option<&TempDir>, config: ToolsConfig) -> (Tools, Arc<StubExporter>) {
        let exporter = Arc::new(StubExporter::default());
        let session = match dir {
            Some(dir) => Session::with_path(demo_document(), dir.path().join("project.moshi")),
            None => Session::new(demo_document()),
        };
        let tools = Tools::with_config(
            SessionHandle::new(session),
            Host::new().with_exporter(exporter.clone()),
            config,
        );
        (tools, exporter)
    }

    #[test]
    fn exports_go_in_the_exports_folder_and_never_overwrite() {
        let dir = TempDir::new("export");
        let (tools, exporter) = tools(Some(&dir), ToolsConfig::default());
        let first = call(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "mp4", "fps": 60 }),
        );
        let path = dir.path().join("exports/Launch.mp4");
        assert_eq!(first["path"], path.to_string_lossy().as_ref());
        assert_eq!(first["frames"], 120);
        assert_eq!(std::fs::read(&path).unwrap(), b"frames");

        let second = call(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "mp4", "file_name": "Launch.mov" }),
        );
        assert_eq!(
            second["path"],
            dir.path()
                .join("exports/Launch 2.mp4")
                .to_string_lossy()
                .as_ref()
        );
        let seen = exporter.seen.lock().unwrap();
        assert_eq!(seen[0].1.fps, 60.0);
        assert_eq!(seen[1].1.fps, 30.0);
    }

    #[test]
    fn a_png_takes_a_moment_and_a_name() {
        let dir = TempDir::new("export-png");
        let (tools, exporter) = tools(Some(&dir), ToolsConfig::default());
        let result = call(
            &tools,
            "export",
            json!({ "artboard": "second", "format": "png", "time": 250, "scale": 2,
                    "file_name": "../../poster" }),
        );
        assert_eq!(
            result["path"],
            dir.path()
                .join("exports/poster.png")
                .to_string_lossy()
                .as_ref()
        );
        let options = exporter.seen.lock().unwrap()[0].1;
        assert_eq!((options.time_ms, options.scale), (Some(250.0), 2.0));
    }

    #[test]
    fn settings_that_do_not_fit_the_format_are_refused() {
        let dir = TempDir::new("export-bad");
        let (tools, _) = tools(Some(&dir), ToolsConfig::default());
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "webm" }),
        );
        assert!(
            error.starts_with("This host cannot write webm files"),
            "{error}"
        );
        assert!(error.contains("It can write: png, gif, mp4."), "{error}");
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "mp4", "transparent": true }),
        );
        assert!(error.contains("cannot be transparent"), "{error}");
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif", "time": 100 }),
        );
        assert!(error.contains("picks the moment for a png"), "{error}");
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif", "scale": 50 }),
        );
        assert!(error.contains("from 0.1 to 8"), "{error}");
        let error = call_error(&tools, "export", json!({ "format": "gif" }));
        assert!(error.starts_with("\"artboard\" is required"), "{error}");
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif", "path": "/tmp/x.gif" }),
        );
        assert!(
            error.contains("keeps exports in the project's exports folder"),
            "{error}"
        );
        assert!(!dir.path().join("exports").exists());
    }

    #[test]
    fn a_host_may_allow_any_path_or_set_the_folder() {
        let target = TempDir::new("export-anywhere");
        let (tools, _) = tools(
            None,
            ToolsConfig {
                export_anywhere: true,
                export_dir: Some(target.path().join("out")),
                ..ToolsConfig::default()
            },
        );
        let file = target.path().join("teaser.gif");
        let result = call(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif", "path": file }),
        );
        assert_eq!(result["path"], file.to_string_lossy().as_ref());
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif", "path": file }),
        );
        assert!(error.contains("already exists"), "{error}");
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif", "path": "relative.gif" }),
        );
        assert!(error.contains("absolute path"), "{error}");
        // An unsaved project exports into the folder the host chose.
        let result = call(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "gif" }),
        );
        assert_eq!(
            result["path"],
            target
                .path()
                .join("out/Launch.gif")
                .to_string_lossy()
                .as_ref()
        );
    }

    #[test]
    fn without_an_exporter_or_a_folder_it_says_so() {
        let error = call_error(&demo_tools(), "export", json!({ "format": "mp4" }));
        assert!(
            error.starts_with("Exports are not available in this host"),
            "{error}"
        );
        let (tools, _) = tools(None, ToolsConfig::default());
        let error = call_error(
            &tools,
            "export",
            json!({ "artboard": "scene", "format": "mp4" }),
        );
        assert!(error.contains("has not been saved yet"), "{error}");
    }
}
