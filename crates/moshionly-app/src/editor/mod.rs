//! The editor's state: the open document and everything about it that is
//! not the document.
//!
//! The document lives in a [`SessionHandle`], shared with the MCP server and
//! the chat, which may change it from other threads. [`Editor`] keeps a
//! snapshot of it for the frame being drawn and the state of the window
//! around it: the selection, the layer under the pointer, the current
//! artboard, the playhead and the active tool. None of that is saved in the
//! file.
//!
//! The rules are kept apart from the window, in modules that can be tested
//! on their own:
//!
//! - [`assets`]: the image files a project uses, kept with its file.
//! - [`selection`]: what a click selects, groups and locks included.
//! - [`view`]: where things are on the canvas, hit testing and marquees.
//! - [`transform`]: moving, resizing and rotating, from a pointer to edits.
//! - [`drag`]: a drag of the selection as one undo step.
//! - [`create`], [`pen`] and [`text`]: making layers with the tools, and
//!   typing text on the canvas.
//! - [`clipboard`]: copying and pasting layers as Moshionly's JSON.
//! - [`snap`]: snapping to edges and centres, with guides.
//! - [`tree`]: the rows of the layer list.
//! - [`timeline`]: the timeline's rows, from an artboard's animations.
//! - [`describe`]: the names nodes are shown with.
//! - [`inspect`]: reading properties across a selection, mixed or not.

pub mod assets;
pub mod clipboard;
pub mod create;
pub mod describe;
pub mod drag;
pub mod inspect;
pub mod pen;
pub mod selection;
pub mod snap;
pub mod text;
pub mod timeline;
pub mod transform;
pub mod tree;
pub mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use moshionly_agent::{Origin, SessionHandle, Step, Subscription};
use moshionly_core::{Document, Edit, EditError, NodeId, Scene, evaluate, evaluate_at_rest};
use moshionly_render::Renderer;

pub use self::selection::{Pick, Selection};
use crate::shell::Tool;

/// Wakes the window when the document changes from elsewhere.
#[derive(Default)]
struct Notifier {
    /// Set by every change, cleared when the editor has caught up.
    changed: AtomicBool,
    /// The window to repaint, once there is one.
    ctx: Mutex<Option<egui::Context>>,
}

impl Notifier {
    fn notify(&self) {
        self.changed.store(true, Ordering::Release);
        let ctx = self.ctx.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ctx) = ctx.as_ref() {
            ctx.request_repaint();
        }
    }
}

/// A scene kept for hit testing and selection boxes, with what it was
/// evaluated for.
struct CachedScene {
    version: u64,
    time: Option<f64>,
    scene: Option<Arc<Scene>>,
}

/// The open document and the editor's state around it.
pub struct Editor {
    session: SessionHandle,
    notifier: Arc<Notifier>,
    _subscription: Subscription,
    /// The document as of the last [`sync`](Self::sync).
    document: Arc<Document>,
    /// The session's revision that `document` is.
    revision: u64,
    /// What is selected.
    pub selection: Selection,
    /// The layer the pointer is over, as a click would pick it.
    pub hover: Option<NodeId>,
    /// The artboard being worked on: the timeline shows it and new layers
    /// go into it.
    artboard: Option<NodeId>,
    /// The current time, in milliseconds.
    playhead: f64,
    /// Whether the canvas shows the playhead's moment (the Animate tab)
    /// rather than the design at rest (the Design tab).
    animating: bool,
    /// The active tool.
    pub tool: Tool,
    /// Draws, hit-tests and measures. Shared with the canvas workers.
    renderer: Renderer,
    scenes: HashMap<NodeId, CachedScene>,
    /// For each artboard, the revision at which its content last changed.
    versions: HashMap<NodeId, u64>,
    /// The project file images are found next to.
    project: Option<PathBuf>,
    /// The project file changed since [`take_project_change`](Self::take_project_change)
    /// was last asked.
    project_changed: bool,
}

impl Editor {
    /// An editor on `session`, measuring and hit-testing with `renderer`.
    pub fn new(session: SessionHandle, renderer: Renderer) -> Self {
        let notifier = Arc::new(Notifier::default());
        let listener = Arc::clone(&notifier);
        let subscription = session.subscribe(move |_| listener.notify());
        let (document, revision) = session.read(|s| (s.snapshot(), s.revision()));
        let project = session.path();
        renderer.assets().set_project_file(project.clone());
        let mut editor = Self {
            session,
            notifier,
            _subscription: subscription,
            document,
            revision,
            selection: Selection::new(),
            hover: None,
            artboard: None,
            playhead: 0.0,
            animating: false,
            tool: Tool::default(),
            renderer,
            scenes: HashMap::new(),
            versions: HashMap::new(),
            project,
            project_changed: false,
        };
        editor.artboard = editor.document.artboards().first().cloned();
        editor
    }

