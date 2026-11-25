//! `import_media`: bring a video or sound file into the project's assets
//! folder, so a video layer or an audio item can play it.
//!
//! This crate reads no media itself: what a file holds (how long it is,
//! how large its picture is, whether it has sound) comes from the host's
//! [`MediaProbe`](crate::host::MediaProbe), which the editor implements
//! with `ffmpeg`.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use moshionly_core::io::{self, Asset};
use serde_json::{Value, json};

use super::args::Args;
use super::files::{safe_name, split_name};
use super::image::ASSETS_FOLDER;
use super::{Context, ToolDef, input_schema};
use crate::host::{HostError, MediaInfo};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static IMPORT_MEDIA: ToolDef = ToolDef {
    name: "import_media",
    title: "Import video or sound",
    description: "Copies a video or sound file (mp4, mov, webm, mp3, wav, m4a and the like) \
        from a file path into the project's assets folder and returns the \"source\" to give a \
        video layer or an audio item, with the file's duration in milliseconds, its picture size \
        and whether it has sound, and a ready-made node to add with apply_edits. Web addresses \
        are not fetched; download the file first.",
    read_only: false,
    destructive: false,
    schema: || {
        input_schema(
            json!({
                "source": {
                    "type": "string",
                    "description": "An absolute file path, or a path relative to the project file."
                },
                "name": {
                    "type": "string",
                    "description": "A file name for the copy in the assets folder, such as \"intro\". Default: the original name."
                }
            }),
            &["source"],
        )
    },
    run,
};

/// The largest file the tool copies, in bytes: 8 GB.
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let source = args.required_string("source", "the video or sound file's path")?;
    let Some(probe) = context.host.media.as_ref() else {
        return Err(ToolError::NotAvailable {
            what: "Importing video and sound is",
            why: "this host cannot read media files; that needs the Moshionly app with ffmpeg \
                  installed.",
            instead: "Tell the person to install ffmpeg (on a Mac: brew install ffmpeg) and \
                      import the file in the app.",
        });
    };
    let project_file = context.session.path();
    let path = match io::resolve_asset(&source, project_file.as_deref()) {
        Ok(Asset::File(path)) => path,
        Ok(Asset::Data { .. }) => {
            return Err(ToolError::argument(
                "source",
                "Video and sound cannot be embedded as \"data:\" URIs. Give the file's path.",
            ));
        }
        Err(error) => {
            return Err(ToolError::argument(
                "source",
                format!(
                    "{error}. To use a file from the web, ask the person to download it and give \
                     its path."
                ),
            ));
        }
    };
    let length = file_length(&path)?;
    let info = probe.probe(&path).map_err(|error| match error {
        HostError::Unsupported(message) => ToolError::Host(HostError::Unsupported(message)),
        other => ToolError::argument(
            "source",
            format!(
                "\"{}\" cannot be read as video or sound: {other}",
                path.display()
            ),
        ),
    })?;
    if !info.has_video && !info.has_sound {
        return Err(ToolError::argument(
            "source",
            format!("\"{}\" has neither video nor sound.", path.display()),
        ));
    }

    let mut result = json!({});
    let stored = match project_file.as_deref() {
        Some(project) if inside_project(&path, project) => {
            io::asset_source_for(&path, Some(project))
        }
        Some(project) => {
            let folder = project
                .parent()
                .unwrap_or(Path::new("."))
                .join(ASSETS_FOLDER);
            let original = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (_, extension) = split_name(&original);
            let name = args.string("name")?.unwrap_or_else(|| original.clone());
            let stem = safe_name(split_name(&name).0, "media");
            let extension = safe_name(&extension.unwrap_or_default().to_lowercase(), "");
            let copy = store(&folder, &stem, &extension, &path, length)?;
            result["copied_to"] = json!(copy);
            io::asset_source_for(&copy, Some(project))
        }
        None => {
            result["note"] = json!(
                "The project has not been saved yet, so there is no assets folder: the file is \
                 used from where it is, by its full path."
            );
            path.to_string_lossy().into_owned()
        }
    };
    describe(&mut result, &stored, &info);
    Ok(ToolOutput::json(&result))
}

