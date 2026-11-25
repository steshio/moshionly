//! The layers panel on the left of the window: the document's tree, and
//! the file's main components to place instances of.

use egui::{
    Align, CursorIcon, Key, Layout, Order, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui,
    UiBuilder, Vec2, pos2,
};
use moshionly_core::{Edit, NodeId, NodeType, build};

use super::parts::command_item;
use crate::{
    commands::{CommandId, CommandRegistry},
    editor::{
        Editor,
        components::{ComponentEntry, components},
        describe::next_name,
        timeline::{layer_icon, layer_tone},
        tree::{Drop, LayerRow, TreeState, drop_at, drop_edits, layer_rows},
    },
    theme::{Theme, Tone, icons},
    widgets::{
        IconButton, ListRow, MenuItem, TextField, context_menu, control_row, empty_state,
        menu_heading, menu_separator, padded, row_toggle, search_field, tabs,
    },
};

/// What the layers panel lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayersTab {
    /// The layer tree.
    #[default]
    Layers,
    /// The file's main components, to place instances of.
    Components,
}

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
    /// What the panel lists.
    pub tab: &'a mut LayersTab,
    /// The main component whose row is being dragged to the canvas.
    pub component_drag: &'a mut Option<NodeId>,
}

/// What happened in the layers panel.
#[derive(Debug, Default)]
pub(super) struct LayersOutput {
    /// Commands chosen from a row's context menu.
    pub commands: Vec<CommandId>,
    /// An edit was refused: why.
    pub error: Option<String>,
    /// Place an instance of this main component: where its row was let go
    /// (a point on the screen), or in the middle of the current artboard
    /// for a click.
    pub place: Option<(NodeId, Option<Pos2>)>,
    /// Show this main component: select it and bring it into view.
    pub show: Option<NodeId>,
}

/// Draws the layers panel: the Layers and Components tabs, a search field,
/// and the layer tree or the list of components.
pub(super) fn layers_panel(ui: &mut Ui, panel: &mut LayersPanel<'_>) -> LayersOutput {
    let theme = Theme::of(ui);
    let mut out = LayersOutput::default();

    if header(ui, panel.tab) {
        add_artboard(panel.editor, &mut out);
    }
    padded(ui, |ui| {
        control_row(ui, |ui| {
            search_field(ui, panel.query);
        });
    });
    ui.add_space(theme.space.xs);
    if *panel.tab == LayersTab::Components {
        components_list(ui, panel, &mut out);
        return out;
    }
    *panel.component_drag = None;

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
    // Escape lets go of rows being dragged: nothing moves.
    if panel.dragging.is_some() && ui.input(|i| i.key_pressed(Key::Escape)) {
        *panel.dragging = None;
    }
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
                let tint = layer_tone(row.node_type).map(|tone| theme.color.tone(tone).text);
                let mut list_row = ListRow::new(&row.name)
                    .icon(layer_icon(row.node_type))
                    .tint(tint)
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
                // Only a row the pointer is really over takes the drop: not
                // one at the same height while the pointer is over the
                // canvas, nor one scrolled out of view.
                if let (Some(moving), Some(pointer)) = (panel.dragging.as_ref(), pointer)
                    && rect.contains(pointer)
                    && ui.clip_rect().contains(pointer)
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
                    // The component commands that apply to the selection.
                    let components: Vec<CommandId> = [
                        CommandId::CreateComponent,
                        CommandId::InsertInstance,
                        CommandId::GoToMainComponent,
                        CommandId::DetachInstance,
                    ]
                    .into_iter()
                    .filter(|id| panel.registry.is_enabled(*id))
                    .collect();
                    if !components.is_empty() {
                        menu_separator(ui);
                    }
                    for id in components {
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
                        // About the layer, even one already selected: its
                        // actions are let go.
                        panel.editor.selection.select(row.id.clone());
                        panel.editor.set_actions([]);
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

/// The panel's header: the Layers and Components tabs, and on the Layers
/// tab a button that adds an artboard. Returns whether that was clicked.
fn header(ui: &mut Ui, tab: &mut LayersTab) -> bool {
    let theme = Theme::of(ui);
    let pad = theme.space.xl;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.section_header),
        Sense::hover(),
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + pad, rect.top()),
        pos2(rect.right() - pad + theme.space.xs, rect.bottom()),
    );
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
    );
    tabs(
        &mut row,
        tab,
        &[
            (LayersTab::Layers, "Layers"),
            (LayersTab::Components, "Components"),
        ],
    );
    let mut add = false;
    if *tab == LayersTab::Layers {
        row.with_layout(Layout::right_to_left(Align::Center), |ui| {
            add = ui
                .add(
                    IconButton::new(icons::PLUS)
                        .dimmed(true)
                        .tooltip("Add artboard"),
                )
                .clicked();
        });
    }
    add
}

/// How many instances a component has, in a few words.
pub(super) fn instance_count(instances: usize) -> String {
    match instances {
        0 => "No instances".to_owned(),
        1 => "1 instance".to_owned(),
        count => format!("{count} instances"),
    }
}

/// The components whose name contains `query`, ignoring case, by artboard
/// in the order of the layer tree.
fn matching(entries: Vec<ComponentEntry>, query: &str) -> Vec<ComponentEntry> {
    let query = query.trim().to_lowercase();
    entries
        .into_iter()
        .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
        .collect()
}

