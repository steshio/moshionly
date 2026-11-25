//! Bringing video and sound files into a project.
//!
//! Each file is copied into the project's assets folder by the agent's
//! `import_media` tool, exactly as images go through `import_image`, which
//! also says what the file holds (read with `ffmpeg` through a
//! [`MediaProbe`]). A file with a picture becomes a video layer at its own
//! size, made smaller when it would not fit the artboard, placed where it
//! was dropped or in the middle of the artboard. A file with only sound
//! becomes an audio item on the artboard's timeline. Both start at the
//! playhead (at 0 in the Design tab, which shows no time) and last as long
//! as the file; when that runs past the end of the scene, the scene grows
//! to fit. All of it is one undo step.

use std::path::Path;
use std::sync::Arc;

use egui::{Pos2, Rect, Vec2, vec2};
use moshionly_agent::{Host, MediaProbe, Tools};
use moshionly_core::eval::duration_of;
use moshionly_core::model::{Audio, ClipTiming, NodeData};
use moshionly_core::{Document, Edit, Node, NodeId, NodeType, build};
use serde_json::{Value, json};

use super::Editor;
use super::create::{CreateError, first_sentence, fitted};
use super::inspect::tidy;

/// How long the clip of a file that does not say how long it is lasts, in
/// milliseconds; its end can be dragged on the timeline.
pub const UNKNOWN_LENGTH: f64 = 4000.0;

/// A video or sound file in the project, as `import_media` describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaFile {
    /// The source to give a video layer or an audio item.
    pub source: String,
    /// The file's name without its extension, for a video layer.
    pub name: String,
    /// How long the file plays, in milliseconds, when it says.
    pub duration_ms: Option<f64>,
    /// Width of the picture in pixels; 0 without one.
    pub width: f64,
    /// Height of the picture in pixels; 0 without one.
    pub height: f64,
    /// Whether the file has a picture.
    pub has_video: bool,
    /// Whether the file has sound.
    pub has_sound: bool,
}

impl MediaFile {
    /// How long its clip lasts: the file's length, or [`UNKNOWN_LENGTH`].
    pub fn length(&self) -> f64 {
        self.duration_ms
            .filter(|ms| ms.is_finite() && *ms > 0.0)
            .map_or(UNKNOWN_LENGTH, |ms| ms.round().max(1.0))
    }
}

/// What importing media made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedMedia {
    /// The new video layers.
    pub videos: Vec<NodeId>,
    /// The new audio items.
    pub sounds: Vec<NodeId>,
    /// The scene's new length in milliseconds, when the clips made it
    /// longer.
    pub scene_length: Option<f64>,
    /// The names of files that do not say how long they are: their clips
    /// last [`UNKNOWN_LENGTH`].
    pub unknown_length: Vec<String>,
    /// A message for each file that could not be used.
    pub problems: Vec<String>,
}

impl ImportedMedia {
    /// The undo step's name: "Add video", "Add sounds" and so on.
    pub fn summary(&self) -> &'static str {
        match (self.videos.len(), self.sounds.len()) {
            (1, 0) => "Add video",
            (_, 0) => "Add videos",
            (0, 1) => "Add sound",
            (0, _) => "Add sounds",
            _ => "Add video and sound",
        }
    }
}

/// The edits that add `files` to `artboard` in `document`, starting at
/// `start` milliseconds: videos centred on `center` (in the artboard's
/// coordinates; its middle when `None`), fanned out a little so none
/// hides another, and sound on the artboard's timeline. The scene grows to
/// the end of the last clip when that is later than its end.
pub fn media_edits(
    document: &Document,
    artboard: &NodeId,
    files: &[MediaFile],
    center: Option<Vec2>,
    start: f64,
) -> Result<(Vec<Edit>, ImportedMedia), CreateError> {
    let board = document
        .get(artboard)
        .filter(|node| node.node_type() == NodeType::Artboard)
        .and_then(Node::layer)
        .ok_or(CreateError::NoArtboard)?;
    let room = vec2(board.width as f32, board.height as f32);
    let center = center.unwrap_or(room / 2.0);
    let start = start.max(0.0).round();
    // A scratch copy, so that each new node sees the ones before it.
    let mut working = document.clone();
    let mut edits = Vec::new();
    let mut made = ImportedMedia::default();
    let mut last_end = 0.0f64;
    for file in files {
        let clip = ClipTiming::new(start, start + file.length());
        if file.duration_ms.is_none_or(|ms| !(ms > 0.0)) {
            made.unknown_length.push(file.name.clone());
        }
        let (id, more) = if file.has_video {
            let size = fitted(vec2(file.width as f32, file.height as f32), room);
            let offset = vec2(24.0, 24.0) * made.videos.len() as f32;
            let rect = Rect::from_center_size(Pos2::ZERO + center + offset, size);
            let mut data =
                build::default_layer(NodeType::Video).ok_or(CreateError::NotADrawingTool)?;
            if let NodeData::Video(video) = &mut data {
                video.source.clone_from(&file.source);
                video.layer.name.clone_from(&file.name);
                video.layer.x = tidy(f64::from(rect.min.x));
                video.layer.y = tidy(f64::from(rect.min.y));
                video.layer.width = tidy(f64::from(rect.width()));
                video.layer.height = tidy(f64::from(rect.height()));
                video.clip = clip;
            }
            build::add_layer(&working, artboard, data)?
        } else {
            // The timeline shows the file's name until the sound is named.
            let audio = Audio {
                name: String::new(),
                source: file.source.clone(),
                clip,
            };
            build::add_audio(&working, artboard, audio)?
        };
        working.apply(&more)?;
        edits.extend(more);
        if file.has_video {
            made.videos.push(id);
        } else {
            made.sounds.push(id);
        }
        last_end = last_end.max(clip.end);
    }
    let length = duration_of(&working, artboard).unwrap_or(0.0);
    if last_end > length {
        let grown = last_end.ceil();
        edits.push(Edit::set(artboard.clone(), "duration", grown));
        made.scene_length = Some(grown);
    }
    Ok((edits, made))
}