/// Fills in what the file holds and the node to add for it.
fn describe(result: &mut Value, stored: &str, info: &MediaInfo) {
    let stem = Path::new(stored)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    result["source"] = json!(stored);
    result["duration_ms"] = json!(info.duration_ms.map(|ms| ms.round()));
    result["has_video"] = json!(info.has_video);
    result["has_sound"] = json!(info.has_sound);
    // Without a known length, a clip of four seconds to adjust.
    let end = info.duration_ms.map_or(4000.0, |ms| ms.round().max(1.0));
    if info.has_video {
        result["width"] = json!(info.width);
        result["height"] = json!(info.height);
        result["node"] = json!({
            "type": "video",
            "name": stem,
            "source": stored,
            "width": info.width,
            "height": info.height,
            "start": 0,
            "end": end,
        });
        result["hint"] = json!(
            "Add it with apply_edits, {\"op\":\"add\",\"parent\":\"<artboard or group>\",\"node\":\
             <node above>}. Change width and height by the same factor to keep its proportions. \
             \"start\" and \"end\" place it on the timeline; it plays its file from \
             \"trim_start\", so keep end - start at most the duration minus trim_start. Set \
             \"volume\" to 0 to silence it."
        );
    } else {
        result["node"] = json!({
            "type": "audio",
            "name": stem,
            "source": stored,
            "start": 0,
            "end": end,
        });
        result["hint"] = json!(
            "Add it with apply_edits, {\"op\":\"add\",\"parent\":\"<artboard>\",\"node\":<node \
             above>}: sound goes on an artboard's timeline. \"start\" and \"end\" place it in \
             time; it plays its file from \"trim_start\", so keep end - start at most the \
             duration minus trim_start."
        );
    }
}

/// The size of the file at `path`, refused when it is missing, a folder,
/// or too large.
fn file_length(path: &Path) -> Result<u64, ToolError> {
    let shown = path.display();
    let metadata = std::fs::metadata(path).map_err(|error| {
        ToolError::argument(
            "source",
            format!("\"{shown}\" cannot be read: {error}. Check the path."),
        )
    })?;
    if !metadata.is_file() {
        return Err(ToolError::argument(
            "source",
            format!("\"{shown}\" is not a file."),
        ));
    }
    if metadata.len() > MAX_BYTES {
        return Err(ToolError::argument(
            "source",
            format!(
                "\"{shown}\" is {} GB; files up to {} GB can be imported.",
                metadata.len() >> 30,
                MAX_BYTES >> 30
            ),
        ));
    }
    Ok(metadata.len())
}

/// Whether `path` is in the folder of the project file, or below it.
fn inside_project(path: &Path, project: &Path) -> bool {
    let folder = project.parent().unwrap_or(Path::new("."));
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    canonical(path).starts_with(canonical(folder))
}

/// Copies `original` to a free name in `folder`, or finds the same file
/// already stored under that name.
fn store(
    folder: &Path,
    stem: &str,
    extension: &str,
    original: &Path,
    length: u64,
) -> Result<PathBuf, ToolError> {
    let failed = |path: &Path, source: std::io::Error| {
        ToolError::Host(HostError::Io {
            path: path.to_owned(),
            source,
        })
    };
    std::fs::create_dir_all(folder).map_err(|error| failed(folder, error))?;
    let name = |n: usize| {
        let stem = if n == 1 {
            stem.to_owned()
        } else {
            format!("{stem} {n}")
        };
        match extension {
            "" => stem,
            extension => format!("{stem}.{extension}"),
        }
    };
    for n in 1.. {
        let path = folder.join(name(n));
        if path.exists() {
            if same_file(&path, original, length) {
                return Ok(path);
            }
            continue;
        }
        // Copied under a temporary name and moved into place, so a copy
        // cut short is never taken for the file.
        let partial = folder.join(format!(".{}.part", name(n)));
        std::fs::copy(original, &partial).map_err(|error| failed(&partial, error))?;
        match std::fs::hard_link(&partial, &path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&partial);
                return Ok(path);
            }
            // Another call took the name a moment ago.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&partial);
            }
            Err(_) => {
                // A file system without hard links: move it, which may
                // replace a file made meanwhile, as copying would.
                std::fs::rename(&partial, &path).map_err(|error| failed(&path, error))?;
                return Ok(path);
            }
        }
    }
    unreachable!("the names never run out")
}