/// The Components tab: every main component in the file, under the name of
/// its artboard. A click on a row places an instance in the middle of the
/// current artboard; dragging a row onto the canvas places one where it is
/// let go.
fn components_list(ui: &mut Ui, panel: &mut LayersPanel<'_>, out: &mut LayersOutput) {
    let theme = Theme::of(ui);
    let document = std::sync::Arc::clone(panel.editor.document());
    let all = components(&document);
    if all.is_empty() {
        *panel.component_drag = None;
        empty_state(
            ui,
            icons::DIAMONDS_FOUR,
            "No components yet",
            "Select layers and choose Create component (⌥⌘K) to reuse them: edit the main and every instance follows.",
            None,
        );
        return;
    }
    let entries = matching(all, panel.query);
    if entries.is_empty() {
        empty_state(
            ui,
            icons::MAGNIFYING_GLASS,
            "No components found",
            "No component name contains what you typed.",
            None,
        );
    }

    // A drag let go anywhere places an instance there, if that is the
    // canvas; Escape lets go of it without placing anything.
    let (released, escape, pointer) = ui.input(|i| {
        (
            i.pointer.primary_released(),
            i.key_pressed(Key::Escape),
            i.pointer.latest_pos(),
        )
    });
    if escape {
        *panel.component_drag = None;
    }
    if released && let Some(main) = panel.component_drag.take() {
        out.place = Some((main, pointer));
    }

    let violet = theme.color.tone(Tone::Violet).text;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut artboard: Option<&str> = None;
            for entry in &entries {
                if artboard != Some(entry.artboard.as_str()) {
                    artboard = Some(entry.artboard.as_str());
                    padded(ui, |ui| menu_heading(ui, &entry.artboard));
                }
                let count = instance_count(entry.instances);
                let mut show = false;
                let shown = ListRow::new(&entry.name)
                    .icon(icons::DIAMONDS_FOUR)
                    .tint(Some(violet))
                    .selected(panel.editor.selection.contains(&entry.id))
                    .id_salt(format!("component {}", entry.id))
                    .show(ui, |ui, hovered| {
                        if hovered {
                            show = ui
                                .add(
                                    IconButton::new(icons::ARROW_SQUARE_OUT)
                                        .dimmed(true)
                                        .tooltip("Go to main component"),
                                )
                                .clicked();
                        }
                        ui.label(
                            RichText::new(&count)
                                .font(theme.small())
                                .color(theme.color.text_tertiary),
                        );
                    });
                let response = shown
                    .response
                    .on_hover_text("Click to place an instance, or drag it onto the canvas");
                if show {
                    out.show = Some(entry.id.clone());
                } else if response.drag_started() {
                    *panel.component_drag = Some(entry.id.clone());
                } else if response.clicked() {
                    out.place = Some((entry.id.clone(), None));
                }
            }
            ui.add_space(theme.space.md);
        });

    // What is being dragged follows the pointer.
    if let (Some(main), Some(pointer)) = (panel.component_drag.as_ref(), pointer)
        && let Some(entry) = entries.iter().find(|entry| entry.id == *main)
    {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        drag_label(ui, pointer, &entry.name);
    }
}

/// A small label with the dragged component's name, beside the pointer.
fn drag_label(ui: &Ui, pointer: Pos2, name: &str) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        Order::Tooltip,
        ui.make_persistent_id("component-drag"),
    ));
    let galley = painter.layout_no_wrap(name.to_owned(), theme.body(), c.text);
    let icon = theme.size.icon;
    let pad = theme.space.sm;
    let size = Vec2::new(
        pad + icon + theme.space.xs + galley.size().x + pad,
        theme.size.control,
    );
    let rect = Rect::from_min_size(pointer + Vec2::new(12.0, 12.0), size);
    painter.rect_filled(rect, theme.radius.sm, c.raised);
    painter.rect_stroke(
        rect,
        theme.radius.sm,
        Stroke::new(1.0, c.tone(Tone::Violet).outline),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        &painter,
        pos2(rect.left() + pad + icon / 2.0, rect.center().y),
        icons::DIAMONDS_FOUR,
        icon,
        c.tone(Tone::Violet).text,
    );
    let text_pos = pos2(
        rect.left() + pad + icon + theme.space.xs,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(text_pos, galley, c.text);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_rows_count_their_instances() {
        assert_eq!(instance_count(0), "No instances");
        assert_eq!(instance_count(1), "1 instance");
        assert_eq!(instance_count(4), "4 instances");
    }

    #[test]
    fn the_search_field_filters_components_by_name() {
        let entry = |name: &str| ComponentEntry {
            id: NodeId::from(name),
            name: name.to_owned(),
            artboard: "Scene".to_owned(),
            instances: 0,
        };
        let all = vec![entry("Card"), entry("Badge"), entry("Big card")];
        let names = |query: &str| -> Vec<String> {
            matching(all.clone(), query)
                .into_iter()
                .map(|entry| entry.name)
                .collect()
        };
        assert_eq!(names(""), ["Card", "Badge", "Big card"]);
        assert_eq!(names(" CARD "), ["Card", "Big card"]);
        assert!(names("nothing").is_empty());
    }
}
