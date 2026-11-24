//! Projects on disk: new, open, save, save as, and the question asked
//! before unsaved changes would be lost.
//!
//! Anything that would replace the document or close the window first
//! checks for unsaved changes. If there are some, a prompt asks whether to
//! save them, throw them away or stay; the action carries on only after
//! the answer, and only if saving (when chosen) worked.

use std::path::{Path, PathBuf};

use egui::{Context, RichText};
use moshionly_core::io;

use super::{InspectorTab, Shell, new_document};
use crate::{
    dialogs::file_name,
    editor::assets,
    theme::Theme,
    widgets::{ButtonKind, ModalFrame, TextButton, Toast, modal_footer},
};

/// Something that replaces the document or ends the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileAction {
    /// Start a new, untitled document.
    New,
    /// Open a project: this one, or one chosen in a dialog.
    Open(Option<PathBuf>),
    /// Close the window.
    Close,
}

/// An answer to "save the changes first?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Save, then carry on.
    Save,
    /// Throw the changes away and carry on.
    DontSave,
    /// Stay with the document as it is.
    Cancel,
}

/// Whether an action may go ahead after `answer`. `save` saves the
/// document and says whether that worked; it is only called for
/// [`Answer::Save`].
pub fn carry_on(answer: Answer, save: impl FnOnce() -> bool) -> bool {
    match answer {
        Answer::Save => save(),
        Answer::DontSave => true,
        Answer::Cancel => false,
    }
}

/// What the prompt says the action is, for its question.
fn doing(action: &FileAction) -> &'static str {
    match action {
        FileAction::New => "starting a new project",
        FileAction::Open(_) => "opening another project",
        FileAction::Close => "closing",
    }
}

impl Shell {
    /// Carries out `action`, asking about unsaved changes first when there
    /// are some.
    pub fn request(&mut self, action: FileAction) {
        self.finish_work_in_progress();
        if self.editor.has_unsaved_changes() {
            self.pending = Some(action);
        } else {
            self.perform(action);
        }
    }

    /// Whether the window may close right now. When it may not, the shell
    /// asks about the unsaved changes and closes it later itself (see
    /// [`ShellOutput::close_window`](super::ShellOutput::close_window)).
    pub fn may_close(&mut self) -> bool {
        if self.closing {
            return true;
        }
        // Words being typed and a path being drawn are not in the document
        // yet: put them there first, so they count as changes to save.
        self.finish_work_in_progress();
        if !self.editor.has_unsaved_changes() {
            return true;
        }
        if self.pending.is_none() {
            self.request(FileAction::Close);
        }
        false
    }

    /// Puts what is being made on the canvas into the document: a drag in
    /// progress, the words being typed, the path being drawn with the pen.
    pub(super) fn finish_work_in_progress(&mut self) {
        self.end_canvas_drag();
        self.finish_text_edit();
        if self.pen.is_some() {
            self.finish_pen(false);
        }
    }

    /// Answers the open question about unsaved changes, as the prompt's
    /// buttons do.
    pub fn answer(&mut self, answer: Answer) {
        let Some(action) = self.pending.take() else {
            return;
        };
        if carry_on(answer, || self.save()) {
            self.perform(action);
        }
    }

    /// Whether the question about unsaved changes is showing.
    pub fn is_asking(&self) -> bool {
        self.pending.is_some()
    }

    fn perform(&mut self, action: FileAction) {
        match action {
            FileAction::New => {
                self.editor.session().write(|session| {
                    session.replace(new_document(), None);
                    // Nothing in a blank document is worth asking about.
                    session.mark_clean();
                });
                self.after_replace();
            }
            FileAction::Open(path) => {
                let path = path.or_else(|| self.dialogs.open_project());
                if let Some(path) = path {
                    self.open_path(&path);
                }
            }
            FileAction::Close => self.closing = true,
        }
    }

    /// Opens the project at `path` in place of the current one, or says in
    /// plain words why it cannot. Does not ask about unsaved changes; use
    /// [`request`](Self::request) for that.
    pub fn open_path(&mut self, path: &Path) {
        match io::load(path) {
            Ok(document) => {
                let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
                self.editor
                    .session()
                    .write(|session| session.replace(document, Some(path.clone())));
                self.settings.remember(&path);
                self.after_replace();
            }
            Err(error) => {
                self.settings.forget(path);
                self.toasts.push(Toast::error(format!(
                    "Could not open {}: {}",
                    display_name(path),
                    plain(&error.to_string())
                )));
            }
        }
    }

    /// Saves the project to its file, asking where first when it has none.
    /// Returns whether it was saved.
    pub fn save(&mut self) -> bool {
        self.finish_text_edit();
        if self.editor.session().path().is_none() {
            return self.save_as();
        }
        match self.editor.session().write(|session| session.save()) {
            Ok(path) => {
                self.settings.remember(&path);
                true
            }
            Err(error) => {
                self.toasts.push(Toast::error(sentence(&error.to_string())));
                false
            }
        }
    }

