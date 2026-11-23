//! The layers panel on the left of the window: the document's tree.

use egui::{Key, Rect, ScrollArea, Stroke, Ui, Vec2, pos2};
use moshionly_core::{Edit, NodeId, NodeType, build};

use super::parts::command_item;
use crate::{
    commands::{CommandId, CommandRegistry},
    editor::{
        Editor,
        describe::next_name,
        timeline::layer_icon,
        tree::{Drop, LayerRow, TreeState, drop_at, drop_edits, layer_rows},
    },
    theme::{Theme, icons},
    widgets::{
        IconButton, ListRow, MenuItem, TextField, context_menu, control_row, empty_state,
        menu_separator, padded, row_toggle, search_field, section_header,
    },
};

/// What the layers panel works on.
pub(super) struct LayersPanel<'a> {
    /// The document and the selection.
    pub editor: &'a mut Editor,
    /// Which rows are open.
    pub tree: &'a mut TreeState,
    /// What is typed into the search field.
    pub query: &'a mut String,
    /// Which commands are enabled, for the context menu.
    pub registry: &'a CommandRegistry,
    /// A row to scroll into view.
    pub reveal: Option<&'a NodeId>,
    /// The row being renamed, with the name typed so far.
    pub renaming: &'a mut Option<(NodeId, String)>,
    /// The layers whose rows are being dragged.
    pub dragging: &'a mut Option<Vec<NodeId>>,
}

/// What happened in the layers panel.
#[derive(Debug, Default)]
pub(super) struct LayersOutput {
    /// Commands chosen from a row's context menu.
    pub commands: Vec<CommandId>,
    /// An edit was refused: why.
    pub error: Option<String>,
}

