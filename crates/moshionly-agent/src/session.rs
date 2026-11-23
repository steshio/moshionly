//! The session: one open document, its undo history and who changed what.
//!
//! A [`Session`] is what the editor window, the built-in chat and the MCP
//! server all work on. It owns the [`Document`], the [`History`] that undoes
//! changes to it, the path of the project file and a revision counter.
//!
//! Every change goes through [`Session::apply`], which hands the edits to
//! `Document::apply` by way of the history. Each call becomes one undo step,
//! and the session remembers for every step who made it (the person, an
//! agent, or an import) and a few words saying what it was.
//!
//! A [`SessionHandle`] wraps a session so that several threads can share it:
//! the UI thread, the HTTP server and a chat running on a worker. It is cheap
//! to clone. Anyone can [`subscribe`](SessionHandle::subscribe) to be told
//! when the document changes, which is how the editor knows to repaint after
//! an agent's edit.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak, mpsc};

use moshionly_core::io::{self, LoadError, SaveError};
use moshionly_core::{Document, Edit, EditError, History};

/// Who made a change to the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The person using the editor.
    User,
    /// An AI agent, through the tools in this crate.
    Agent,
    /// A design brought in from another tool.
    Import,
}

impl Origin {
    /// The origin in one lower-case word: `user`, `agent` or `import`.
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::User => "user",
            Origin::Agent => "agent",
            Origin::Import => "import",
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One step of the undo history: who made it and what it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Who made the change.
    pub origin: Origin,
    /// A few words saying what changed, for menus and messages.
    pub summary: String,
}

impl Step {
    fn new(origin: Origin, summary: impl Into<String>) -> Self {
        Self {
            origin,
            summary: summary.into(),
        }
    }
}

/// What happened to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Edits were applied.
    Applied,
    /// A step was undone, or an unfinished gesture was cancelled.
    Undone,
    /// A step was redone.
    Redone,
    /// Another document took the place of the old one.
    Replaced,
    /// The project was given another name.
    Renamed,
}

/// A notice that the document of a session changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The revision of the session after the change.
    pub revision: u64,
    /// Who made the change. For an undo or redo, who made the step.
    pub origin: Origin,
    /// What happened.
    pub kind: ChangeKind,
}

/// Why a session could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum SessionSaveError {
    /// The project has never been saved, so there is no file to write to.
    #[error("the project has no file yet; choose where to save it first")]
    NoPath,
    /// Writing the file failed.
    #[error(transparent)]
    Save(#[from] SaveError),
}

/// One open document with its undo history, file path and revision counter.
///
/// See the [module documentation](self) for how it is meant to be used. A
/// session on its own is for one thread; wrap it in a [`SessionHandle`] to
/// share it.
#[derive(Debug)]
pub struct Session {
    document: Arc<Document>,
    history: History,
    path: Option<PathBuf>,
    revision: u64,
    saved_revision: Option<u64>,
    /// Who made each step that can be undone, oldest first. Always as long
    /// as the history's undo stack.
    undo: Vec<Step>,
    /// Who made each step that can be redone. Always as long as the
    /// history's redo stack.
    redo: Vec<Step>,
    /// The gesture that is open, with the document as it was when it began.
    gesture: Option<(Step, Arc<Document>)>,
    /// Changes that have not been reported to subscribers yet.
    pending: Vec<Change>,
}

/// `path` made absolute against the current folder, so that everything that
/// uses the project's folder (assets, exports, image sources) sees a real
/// one. A bare `demo.moshi` would otherwise have an empty folder, which
/// every path seems to be inside. The path stays as given when the current
/// folder cannot be found.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

impl Session {
    /// How many undo steps a session keeps.
    pub const UNDO_LIMIT: usize = History::DEFAULT_LIMIT;

    /// A session on a document that has no file yet.
    pub fn new(document: Document) -> Self {
        Self {
            document: Arc::new(document),
            history: History::with_limit(Self::UNDO_LIMIT),
            path: None,
            revision: 0,
            saved_revision: None,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
            pending: Vec::new(),
        }
    }

