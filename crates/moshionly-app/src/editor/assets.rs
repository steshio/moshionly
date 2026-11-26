//! The image, video and sound files a project uses, kept with its project
//! file.
//!
//! A saved project is meant to be self-contained: the files its image and
//! video layers and its audio items play sit in its folder (imports put
//! them in `assets/`), and they are named relative to the project file, so
//! the folder can be moved or shared as a whole. Saving the project in
//! another folder, and pasting layers or sound into another project, keep
//! that true: the files are copied along and the nodes point at the
//! copies.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use moshionly_core::io::{self, Asset};
use moshionly_core::{Document, Edit, Node, NodeType};

/// The folder next to the project file that files are copied into.
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

/// The file `source` names, as an absolute path, for a project saved as
/// `project_file` (or never saved). `None` for a `data:` URI or a link.
pub fn asset_file(source: &str, project_file: Option<&Path>) -> Option<PathBuf> {
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

/// Copies `file` to `preferred`, or beside it under the first free name
/// (`hero 2.png`, `hero 3.png`, ...) when another file already has that
/// name. A file there with the same content is used as it is. Returns
/// where the copy is. Files are read a piece at a time, so a long video
/// is never held in memory whole.
fn copy_to(file: &Path, preferred: &Path) -> std::io::Result<PathBuf> {
    let length = std::fs::metadata(file)?.len();
    let folder = preferred.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder)?;
    let stem = preferred
        .file_stem()
        .map_or_else(|| "file".to_owned(), |s| s.to_string_lossy().into_owned());
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
        match same_content(file, &candidate, length) {
            Ok(true) => return Ok(candidate),
            Ok(false) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate);
        match created {
            Ok(mut out) => {
                let copied = File::open(file).and_then(|mut input| {
                    std::io::copy(&mut input, &mut out)?;
                    out.sync_all()
                });
                if let Err(error) = copied {
                    // Leave no half copy behind.
                    drop(out);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error);
                }
                return Ok(candidate);
            }
            // Someone else took the name meanwhile: look at it again.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("no free name for the copy"))
}

/// Whether the file at `other` holds the same bytes as `file`, which is
/// `length` bytes long. An error of kind `NotFound` when there is no file
/// at `other`.
fn same_content(file: &Path, other: &Path, length: u64) -> std::io::Result<bool> {
    let metadata = std::fs::metadata(other)?;
    if !metadata.is_file() || metadata.len() != length {
        return Ok(false);
    }
    let mut a = BufReader::new(File::open(file)?);
    let mut b = BufReader::new(File::open(other)?);
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    let mut left = length;
    while left > 0 {
        let piece = left.min(x.len() as u64) as usize;
        a.read_exact(&mut x[..piece])?;
        b.read_exact(&mut y[..piece])?;
        if x[..piece] != y[..piece] {
            return Ok(false);
        }
        left -= piece as u64;
    }
    Ok(true)
}

