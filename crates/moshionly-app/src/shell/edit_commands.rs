//! The Edit commands that work on the selected layers: copy, cut, paste,
//! duplicate, group, ungroup and the stacking order.

use egui::Context;
use moshionly_core::build::{self, GroupKind, Place, Reorder};
use moshionly_core::{Document, Edit, Family, NodeId, NodeType};

use super::Shell;
use crate::{
    commands::{CommandId, CommandRegistry},
    editor::{Editor, assets, clipboard},
    widgets::Toast,
};

impl Shell {
    /// Carries out an Edit command on the selection. Returns false for
    /// commands that are not about the selection.
    pub(super) fn edit_command(&mut self, ctx: &Context, id: CommandId) -> bool {
        match id {
            CommandId::Copy => {
                self.copy(ctx);
            }
            CommandId::Cut => {
                if self.copy(ctx) {
                    self.delete_selection();
                }
            }
            CommandId::Paste => {
                let text = self
                    .commands
                    .take_pasted()
                    .or_else(|| self.clipboard.clone());
                self.paste(text.as_deref());
            }
            CommandId::Duplicate => self.duplicate(),
            CommandId::Group => self.group(),
            CommandId::Ungroup => self.ungroup(),
            CommandId::BringToFront => self.reorder(Reorder::ToFront),
            CommandId::BringForward => self.reorder(Reorder::Forward),
            CommandId::SendBackward => self.reorder(Reorder::Backward),
            CommandId::SendToBack => self.reorder(Reorder::ToBack),
            _ => return false,
        }
        true
    }

    /// Puts the selected layers on the clipboard as Moshionly's JSON.
    /// Returns whether there was anything to copy.
    fn copy(&mut self, ctx: &Context) -> bool {
        let Some(text) = clipboard::copy(
            self.editor.document(),
            self.editor.selection.ids(),
            self.editor.project_file(),
        ) else {
            return false;
        };
        ctx.copy_text(text.clone());
        self.clipboard = Some(text);
        true
    }

    /// Pastes Moshionly layers from `text`: into the selected artboard or
    /// group, or next to the selected layer, or into the current artboard.
    fn paste(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        // A design copied with the Figma plugin's Copy button is added at
        // once, as one sent to the app is.
        if self.paste_envelope(text) {
            return;
        }
        let copied = match clipboard::read(text) {
            Ok(copied) => copied,
            Err(error) => {
                self.toasts.push(Toast::info(error.to_string()));
                return;
            }
        };
        let document = std::sync::Arc::clone(self.editor.document());
        // Instances whose main is not in this document come detached.
        let mut nodes = copied.nodes_for(&document);
        // Images from another project are copied into this one.
        for problem in assets::adopt_images(&mut nodes, self.editor.project_file()) {
            self.toasts.push(Toast::warning(problem));
        }
        let (parent, place) = paste_target(&document, &self.editor);
        let result = clipboard::paste(&document, &nodes, parent.as_ref(), &place);
        match result {
            Ok((edits, roots)) => {
                if self.apply_or_toast("Paste", &edits) {
                    self.editor.selection.set(roots);
                }
            }
            Err(error) => self.toasts.push(Toast::warning(error.to_string())),
        }
    }

    /// Copies the selected layers in place, just in front of themselves.
    fn duplicate(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if ids.is_empty() {
            return;
        }
        match build::duplicate(self.editor.document(), &ids) {
            Ok((edits, copies)) => {
                if self.apply_or_toast("Duplicate", &edits) {
                    self.editor
                        .selection
                        .set(ids.iter().filter_map(|id| copies.get(id).cloned()));
                }
            }
            Err(error) => self.toasts.push(Toast::warning(error.to_string())),
        }
    }

    /// Puts the selected layers in a new group and selects it.
    fn group(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        // Text that hugs its words is given the box it is drawn in first,
        // in the same step, so that the group is made around the words.
        let settle = self.editor.settle_edits(&ids, false);
        let mut working = (**self.editor.document()).clone();
        if let Err(error) = working.apply(&settle) {
            self.toasts.push(Toast::error(error.problem.to_string()));
            return;
        }
        match build::group(&working, &ids, GroupKind::Group) {
            Ok((group, edits)) => {
                if self.apply_or_toast("Group", &[settle, edits].concat()) {
                    self.editor.selection.select(group);
                }
            }
            Err(error) => self.toasts.push(Toast::warning(error.to_string())),
        }
    }

