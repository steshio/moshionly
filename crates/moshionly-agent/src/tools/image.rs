//! `import_image`: bring an image into the project's assets folder, so an
//! image layer can show it.

use std::path::{Path, PathBuf};

use moshionly_core::io::{self, Asset, AssetError};
use serde_json::json;

use super::args::Args;
use super::files::{free_path, safe_name, split_name};
use super::{Context, ToolDef, input_schema};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static IMPORT_IMAGE: ToolDef = ToolDef {
    name: "import_image",
    title: "Import an image",
    description: "Copies a PNG, JPEG, GIF or WebP image from a file path or a \"data:\" URI into \
        the project's assets folder and returns the \"source\" to give an image layer, with the \
        image's size in pixels and a ready-made node to add with apply_edits. Web addresses are \
        not fetched; download the image first.",
    read_only: false,
    destructive: false,
    schema: || {
        input_schema(
            json!({
                "source": {
                    "type": "string",
                    "description": "An absolute file path, a path relative to the project file, or a \"data:image/...;base64,\" URI."
                },
                "name": {
                    "type": "string",
                    "description": "A file name for the copy in the assets folder, such as \"hero\". Default: the original name."
                }
            }),
            &["source"],
        )
    },
    run,
};

/// The largest image the tool copies, in bytes.
const MAX_BYTES: u64 = 40 * 1024 * 1024;
/// The folder next to the project file that images are copied into.
pub(crate) const ASSETS_FOLDER: &str = "assets";

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let source = args.required_string("source", "the image's file path or \"data:\" URI")?;
    let project_file = context.session.path();
    let asset = io::resolve_asset(&source, project_file.as_deref()).map_err(|error| {
        let message = match error {
            AssetError::Unsupported(_) => format!(
                "{error}. To use an image from the web, ask the person to download it and give \
                 its path."
            ),
            other => other.to_string(),
        };
        ToolError::argument("source", capitalised(&message))
    })?;
    let (bytes, original) = match &asset {
        Asset::File(path) => (read_image(path)?, Some(path.clone())),
        Asset::Data { bytes, .. } => (bytes.clone(), None),
    };
    let info = sniff(&bytes).ok_or_else(|| {
        ToolError::argument(
            "source",
            "This is not a PNG, JPEG, GIF or WebP image, which are the kinds an image layer can \
             show. Convert it first; an SVG can be redrawn with path layers.",
        )
    })?;

    let mut result = json!({});
    let stored = match (&original, project_file.as_deref()) {
        // Already in the project: use it where it is.
        (Some(path), Some(project)) if inside_project(path, project) => {
            io::asset_source_for(path, Some(project))
        }
        (_, Some(project)) => {
            let folder = project
                .parent()
                .unwrap_or(Path::new("."))
                .join(ASSETS_FOLDER);
            let name = match (args.string("name")?, &original) {
                (Some(name), _) => name,
                (None, Some(path)) => path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                (None, None) => String::new(),
            };
            let name = safe_name(&name, "image");
            let copy = store(&folder, split_name(&name).0, info.extension, &bytes)?;
            result["copied_to"] = json!(copy);
            io::asset_source_for(&copy, Some(project))
        }
        (Some(path), None) => {
            result["note"] = json!(
                "The project has not been saved yet, so there is no assets folder: the image is \
                 used from where it is, by its full path."
            );
            path.to_string_lossy().into_owned()
        }
        (None, None) => {
            return Err(ToolError::Rejected(
                "The project has not been saved yet, so there is no assets folder to put the \
                 image in. Ask the person to save the project first, or give the image as a \
                 file path."
                    .to_owned(),
            ));
        }
    };
    let stem = Path::new(&stored)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    result["source"] = json!(stored);
    result["media_type"] = json!(info.media_type);
    result["width"] = json!(info.width);
    result["height"] = json!(info.height);
    result["node"] = json!({
        "type": "image",
        "name": stem,
        "source": stored,
        "width": info.width,
        "height": info.height,
    });
    result["hint"] = json!(
        "Add it with apply_edits, {\"op\":\"add\",\"parent\":\"<artboard or group>\",\"node\":\
         <node above>}, and change width and height by the same factor to keep its proportions."
    );
    Ok(ToolOutput::json(&result))
}

