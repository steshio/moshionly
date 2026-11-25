//! The Component section of the Design tab: which main an instance shows
//! and how much later it plays the main's animations, and how many
//! instances a main has.

use egui::{Label, RichText, Ui};
use moshionly_core::model::NodeData;
use moshionly_core::{Document, NodeId, NodeType};

use super::{Bind, numeric};
use crate::{
    commands::CommandId,
    editor::{
        describe::display_name,
        inspect::{Inspected, set_all, tidy},
    },
    theme::{Theme, Tone, icons},
    widgets::{ButtonKind, TextButton, control_row, property_row, section},
};

/// The furthest an instance's time offset goes either way: an hour, in
/// milliseconds.
const MAX_OFFSET: f64 = 3_600_000.0;

/// The Component section, when every selected layer is an instance or
/// every selected layer is a main component.
pub(super) fn component_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    if inspected.all_of(NodeType::Instance) {
        instance_section(ui, bind, inspected);
    } else if inspected.all_of(NodeType::Component) {
        main_section(ui, bind, inspected);
    }
}

/// The time offset of an instance.
fn time_offset(data: &NodeData) -> Option<f64> {
    match data {
        NodeData::Instance(instance) => Some(instance.time_offset),
        _ => None,
    }
}

/// What the Main row says: the name of the main the instances show.
fn main_label(document: &Document, mains: &[&NodeId]) -> String {
    match mains {
        [one] => document
            .get(one)
            .map_or_else(|| "Missing".to_owned(), display_name),
        _ => "Several components".to_owned(),
    }
}

/// "Main component · 3 instances".
pub(super) fn main_summary(instances: usize) -> String {
    match instances {
        1 => "Main component · 1 instance".to_owned(),
        count => format!("Main component · {count} instances"),
    }
}

/// A line of quiet, wrapping text under the controls.
fn note(ui: &mut Ui, text: &str) {
    let theme = Theme::of(ui);
    ui.add_space(theme.space.xs);
    ui.add(
        Label::new(
            RichText::new(text)
                .font(theme.small())
                .color(theme.color.text_tertiary),
        )
        .wrap(),
    );
    ui.add_space(theme.space.sm);
}

/// The main the selected instances show, their time offset, and the
/// buttons to go to the main or detach them.
fn instance_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let document = std::sync::Arc::clone(bind.editor.document());
    let ids = inspected.ids();
    let Some(offset) = inspected.number(|node| time_offset(&node.data)) else {
        return;
    };
    let mut mains: Vec<&NodeId> = inspected
        .nodes()
        .iter()
        .filter_map(|node| node.component())
        .collect();
    mains.sort();
    mains.dedup();
    let main = main_label(&document, &mains);
    let mut open = bind.state.open.component;
    let mut changed = None;
    let (mut go, mut detach) = (false, false);
    section(
        ui,
        "Component",
        &mut open,
        |_| {},
        |ui| {
            let theme = Theme::of(ui);
            let violet = theme.color.tone(Tone::Violet).text;
            property_row(ui, "Main", |ui| {
                ui.label(RichText::new(&main).font(theme.body_medium()).color(violet));
            });
            property_row(ui, "Time offset", |ui| {
                let seconds: &[(&str, f64)] = &[("s", 1000.0)];
                changed = Some(numeric(ui, &offset, "Time offset", |field| {
                    field
                        .icon(icons::TIMER)
                        .unit("ms")
                        .conversions(seconds)
                        .range(-MAX_OFFSET..=MAX_OFFSET)
                        .step(10.0)
                        .decimals(0)
                }));
            });
            note(
                ui,
                "The copy plays the main's animations this much later. \
                 Give each copy a little more to stagger them.",
            );
            control_row(ui, |ui| {
                go = ui
                    .add(
                        TextButton::new("Go to main")
                            .kind(ButtonKind::Quiet)
                            .icon(icons::ARROW_SQUARE_OUT),
                    )
                    .clicked();
                detach = ui
                    .add(
                        TextButton::new("Detach")
                            .kind(ButtonKind::Quiet)
                            .icon(icons::LINK_SIMPLE_BREAK)
                            .tooltip("Turn into a plain group that looks the same"),
                    )
                    .clicked();
            });
        },
    );
    bind.state.open.component = open;
    if let Some((response, value)) = changed {
        bind.field(&response, "Change time offset", || {
            set_all(&ids, "time_offset", tidy(value))
        });
    }
    if go {
        bind.out.commands.push(CommandId::GoToMainComponent);
    }
    if detach {
        bind.out.commands.push(CommandId::DetachInstance);
    }
}

/// How many instances the selected mains have, and a button to place one
/// more.
fn main_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let document = std::sync::Arc::clone(bind.editor.document());
    let instances: usize = inspected
        .nodes()
        .iter()
        .map(|node| document.instances_of(&node.id).len())
        .sum();
    let summary = main_summary(instances);
    let mut open = bind.state.open.component;
    let mut insert = false;
    section(
        ui,
        "Component",
        &mut open,
        |_| {},
        |ui| {
            let theme = Theme::of(ui);
            let violet = theme.color.tone(Tone::Violet).text;
            control_row(ui, |ui| {
                ui.label(
                    RichText::new(&summary)
                        .font(theme.body_medium())
                        .color(violet),
                );
            });
            note(
                ui,
                "Every instance shows this component and its animations: \
                 change it here and they all change.",
            );
            control_row(ui, |ui| {
                insert = ui
                    .add(
                        TextButton::new("Insert instance")
                            .kind(ButtonKind::Quiet)
                            .icon(icons::DIAMOND),
                    )
                    .clicked();
            });
        },
    );
    bind.state.open.component = open;
    if insert {
        bind.out.commands.push(CommandId::InsertInstance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_main_says_how_many_instances_it_has() {
        assert_eq!(main_summary(0), "Main component · 0 instances");
        assert_eq!(main_summary(1), "Main component · 1 instance");
        assert_eq!(main_summary(3), "Main component · 3 instances");
    }

    #[test]
    fn the_main_row_names_one_main() {
        let document = moshionly_core::io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"card","parent":"ab","index":"a0","type":"component","name":"Card"}
            ]}"##,
        )
        .unwrap();
        let card = NodeId::from("card");
        let gone = NodeId::from("gone");
        assert_eq!(main_label(&document, &[&card]), "Card");
        assert_eq!(main_label(&document, &[&gone]), "Missing");
        assert_eq!(main_label(&document, &[&card, &gone]), "Several components");
    }
}