impl Editor {
    /// Brings video and sound files into the project, read with `probe`,
    /// and adds them to the artboard under `at` (or the current one) as
    /// [described above](self), as one undo step. Videos become the
    /// selection; sound alone becomes the selected actions.
    pub fn import_media(
        &mut self,
        paths: &[&Path],
        at: Option<Pos2>,
        probe: Arc<dyn MediaProbe>,
    ) -> ImportedMedia {
        let artboard = at
            .and_then(|at| self.artboard_at(at))
            .or_else(|| self.artboard.clone());
        let Some(artboard) = artboard else {
            return ImportedMedia {
                problems: vec![CreateError::NoArtboard.to_string()],
                ..ImportedMedia::default()
            };
        };
        let center = at
            .zip(self.artboard_rect(&artboard))
            .map(|(at, board)| at - board.min);
        let tools = Tools::new(self.session.clone(), Host::new().with_media(probe));
        let mut files = Vec::new();
        let mut problems = Vec::new();
        for path in paths {
            match read_media(&tools, path) {
                Ok(file) => files.push(file),
                Err(problem) => problems.push(problem),
            }
        }
        let start = self.view_time().unwrap_or(0.0);
        let mut made = if files.is_empty() {
            ImportedMedia::default()
        } else {
            match media_edits(&self.document, &artboard, &files, center, start) {
                Ok((edits, made)) => match self.apply(made.summary(), &edits) {
                    Ok(_) => {
                        if made.videos.is_empty() {
                            self.selection.clear();
                            self.set_actions(made.sounds.clone());
                        } else {
                            self.selection.set(made.videos.clone());
                            self.set_actions([]);
                        }
                        made
                    }
                    Err(error) => {
                        problems.push(error.problem.message);
                        ImportedMedia::default()
                    }
                },
                Err(error) => {
                    problems.push(error.to_string());
                    ImportedMedia::default()
                }
            }
        };
        made.problems = problems;
        made
    }

    /// Plays the file at `path`, read with `probe`, in the video layers
    /// and audio items `ids` instead, which keep their boxes and timing.
    /// One undo step. A video layer needs a file with a picture, and an
    /// audio item one with sound.
    pub fn replace_media(
        &mut self,
        ids: &[NodeId],
        path: &Path,
        probe: Arc<dyn MediaProbe>,
    ) -> Result<(), String> {
        let tools = Tools::new(self.session.clone(), Host::new().with_media(probe));
        let file = read_media(&tools, path)?;
        let shown = file_name(path);
        let of = |ty: NodeType| {
            ids.iter()
                .filter(|id| self.document.get(id).is_some_and(|n| n.node_type() == ty))
                .count()
        };
        let (videos, sounds) = (of(NodeType::Video), of(NodeType::Audio));
        if videos > 0 && !file.has_video {
            return Err(format!(
                "{shown} has no picture, so a video layer cannot show it."
            ));
        }
        if sounds > 0 && !file.has_sound {
            return Err(format!("{shown} has no sound to play."));
        }
        let edits: Vec<Edit> = ids
            .iter()
            .filter(|id| {
                self.document.get(id).is_some_and(|node| {
                    matches!(node.node_type(), NodeType::Video | NodeType::Audio)
                })
            })
            .map(|id| Edit::set(id.clone(), "source", file.source.clone()))
            .collect();
        let summary = if videos > 0 {
            "Replace video"
        } else {
            "Replace sound"
        };
        self.apply(summary, &edits)
            .map(drop)
            .map_err(|error| error.problem.message)
    }
}