/// The bytes of an image file, refused when it is missing or too large.
fn read_image(path: &Path) -> Result<Vec<u8>, ToolError> {
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
            format!("\"{shown}\" is a folder, not an image file."),
        ));
    }
    if metadata.len() > MAX_BYTES {
        return Err(ToolError::argument(
            "source",
            format!(
                "\"{shown}\" is {} MB; images up to {} MB can be imported. Make it smaller \
                 first.",
                metadata.len() / (1024 * 1024),
                MAX_BYTES / (1024 * 1024)
            ),
        ));
    }
    std::fs::read(path).map_err(|error| {
        ToolError::argument("source", format!("\"{shown}\" cannot be read: {error}."))
    })
}

/// Whether `path` is in the folder of the project file, or below it.
fn inside_project(path: &Path, project: &Path) -> bool {
    let folder = project.parent().unwrap_or(Path::new("."));
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    canonical(path).starts_with(canonical(folder))
}

/// Writes `bytes` to a free name in `folder`, or finds the same image
/// already stored under that name.
fn store(folder: &Path, stem: &str, extension: &str, bytes: &[u8]) -> Result<PathBuf, ToolError> {
    let failed = |path: &Path, source: std::io::Error| {
        ToolError::Host(crate::host::HostError::Io {
            path: path.to_owned(),
            source,
        })
    };
    std::fs::create_dir_all(folder).map_err(|error| failed(folder, error))?;
    let same = folder.join(format!("{stem}.{extension}"));
    if std::fs::read(&same).is_ok_and(|existing| existing == bytes) {
        return Ok(same);
    }
    let path = free_path(folder, stem, Some(extension));
    std::fs::write(&path, bytes).map_err(|error| failed(&path, error))?;
    Ok(path)
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What the first bytes of an image file say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageInfo {
    pub media_type: &'static str,
    pub extension: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Recognises a PNG, JPEG, GIF or WebP file and reads its size, without
/// decoding it.
pub(crate) fn sniff(bytes: &[u8]) -> Option<ImageInfo> {
    let info = |media_type, extension, width, height| {
        Some(ImageInfo {
            media_type,
            extension,
            width,
            height,
        })
    };
    let be16 = |at: usize| {
        Some(u32::from(u16::from_be_bytes([
            *bytes.get(at)?,
            *bytes.get(at + 1)?,
        ])))
    };
    let le16 = |at: usize| {
        Some(u32::from(u16::from_le_bytes([
            *bytes.get(at)?,
            *bytes.get(at + 1)?,
        ])))
    };
    let le24 = |at: usize| {
        Some(u32::from_le_bytes([
            *bytes.get(at)?,
            *bytes.get(at + 1)?,
            *bytes.get(at + 2)?,
            0,
        ]))
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return info("image/png", "png", width, height);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return info("image/gif", "gif", le16(6)?, le16(8)?);
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut at = 2;
        while at + 3 < bytes.len() {
            if bytes[at] != 0xFF {
                return None;
            }
            let marker = bytes[at + 1];
            match marker {
                0xFF => at += 1,
                0x01 | 0xD0..=0xD9 => at += 2,
                0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                    return info("image/jpeg", "jpg", be16(at + 7)?, be16(at + 5)?);
                }
                _ => at += 2 + usize::try_from(be16(at + 2)?).ok()?,
            }
        }
        return None;
    }
    if bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        let (width, height) = match bytes.get(12..16)? {
            b"VP8 " => (le16(26)? & 0x3FFF, le16(28)? & 0x3FFF),
            b"VP8L" => {
                let b = bytes.get(21..25)?;
                let width = 1 + (u32::from(b[0]) | (u32::from(b[1] & 0x3F) << 8));
                let height = 1
                    + ((u32::from(b[1]) >> 6)
                        | (u32::from(b[2]) << 2)
                        | (u32::from(b[3] & 0x0F) << 10));
                (width, height)
            }
            b"VP8X" => (1 + le24(24)?, 1 + le24(27)?),
            _ => return None,
        };
        return info("image/webp", "webp", width, height);
    }
    None
}

#[cfg(test)]
mod tests {
    use moshionly_core::build::PLACEHOLDER_IMAGE;
    use moshionly_core::io::{self, Asset};
    use serde_json::json;

    use super::*;
    use crate::tools::testing::{TempDir, call, call_error, demo_document, demo_tools};
    use crate::{Host, Session, SessionHandle, Tools};

    fn png() -> Vec<u8> {
        match io::resolve_asset(PLACEHOLDER_IMAGE, None).unwrap() {
            Asset::Data { bytes, .. } => bytes,
            Asset::File(_) => unreachable!(),
        }
    }

    /// Tools on the demo document saved as `project.moshi` in `dir`.
    fn saved_tools(dir: &TempDir) -> Tools {
        let file = dir.path().join("project.moshi");
        let session = Session::with_path(demo_document(), &file);
        Tools::new(SessionHandle::new(session), Host::new())
    }