/// Whether the files at `a` and `b` hold the same bytes; `b` is `length`
/// bytes long.
fn same_file(a: &Path, b: &Path, length: u64) -> bool {
    if std::fs::metadata(a).map(|metadata| metadata.len()).ok() != Some(length) {
        return false;
    }
    let (Ok(a), Ok(b)) = (File::open(a), File::open(b)) else {
        return false;
    };
    let (mut a, mut b) = (BufReader::new(a), BufReader::new(b));
    let (mut left, mut right) = ([0u8; 8192], [0u8; 8192]);
    loop {
        let (Ok(read), Ok(other)) = (a.read(&mut left), b.read(&mut right)) else {
            return false;
        };
        if read != other || left[..read] != right[..other] {
            return false;
        }
        if read == 0 {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::host::{HostError, MediaInfo, MediaProbe};
    use crate::tools::testing::{TempDir, call, call_error, demo_document, demo_tools};
    use crate::{Host, Session, SessionHandle, Tools};

    /// Says that `.mp4` files are 1.5 second 640 by 360 videos with sound
    /// and `.wav` files 2 seconds of sound, and cannot read anything else.
    struct FakeProbe;

    impl MediaProbe for FakeProbe {
        fn probe(&self, path: &std::path::Path) -> Result<MediaInfo, HostError> {
            let extension = path
                .extension()
                .map(|extension| extension.to_string_lossy().to_lowercase());
            match extension.as_deref() {
                Some("mp4") => Ok(MediaInfo {
                    duration_ms: Some(1500.4),
                    width: 640,
                    height: 360,
                    has_video: true,
                    has_sound: true,
                }),
                Some("wav") => Ok(MediaInfo {
                    duration_ms: Some(2000.0),
                    width: 0,
                    height: 0,
                    has_video: false,
                    has_sound: true,
                }),
                _ => Err(HostError::Failed("Invalid data found".into())),
            }
        }
    }

    /// Tools on the demo document saved as `project.moshi` in `dir`, with
    /// the fake probe.
    fn saved_tools(dir: &TempDir) -> Tools {
        let file = dir.path().join("project.moshi");
        let session = Session::with_path(demo_document(), &file);
        Tools::new(
            SessionHandle::new(session),
            Host::new().with_media(Arc::new(FakeProbe)),
        )
    }

    #[test]
    fn a_video_is_copied_into_the_assets_folder_once_and_can_be_added() {
        let dir = TempDir::new("media-video");
        let outside = TempDir::new("media-outside");
        let clip = outside.path().join("Launch Clip.MP4");
        std::fs::write(&clip, b"not really a video").unwrap();
        let clip_lower = outside.path().join("launch.mp4");
        std::fs::write(&clip_lower, b"not really a video").unwrap();
        let tools = saved_tools(&dir);

        let result = call(&tools, "import_media", json!({ "source": clip_lower }));
        assert_eq!(result["source"], "assets/launch.mp4");
        assert_eq!(result["duration_ms"], json!(1500.0));
        assert_eq!(
            (result["width"].clone(), result["height"].clone()),
            (json!(640), json!(360))
        );
        assert_eq!(result["has_sound"], true);
        assert_eq!(
            result["node"],
            json!({ "type": "video", "name": "launch", "source": "assets/launch.mp4",
                    "width": 640, "height": 360, "start": 0, "end": 1500.0 })
        );
        assert_eq!(
            std::fs::read(dir.path().join("assets/launch.mp4")).unwrap(),
            b"not really a video"
        );
        // The same file again is the same copy.
        let again = call(&tools, "import_media", json!({ "source": clip_lower }));
        assert_eq!(again["source"], "assets/launch.mp4");
        let other = call(
            &tools,
            "import_media",
            json!({ "source": clip, "name": "launch.mp4" }),
        );
        assert_eq!(other["source"], "assets/launch.mp4", "the same bytes");

        let added = call(
            &tools,
            "apply_edits",
            json!({ "operations": [{ "op": "add", "parent": "scene", "node": result["node"] }] }),
        );
        assert_eq!(added["created"][0]["type"], "video");
        assert!(!dir.path().join("assets").read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        }));
    }

    #[test]
    fn a_sound_becomes_an_audio_item_on_a_timeline() {
        let dir = TempDir::new("media-sound");
        let tools = saved_tools(&dir);
        std::fs::write(dir.path().join("music.wav"), b"sound").unwrap();
        // Inside the project, it is used where it is.
        let result = call(&tools, "import_media", json!({ "source": "music.wav" }));
        assert_eq!(result["source"], "music.wav");
        assert!(result.get("copied_to").is_none());
        assert!(result.get("width").is_none());
        assert_eq!(result["node"]["type"], "audio");
        assert_eq!(result["node"]["end"], json!(2000.0));
        let added = call(
            &tools,
            "apply_edits",
            json!({ "operations": [{ "op": "add", "parent": "scene", "node": result["node"] }] }),
        );
        assert_eq!(added["created"][0]["type"], "audio");
    }

    #[test]
    fn what_cannot_be_read_is_refused_and_a_host_without_a_probe_says_so() {
        let dir = TempDir::new("media-bad");
        let tools = saved_tools(&dir);
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        let error = call_error(&tools, "import_media", json!({ "source": text }));
        assert!(
            error.contains("cannot be read as video or sound"),
            "{error}"
        );
        let error = call_error(
            &tools,
            "import_media",
            json!({ "source": "data:audio/wav;base64,AAAA" }),
        );
        assert!(error.contains("cannot be embedded"), "{error}");
        let error = call_error(
            &tools,
            "import_media",
            json!({ "source": "https://example.com/a.mp4" }),
        );
        assert!(error.contains("download it"), "{error}");
        let error = call_error(
            &tools,
            "import_media",
            json!({ "source": dir.path().join("missing.mp4") }),
        );
        assert!(error.contains("cannot be read"), "{error}");
        assert!(!dir.path().join("assets").exists());

        let error = call_error(&demo_tools(), "import_media", json!({ "source": "a.mp4" }));
        assert!(error.contains("ffmpeg"), "{error}");
    }
}