    /// A session on a document that was read from, or will be saved to,
    /// `path`. A relative path is taken from the current folder.
    pub fn with_path(document: Document, path: impl Into<PathBuf>) -> Self {
        let mut session = Self::new(document);
        session.path = Some(absolute(path.into()));
        session.saved_revision = Some(0);
        session
    }

    /// Opens a project file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        Ok(Self::with_path(io::load(path)?, path))
    }

    /// The document as it is now.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The document as it is now, to keep. Taking a snapshot copies nothing;
    /// the session copies the document only if it changes while a snapshot
    /// is still held.
    pub fn snapshot(&self) -> Arc<Document> {
        Arc::clone(&self.document)
    }

    /// The project file, when the project has one. The path is absolute
    /// whenever the current folder could be found when it was given.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Says where the project file is, or that there is none. A relative
    /// path is taken from the current folder.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        let path = path.map(absolute);
        if self.path != path {
            self.path = path;
            self.saved_revision = None;
        }
    }

    /// The name of the project file, or `Untitled.moshi` for a project that
    /// has not been saved.
    pub fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Untitled.{}", io::EXTENSION))
    }

    /// A number that grows every time the document changes: by edits, an
    /// undo, a redo, a rename, or another document taking its place.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the document differs from what is on disk.
    pub fn has_unsaved_changes(&self) -> bool {
        self.saved_revision != Some(self.revision)
    }

    /// Says that the document as it is now has nothing worth saving, as if
    /// it had just been saved: for a new document nobody has touched yet,
    /// which has no file. Any later change makes it unsaved again.
    pub fn mark_clean(&mut self) {
        self.saved_revision = Some(self.revision);
    }

    /// Applies a batch of edits as one undo step made by `origin`.
    ///
    /// The batch is applied completely or not at all. Returns whether it
    /// changed anything; a batch that changes nothing records no step.
    ///
    /// While a gesture by the same origin is open the batch becomes part of
    /// that gesture's step. A batch by someone else (an agent editing while
    /// the person drags) is still a step of its own: the gesture so far is
    /// recorded first, and what follows of it becomes a further step.
    pub fn apply(
        &mut self,
        origin: Origin,
        summary: impl Into<String>,
        edits: &[Edit],
    ) -> Result<bool, EditError> {
        match &self.gesture {
            Some((gesture, _)) if gesture.origin == origin => self.apply_to_history(edits, origin),
            Some((gesture, _)) => {
                let gesture = gesture.clone();
                self.close_gesture();
                let result = self.apply_step(Step::new(origin, summary), edits);
                self.history.begin();
                self.gesture = Some((gesture, self.snapshot()));
                result
            }
            None => self.apply_step(Step::new(origin, summary), edits),
        }
    }

    /// Applies edits outside a gesture and records them as `step`.
    fn apply_step(&mut self, step: Step, edits: &[Edit]) -> Result<bool, EditError> {
        let changed = self.apply_to_history(edits, step.origin)?;
        if changed {
            self.record(step);
        }
        Ok(changed)
    }

    /// Runs edits through the history, and bumps the revision when they
    /// changed something.
    fn apply_to_history(&mut self, edits: &[Edit], origin: Origin) -> Result<bool, EditError> {
        let before = self.history.revision();
        self.history
            .apply(Arc::make_mut(&mut self.document), edits)?;
        let changed = self.history.revision() != before;
        if changed {
            self.redo.clear();
            self.bump(origin, ChangeKind::Applied);
        }
        Ok(changed)
    }

    /// Notes that the history took a new undo step.
    fn record(&mut self, step: Step) {
        self.redo.clear();
        self.undo.push(step);
        let depth = self.history.undo_depth();
        if self.undo.len() > depth {
            // The history forgot its oldest steps to stay under its limit.
            let excess = self.undo.len() - depth;
            self.undo.drain(..excess);
        }
        self.check_steps();
    }

    /// Keeps the step lists as long as the history's stacks, whatever
    /// happens, so that an undo always finds its label.
    fn check_steps(&mut self) {
        let unknown = || Step::new(Origin::User, "");
        debug_assert_eq!(self.undo.len(), self.history.undo_depth());
        debug_assert_eq!(self.redo.len(), self.history.redo_depth());
        self.undo.resize_with(self.history.undo_depth(), unknown);
        self.redo.resize_with(self.history.redo_depth(), unknown);
    }

    fn bump(&mut self, origin: Origin, kind: ChangeKind) {
        self.revision += 1;
        self.pending.push(Change {
            revision: self.revision,
            origin,
            kind,
        });
    }

    /// Opens a gesture: everything `origin` applies until
    /// [`Session::commit_gesture`] becomes one undo step.
    ///
    /// Use it for a continuous change such as dragging a layer, which
    /// applies an edit on every mouse move. Opening a gesture while one is
    /// open commits that one first.
    pub fn begin_gesture(&mut self, origin: Origin, summary: impl Into<String>) {
        self.close_gesture();
        self.history.begin();
        self.gesture = Some((Step::new(origin, summary), self.snapshot()));
    }

    /// Whether a gesture is open.
    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// Closes the open gesture and records it as one undo step. A gesture
    /// that changed nothing records nothing.
    pub fn commit_gesture(&mut self) {
        self.close_gesture();
    }

    fn close_gesture(&mut self) {
        let Some((step, before)) = self.gesture.take() else {
            return;
        };
        let depth = self.history.undo_depth();
        self.history.commit(&self.document);
        // The history records a step exactly when the gesture left the
        // document different. Its depth says so, except when it is full and
        // drops an old step for every new one.
        let recorded = match self.history.undo_depth() {
            after if after > depth => true,
            _ if depth == Self::UNDO_LIMIT => *before != *self.document,
            _ => false,
        };
        if recorded {
            self.record(step);
        }
    }

    /// Closes the open gesture and puts the document back as it was when
    /// the gesture began. Nothing is recorded.
    pub fn cancel_gesture(&mut self) -> Result<(), EditError> {
        let Some((step, _)) = self.gesture.take() else {
            return Ok(());
        };
        let before = self.history.revision();
        self.history.cancel(Arc::make_mut(&mut self.document))?;
        if self.history.revision() != before {
            self.bump(step.origin, ChangeKind::Undone);
        }
        Ok(())
    }

    /// Whether there is a step to undo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The steps that can be undone, oldest first. The last one is undone
    /// next.
    pub fn undo_steps(&self) -> &[Step] {
        &self.undo
    }

    /// The steps that can be redone. The last one is redone next.
    pub fn redo_steps(&self) -> &[Step] {
        &self.redo
    }

    /// Undoes the most recent step and returns it, or `None` when there is
    /// nothing to undo. An open gesture is committed first and is then the
    /// step that is undone.
    pub fn undo(&mut self) -> Result<Option<Step>, EditError> {
        self.close_gesture();
        let outcome = self.history.undo(Arc::make_mut(&mut self.document));
        self.after_undo_or_redo(outcome, ChangeKind::Undone)
    }

    /// Redoes the most recently undone step and returns it, or `None` when
    /// there is nothing to redo.
    pub fn redo(&mut self) -> Result<Option<Step>, EditError> {
        self.close_gesture();
        let outcome = self.history.redo(Arc::make_mut(&mut self.document));
        self.after_undo_or_redo(outcome, ChangeKind::Redone)
    }

    fn after_undo_or_redo(
        &mut self,
        outcome: Result<bool, EditError>,
        kind: ChangeKind,
    ) -> Result<Option<Step>, EditError> {
        let (from, to) = match kind {
            ChangeKind::Redone => (&mut self.redo, &mut self.undo),
            _ => (&mut self.undo, &mut self.redo),
        };
        let step = match outcome {
            Ok(true) => from.pop(),
            Ok(false) => None,
            Err(error) => {
                // The history has emptied itself: it no longer matches the
                // document.
                self.undo.clear();
                self.redo.clear();
                return Err(error);
            }
        };
        if let Some(step) = &step {
            to.push(step.clone());
            let origin = step.origin;
            self.bump(origin, kind);
        }
        self.check_steps();
        Ok(step)
    }

    /// Puts another document in the place of the current one, for example
    /// when a file is opened. The undo history is forgotten.
    pub fn replace(&mut self, document: Document, path: Option<PathBuf>) {
        self.document = Arc::new(document);
        self.history = History::with_limit(Self::UNDO_LIMIT);
        self.undo.clear();
        self.redo.clear();
        self.gesture = None;
        self.saved_revision = None;
        self.bump(Origin::User, ChangeKind::Replaced);
        if path.is_some() {
            self.saved_revision = Some(self.revision);
        }
        self.path = path.map(absolute);
    }

    /// Gives the project another name. The name is not a node, so this is
    /// not an edit and cannot be undone.
    pub fn rename(&mut self, name: impl Into<String>) {
        let name = name.into();
        if self.document.name != name {
            Arc::make_mut(&mut self.document).name = name;
            self.bump(Origin::User, ChangeKind::Renamed);
        }
    }

    /// Writes the document to its project file and returns the file's path.
    pub fn save(&mut self) -> Result<PathBuf, SessionSaveError> {
        let path = self.path.clone().ok_or(SessionSaveError::NoPath)?;
        io::save(&self.document, &path)?;
        self.saved_revision = Some(self.revision);
        Ok(path)
    }

    /// Writes the document to `path`, which becomes the project file.
    pub fn save_as(&mut self, path: impl Into<PathBuf>) -> Result<(), SaveError> {
        let path = absolute(path.into());
        io::save(&self.document, &path)?;
        self.path = Some(path);
        self.saved_revision = Some(self.revision);
        Ok(())
    }

    /// Takes the changes that have happened since the last call. A
    /// [`SessionHandle`] does this by itself to tell its subscribers.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.pending)
    }
}

