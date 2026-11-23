//! The Animate tab: what the selected layers do over time.
//!
//! The tab lists the actions of the selected layers in time order, with an
//! "Add action" menu holding every action that works on all of them. A new
//! action starts at the playhead; with several layers selected each gets
//! one, a stagger apart. Clicking an action selects it, here and on the
//! timeline, and its settings appear below the list. With several actions
//! selected the tab offers what works on them together: spacing them out,
//! grouping, duplicating and deleting.

use egui::{Popup, PopupCloseBehavior, RichText, ScrollArea, Ui, Vec2};
use moshionly_core::{Node, NodeId, NodeType};

use super::{Bind, action};
use crate::{
    editor::{
        actions::{MenuSection, actions_of, menu_for},
        describe::{display_name, type_label},
        inspect::Inspected,
        timeline::action_icon,
    },
    theme::{Theme, icons},
    widgets::{
        IconButton, ListRow, MenuItem, NumericField, TextButton, divider, empty_state,
        menu_heading, menu_separator, numeric::format_value, padded, popup_frame, property_row,
        section_header,
    },
};

/// The tallest the add menu grows before it scrolls, in points.
const MENU_HEIGHT: f32 = 420.0;

/// The Animate tab for the selected layers.
pub(super) fn animate_tab(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let theme = Theme::of(ui);
    let document = std::sync::Arc::clone(bind.editor.document());
    let layers = inspected.ids();
    let several_layers = layers.len() > 1;
    let actions = actions_of(&document, &layers);
    let menu = menu_for(&document, &layers);

    divider(ui);
    let mut chosen: Option<NodeType> = None;
    section_header(ui, "Actions", Some(&mut bind.state.open.actions), |ui| {
        if !menu.is_empty() {
            chosen = add_menu_button(ui, &menu);
        }
    });
    if bind.state.open.actions {
        if several_layers && !menu.is_empty() {
            padded(ui, |ui| {
                stagger_row(ui, &mut bind.state.stagger, "Stagger new")
            });
        }
        if actions.is_empty() {
            empty_state(
                ui,
                icons::LIGHTNING,
                "No actions yet",
                "Actions make this layer move, fade or change over time. Add one at the playhead.",
                None,
            );
            if !menu.is_empty() {
                padded(ui, |ui| {
                    let button = TextButton::primary("Add action")
                        .icon(icons::PLUS)
                        .fill_width(true);
                    let response = ui.add(button);
                    if let Some(ty) = add_menu(ui, &response, &menu) {
                        chosen = Some(ty);
                    }
                });
                ui.add_space(theme.space.md);
            }
        } else {
            action_list(ui, bind, &document, &actions, several_layers);
            ui.add_space(theme.space.md);
        }
    }
    if let Some(ty) = chosen {
        let stagger = if several_layers {
            bind.state.stagger
        } else {
            0.0
        };
        if let Err(error) = bind.editor.add_actions(ty, stagger) {
            bind.out.error = Some(error.to_string());
        }
    }

    let selected: Vec<NodeId> = bind.editor.actions().to_vec();
    match selected.as_slice() {
        [] => {}
        [one] => {
            if let Some(node) = bind.editor.document().get(one).cloned() {
                action::action_settings(ui, bind, &node);
            }
        }
        several => several_actions(ui, bind, several.len()),
    }
}

/// The "+" button in the section header and its menu. Returns the type
/// chosen.
fn add_menu_button(ui: &mut Ui, menu: &[(MenuSection, NodeType)]) -> Option<NodeType> {
    let response = ui.add(IconButton::new(icons::PLUS).tooltip("Add action"));
    add_menu(ui, &response, menu)
}

/// The menu of actions that opens under `response`, by section. Returns the
/// type chosen.
fn add_menu(
    ui: &Ui,
    response: &egui::Response,
    menu: &[(MenuSection, NodeType)],
) -> Option<NodeType> {
    let theme = Theme::of(ui);
    let mut chosen = None;
    Popup::menu(response)
        .frame(popup_frame(ui))
        .gap(theme.space.xs)
        .width(theme.size.menu_width)
        .close_behavior(PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            ScrollArea::vertical()
                .max_height(MENU_HEIGHT)
                .show(ui, |ui| {
                    let mut section = None;
                    for (part, ty) in menu {
                        if section != Some(*part) {
                            if section.is_some() {
                                menu_separator(ui);
                            }
                            menu_heading(ui, part.label());
                            section = Some(*part);
                        }
                        let item = MenuItem::new(type_label(*ty)).icon(action_icon(*ty));
                        if ui.add(item).clicked() {
                            chosen = Some(*ty);
                        }
                    }
                });
        });
    chosen
}

