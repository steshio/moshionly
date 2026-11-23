//! The image files a project uses, kept with its project file.
//!
//! A saved project is meant to be self-contained: the images its layers
//! show sit in its folder (imports put them in `assets/`), and image layers
//! name them relative to the project file, so the folder can be moved or
//! shared as a whole. Saving the project in another folder keeps that true:
//! the images are copied along and the layers point at the copies.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use moshionly_core::io::{self, Asset};
use moshionly_core::{Document, Edit};

use super::inspect::image_of;

/// The folder next to the project file that images are copied into.
pub const ASSETS_FOLDER: &str = "assets";

/// The most copies of one name tried before giving up on a free one.
const MAX_NAMES: usize = 10_000;

/// Whether `source` is a `data:` URI, which carries the image itself. Only
/// the start is looked at: the rest can be megabytes long.
pub fn is_data_uri(source: &str) -> bool {
    source
        .trim_start()
        .get(..5)
        .is_some_and(|start| start.eq_ignore_ascii_case("data:"))
}

/// The image file `source` names, as an absolute path, for a project saved
/// as `project_file` (or never saved). `None` for a `data:` URI or a link.
pub fn image_file(source: &str, project_file: Option<&Path>) -> Option<PathBuf> {
    if is_data_uri(source) {
        return None;
    }
    match io::resolve_asset(source, project_file) {
        Ok(Asset::File(path)) => Some(std::path::absolute(&path).unwrap_or(path)),
        _ => None,
    }
}

/// The folder of a project file.
fn folder_of(project_file: &Path) -> &Path {
    project_file.parent().unwrap_or(Path::new("."))
}

/// `path` with links and `..` resolved when it exists, as it is otherwise.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

/// Whether two project files are in the same folder.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    canonical(folder_of(a)) == canonical(folder_of(b))
}

/// `file` relative to `folder`, written with `/` as a source is, when it is
/// in that folder or below it.
fn relative_source(file: &Path, folder: &Path) -> Option<String> {
    let relative = canonical(file)
        .strip_prefix(canonical(folder))
        .ok()?
        .to_owned();
    let parts: Option<Vec<String>> = relative
        .components()
        .map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts
        .filter(|parts| !parts.is_empty())
        .map(|parts| parts.join("/"))
}

/// A file's name for messages.
fn shown_name(file: &Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Why a copy failed, in plain words.
fn reason(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => "the file is not there any more".to_owned(),
        std::io::ErrorKind::PermissionDenied => "it may not be read or written".to_owned(),
        _ => error.to_string(),
    }
}

/// Copies the image `file` to `preferred`, or beside it under the first free
/// name (`hero 2.png`, `hero 3.png`, ...) when another image already has
/// that name. A file there with the same content is used as it is. Returns
/// where the copy is.
fn copy_to(file: &Path, preferred: &Path) -> std::io::Result<PathBuf> {
    let bytes = std::fs::read(file)?;
    let folder = preferred.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder)?;
    let stem = preferred
        .file_stem()
        .map_or_else(|| "image".to_owned(), |s| s.to_string_lossy().into_owned());
    let extension = preferred
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..=MAX_NAMES {
        let candidate = if n == 1 {
            preferred.to_owned()
        } else {
            folder.join(format!("{stem} {n}{extension}"))
        };
        match std::fs::read(&candidate) {
            Ok(existing) if existing == bytes => return Ok(candidate),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate);
        match created {
            Ok(mut out) => {
                std::io::Write::write_all(&mut out, &bytes)?;
                return Ok(candidate);
            }
            // Someone else took the name meanwhile: look at it again.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("no free name for the copy"))
}

/// The source that shows the same image from a project saved as `to`,
/// copying the image into its folder when it is not there already. `from`
/// is the project file `source` is relative to now, if any. `Ok(None)` when
/// the source needs no change: a `data:` URI or a link.
fn follow(source: &str, from: Option<&Path>, to: &Path) -> Result<Option<String>, String> {
    let Some(file) = image_file(source, from) else {
        return Ok(None);
    };
    let target = folder_of(to);
    if let Some(inside) = relative_source(&file, target) {
        return Ok(Some(inside));
    }
    // A relative source keeps its place in the folder, so that sources do
    // not change unless a name is taken; anything else goes in `assets`.
    let kept = from
        .filter(|_| Path::new(source.trim()).is_relative())
        .and_then(|from| relative_source(&file, folder_of(from)));
    let preferred = match kept {
        Some(relative) => target.join(relative),
        None => target
            .join(ASSETS_FOLDER)
            .join(file.file_name().unwrap_or(std::ffi::OsStr::new("image"))),
    };
    let copy = copy_to(&file, &preferred).map_err(|error| {
        format!(
            "Could not copy {} into the project's folder: {}.",
            shown_name(&file),
            reason(&error)
        )
    })?;
    Ok(Some(
        relative_source(&copy, target).unwrap_or_else(|| copy.to_string_lossy().into_owned()),
    ))
}

/// What following a project to a new place needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gathered {
    /// The edits that point image layers at their copies.
    pub edits: Vec<Edit>,
    /// A message for each image that could not be copied. Its layers keep
    /// their source.
    pub problems: Vec<String>,
}