/// Decides whether a listener stays subscribed: `false` removes it.
type Listener = Arc<dyn Fn(&Change) -> bool + Send + Sync>;

struct Shared {
    session: Mutex<Session>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next_listener: AtomicU64,
}

impl Shared {
    fn listeners(&self) -> MutexGuard<'_, Vec<(u64, Listener)>> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// A [`Session`] that threads can share.
///
/// Cloning a handle is cheap and every clone works on the same session.
/// [`SessionHandle::read`] and [`SessionHandle::write`] give access to the
/// session itself; the other methods are the common cases of those two.
///
/// ```
/// use moshionly_agent::{Origin, Session, SessionHandle};
/// use moshionly_core::{Document, build};
///
/// let session = SessionHandle::new(Session::new(Document::new("Hello")));
/// let changes = session.changes();
///
/// let (_, edits) = build::add_artboard(&session.document(), "Scene", 800.0, 600.0)?;
/// session.apply(Origin::User, "Add artboard", &edits)?;
///
/// assert_eq!(session.document().artboards().len(), 1);
/// assert_eq!(changes.try_recv()?.origin, Origin::User);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct SessionHandle {
    shared: Arc<Shared>,
}

impl fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.read(|session| {
            f.debug_struct("SessionHandle")
                .field("file", &session.file_name())
                .field("revision", &session.revision())
                .finish()
        })
    }
}