/// Draws the layers panel: a header, a search field and the layer tree.
pub(super) fn layers_panel(ui: &mut Ui, panel: &mut LayersPanel<'_>) -> LayersOutput {
    let theme = Theme::of(ui);
    let mut out = LayersOutput::default();

    section_header(ui, "Layers", None, |ui| {
        let add = ui.add(
            IconButton::new(icons::PLUS)
                .dimmed(true)
                .tooltip("Add artboard"),
        );
        if add.clicked() {
            add_artboard(panel.editor, &mut out);
        }
    });
    padded(ui, |ui| {
        control_row(ui, |ui| {
            search_field(ui, panel.query);
        });
    });
    ui.add_space(theme.space.xs);

    let document = std::sync::Arc::clone(panel.editor.document());
    let rows = layer_rows(&document, panel.tree, panel.query);
    if rows.is_empty() {
        if panel.query.trim().is_empty() {
            empty_state(
                ui,
                icons::FRAME_CORNERS,
                "No artboards",
                "Draw one with the Artboard tool (A), or add one with +.",
                None,
            );
        } else {
            empty_state(
                ui,
                icons::MAGNIFYING_GLASS,
                "No layers found",
                "No layer name contains what you typed.",
                None,
            );
        }
        return out;
    }

    let delete_shortcut = CommandId::Delete.shortcut_label().unwrap_or_default();
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut drop: Option<Drop> = None;
    let mut dropped = false;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows {
                if let Some((renamed, name)) = panel.renaming.as_mut()
                    && *renamed == row.id
                {
                    if let Some(done) = rename_field(ui, row, name) {
                        let id = renamed.clone();
                        let name = name.trim().to_owned();
                        *panel.renaming = None;
                        if done && !name.is_empty() {
                            rename(panel.editor, &id, &name, &mut out);
                        }
                    }
                    continue;
                }
                let selected = panel.editor.selection.contains(&row.id);
                // Rows inside a selected layer get a faint tint.
                let inside_selected = !selected
                    && document
                        .ancestors(&row.id)
                        .any(|ancestor| panel.editor.selection.contains(&ancestor.id));
                let mut list_row = ListRow::new(&row.name)
                    .icon(layer_icon(row.node_type))
                    .depth(row.depth)
                    .selected(selected)
                    .parent_selected(inside_selected)
                    .dimmed(row.hidden)
                    .strong(row.depth == 0 && row.node_type == NodeType::Artboard)
                    .id_salt(row.id.as_str());
                if let Some(expanded) = row.expanded {
                    list_row = list_row.expanded(expanded);
                }

                let (mut hidden, mut locked) = (row.hidden, row.locked);
                let is_artboard = row.node_type == NodeType::Artboard;
                let shown = list_row.show(ui, |ui, hovered| {
                    if !is_artboard {
                        row_toggle(ui, &mut hidden, icons::EYE_SLASH, icons::EYE, hovered);
                    }
                    row_toggle(
                        ui,
                        &mut locked,
                        icons::LOCK_SIMPLE,
                        icons::LOCK_SIMPLE_OPEN,
                        hovered,
                    );
                });
                if panel.reveal == Some(&row.id) {
                    shown.response.scroll_to_me(None);
                }

                // Dragging rows to reorder and regroup.
                if shown.response.drag_started() {
                    let ids = if panel.editor.selection.contains(&row.id) {
                        panel.editor.selection.ids().to_vec()
                    } else {
                        panel.editor.selection.select(row.id.clone());
                        vec![row.id.clone()]
                    };
                    *panel.dragging = Some(ids);
                }
                let rect = shown.response.rect;
                if let (Some(moving), Some(pointer)) = (panel.dragging.as_ref(), pointer)
                    && rect.y_range().contains(pointer.y)
                    && !moving.contains(&row.id)
                {
                    let moving_artboards = moving.iter().any(|id| {
                        document
                            .get(id)
                            .is_some_and(|node| node.node_type() == NodeType::Artboard)
                    });
                    let fraction = (pointer.y - rect.top()) / rect.height().max(1.0);
                    drop = drop_at(row, fraction, moving_artboards);
                    if let Some(at) = &drop {
                        paint_drop(ui, rect, at);
                    }
                }
                if shown.response.drag_stopped() {
                    dropped = true;
                }

                context_menu(&shown.response, |ui| {
                    for id in [CommandId::Duplicate, CommandId::Group, CommandId::Ungroup] {
                        if command_item(ui, id, panel.registry, None) {
                            out.commands.push(id);
                        }
                    }
                    menu_separator(ui);
                    for id in [
                        CommandId::BringToFront,
                        CommandId::BringForward,
                        CommandId::SendBackward,
                        CommandId::SendToBack,
                    ] {
                        if command_item(ui, id, panel.registry, None) {
                            out.commands.push(id);
                        }
                    }
                    menu_separator(ui);
                    let delete = MenuItem::new(CommandId::Delete.name())
                        .icon(icons::TRASH)
                        .shortcut(&delete_shortcut)
                        .danger(true);
                    if ui
                        .add_enabled(panel.registry.is_enabled(CommandId::Delete), delete)
                        .clicked()
                    {
                        out.commands.push(CommandId::Delete);
                    }
                });

                if hidden != row.hidden {
                    let summary = if hidden { "Hide layer" } else { "Show layer" };
                    toggle(panel.editor, row, summary, "hidden", hidden, &mut out);
                } else if locked != row.locked {
                    let summary = if locked { "Lock layer" } else { "Unlock layer" };
                    toggle(panel.editor, row, summary, "locked", locked, &mut out);
                } else if shown.toggled {
                    let open = !row.expanded.unwrap_or(false);
                    panel.tree.set_expanded(&document, &row.id, open);
                } else if shown.response.double_clicked() {
                    *panel.renaming = Some((row.id.clone(), row.name.clone()));
                } else if shown.response.clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    if modifiers.shift || modifiers.command {
                        panel.editor.selection.toggle(row.id.clone());
                    } else {
                        panel.editor.selection.select(row.id.clone());
                    }
                } else if shown.response.secondary_clicked()
                    && !panel.editor.selection.contains(&row.id)
                {
                    panel.editor.selection.select(row.id.clone());
                }
            }
            ui.add_space(theme.space.md);
        });
    if dropped {
        if let (Some(ids), Some(drop)) = (panel.dragging.take(), drop) {
            let edits = drop_edits(&document, &ids, &drop);
            if let Err(error) = panel.editor.apply("Move in layers", &edits) {
                out.error = Some(error.problem.message);
            }
        }
        *panel.dragging = None;
    }
    out
}