/// The source that plays the same file from a project saved as `to`,
/// copying the file into its folder when it is not there already. `from`
/// is the project file `source` is relative to now, if any. `Ok(None)` when
/// the source needs no change: a `data:` URI or a link.
fn follow(source: &str, from: Option<&Path>, to: &Path) -> Result<Option<String>, String> {
    let Some(file) = asset_file(source, from) else {
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
            .join(file.file_name().unwrap_or(std::ffi::OsStr::new("file"))),
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
    /// The edits that point image and video layers and audio items at
    /// their copies.
    pub edits: Vec<Edit>,
    /// A message for each file that could not be copied. Its nodes keep
    /// their source.
    pub problems: Vec<String>,
}

/// Copies the images, videos and sound `document` uses into the folder of
/// `to`, the project file it is about to be saved as, and returns the
/// edits that point the nodes at the copies. `from` is the project file
/// the document was saved as until now, or `None` for a project never
/// saved.
///
/// Files already in that folder are only named relative to it. Embedded
/// images (`data:` URIs) stay as they are.
pub fn gather_files(document: &Document, from: Option<&Path>, to: &Path) -> Gathered {
    let mut gathered = Gathered::default();
    let mut followed: HashMap<&str, Option<String>> = HashMap::new();
    for node in document.nodes() {
        let Some(source) = node.data.source() else {
            continue;
        };
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

/// Makes the sources of the image and video layers and audio items among
/// `nodes`, copied from a project saved as `project_file`, absolute, so
/// that they can be found from anywhere: for the clipboard.
pub fn absolute_sources(nodes: &mut [Node], project_file: Option<&Path>) {
    for node in nodes {
        if let Some(source) = node.data.source_mut()
            && let Some(file) = asset_file(source, project_file)
        {
            *source = file.to_string_lossy().into_owned();
        }
    }
}

/// What is said when the file of a node of type `ty` is missing.
fn missing(ty: NodeType, file: &Path) -> String {
    let name = shown_name(file);
    match ty {
        NodeType::Video => format!("The video {name} is missing, so its layer shows nothing."),
        NodeType::Audio => format!("The sound {name} is missing, so it is not heard."),
        _ => format!("The image {name} is missing, so its layer shows nothing."),
    }
}

/// Makes the files of `nodes`, about to be pasted into a project saved as
/// `project_file`, part of it: an image, video or sound from elsewhere is
/// copied into its assets folder and its node points at the copy. A
/// project never saved has no folder, so its files keep their full paths.
/// Returns a message for each file that could not be copied, or that is
/// missing.
pub fn adopt_files(nodes: &mut [Node], project_file: Option<&Path>) -> Vec<String> {
    let mut problems = Vec::new();
    for node in nodes {
        let ty = node.node_type();
        let Some(source) = node.data.source_mut() else {
            continue;
        };
        let Some(file) = asset_file(source, None) else {
            continue;
        };
        if !file.exists() {
            problems.push(missing(ty, &file));
            continue;
        }
        let Some(project) = project_file else {
            *source = file.to_string_lossy().into_owned();
            continue;
        };
        match follow(&file.to_string_lossy(), None, project) {
            Ok(Some(next)) => *source = next,
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use moshionly_core::NodeId;

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
            "format": "moshionly", "version": 3, "name": "Test", "nodes": nodes
        });
        io::parse(&text.to_string()).unwrap()
    }

    fn sources(document: &Document, edits: &[Edit]) -> Vec<String> {
        let mut document = document.clone();
        document.apply(edits).unwrap();
        document
            .nodes()
            .filter_map(|node| node.data.source())
            .map(ToOwned::to_owned)
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
        let gathered = gather_files(&doc, Some(&a.join("p.moshi")), &b.join("q.moshi"));
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
        let again = gather_files(&doc, Some(&a.join("p.moshi")), &b.join("q.moshi"));
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
        let gathered = gather_files(&doc, None, &project);
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
        let gathered = gather_files(&doc, Some(&root.join("a/p.moshi")), &root.join("b/q.moshi"));
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
    fn pasted_images_join_the_project_they_are_pasted_into() {
        let root = scratch("paste");
        write(&root.join("a/assets/hero.png"), b"hero");
        let doc = document(&["assets/hero.png", "assets/gone.png"]);
        let mut nodes: Vec<Node> = ["i0", "i1"]
            .iter()
            .map(|id| doc.get(&NodeId::from(*id)).unwrap().clone())
            .collect();
        absolute_sources(&mut nodes, Some(&root.join("a/p.moshi")));
        let source = |nodes: &[Node], at: usize| nodes[at].data.source().unwrap().to_owned();
        assert!(Path::new(&source(&nodes, 0)).is_absolute());

        // Into a project never saved: full paths.
        let mut unsaved = nodes.clone();
        let problems = adopt_files(&mut unsaved, None);
        assert_eq!(problems.len(), 1, "the missing image is reported");
        assert_eq!(source(&unsaved, 0), source(&nodes, 0));

        // Into a saved project elsewhere: copied into its assets.
        let mut saved = nodes.clone();
        let problems = adopt_files(&mut saved, Some(&root.join("b/q.moshi")));
        assert_eq!(problems.len(), 1);
        assert_eq!(source(&saved, 0), "assets/hero.png");
        assert!(root.join("b/assets/hero.png").exists());

        // Back into the project it came from: named as before, no copy.
        let mut home = nodes.clone();
        adopt_files(&mut home, Some(&root.join("a/p.moshi")));
        assert_eq!(source(&home, 0), "assets/hero.png");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn video_and_sound_go_with_the_project_too() {
        let root = scratch("media");
        write(&root.join("a/assets/beach.mp4"), b"video");
        write(&root.join("a/assets/song.mp3"), b"sound");
        let doc = io::parse(
            &serde_json::json!({
                "format": "moshionly", "version": 3, "name": "Media", "nodes": [
                    {"id": "ab", "parent": null, "index": "a0", "type": "artboard"},
                    {"id": "film", "parent": "ab", "index": "a0", "type": "video",
                     "source": "assets/beach.mp4", "start": 0, "end": 1000},
                    {"id": "song", "parent": "ab", "index": "a0", "type": "audio",
                     "source": "assets/song.mp3", "start": 0, "end": 1000},
                    {"id": "gone", "parent": "ab", "index": "a1", "type": "audio",
                     "source": "assets/gone.wav", "start": 0, "end": 1000}
                ]
            })
            .to_string(),
        )
        .unwrap();
        // Saved elsewhere: copied along.
        let gathered = gather_files(&doc, Some(&root.join("a/p.moshi")), &root.join("b/q.moshi"));
        assert_eq!(gathered.problems.len(), 1, "the missing sound is said");
        assert_eq!(
            std::fs::read(root.join("b/assets/beach.mp4")).unwrap(),
            b"video"
        );
        assert_eq!(
            std::fs::read(root.join("b/assets/song.mp3")).unwrap(),
            b"sound"
        );
        // Copied and pasted into another project: copied into it.
        let mut nodes: Vec<Node> = ["film", "song", "gone"]
            .iter()
            .map(|id| doc.get(&NodeId::from(*id)).unwrap().clone())
            .collect();
        absolute_sources(&mut nodes, Some(&root.join("a/p.moshi")));
        assert!(Path::new(nodes[0].data.source().unwrap()).is_absolute());
        let problems = adopt_files(&mut nodes, Some(&root.join("c/r.moshi")));
        assert_eq!(nodes[0].data.source(), Some("assets/beach.mp4"));
        assert_eq!(nodes[1].data.source(), Some("assets/song.mp3"));
        assert_eq!(
            problems,
            ["The sound gone.wav is missing, so it is not heard."]
        );
        assert!(root.join("c/assets/song.mp3").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copies_are_compared_by_content() {
        let root = scratch("compare");
        let file = root.join("a.bin");
        write(&file, &[7u8; 200_000]);
        let mut other = vec![7u8; 200_000];
        write(&root.join("same.bin"), &other);
        assert!(same_content(&file, &root.join("same.bin"), 200_000).unwrap());
        other[150_000] = 8;
        write(&root.join("near.bin"), &other);
        assert!(!same_content(&file, &root.join("near.bin"), 200_000).unwrap());
        write(&root.join("short.bin"), &[7u8; 10]);
        assert!(!same_content(&file, &root.join("short.bin"), 200_000).unwrap());
        assert_eq!(
            same_content(&file, &root.join("none.bin"), 200_000)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        // A taken name with other content: the copy goes beside it.
        let copy = copy_to(&file, &root.join("near.bin")).unwrap();
        assert_eq!(copy, root.join("near 2.bin"));
        assert_eq!(
            copy_to(&file, &root.join("same.bin")).unwrap(),
            root.join("same.bin")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn data_uris_are_seen_from_their_start() {
        assert!(is_data_uri("data:image/png;base64,AAAA"));
        assert!(is_data_uri("  DATA:image/png,x"));
        assert!(!is_data_uri("assets/data.png"));
        assert!(!is_data_uri("dat"));
        assert_eq!(asset_file("data:image/png;base64,AAAA", None), None);
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