impl From<Session> for SessionHandle {
    fn from(session: Session) -> Self {
        Self::new(session)
    }
}

impl SessionHandle {
    /// Makes a session shareable.
    pub fn new(mut session: Session) -> Self {
        // Nobody is subscribed yet, so what happened before is not news.
        session.take_changes();
        Self {
            shared: Arc::new(Shared {
                session: Mutex::new(session),
                listeners: Mutex::new(Vec::new()),
                next_listener: AtomicU64::new(0),
            }),
        }
    }

    /// A shareable session on a new, unsaved document.
    pub fn from_document(document: Document) -> Self {
        Self::new(Session::new(document))
    }

    /// Opens a project file as a shareable session.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        Session::open(path).map(Self::new)
    }

    fn lock(&self) -> MutexGuard<'_, Session> {
        // A panic while the lock was held cannot have left the document half
        // changed, because documents only change through an atomic apply.
        self.shared
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Looks at the session. Other threads wait meanwhile, so keep it short.
    pub fn read<R>(&self, look: impl FnOnce(&Session) -> R) -> R {
        look(&self.lock())
    }

    /// Works on the session, then tells the subscribers what changed.
    ///
    /// Everything done inside `work` happens without another thread getting
    /// in between, which is what a tool needs to check a document and then
    /// edit exactly what it checked. Subscribers are called after the
    /// session is free again, so they may use the handle themselves.
    pub fn write<R>(&self, work: impl FnOnce(&mut Session) -> R) -> R {
        let (result, changes) = {
            let mut session = self.lock();
            let result = work(&mut session);
            (result, session.take_changes())
        };
        self.notify(&changes);
        result
    }

    fn notify(&self, changes: &[Change]) {
        if changes.is_empty() {
            return;
        }
        let listeners: Vec<(u64, Listener)> = self.shared.listeners().clone();
        let mut gone = Vec::new();
        for (id, listener) in &listeners {
            if !changes.iter().all(|change| listener(change)) {
                gone.push(*id);
            }
        }
        if !gone.is_empty() {
            self.shared.listeners().retain(|(id, _)| !gone.contains(id));
        }
    }

    /// The document as it is now. See [`Session::snapshot`].
    pub fn document(&self) -> Arc<Document> {
        self.read(Session::snapshot)
    }

    /// See [`Session::revision`].
    pub fn revision(&self) -> u64 {
        self.read(Session::revision)
    }

    /// The project file, when the project has one.
    pub fn path(&self) -> Option<PathBuf> {
        self.read(|session| session.path().map(Path::to_owned))
    }

    /// See [`Session::apply`].
    pub fn apply(
        &self,
        origin: Origin,
        summary: impl Into<String>,
        edits: &[Edit],
    ) -> Result<bool, EditError> {
        self.write(|session| session.apply(origin, summary, edits))
    }

    /// See [`Session::undo`].
    pub fn undo(&self) -> Result<Option<Step>, EditError> {
        self.write(Session::undo)
    }

    /// See [`Session::redo`].
    pub fn redo(&self) -> Result<Option<Step>, EditError> {
        self.write(Session::redo)
    }

    /// Calls `listener` after every change to the document, on the thread
    /// that made the change. Keep it quick: ask for a repaint, send a
    /// message, set a flag.
    ///
    /// The listener stays subscribed for as long as the returned
    /// [`Subscription`] is kept.
    pub fn subscribe(&self, listener: impl Fn(&Change) + Send + Sync + 'static) -> Subscription {
        let id = self.add_listener(Arc::new(move |change| {
            listener(change);
            true
        }));
        Subscription {
            shared: Arc::downgrade(&self.shared),
            id,
        }
    }

    /// A channel that receives every change to the document from now on.
    /// Dropping the receiver ends the subscription.
    pub fn changes(&self) -> mpsc::Receiver<Change> {
        let (sender, receiver) = mpsc::channel();
        // A `Sender` may be used from one thread at a time.
        let sender = Mutex::new(sender);
        self.add_listener(Arc::new(move |change| {
            sender
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(*change)
                .is_ok()
        }));
        receiver
    }

    fn add_listener(&self, listener: Listener) -> u64 {
        let id = self.shared.next_listener.fetch_add(1, Ordering::Relaxed);
        self.shared.listeners().push((id, listener));
        id
    }
}