    #[test]
    fn a_data_uri_is_stored_in_the_assets_folder() {
        let dir = TempDir::new("import-data");
        let tools = saved_tools(&dir);
        let result = call(
            &tools,
            "import_image",
            json!({ "source": PLACEHOLDER_IMAGE, "name": "Hero shot.webp" }),
        );
        // The real kind of the image decides the extension.
        assert_eq!(result["source"], "assets/Hero shot.png");
        assert_eq!(result["media_type"], "image/png");
        assert_eq!(
            (result["width"].clone(), result["height"].clone()),
            (json!(1), json!(1))
        );
        assert_eq!(
            std::fs::read(dir.path().join("assets/Hero shot.png")).unwrap(),
            png()
        );
        assert_eq!(result["node"]["type"], "image");
        assert_eq!(result["node"]["source"], "assets/Hero shot.png");

        // The node it suggests can be added as it is.
        let added = call(
            &tools,
            "apply_edits",
            json!({ "operations": [{ "op": "add", "parent": "scene", "node": result["node"] }] }),
        );
        assert_eq!(added["created"][0]["type"], "image");
    }

    #[test]
    fn a_file_is_copied_once_and_a_project_file_is_used_in_place() {
        let dir = TempDir::new("import-file");
        let outside = TempDir::new("import-outside");
        let photo = outside.path().join("photo.png");
        std::fs::write(&photo, png()).unwrap();
        let tools = saved_tools(&dir);

        let first = call(&tools, "import_image", json!({ "source": photo }));
        assert_eq!(first["source"], "assets/photo.png");
        let again = call(&tools, "import_image", json!({ "source": photo }));
        assert_eq!(
            again["source"], "assets/photo.png",
            "the same image is reused"
        );

        std::fs::write(dir.path().join("logo.png"), png()).unwrap();
        let inside = call(&tools, "import_image", json!({ "source": "logo.png" }));
        assert_eq!(inside["source"], "logo.png");
        assert!(inside.get("copied_to").is_none());
        assert!(!dir.path().join("assets/logo.png").exists());
    }

    #[test]
    fn an_unsaved_project_uses_files_where_they_are_and_refuses_data() {
        let outside = TempDir::new("import-unsaved");
        let photo = outside.path().join("photo.png");
        std::fs::write(&photo, png()).unwrap();
        let tools = demo_tools();
        let result = call(&tools, "import_image", json!({ "source": photo }));
        assert_eq!(result["source"], photo.to_string_lossy().as_ref());
        assert!(result["note"].as_str().unwrap().contains("not been saved"));

        let error = call_error(
            &tools,
            "import_image",
            json!({ "source": PLACEHOLDER_IMAGE }),
        );
        assert!(error.contains("save the project first"), "{error}");
    }

    #[test]
    fn what_is_not_an_image_is_refused() {
        let dir = TempDir::new("import-bad");
        let tools = saved_tools(&dir);
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        let error = call_error(&tools, "import_image", json!({ "source": text }));
        assert!(
            error.starts_with("This is not a PNG, JPEG, GIF or WebP image"),
            "{error}"
        );
        let error = call_error(
            &tools,
            "import_image",
            json!({ "source": "https://example.com/a.png" }),
        );
        assert!(error.contains("download it"), "{error}");
        let error = call_error(
            &tools,
            "import_image",
            json!({ "source": dir.path().join("missing.png") }),
        );
        assert!(error.contains("cannot be read"), "{error}");
        assert!(!dir.path().join(ASSETS_FOLDER).exists());
    }

    #[test]
    fn sizes_are_read_from_the_headers() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend([0x40, 0x01, 0xF0, 0x00]);
        assert_eq!(sniff(&gif).map(|i| (i.width, i.height)), Some((320, 240)));

        // A JPEG with an APP0 segment before the frame header.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend([0xFF, 0xC0, 0x00, 0x11, 0x08, 0x02, 0x58, 0x03, 0x20]);
        let info = sniff(&jpeg).unwrap();
        assert_eq!(
            (info.media_type, info.width, info.height),
            ("image/jpeg", 800, 600)
        );

        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend([0; 8]);
        webp.extend([0x7F, 0x07, 0x00, 0x37, 0x04, 0x00]);
        let info = sniff(&webp).unwrap();
        assert_eq!(
            (info.extension, info.width, info.height),
            ("webp", 1920, 1080)
        );

        assert_eq!(sniff(b"<svg xmlns=..."), None);
        assert_eq!(sniff(&[0xFF, 0xD8, 0x00]), None);
    }
}