    /// The shared session the document lives in.
    pub fn session(&self) -> &SessionHandle {
        &self.session
    }

    /// The renderer used for hit testing and measuring text.
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Tells the editor which window to repaint when the document changes
    /// from another thread. Cheap; call it every frame.
    pub fn attach(&self, ctx: &egui::Context) {
        let mut slot = self
            .notifier
            .ctx
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(ctx.clone());
        }
    }

    /// The document as of the last [`sync`](Self::sync).
    pub fn document(&self) -> &Arc<Document> {
        &self.document
    }

    /// The session's revision the document snapshot is at.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The revision at which the content of `artboard` last changed: what
    /// its picture depends on.
    pub fn artboard_version(&self, artboard: &NodeId) -> u64 {
        self.versions
            .get(artboard)
            .copied()
            .unwrap_or(self.revision)
    }

    /// Catches up with the session: takes the newest document and lets go
    /// of selected and hovered nodes that no longer exist. Returns whether
    /// the document, or the file it is saved as, changed.
    pub fn sync(&mut self) -> bool {
        self.notifier.changed.store(false, Ordering::Release);
        let (revision, document, path) = self
            .session
            .read(|s| (s.revision(), s.snapshot(), s.path().map(ToOwned::to_owned)));
        // Saving as another file changes where relative image paths are
        // found without changing the document.
        let moved = self.project != path;
        if moved {
            self.renderer.assets().set_project_file(path.clone());
            self.project = path;
            self.project_changed = true;
        }
        if revision == self.revision {
            return moved;
        }
        let previous = std::mem::replace(&mut self.document, document);
        self.revision = revision;
        // Artboards whose content did not change keep their version, so
        // their pictures are not drawn again.
        let mut versions = HashMap::new();
        for artboard in self.document.artboards() {
            let version = match self.versions.get(artboard) {
                Some(version) if same_artboard(&previous, &self.document, artboard) => *version,
                _ => revision,
            };
            versions.insert(artboard.clone(), version);
        }
        self.versions = versions;
        self.selection.retain_existing(&self.document);
        if self
            .hover
            .as_ref()
            .is_some_and(|id| !self.document.contains(id))
        {
            self.hover = None;
        }
        if self
            .artboard
            .as_ref()
            .is_none_or(|id| !self.document.contains(id))
        {
            self.artboard = self.document.artboards().first().cloned();
        }
        self.scenes
            .retain(|artboard, _| self.document.contains(artboard));
        true
    }

    /// The project file the document is saved as, which relative image
    /// paths are found next to. `None` for a project never saved.
    pub fn project_file(&self) -> Option<&std::path::Path> {
        self.project.as_deref()
    }

    /// Whether the project file changed (opened, saved as another file)
    /// since the last time this was asked: pictures with images in them
    /// need drawing again.
    pub fn take_project_change(&mut self) -> bool {
        std::mem::take(&mut self.project_changed)
    }

    /// Whether something changed the document since the last sync, for
    /// example an agent on another thread.
    pub fn is_behind(&self) -> bool {
        self.notifier.changed.load(Ordering::Acquire)
    }

    /// Applies a batch of edits as one undo step made by the person, and
    /// catches up. Returns whether anything changed.
    pub fn apply(&mut self, summary: &str, edits: &[Edit]) -> Result<bool, EditError> {
        if edits.is_empty() {
            return Ok(false);
        }
        let changed = self.session.apply(Origin::User, summary, edits)?;
        self.sync();
        Ok(changed)
    }

    /// Opens a gesture: everything applied until
    /// [`commit_gesture`](Self::commit_gesture) is one undo step.
    pub fn begin_gesture(&mut self, summary: &str) {
        self.session
            .write(|s| s.begin_gesture(Origin::User, summary));
    }

    /// Whether a gesture is open.
    pub fn in_gesture(&self) -> bool {
        self.session.read(|s| s.in_gesture())
    }

    /// Closes the open gesture as one undo step.
    pub fn commit_gesture(&mut self) {
        self.session.write(|s| s.commit_gesture());
        self.sync();
    }

    /// Closes the open gesture and puts the document back as it was when
    /// the gesture began.
    pub fn cancel_gesture(&mut self) -> Result<(), EditError> {
        let result = self.session.write(|s| s.cancel_gesture());
        self.sync();
        result
    }

    /// Undoes the most recent step. Returns it, or `None` when there was
    /// nothing to undo.
    pub fn undo(&mut self) -> Result<Option<Step>, EditError> {
        let step = self.session.undo();
        self.sync();
        step
    }

    /// Redoes the most recently undone step.
    pub fn redo(&mut self) -> Result<Option<Step>, EditError> {
        let step = self.session.redo();
        self.sync();
        step
    }

    /// The menu label for undo: "Undo Move", or plain "Undo" when the step
    /// has no summary. `None` when there is nothing to undo.
    pub fn undo_label(&self) -> Option<String> {
        self.session.read(|s| {
            s.can_undo()
                .then(|| step_label("Undo", s.undo_steps().last()))
        })
    }

    /// The menu label for redo, like [`undo_label`](Self::undo_label).
    pub fn redo_label(&self) -> Option<String> {
        self.session.read(|s| {
            s.can_redo()
                .then(|| step_label("Redo", s.redo_steps().last()))
        })
    }

    /// Whether the document differs from its file.
    pub fn has_unsaved_changes(&self) -> bool {
        self.session.read(|s| s.has_unsaved_changes())
    }

    /// The artboard being worked on.
    pub fn artboard(&self) -> Option<&NodeId> {
        self.artboard.as_ref()
    }

    /// Makes `artboard` the one being worked on.
    pub fn set_artboard(&mut self, artboard: Option<NodeId>) {
        if artboard
            .as_ref()
            .is_none_or(|id| self.document.contains(id))
        {
            self.artboard = artboard;
        }
    }

    /// Makes the artboard of the selection the one being worked on.
    pub fn follow_selection(&mut self) {
        let artboard = self
            .selection
            .ids()
            .first()
            .and_then(|id| self.document.artboard_of(id))
            .map(|node| node.id.clone());
        if artboard.is_some() {
            self.artboard = artboard;
        }
    }

    /// The current time, in milliseconds.
    pub fn playhead(&self) -> f64 {
        self.playhead
    }

    /// Moves the playhead. Negative and undefined times count as 0.
    pub fn set_playhead(&mut self, time: f64) {
        self.playhead = if time.is_finite() { time.max(0.0) } else { 0.0 };
    }

    /// Whether the canvas shows the playhead's moment rather than the
    /// design at rest.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Shows the playhead's moment (true) or the design at rest (false).
    pub fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    /// The time the canvas shows: `None` for the design at rest.
    pub fn view_time(&self) -> Option<f64> {
        self.animating.then_some(self.playhead)
    }

    /// The scene of `artboard` as the canvas shows it now, evaluated once
    /// per revision and time. `None` when the artboard cannot be evaluated.
    pub fn scene(&mut self, artboard: &NodeId) -> Option<Arc<Scene>> {
        let time = self.view_time();
        let version = self.artboard_version(artboard);
        if let Some(cached) = self.scenes.get(artboard)
            && cached.version == version
            && cached.time == time
        {
            return cached.scene.clone();
        }
        let fonts = self.renderer.fonts();
        let scene = match time {
            Some(time) => evaluate(&self.document, artboard, time, fonts),
            None => evaluate_at_rest(&self.document, artboard, fonts),
        }
        .ok()
        .map(Arc::new);
        self.scenes.insert(
            artboard.clone(),
            CachedScene {
                version,
                time,
                scene: scene.clone(),
            },
        );
        scene
    }
}