/// A file's name for messages.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Copies the file at `path` into the project with the `import_media`
/// tool, and says what it holds; or why it cannot be used.
fn read_media(tools: &Tools, path: &Path) -> Result<MediaFile, String> {
    let shown = file_name(path);
    let output = tools.call("import_media", json!({ "source": path.to_string_lossy() }));
    let text = output.joined_text();
    if output.is_error {
        return Err(format!("Could not use {shown}: {}", first_sentence(&text)));
    }
    let result: Value =
        serde_json::from_str(&text).map_err(|_| format!("Could not use {shown}."))?;
    let source = result["source"]
        .as_str()
        .ok_or_else(|| format!("Could not use {shown}."))?;
    let name = path.file_stem().map_or_else(
        || "Video".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    Ok(MediaFile {
        source: source.to_owned(),
        name,
        duration_ms: result["duration_ms"].as_f64(),
        width: result["width"].as_f64().unwrap_or(0.0),
        height: result["height"].as_f64().unwrap_or(0.0),
        has_video: result["has_video"].as_bool().unwrap_or(false),
        has_sound: result["has_sound"].as_bool().unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use moshionly_agent::{HostError, MediaInfo, SessionHandle};
    use moshionly_core::io;
    use moshionly_render::{AssetLoader, FontLibrary, Renderer};

    use super::*;

    /// Reads files by their extension: `.mp4` is a 1920 by 1080 video of 6
    /// seconds with sound, `.mp3` 3 seconds of sound, `.webm` a video that
    /// does not say how long it is, anything else is not media.
    struct FakeProbe;

    impl MediaProbe for FakeProbe {
        fn probe(&self, path: &Path) -> Result<MediaInfo, HostError> {
            let info = |duration_ms, width, height, has_video| MediaInfo {
                duration_ms,
                width,
                height,
                has_video,
                has_sound: true,
            };
            match path.extension().and_then(|e| e.to_str()) {
                Some("mp4") => Ok(info(Some(6000.0), 1920, 1080, true)),
                Some("mp3") => Ok(info(Some(3000.0), 0, 0, false)),
                Some("webm") => Ok(info(None, 640, 360, true)),
                _ => Err(HostError::Failed("not a media file".into())),
            }
        }
    }

    fn editor() -> Editor {
        let document = io::parse(
            r##"{"format":"moshionly","version":3,"name":"Media","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","x":0,"y":0,
             "width":1080,"height":1080,"duration":4000}
            ]}"##,
        )
        .unwrap();
        let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
        Editor::new(SessionHandle::from_document(document), renderer)
    }

    /// Empty files with these names in a folder of their own.
    fn files(test: &str, names: &[&str]) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
        let folder =
            std::env::temp_dir().join(format!("moshionly-app-media-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let paths = names
            .iter()
            .map(|name| {
                let path = folder.join(name);
                std::fs::write(&path, b"not really media").unwrap();
                path
            })
            .collect();
        (folder, paths)
    }

    fn file(name: &str, duration_ms: Option<f64>, size: (f64, f64), has_video: bool) -> MediaFile {
        MediaFile {
            source: format!("assets/{name}"),
            name: name.to_owned(),
            duration_ms,
            width: size.0,
            height: size.1,
            has_video,
            has_sound: true,
        }
    }

    #[test]
    fn media_becomes_videos_and_sound_and_the_scene_grows() {
        let editor = editor();
        let document = editor.document();
        let ab = NodeId::from("ab");
        let files = [
            file("beach", Some(6000.0), (1920.0, 1080.0), true),
            file("song", Some(3000.0), (0.0, 0.0), false),
            file("clip", None, (200.0, 100.0), true),
        ];
        let (edits, made) = media_edits(document, &ab, &files, None, 500.0).unwrap();
        assert_eq!((made.videos.len(), made.sounds.len()), (2, 1));
        assert_eq!(made.scene_length, Some(6500.0));
        assert_eq!(made.unknown_length, ["clip"]);
        assert_eq!(made.summary(), "Add video and sound");
        let mut after = document.as_ref().clone();
        after.apply(&edits).unwrap();
        // The large video is made to fit the artboard, in its middle.
        let beach = after.get(&made.videos[0]).unwrap();
        let layer = beach.layer().unwrap();
        assert_eq!(beach.name(), "beach");
        assert_eq!((layer.width, layer.height), (1080.0, 607.5));
        assert_eq!((layer.x, layer.y), (0.0, 236.25));
        let clip = beach.data.clip().unwrap();
        assert_eq!(
            (clip.start, clip.end, clip.trim_start),
            (500.0, 6500.0, 0.0)
        );
        // The small one keeps its size, a little further on.
        let small = after.get(&made.videos[1]).unwrap().layer().unwrap();
        assert_eq!((small.width, small.height), (200.0, 100.0));
        assert_eq!((small.x, small.y), (464.0, 514.0));
        assert_eq!(
            after.get(&made.videos[1]).unwrap().data.clip().unwrap().end,
            500.0 + UNKNOWN_LENGTH
        );
        // Sound goes on the timeline, from the playhead for its length.
        let song = after.get(&made.sounds[0]).unwrap();
        assert_eq!(song.node_type(), NodeType::Audio);
        assert_eq!(song.parent.as_ref(), Some(&ab));
        assert_eq!(song.data.span(), Some((500.0, 3500.0)));
        assert_eq!(duration_of(&after, &ab), Some(6500.0));
        // A clip within the scene leaves its length alone.
        let (edits, made) = media_edits(document, &ab, &files[1..2], None, 0.0).unwrap();
        assert_eq!(made.scene_length, None);
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            media_edits(document, &NodeId::from("nope"), &files, None, 0.0),
            Err(CreateError::NoArtboard)
        ));
    }

    #[test]
    fn importing_is_one_undo_step() {
        let mut editor = editor();
        let (folder, paths) = files("import", &["beach.mp4", "song.mp3", "notes.txt"]);
        let paths: Vec<&Path> = paths.iter().map(|path| path.as_path()).collect();
        let made = editor.import_media(&paths, Some(Pos2::new(300.0, 400.0)), Arc::new(FakeProbe));
        assert_eq!(made.problems.len(), 1, "{:?}", made.problems);
        assert!(
            made.problems[0].contains("notes.txt"),
            "{:?}",
            made.problems
        );
        assert_eq!((made.videos.len(), made.sounds.len()), (1, 1));
        assert_eq!(made.scene_length, Some(6000.0));
        let document = std::sync::Arc::clone(editor.document());
        // Not saved yet: the files are used where they are.
        let video = document.get(&made.videos[0]).unwrap();
        assert_eq!(
            video.data.source(),
            Some(folder.join("beach.mp4").to_string_lossy().as_ref())
        );
        // Placed where it was dropped.
        let layer = video.layer().unwrap();
        assert_eq!(
            (layer.x + layer.width / 2.0, layer.y + layer.height / 2.0),
            (300.0, 400.0)
        );
        assert_eq!(editor.selection.ids(), made.videos.as_slice());
        assert_eq!(
            editor.undo_label().as_deref(),
            Some("Undo Add video and sound")
        );
        editor.undo().unwrap();
        let ab = NodeId::from("ab");
        assert_eq!(duration_of(editor.document(), &ab), Some(4000.0));
        assert!(!editor.document().contains(&made.videos[0]));
        assert!(!editor.document().contains(&made.sounds[0]));
        assert_eq!(editor.undo_label(), None);

        // Sound alone, at the playhead of the Animate tab, is selected as
        // an action.
        editor.set_animating(true);
        editor.set_playhead(1500.0);
        let made = editor.import_media(&paths[1..2], None, Arc::new(FakeProbe));
        assert!(editor.selection.is_empty());
        assert_eq!(editor.actions(), made.sounds.as_slice());
        let song = editor.document().get(&made.sounds[0]).unwrap();
        assert_eq!(song.data.span(), Some((1500.0, 4500.0)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn replacing_needs_the_right_kind_of_file() {
        let mut editor = editor();
        let (folder, paths) = files("replace", &["beach.mp4", "song.mp3", "other.webm"]);
        let made = editor.import_media(
            &[paths[0].as_path(), paths[1].as_path()],
            None,
            Arc::new(FakeProbe),
        );
        let video = made.videos[0].clone();
        let song = made.sounds[0].clone();
        // A sound cannot be shown by a video layer.
        let refused =
            editor.replace_media(std::slice::from_ref(&video), &paths[1], Arc::new(FakeProbe));
        assert!(refused.unwrap_err().contains("no picture"));
        editor
            .replace_media(std::slice::from_ref(&video), &paths[2], Arc::new(FakeProbe))
            .unwrap();
        let source = |editor: &Editor, id: &NodeId| {
            editor
                .document()
                .get(id)
                .unwrap()
                .data
                .source()
                .unwrap()
                .to_owned()
        };
        assert!(source(&editor, &video).ends_with("other.webm"));
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Replace video"));
        // A video's sound can play in an audio item.
        editor
            .replace_media(std::slice::from_ref(&song), &paths[0], Arc::new(FakeProbe))
            .unwrap();
        assert!(source(&editor, &song).ends_with("beach.mp4"));
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Replace sound"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
