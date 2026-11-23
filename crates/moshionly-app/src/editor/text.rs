//! Editing a text layer's words in place on the canvas.
//!
//! While text is being edited the canvas leaves the layer out of its
//! picture and the editor draws a text field over it instead. Escape or a
//! click elsewhere commits the words as one undo step. Text made with the
//! text tool is added and typed in one step; if nothing was typed it is not
//! kept at all.

use moshionly_core::{Edit, EditError, NodeId, build};

use super::Editor;
use super::inspect::text_of;

/// Text being edited on the canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEditing {
    /// The text layer.
    pub id: NodeId,
    /// The words as typed so far.
    pub buffer: String,
    /// The words before editing began.
    pub original: String,
    /// The layer was made for this edit, in a gesture that is still open.
    pub created: bool,
}

impl Editor {
    /// Starts editing the words of the text layer `id`. With `created`,
    /// the layer was just added inside a gesture that the end of the edit
    /// closes.
    pub fn begin_text_edit(&mut self, id: &NodeId, created: bool) -> Option<TextEditing> {
        let text = text_of(self.document.get(id)?)?;
        self.selection.select(id.clone());
        Some(TextEditing {
            id: id.clone(),
            buffer: text.content.clone(),
            original: text.content.clone(),
            created,
        })
    }

    /// Commits the words typed. A new text layer left empty is taken away
    /// again, leaving nothing in the undo history.
    pub fn finish_text_edit(&mut self, editing: TextEditing) -> Result<(), EditError> {
        let exists = self.document.contains(&editing.id);
        let empty = editing.buffer.trim().is_empty();
        if editing.created {
            if empty {
                if self.in_gesture() {
                    self.cancel_gesture()?;
                } else if exists {
                    // The gesture was closed by something else meanwhile.
                    let edits = build::remove(&self.document, std::slice::from_ref(&editing.id));
                    self.apply("Delete layer", &edits)?;
                }
                self.selection.clear();
                return Ok(());
            }
            if exists {
                self.apply(
                    "Add text",
                    &[Edit::set(editing.id.clone(), "content", editing.buffer)],
                )?;
            }
            self.commit_gesture();
            return Ok(());
        }
        if exists && editing.buffer != editing.original {
            self.apply(
                "Edit text",
                &[Edit::set(editing.id, "content", editing.buffer)],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::super::view::tests::editor;
    use super::*;
    use crate::shell::Tool;

    fn content(editor: &Editor, id: &NodeId) -> String {
        text_of(editor.document().get(id).unwrap())
            .unwrap()
            .content
            .clone()
    }

    #[test]
    fn typed_words_are_one_undo_step() {
        let mut editor = editor();
        let id = editor.create_at(Tool::Text, pos2(100.0, 600.0)).unwrap();
        let before = content(&editor, &id);
        let mut editing = editor.begin_text_edit(&id, false).unwrap();
        assert_eq!(editing.buffer, before);
        editing.buffer = "Hello\nthere".to_owned();
        editor.finish_text_edit(editing).unwrap();
        assert_eq!(content(&editor, &id), "Hello\nthere");
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Edit text"));
        editor.undo().unwrap();
        assert_eq!(content(&editor, &id), before);
    }

    #[test]
    fn new_text_is_added_and_typed_in_one_step() {
        let mut editor = editor();
        editor.begin_gesture("Add text");
        let id = editor.create_at(Tool::Text, pos2(100.0, 600.0)).unwrap();
        let mut editing = editor.begin_text_edit(&id, true).unwrap();
        editing.buffer = "Launch".to_owned();
        editor.finish_text_edit(editing).unwrap();
        assert!(!editor.in_gesture());
        assert_eq!(content(&editor, &id), "Launch");
        editor.undo().unwrap();
        assert!(!editor.document().contains(&id), "one undo removes it");
    }

    #[test]
    fn new_text_left_empty_is_not_kept() {
        let mut editor = editor();
        editor.begin_gesture("Add text");
        let id = editor.create_at(Tool::Text, pos2(100.0, 600.0)).unwrap();
        let mut editing = editor.begin_text_edit(&id, true).unwrap();
        editing.buffer = "  ".to_owned();
        editor.finish_text_edit(editing).unwrap();
        assert!(!editor.document().contains(&id));
        assert!(editor.selection.is_empty());
        assert_eq!(editor.undo_label(), None, "nothing to undo");
    }

    #[test]
    fn only_text_layers_can_be_edited() {
        let mut editor = editor();
        assert!(
            editor
                .begin_text_edit(&NodeId::from("loose"), false)
                .is_none()
        );
        assert!(
            editor
                .begin_text_edit(&NodeId::from("missing"), false)
                .is_none()
        );
    }
}