/// Copies the images `document` uses into the folder of `to`, the project
/// file it is about to be saved as, and returns the edits that point the
/// image layers at the copies. `from` is the project file the document was
/// saved as until now, or `None` for a project never saved.
///
/// Images already in that folder are only named relative to it. Embedded
/// images (`data:` URIs) stay as they are.
pub fn gather_images(document: &Document, from: Option<&Path>, to: &Path) -> Gathered {
    let mut gathered = Gathered::default();
    let mut followed: HashMap<&str, Option<String>> = HashMap::new();
    for node in document.nodes() {
        let Some(image) = image_of(node) else {
            continue;
        };
        let source = image.source.as_str();
        let next = followed
            .entry(source)
            .or_insert_with(|| match follow(source, from, to) {
                Ok(next) => next,
                Err(problem) => {
                    gathered.problems.push(problem);
                    None
                }
            });
        if let Some(next) = next
            && next != source
        {
            gathered
                .edits
                .push(Edit::set(node.id.clone(), "source", next.clone()));
        }
    }
    gathered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for one test, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "moshionly-app-assets-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A document with one image layer per source.
    fn document(sources: &[&str]) -> Document {
        let mut nodes = vec![serde_json::json!({
            "id": "ab", "parent": null, "index": "a0", "type": "artboard"
        })];
        for (n, source) in sources.iter().enumerate() {
            nodes.push(serde_json::json!({
                "id": format!("i{n}"), "parent": "ab", "index": format!("a{n}"),
                "type": "image", "source": source
            }));
        }
        let text = serde_json::json!({
            "format": "moshionly", "version": 1, "name": "Test", "nodes": nodes
        });
        io::parse(&text.to_string()).unwrap()
    }

    fn sources(document: &Document, edits: &[Edit]) -> Vec<String> {
        let mut document = document.clone();
        document.apply(edits).unwrap();
        document
            .nodes()
            .filter_map(image_of)
            .map(|image| image.source.clone())
            .collect()
    }

    #[test]
    fn images_follow_a_project_to_another_folder() {
        let root = scratch("follow");
        let (a, b) = (root.join("a"), root.join("b"));
        write(&a.join("assets/hero.png"), b"hero");
        write(&a.join("assets/logo.png"), b"logo");
        // The new folder already has another image called logo.png.
        write(&b.join("assets/logo.png"), b"something else");
        let outside = root.join("downloads/photo.png");
        write(&outside, b"photo");
        let embedded = "data:image/png;base64,AAAA";
        let doc = document(&[
            "assets/hero.png",
            "assets/logo.png",
            outside.to_str().unwrap(),
            embedded,
            "assets/hero.png",
        ]);
        let gathered = gather_images(&doc, Some(&a.join("p.moshi")), &b.join("q.moshi"));
        assert!(gathered.problems.is_empty(), "{:?}", gathered.problems);
        assert_eq!(
            sources(&doc, &gathered.edits),
            [
                "assets/hero.png",
                "assets/logo 2.png",
                "assets/photo.png",
                embedded,
                "assets/hero.png",
            ]
        );
        assert_eq!(std::fs::read(b.join("assets/hero.png")).unwrap(), b"hero");
        assert_eq!(std::fs::read(b.join("assets/logo 2.png")).unwrap(), b"logo");
        assert_eq!(std::fs::read(b.join("assets/photo.png")).unwrap(), b"photo");
        // Only the layers whose source changed are edited.
        assert_eq!(gathered.edits.len(), 2);
        // Doing it again finds the same copies.
        let again = gather_images(&doc, Some(&a.join("p.moshi")), &b.join("q.moshi"));
        assert_eq!(again, gathered);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_first_save_takes_images_in_by_their_full_path() {
        let root = scratch("first");
        let photo = root.join("downloads/photo.png");
        write(&photo, b"photo");
        let project = root.join("work/teaser.moshi");
        write(&root.join("work/assets/inside.png"), b"inside");
        let inside = root.join("work/assets/inside.png");
        let doc = document(&[photo.to_str().unwrap(), inside.to_str().unwrap()]);
        let gathered = gather_images(&doc, None, &project);
        assert!(gathered.problems.is_empty(), "{:?}", gathered.problems);
        // The photo is copied in; the image already there is only named.
        assert_eq!(
            sources(&doc, &gathered.edits),
            ["assets/photo.png", "assets/inside.png"]
        );
        assert!(root.join("work/assets/photo.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_image_is_reported_and_left_alone() {
        let root = scratch("missing");
        let doc = document(&["assets/gone.png"]);
        let gathered = gather_images(&doc, Some(&root.join("a/p.moshi")), &root.join("b/q.moshi"));
        assert!(gathered.edits.is_empty());
        assert_eq!(gathered.problems.len(), 1);
        assert!(
            gathered.problems[0].contains("gone.png"),
            "{:?}",
            gathered.problems
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn data_uris_are_seen_from_their_start() {
        assert!(is_data_uri("data:image/png;base64,AAAA"));
        assert!(is_data_uri("  DATA:image/png,x"));
        assert!(!is_data_uri("assets/data.png"));
        assert!(!is_data_uri("dat"));
        assert_eq!(image_file("data:image/png;base64,AAAA", None), None);
        assert!(same_folder(
            Path::new("/x/y/a.moshi"),
            Path::new("/x/y/b.moshi")
        ));
        assert!(!same_folder(
            Path::new("/x/y/a.moshi"),
            Path::new("/x/z/a.moshi")
        ));
    }
}