/// Keeps a listener subscribed to a [`SessionHandle`]. Dropping it ends the
/// subscription.
#[must_use = "the listener is removed when the subscription is dropped"]
pub struct Subscription {
    shared: Weak<Shared>,
    id: u64,
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .finish()
    }
}

impl Subscription {
    /// Keeps the listener subscribed for as long as the session lives.
    pub fn detach(mut self) {
        self.shared = Weak::new();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            shared.listeners().retain(|(id, _)| *id != self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::{Node, NodeId};
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicUsize;

    fn add(value: Value) -> Edit {
        Edit::add(Node::from_value(value).unwrap())
    }

    /// A session with an artboard `ab` holding a rect `r1`, made in one
    /// step by the user.
    fn session() -> Session {
        let mut session = Session::new(Document::new("Test"));
        session
            .apply(
                Origin::User,
                "Set up",
                &[
                    add(json!({ "id": "ab", "index": "a0", "type": "artboard" })),
                    add(json!({ "id": "r1", "parent": "ab", "index": "a0", "type": "rect" })),
                ],
            )
            .unwrap();
        session
    }

    fn x_of(session: &Session) -> f64 {
        let node = session.document().get(&NodeId::from("r1")).unwrap();
        node.layer().unwrap().x
    }

    fn origins(steps: &[Step]) -> Vec<Origin> {
        steps.iter().map(|step| step.origin).collect()
    }

    #[test]
    fn every_batch_is_one_step_labelled_with_its_origin() {
        let mut session = session();
        assert_eq!(session.revision(), 1);
        let changed = session
            .apply(
                Origin::Agent,
                "Move the card",
                &[Edit::set("r1", "x", 10), Edit::set("r1", "y", 20)],
            )
            .unwrap();
        assert!(changed);
        assert_eq!(session.revision(), 2);
        assert_eq!(origins(session.undo_steps()), [Origin::User, Origin::Agent]);
        assert_eq!(session.undo_steps()[1].summary, "Move the card");

        let undone = session.undo().unwrap().unwrap();
        assert_eq!(undone.origin, Origin::Agent);
        assert_eq!(x_of(&session), 0.0);
        assert_eq!(origins(session.redo_steps()), [Origin::Agent]);

        let redone = session.redo().unwrap().unwrap();
        assert_eq!(redone.summary, "Move the card");
        assert_eq!(x_of(&session), 10.0);
        assert!(session.redo_steps().is_empty());
        assert_eq!(session.revision(), 4);
    }

    #[test]
    fn a_refused_batch_changes_nothing() {
        let mut session = session();
        let before = session.snapshot();
        let error = session
            .apply(
                Origin::Agent,
                "Break things",
                &[Edit::set("r1", "x", 10), Edit::set("nope", "x", 1)],
            )
            .unwrap_err();
        assert_eq!(error.index, 1);
        assert_eq!(*session.document(), *before);
        assert_eq!(session.revision(), 1);
        assert_eq!(session.undo_steps().len(), 1);
        assert!(session.take_changes().len() == 1, "only the set-up");
    }

    #[test]
    fn a_batch_that_changes_nothing_records_no_step() {
        let mut session = session();
        let changed = session
            .apply(Origin::Agent, "Nothing", &[Edit::set("r1", "x", 0)])
            .unwrap();
        assert!(!changed);
        assert_eq!(session.undo_steps().len(), 1);
        assert_eq!(session.revision(), 1);
    }

    #[test]
    fn a_new_step_forgets_what_could_be_redone() {
        let mut session = session();
        session
            .apply(Origin::User, "Move", &[Edit::set("r1", "x", 5)])
            .unwrap();
        session.undo().unwrap();
        assert_eq!(session.redo_steps().len(), 1);
        session
            .apply(Origin::Agent, "Other", &[Edit::set("r1", "y", 5)])
            .unwrap();
        assert!(session.redo_steps().is_empty());
        assert!(!session.can_redo());
    }

    #[test]
    fn a_gesture_is_one_step() {
        let mut session = session();
        session.begin_gesture(Origin::User, "Drag");
        for x in 1..=5 {
            session
                .apply(Origin::User, "ignored", &[Edit::set("r1", "x", x)])
                .unwrap();
        }
        assert!(session.in_gesture());
        session.commit_gesture();
        assert_eq!(session.undo_steps().len(), 2);
        assert_eq!(session.undo_steps()[1].summary, "Drag");
        session.undo().unwrap();
        assert_eq!(x_of(&session), 0.0);
    }

    #[test]
    fn a_gesture_that_ends_where_it_began_records_nothing() {
        let mut session = session();
        session.begin_gesture(Origin::User, "Drag");
        session
            .apply(Origin::User, "", &[Edit::set("r1", "x", 9)])
            .unwrap();
        session
            .apply(Origin::User, "", &[Edit::set("r1", "x", 0)])
            .unwrap();
        session.commit_gesture();
        assert_eq!(session.undo_steps().len(), 1);
    }

    #[test]
    fn a_cancelled_gesture_puts_the_document_back() {
        let mut session = session();
        session.begin_gesture(Origin::User, "Drag");
        session
            .apply(Origin::User, "", &[Edit::set("r1", "x", 9)])
            .unwrap();
        session.cancel_gesture().unwrap();
        assert_eq!(x_of(&session), 0.0);
        assert_eq!(session.undo_steps().len(), 1);
        assert!(!session.in_gesture());
    }

    #[test]
    fn an_agent_batch_during_a_drag_is_still_its_own_step() {
        let mut session = session();
        session.begin_gesture(Origin::User, "Drag");
        session
            .apply(Origin::User, "", &[Edit::set("r1", "x", 3)])
            .unwrap();
        session
            .apply(
                Origin::Agent,
                "Recolour",
                &[Edit::set("r1", "fill", "#FF0000")],
            )
            .unwrap();
        session
            .apply(Origin::User, "", &[Edit::set("r1", "x", 7)])
            .unwrap();
        session.commit_gesture();

        let steps: Vec<(Origin, &str)> = session
            .undo_steps()
            .iter()
            .map(|step| (step.origin, step.summary.as_str()))
            .collect();
        assert_eq!(
            steps,
            [
                (Origin::User, "Set up"),
                (Origin::User, "Drag"),
                (Origin::Agent, "Recolour"),
                (Origin::User, "Drag"),
            ]
        );
        // Undoing the rest of the drag leaves the agent's colour in place.
        session.undo().unwrap();
        assert_eq!(x_of(&session), 3.0);
        let node = session.document().get(&NodeId::from("r1")).unwrap();
        assert_eq!(node.to_value()["fill"], "#FF0000");
    }

    #[test]
    fn steps_stay_in_line_with_the_history_past_its_limit() {
        let mut session = session();
        for x in 1..=(Session::UNDO_LIMIT as i64 + 5) {
            let origin = if x % 2 == 0 {
                Origin::Agent
            } else {
                Origin::User
            };
            session
                .apply(origin, format!("x = {x}"), &[Edit::set("r1", "x", x)])
                .unwrap();
        }
        assert_eq!(session.undo_steps().len(), Session::UNDO_LIMIT);
        let last = session.undo_steps().last().unwrap();
        assert_eq!(last.summary, format!("x = {}", Session::UNDO_LIMIT + 5));

        // A gesture at the limit is labelled too.
        session.begin_gesture(Origin::Import, "Gesture");
        session
            .apply(Origin::Import, "", &[Edit::set("r1", "y", 1)])
            .unwrap();
        session.commit_gesture();
        assert_eq!(session.undo_steps().len(), Session::UNDO_LIMIT);
        assert_eq!(session.undo().unwrap().unwrap().summary, "Gesture");
    }

    #[test]
    fn a_relative_project_path_is_made_absolute() {
        let here = std::env::current_dir().unwrap();
        let session = Session::with_path(Document::new("Demo"), "demo.moshi");
        assert_eq!(session.path(), Some(here.join("demo.moshi").as_path()));
        assert_eq!(session.file_name(), "demo.moshi");

        let mut session = Session::new(Document::new("Demo"));
        session.set_path(Some(PathBuf::from("demo.moshi")));
        assert_eq!(session.path(), Some(here.join("demo.moshi").as_path()));
        session.replace(Document::new("Other"), Some(PathBuf::from("other.moshi")));
        assert_eq!(session.path(), Some(here.join("other.moshi").as_path()));
    }

    #[test]
    fn replacing_the_document_forgets_the_history() {
        let mut session = session();
        session.replace(Document::new("Other"), Some(PathBuf::from("/tmp/o.moshi")));
        assert!(session.document().is_empty());
        assert!(!session.can_undo());
        assert!(session.undo_steps().is_empty());
        assert_eq!(session.file_name(), "o.moshi");
        assert!(!session.has_unsaved_changes());
        assert_eq!(session.revision(), 2);
    }

    #[test]
    fn saving_tracks_unsaved_changes() {
        let dir = std::env::temp_dir().join(format!(
            "moshionly-agent-session-{}-{}",
            std::process::id(),
            NodeId::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("project.moshi");

        let mut session = session();
        assert_eq!(session.file_name(), "Untitled.moshi");
        assert!(matches!(session.save(), Err(SessionSaveError::NoPath)));
        assert!(session.has_unsaved_changes());
        session.save_as(&file).unwrap();
        assert!(!session.has_unsaved_changes());
        session.rename("Renamed");
        assert!(session.has_unsaved_changes());
        assert_eq!(session.save().unwrap(), file);

        let reopened = Session::open(&file).unwrap();
        assert_eq!(*reopened.document(), *session.document());
        assert_eq!(reopened.document().name, "Renamed");
        assert!(!reopened.has_unsaved_changes());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_document_can_start_clean() {
        let mut session = session();
        assert!(session.has_unsaved_changes());
        session.mark_clean();
        assert!(!session.has_unsaved_changes());
        assert_eq!(session.path(), None);
        session.rename("Renamed");
        assert!(session.has_unsaved_changes(), "a change counts again");
        session.replace(Document::new("New"), None);
        assert!(session.has_unsaved_changes());
        session.mark_clean();
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn a_snapshot_is_not_touched_by_later_edits() {
        let handle = SessionHandle::new(session());
        let before = handle.document();
        handle
            .apply(Origin::Agent, "Move", &[Edit::set("r1", "x", 50)])
            .unwrap();
        let x = |document: &Document| {
            let node = document.get(&NodeId::from("r1")).unwrap();
            node.layer().unwrap().x
        };
        assert_eq!(x(&before), 0.0);
        assert_eq!(x(&handle.document()), 50.0);
    }

    #[test]
    fn subscribers_hear_about_changes_until_they_let_go() {
        let handle = SessionHandle::new(session());
        let receiver = handle.changes();
        let calls = Arc::new(AtomicUsize::new(0));
        let subscription = handle.subscribe({
            let calls = Arc::clone(&calls);
            let handle = handle.clone();
            move |change| {
                // A listener may use the session: it is not locked here.
                assert_eq!(handle.revision(), change.revision);
                calls.fetch_add(1, Ordering::SeqCst);
            }
        });

        handle
            .apply(Origin::Agent, "Move", &[Edit::set("r1", "x", 1)])
            .unwrap();
        handle.undo().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let first = receiver.try_recv().unwrap();
        assert_eq!(
            (first.origin, first.kind, first.revision),
            (Origin::Agent, ChangeKind::Applied, 2)
        );
        assert_eq!(receiver.try_recv().unwrap().kind, ChangeKind::Undone);

        drop(subscription);
        drop(receiver);
        handle.redo().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(handle.shared.listeners().is_empty());
    }

    #[test]
    fn a_handle_is_shared_between_threads() {
        let handle = SessionHandle::new(session());
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    for step in 0..25 {
                        let x = worker * 100 + step + 1;
                        handle
                            .apply(Origin::Agent, "Move", &[Edit::set("r1", "x", x)])
                            .unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(handle.revision(), 101);
        assert_eq!(handle.read(|session| session.undo_steps().len()), 101);
    }
}
