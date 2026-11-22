//! The layers panel on the left of the window.

use egui::{ScrollArea, Ui};

use super::{
    parts::{command_item, sample_tag},
    sample::Sample,
};
use crate::{
    commands::{CommandId, CommandRegistry},
    theme::{Theme, icons},
    widgets::{
        IconButton, ListRow, MenuItem, context_menu, control_row, empty_state, menu_separator,
        padded, row_toggle, search_field, section_header,
    },
};

/// Draws the layers panel: a header, a search field and the layer tree.
///
/// Selection, expanding, hiding and locking act on the sample scene. Returns
/// the commands chosen from a row's context menu.
pub fn layers_panel(
    ui: &mut Ui,
    sample: &mut Sample,
    registry: &CommandRegistry,
) -> Vec<CommandId> {
    let theme = Theme::of(ui);
    let mut commands = Vec::new();

    section_header(ui, "Layers", None, |ui| {
        ui.add(
            IconButton::new(icons::PLUS)
                .dimmed(true)
                .tooltip("Add artboard"),
        );
        sample_tag(ui);
    });
    padded(ui, |ui| {
        control_row(ui, |ui| {
            search_field(ui, &mut sample.layer_query);
        });
    });
    ui.add_space(theme.space.xs);

    let visible = sample.visible_layers();
    if visible.is_empty() {
        empty_state(
            ui,
            icons::MAGNIFYING_GLASS,
            "No layers found",
            "No layer name contains what you typed.",
            None,
        );
        return commands;
    }

    let searching = !sample.layer_query.trim().is_empty();
    let delete_shortcut = CommandId::Delete.shortcut_label().unwrap_or_default();
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut selected_depth: Option<usize> = None;
            for index in visible {
                let layer = &sample.layers[index];
                let selected = sample.selected_layer.as_ref() == Some(&layer.key);
                // Rows inside the selected layer get a faint tint.
                if selected {
                    selected_depth = Some(layer.depth);
                } else if selected_depth.is_some_and(|depth| layer.depth <= depth) {
                    selected_depth = None;
                }
                let inside_selected = !selected && selected_depth.is_some();

                let mut row = ListRow::new(layer.name)
                    .icon(layer.icon)
                    .depth(if searching { 0 } else { layer.depth })
                    .selected(selected)
                    .parent_selected(inside_selected)
                    .dimmed(layer.hidden)
                    .strong(layer.depth == 0)
                    .id_salt(layer.key.as_str());
                if let Some(expanded) = layer.expanded
                    && !searching
                {
                    row = row.expanded(expanded);
                }

                let (mut hidden, mut locked) = (layer.hidden, layer.locked);
                let shown = row.show(ui, |ui, hovered| {
                    row_toggle(ui, &mut hidden, icons::EYE_SLASH, icons::EYE, hovered);
                    row_toggle(
                        ui,
                        &mut locked,
                        icons::LOCK_SIMPLE,
                        icons::LOCK_SIMPLE_OPEN,
                        hovered,
                    );
                });

                context_menu(&shown.response, |ui| {
                    for id in [CommandId::Duplicate, CommandId::Group, CommandId::Ungroup] {
                        if command_item(ui, id, registry, None) {
                            commands.push(id);
                        }
                    }
                    menu_separator(ui);
                    let delete = MenuItem::new(CommandId::Delete.name())
                        .icon(icons::TRASH)
                        .shortcut(&delete_shortcut)
                        .danger(true);
                    if ui
                        .add_enabled(registry.is_enabled(CommandId::Delete), delete)
                        .clicked()
                    {
                        commands.push(CommandId::Delete);
                    }
                });

                let key = layer.key.clone();
                let layer = &mut sample.layers[index];
                layer.hidden = hidden;
                layer.locked = locked;
                if shown.toggled
                    && let Some(expanded) = &mut layer.expanded
                {
                    *expanded = !*expanded;
                } else if shown.response.clicked() || shown.response.secondary_clicked() {
                    sample.select(Some(key));
                }
            }
            ui.add_space(theme.space.md);
        });
    commands
}