    /// Dissolves the selected groups and selects what was in them.
    fn ungroup(&mut self) {
        let document = std::sync::Arc::clone(self.editor.document());
        let groups: Vec<NodeId> = self
            .editor
            .selection
            .ids()
            .iter()
            .filter(|id| is_group(&document, id))
            .cloned()
            .collect();
        let mut working = (*document).clone();
        let mut edits = Vec::new();
        let mut freed = Vec::new();
        for group in &groups {
            freed.extend(working.children(group, Family::Layer).iter().cloned());
            match build::ungroup(&working, group) {
                Ok(more) => {
                    if working.apply(&more).is_ok() {
                        edits.extend(more);
                    }
                }
                Err(error) => self.toasts.push(Toast::warning(error.to_string())),
            }
        }
        if self.apply_or_toast("Ungroup", &edits) {
            // A group freed from a selected group may itself have been
            // dissolved: select only what is left.
            let document = std::sync::Arc::clone(self.editor.document());
            self.editor
                .selection
                .set(freed.into_iter().filter(|id| document.contains(id)));
        }
    }

    /// Moves the selected layers in the stacking order.
    fn reorder(&mut self, step: Reorder) {
        let document = std::sync::Arc::clone(self.editor.document());
        let edits = reorder_edits(&document, self.editor.selection.ids(), step);
        let summary = match step {
            Reorder::ToFront => "Bring to front",
            Reorder::Forward => "Bring forward",
            Reorder::Backward => "Send backward",
            Reorder::ToBack => "Send to back",
        };
        self.apply_or_toast(summary, &edits);
    }
}

/// The edits that move `ids` one step or all the way in the stacking
/// order. The selected layers move as a block: one step forward moves each
/// in front of the next layer that is not selected, and a block already at
/// the front stays as it is, so selected layers never swap with each
/// other. Backward is the same the other way.
pub(super) fn reorder_edits(document: &Document, ids: &[NodeId], step: Reorder) -> Vec<Edit> {
    let mut ids = ids.to_vec();
    // Front first when moving forward, so that each layer moves into a
    // place the one before it left; back first when moving backward.
    let order = |id: &NodeId| {
        document
            .siblings(id)
            .iter()
            .position(|sibling| sibling == id)
            .unwrap_or(0)
    };
    ids.sort_by_key(order);
    if matches!(step, Reorder::Forward | Reorder::ToBack) {
        ids.reverse();
    }
    let mut working = document.clone();
    let mut edits: Vec<Edit> = Vec::new();
    for id in &ids {
        // One step never passes another selected layer.
        let siblings = working.siblings(id);
        let at = siblings.iter().position(|sibling| sibling == id);
        let next = match (step, at) {
            (Reorder::Forward, Some(at)) => siblings.get(at + 1),
            (Reorder::Backward, Some(at)) => at.checked_sub(1).and_then(|at| siblings.get(at)),
            _ => None,
        };
        if next.is_some_and(|next| ids.contains(next)) {
            continue;
        }
        if let Ok(more) = build::reorder(&working, id, step)
            && working.apply(&more).is_ok()
        {
            edits.extend(more);
        }
    }
    edits
}

/// Whether `id` is a layer that holds layers and can be dissolved.
fn is_group(document: &Document, id: &NodeId) -> bool {
    document
        .get(id)
        .is_some_and(|node| matches!(node.node_type(), NodeType::Group | NodeType::MaskGroup))
}

/// Where pasted layers go: into a selected artboard or group, just in
/// front of a selected layer, or on top of the current artboard.
fn paste_target(document: &Document, editor: &Editor) -> (Option<NodeId>, Place) {
    if let Some(selected) = editor.selection.ids().last()
        && let Some(node) = document.get(selected)
    {
        if node.node_type().holds_layers() {
            return (Some(selected.clone()), Place::Last);
        }
        if let Some(parent) = &node.parent {
            return (Some(parent.clone()), Place::After(selected.clone()));
        }
    }
    (editor.artboard().cloned(), Place::Last)
}

