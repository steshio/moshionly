//! The Animate tab: the actions of the selected layer, read-only for now.
//!
//! TODO: adding, editing and removing actions is the next task (the
//! editable timeline and the Animate tab).

use egui::{RichText, Ui};
use moshionly_core::{Family, Node};

use super::Bind;
use crate::{
    editor::{describe::type_label, inspect::Inspected, timeline::action_icon},
    theme::{Theme, icons},
    widgets::{ListRow, divider, empty_state, numeric::format_value, section_header},
};

/// Lists the actions of the first selected layer with their times.
pub(super) fn animate_tab(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let theme = Theme::of(ui);
    let Some(id) = inspected.nodes().first().map(|node| node.id.clone()) else {
        return;
    };
    let document = std::sync::Arc::clone(bind.editor.document());
    let artboard = document.artboard_of(&id).map(|node| node.id.clone());
    let actions: Vec<&Node> = artboard
        .iter()
        .flat_map(|artboard| document.walk(artboard, Family::Animation))
        .map(|(_, node)| node)
        .filter(|node| node.target() == Some(&id))
        .collect();

    divider(ui);
    section_header(ui, "Actions", Some(&mut bind.state.open.actions), |_| {});
    if !bind.state.open.actions {
        return;
    }
    if actions.is_empty() {
        empty_state(
            ui,
            icons::LIGHTNING,
            "No actions yet",
            "Actions make this layer move, fade or change over time.",
            None,
        );
        return;
    }
    for action in actions {
        let (start, end) = action.data.span().unwrap_or((0.0, 0.0));
        let name = match action.name().trim() {
            "" => type_label(action.node_type()).to_owned(),
            name => name.to_owned(),
        };
        ListRow::new(&name)
            .icon(action_icon(action.node_type()))
            .id_salt(action.id.as_str())
            .show(ui, |ui, _| {
                ui.add_space(theme.space.md);
                ui.label(
                    RichText::new(time_span(start, end))
                        .font(theme.small())
                        .color(theme.color.text_secondary),
                );
            });
    }
    ui.add_space(theme.space.md);
}

/// A time range as text: `0.2 – 1.1 s`.
fn time_span(start: f64, end: f64) -> String {
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
    }
}