    /// Asks where to save the project and saves it there. Returns whether
    /// it was saved.
    pub fn save_as(&mut self) -> bool {
        self.finish_text_edit();
        let suggested = file_name(&self.project_name(), io::EXTENSION);
        let Some(path) = self.dialogs.save_project(&suggested) else {
            return false;
        };
        let path = std::path::absolute(&path).unwrap_or(path);
        let gathered = self.gather_images(&path);
        // An untitled project takes the name of its file.
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let untitled = matches!(self.editor.document().name.trim(), "" | "Untitled");
        let result = self.editor.session().write(|session| {
            if untitled && !stem.is_empty() {
                session.rename(stem);
            }
            session.save_as(&path)
        });
        match result {
            Ok(()) => {
                // Pointing the images at their copies was part of saving
                // here, not an edit of the person's: undo must not reach
                // it, or it would point them back at names that mean
                // nothing, or another picture, next to the new file.
                if gathered && self.editor.undo_label() == Some(format!("Undo {GATHERED}")) {
                    self.editor
                        .session()
                        .write(|session| session.forget_last_step());
                }
                if let Some(path) = self.editor.session().path() {
                    self.settings.remember(&path);
                }
                true
            }
            Err(error) => {
                // The images were pointed at copies next to a file that
                // was not written: point them back.
                if gathered && self.editor.undo_label() == Some(format!("Undo {GATHERED}")) {
                    let _ = self.editor.undo();
                }
                self.toasts.push(Toast::error(sentence(&error.to_string())));
                false
            }
        }
    }

    /// Before the project is saved as `path`: when that is in another
    /// folder, or the project was never saved, copies the images it uses
    /// into that folder and points the image layers at the copies, as one
    /// undo step, so that the saved project is complete on its own. Says
    /// which images could not be copied. Returns whether the step was made.
    fn gather_images(&mut self, path: &Path) -> bool {
        let from = self.editor.session().path();
        if from
            .as_deref()
            .is_some_and(|from| assets::same_folder(from, path))
        {
            return false;
        }
        let gathered = assets::gather_images(self.editor.document(), from.as_deref(), path);
        for problem in gathered.problems {
            self.toasts.push(Toast::warning(problem));
        }
        self.apply_or_toast(GATHERED, &gathered.edits)
    }

    /// Starts afresh after another document took the place of the old one:
    /// nothing selected, everything in view, the playhead at the start.
    fn after_replace(&mut self) {
        self.editor.sync();
        self.editor.selection.clear();
        self.editor.hover = None;
        self.editor
            .set_artboard(self.editor.document().artboards().first().cloned());
        self.editor.set_playhead(0.0);
        self.pictures.clear();
        self.tree = Default::default();
        self.collapsed_groups.clear();
        self.editor.set_actions([]);
        self.inspector.tab = InspectorTab::Design;
        self.fitted = false;
        self.playback.pause();
        // The Export dialog was set up for the old document. Closing it
        // cancels an export still running and waits for it to clean up.
        if let Some(dialog) = self.export.take()
            && dialog.is_running()
        {
            self.toasts.push(Toast::info(
                "Export cancelled: another project took its place",
            ));
        }
        // The AI should not carry on about a file that is gone.
        if let Some(chat) = self.chat.as_deref_mut() {
            chat.new_conversation();
        }
    }

    /// The question about unsaved changes, while it is open.
    pub(super) fn unsaved_prompt(&mut self, ctx: &Context) {
        let Some(action) = &self.pending else {
            return;
        };
        let question = format!(
            "Save the changes to “{}” before {}?",
            self.project_name(),
            doing(action)
        );
        let theme = Theme::of_ctx(ctx);
        let mut answer = None;
        let shown = ModalFrame::new("Unsaved changes")
            .width(400.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(question)
                        .font(theme.body())
                        .color(theme.color.text),
                );
                ui.label(
                    RichText::new("If you don't save, your changes since the last save are lost.")
                        .font(theme.body())
                        .color(theme.color.text_secondary),
                );
                modal_footer(ui, |ui| {
                    if ui.add(TextButton::primary("Save")).clicked() {
                        answer = Some(Answer::Save);
                    }
                    if ui.add(TextButton::new("Cancel")).clicked() {
                        answer = Some(Answer::Cancel);
                    }
                    if ui
                        .add(TextButton::new("Don't save").kind(ButtonKind::Quiet))
                        .clicked()
                    {
                        answer = Some(Answer::DontSave);
                    }
                });
            });
        if shown.close_requested && answer.is_none() {
            answer = Some(Answer::Cancel);
        }
        if let Some(answer) = answer {
            self.answer(answer);
        }
    }
}

/// The undo step that points image layers at their copies next to a newly
/// saved project file.
const GATHERED: &str = "Copy images into the project";

/// A message as a sentence: a capital at the start.
fn sentence(message: &str) -> String {
    let mut chars = message.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A file's name for messages.
fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// An error message as a sentence part: without a capital at the start or
/// a full stop at the end, so it reads after "Could not open x:".
fn plain(message: &str) -> String {
    let message = message.trim().trim_end_matches('.');
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "an unknown error".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_decides_whether_to_carry_on() {
        assert!(carry_on(Answer::DontSave, || panic!("no save")));
        assert!(!carry_on(Answer::Cancel, || panic!("no save")));
        assert!(carry_on(Answer::Save, || true));
        // A save that fails or is cancelled stops the action.
        assert!(!carry_on(Answer::Save, || false));
    }

    #[test]
    fn errors_read_as_part_of_a_sentence() {
        assert_eq!(plain("The file is not JSON."), "the file is not JSON");
        assert_eq!(plain("  "), "an unknown error");
        assert_eq!(sentence("could not save \"x\""), "Could not save \"x\"");
        assert_eq!(display_name(Path::new("/a/b/teaser.moshi")), "teaser.moshi");
        assert_eq!(doing(&FileAction::Close), "closing");
    }
}