/// Enables the Edit commands that make sense for the selection.
pub(super) fn enable_edit_commands(registry: &mut CommandRegistry, editor: &Editor) {
    let document = editor.document();
    let ids = editor.selection.ids();
    let selected = !ids.is_empty();
    let layers = ids
        .iter()
        .any(|id| document.get(id).is_some_and(|node| node.parent.is_some()));
    for id in [CommandId::Copy, CommandId::Cut, CommandId::Duplicate] {
        registry.set_enabled(id, selected);
    }
    for id in [
        CommandId::Group,
        CommandId::BringToFront,
        CommandId::BringForward,
        CommandId::SendBackward,
        CommandId::SendToBack,
    ] {
        registry.set_enabled(id, layers);
    }
    registry.set_enabled(
        CommandId::Ungroup,
        ids.iter().any(|id| is_group(document, id)),
    );
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    #[test]
    fn selected_layers_move_in_the_stacking_order_as_a_block() {
        let document = io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"a","parent":"ab","index":"a0","type":"rect"},
            {"id":"b","parent":"ab","index":"a1","type":"rect"},
            {"id":"c","parent":"ab","index":"a2","type":"rect"},
            {"id":"d","parent":"ab","index":"a3","type":"rect"}
            ]}"##,
        )
        .unwrap();
        let order = |selected: &[&str], step: Reorder| -> Vec<String> {
            let ids: Vec<NodeId> = selected.iter().map(|id| NodeId::from(*id)).collect();
            let mut moved = document.clone();
            moved.apply(&reorder_edits(&document, &ids, step)).unwrap();
            moved
                .children(&NodeId::from("ab"), Family::Layer)
                .iter()
                .map(|id| id.to_string())
                .collect()
        };
        // Already at the front or the back: nothing changes.
        assert_eq!(order(&["c", "d"], Reorder::Forward), ["a", "b", "c", "d"]);
        assert_eq!(order(&["a", "b"], Reorder::Backward), ["a", "b", "c", "d"]);
        // A block moves one step together.
        assert_eq!(order(&["a", "b"], Reorder::Forward), ["c", "a", "b", "d"]);
        assert_eq!(order(&["c", "d"], Reorder::Backward), ["a", "c", "d", "b"]);
        // Apart, each moves one step; the front one stays at the front.
        assert_eq!(order(&["b", "d"], Reorder::Forward), ["a", "c", "b", "d"]);
        assert_eq!(order(&["a", "c"], Reorder::Backward), ["a", "c", "b", "d"]);
        // All the way keeps their order.
        assert_eq!(order(&["a", "c"], Reorder::ToFront), ["b", "d", "a", "c"]);
        assert_eq!(order(&["b", "d"], Reorder::ToBack), ["b", "d", "a", "c"]);
    }

    #[test]
    fn layers_are_pasted_beside_the_selection_or_into_a_container() {
        let document = io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"g","parent":"ab","index":"a0","type":"group"},
            {"id":"r","parent":"g","index":"a0","type":"rect"}
            ]}"##,
        )
        .unwrap();
        let session = moshionly_agent::SessionHandle::from_document(document.clone());
        let mut editor = Editor::new(
            session,
            moshionly_render::Renderer::new(
                moshionly_render::FontLibrary::bundled(),
                moshionly_render::AssetLoader::default(),
            ),
        );
        assert_eq!(
            paste_target(&document, &editor),
            (Some(NodeId::from("ab")), Place::Last)
        );
        editor.selection.select(NodeId::from("r"));
        assert_eq!(
            paste_target(&document, &editor),
            (Some(NodeId::from("g")), Place::After(NodeId::from("r")))
        );
        editor.selection.select(NodeId::from("g"));
        assert_eq!(
            paste_target(&document, &editor),
            (Some(NodeId::from("g")), Place::Last)
        );
        let mut registry = CommandRegistry::new();
        enable_edit_commands(&mut registry, &editor);
        assert!(registry.is_enabled(CommandId::Ungroup));
        editor.selection.select(NodeId::from("ab"));
        enable_edit_commands(&mut registry, &editor);
        assert!(!registry.is_enabled(CommandId::Ungroup));
        assert!(
            !registry.is_enabled(CommandId::Group),
            "artboards are not grouped"
        );
        assert!(registry.is_enabled(CommandId::Copy));
    }
}