/// A field for the delay between actions, in milliseconds.
fn stagger_row(ui: &mut Ui, stagger: &mut f64, label: &str) -> bool {
    property_row(ui, label, |ui| {
        NumericField::new(stagger, "Stagger")
            .icon(icons::TIMER)
            .unit("ms")
            .range(0.0..=10_000.0)
            .step(10.0)
            .decimals(0)
            .show(ui)
            .response
            .changed()
    })
}

/// The actions as rows: icon, name and time span. A click selects one;
/// Shift or ⌘ adds it to the selection or takes it away.
fn action_list(
    ui: &mut Ui,
    bind: &mut Bind<'_>,
    document: &moshionly_core::Document,
    actions: &[NodeId],
    several_layers: bool,
) {
    let theme = Theme::of(ui);
    let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    let mut clicked: Option<NodeId> = None;
    for id in actions {
        let Some(node) = document.get(id) else {
            continue;
        };
        let mut name = action_name(node);
        if several_layers
            && let Some(target) = node.target().and_then(|target| document.get(target))
        {
            name = format!("{} · {name}", display_name(target));
        }
        let (start, end) = node.data.span().unwrap_or((0.0, 0.0));
        let selected = bind.editor.actions().contains(id);
        let row = ListRow::new(&name)
            .icon(action_icon(node.node_type()))
            .selected(selected)
            .id_salt(("action", id.as_str()))
            .show(ui, |ui, _| {
                ui.add_space(theme.space.md);
                ui.label(
                    RichText::new(time_span(start, end))
                        .font(theme.small())
                        .color(theme.color.text_secondary),
                );
            });
        if row.response.clicked() {
            clicked = Some(id.clone());
        }
    }
    if let Some(id) = clicked {
        let mut next: Vec<NodeId> = bind.editor.actions().to_vec();
        if toggle {
            match next.iter().position(|selected| *selected == id) {
                Some(at) => {
                    next.remove(at);
                }
                None => next.push(id),
            }
        } else {
            next = vec![id];
        }
        bind.editor.set_actions(next);
    }
}

/// What several selected actions can do together.
fn several_actions(ui: &mut Ui, bind: &mut Bind<'_>, count: usize) {
    let theme = Theme::of(ui);
    divider(ui);
    let title = format!("{count} actions");
    let mut duplicate = false;
    let mut delete = false;
    section_header(ui, &title, None, |ui| {
        delete = ui
            .add(IconButton::new(icons::TRASH).tooltip("Delete actions"))
            .clicked();
        duplicate = ui
            .add(IconButton::new(icons::COPY).tooltip("Duplicate actions"))
            .clicked();
    });
    let mut stagger = false;
    let mut group = false;
    let mut ungroup = false;
    padded(ui, |ui| {
        stagger_row(ui, &mut bind.state.stagger, "Delay");
        ui.add_space(theme.space.sm);
        stagger = ui
            .add(
                TextButton::new("Play one after another")
                    .icon(icons::ROWS)
                    .fill_width(true),
            )
            .clicked();
        ui.add_space(theme.space.sm);
        ui.horizontal(|ui| {
            group = ui
                .add(TextButton::new("Group").icon(icons::FOLDER_SIMPLE))
                .clicked();
            ungroup = ui.add(TextButton::new("Ungroup")).clicked();
        });
    });
    ui.add_space(theme.space.md);
    let result = if stagger {
        bind.editor.stagger_actions(bind.state.stagger).map(drop)
    } else if group {
        bind.editor.group_actions().map(drop)
    } else if ungroup {
        bind.editor.ungroup_actions().map(drop)
    } else if duplicate {
        bind.editor.duplicate_actions().map(drop)
    } else if delete {
        bind.editor.delete_actions().map(drop)
    } else {
        Ok(())
    };
    if let Err(error) = result {
        bind.out.error = Some(error.to_string());
    }
}

/// The name an action is shown with: its own, or its type's.
pub(super) fn action_name(node: &Node) -> String {
    match node.name().trim() {
        "" => type_label(node.node_type()).to_owned(),
        name => name.to_owned(),
    }
}

/// A time range as text: `0.2 – 1.1 s`, or one moment: `0.2 s`.
pub(super) fn time_span(start: f64, end: f64) -> String {
    if start == end {
        return format!("{} s", format_value(start / 1000.0, 2));
    }
    format!(
        "{} – {} s",
        format_value(start / 1000.0, 2),
        format_value(end / 1000.0, 2)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_read_in_seconds() {
        assert_eq!(time_span(200.0, 1100.0), "0.2 – 1.1 s");
        assert_eq!(time_span(0.0, 4000.0), "0 – 4 s");
        assert_eq!(time_span(1250.0, 1500.0), "1.25 – 1.5 s");
        assert_eq!(time_span(800.0, 800.0), "0.8 s");
    }
}