/// Whether an artboard and everything in it are the same in two documents.
fn same_artboard(a: &Document, b: &Document, artboard: &NodeId) -> bool {
    use moshionly_core::Family;
    if a.get(artboard) != b.get(artboard) {
        return false;
    }
    [Family::Layer, Family::Animation]
        .into_iter()
        .all(|family| {
            let mut left = a.walk(artboard, family);
            let mut right = b.walk(artboard, family);
            loop {
                match (left.next(), right.next()) {
                    (None, None) => return true,
                    (Some((_, x)), Some((_, y))) if x == y => {}
                    _ => return false,
                }
            }
        })
}

/// "Undo Move" from "Undo" and a step summarised as "Move".
fn step_label(verb: &str, step: Option<&Step>) -> String {
    match step.map(|step| step.summary.trim()) {
        Some(summary) if !summary.is_empty() => format!("{verb} {summary}"),
        _ => verb.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use moshionly_agent::Session;
    use moshionly_core::build;
    use moshionly_render::{AssetLoader, FontLibrary};

    use super::*;

    fn editor() -> Editor {
        let mut document = Document::new("Test");
        let (_, edits) = build::add_artboard(&document, "Scene 1", 1080.0, 1080.0).unwrap();
        document.apply(&edits).unwrap();
        let session = SessionHandle::new(Session::new(document));
        Editor::new(
            session,
            Renderer::new(FontLibrary::bundled(), AssetLoader::default()),
        )
    }

    #[test]
    fn edits_from_another_thread_are_picked_up() {
        let mut editor = editor();
        let artboard = editor.artboard().cloned().unwrap();
        let rect = build::default_layer(moshionly_core::NodeType::Rect).unwrap();
        let (id, edits) = build::add_layer(editor.document(), &artboard, rect).unwrap();
        editor.apply("Add rectangle", &edits).unwrap();
        editor.selection.select(id.clone());
        assert!(!editor.is_behind());

        let session = editor.session().clone();
        let remover = id.clone();
        std::thread::spawn(move || {
            session
                .apply(Origin::Agent, "Remove", &[Edit::remove(remover)])
                .unwrap();
        })
        .join()
        .unwrap();
        assert!(editor.is_behind());
        assert!(editor.document().contains(&id), "not synced yet");
        assert!(editor.sync());
        assert!(!editor.document().contains(&id));
        assert!(editor.selection.is_empty(), "the removed layer is let go");
        assert!(!editor.sync(), "nothing new");
    }

    #[test]
    fn undo_is_labelled_with_the_step() {
        let mut editor = editor();
        assert_eq!(editor.undo_label(), None);
        let artboard = editor.artboard().cloned().unwrap();
        editor
            .apply("Rename", &[Edit::set(artboard.clone(), "name", "Intro")])
            .unwrap();
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Rename"));
        assert_eq!(editor.redo_label(), None);
        editor.undo().unwrap();
        assert_eq!(editor.redo_label().as_deref(), Some("Redo Rename"));
        assert_eq!(editor.document().get(&artboard).unwrap().name(), "Scene 1");
    }

    #[test]
    fn a_gesture_is_one_undo_step() {
        let mut editor = editor();
        let artboard = editor.artboard().cloned().unwrap();
        editor.begin_gesture("Move");
        for x in [10.0, 20.0, 30.0] {
            editor
                .apply("Move", &[Edit::set(artboard.clone(), "x", x)])
                .unwrap();
        }
        editor.commit_gesture();
        assert!(!editor.in_gesture());
        let x = |editor: &Editor| editor.document().get(&artboard).unwrap().layer().unwrap().x;
        assert_eq!(x(&editor), 30.0);
        editor.undo().unwrap();
        assert_eq!(x(&editor), 0.0);
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn artboards_keep_their_version_while_others_change() {
        let mut editor = editor();
        let first = editor.artboard().cloned().unwrap();
        let (second, edits) =
            build::add_artboard(editor.document(), "Scene 2", 500.0, 500.0).unwrap();
        editor.apply("Add artboard", &edits).unwrap();
        let before = editor.artboard_version(&first);
        editor
            .apply("Rename", &[Edit::set(second.clone(), "name", "Outro")])
            .unwrap();
        assert_eq!(editor.artboard_version(&first), before);
        assert_eq!(editor.artboard_version(&second), editor.revision());
        let rect = build::default_layer(moshionly_core::NodeType::Rect).unwrap();
        let (_, edits) = build::add_layer(editor.document(), &first, rect).unwrap();
        editor.apply("Add rectangle", &edits).unwrap();
        assert_eq!(editor.artboard_version(&first), editor.revision());
        assert!(editor.artboard_version(&second) < editor.revision());
    }

    #[test]
    fn saving_as_another_file_is_noticed_without_an_edit() {
        let mut editor = editor();
        assert!(!editor.take_project_change());
        let revision = editor.revision();
        let file = std::path::absolute("elsewhere/teaser.moshi").unwrap();
        editor.session().write(|s| s.set_path(Some(file.clone())));
        assert_eq!(editor.session().revision(), revision, "not an edit");
        assert!(editor.sync());
        assert_eq!(editor.project_file(), Some(file.as_path()));
        assert!(editor.take_project_change());
        assert!(!editor.take_project_change(), "told once");
        assert!(!editor.sync(), "nothing new");
    }

    #[test]
    fn the_canvas_shows_the_design_at_rest_until_animating() {
        let mut editor = editor();
        editor.set_playhead(1200.0);
        assert_eq!(editor.view_time(), None);
        editor.set_animating(true);
        assert_eq!(editor.view_time(), Some(1200.0));
        editor.set_playhead(-5.0);
        assert_eq!(editor.playhead(), 0.0);
        let artboard = editor.artboard().cloned().unwrap();
        let scene = editor.scene(&artboard).unwrap();
        assert_eq!(scene.width, 1080.0);
        // Cached until the document or the time changes.
        assert!(Arc::ptr_eq(&scene, &editor.scene(&artboard).unwrap()));
    }
}