/// A text field in the place of a row, for renaming it. Returns
/// `Some(true)` when the name is to be kept, `Some(false)` when renaming
/// was cancelled, and `None` while it goes on.
fn rename_field(ui: &mut Ui, row: &LayerRow, name: &mut String) -> Option<bool> {
    let theme = Theme::of(ui);
    let indent = theme.space.md
        + theme.space.xs
        + row.depth as f32 * theme.size.list_indent
        + 16.0
        + theme.size.icon
        + theme.space.sm;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.list_row),
        egui::Sense::hover(),
    );
    let field_rect = Rect::from_min_max(
        pos2(rect.left() + indent - theme.space.md, rect.top()),
        pos2(rect.right() - theme.space.md, rect.bottom()),
    );
    let id = ui.make_persistent_id(("rename", row.id.as_str()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
    let field = TextField::new(name)
        .id(id)
        .name("Layer name")
        .show(&mut child);
    if !field.response.has_focus() && !field.response.lost_focus() {
        field.response.request_focus();
    }
    if field.response.lost_focus() {
        let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
        return Some(!cancelled);
    }
    None
}

/// Renames a layer.
fn rename(editor: &mut Editor, id: &NodeId, name: &str, out: &mut LayersOutput) {
    if let Err(error) = editor.apply("Rename", &[Edit::set(id.clone(), "name", name)]) {
        out.error = Some(error.problem.message);
    }
}

/// Shows where a dragged row would land: a line above or below the row,
/// or a frame around it.
fn paint_drop(ui: &Ui, rect: Rect, drop: &Drop) {
    let theme = Theme::of(ui);
    let accent = theme.color.accent;
    let painter = ui.painter();
    let inset = rect.shrink2(Vec2::new(theme.space.md, 0.0));
    match drop {
        Drop::Above(_) => {
            painter.hline(inset.x_range(), inset.top(), Stroke::new(2.0, accent));
        }
        Drop::Below(_) => {
            painter.hline(inset.x_range(), inset.bottom(), Stroke::new(2.0, accent));
        }
        Drop::Inside(_) => {
            painter.rect_stroke(
                inset,
                theme.radius.sm,
                Stroke::new(1.5, accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Sets a yes-or-no property of a row's layer.
fn toggle(
    editor: &mut Editor,
    row: &LayerRow,
    summary: &str,
    field: &str,
    value: bool,
    out: &mut LayersOutput,
) {
    if let Err(error) = editor.apply(summary, &[Edit::set(row.id.clone(), field, value)]) {
        out.error = Some(error.problem.message);
    }
}

/// Adds an empty 1080 by 1080 artboard to the right of the others and
/// selects it.
fn add_artboard(editor: &mut Editor, out: &mut LayersOutput) {
    let document = std::sync::Arc::clone(editor.document());
    let names = document
        .artboards()
        .iter()
        .filter_map(|id| document.get(id))
        .map(|node| node.name());
    let name = next_name("Scene", names);
    match build::add_artboard(&document, &name, 1080.0, 1080.0) {
        Ok((id, edits)) => match editor.apply("Add artboard", &edits) {
            Ok(_) => editor.selection.select(id),
            Err(error) => out.error = Some(error.problem.message),
        },
        Err(error) => out.error = Some(error.to_string()),
    }
}
