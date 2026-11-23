//! The settings of the one selected action: when it runs, how it is paced,
//! and what its type lets you choose.
//!
//! Times are shown in seconds and stored in milliseconds. A type's own
//! settings (the direction of a slide, what a text animation splits its
//! words into and so on) are read from the schema, so a setting added to a
//! type in the engine shows up here with a sensible control.

use egui::Ui;
use moshionly_core::schema::{self, FieldInfo};
use moshionly_core::{Edit, Node, NodeId, NodeType};
use serde_json::Value;

use super::{Bind, animate::action_name};
use crate::{
    editor::actions::retime,
    theme::icons,
    widgets::{
        Dropdown, IconButton, NumericField, NumericResponse, divider, padded, property_row,
        section_header, segmented,
    },
};

/// Fields every action has, which the timing and value sections show.
const COMMON: [&str; 11] = [
    "id", "parent", "index", "type", "name", "target", "start", "end", "easing", "from", "to",
];

/// Typed units for time fields: seconds, or milliseconds.
const SECONDS: &[(&str, f64)] = &[("ms", 0.001)];

/// The settings of the selected action `node`.
pub(super) fn action_settings(ui: &mut Ui, bind: &mut Bind<'_>, node: &Node) {
    let theme = crate::theme::Theme::of(ui);
    divider(ui);
    let mut duplicate = false;
    let mut delete = false;
    section_header(ui, &action_name(node), None, |ui| {
        delete = ui
            .add(IconButton::new(icons::TRASH).tooltip("Delete action"))
            .clicked();
        duplicate = ui
            .add(IconButton::new(icons::COPY).tooltip("Duplicate action"))
            .clicked();
    });
    padded(ui, |ui| timing(ui, bind, node));
    ui.add_space(theme.space.md);
    settings_section(ui, bind, node);

    let result = if delete {
        bind.editor.delete_actions().map(drop)
    } else if duplicate {
        bind.editor.duplicate_actions().map(drop)
    } else {
        Ok(())
    };
    if let Err(error) = result {
        bind.out.error = Some(error.to_string());
    }
}

/// A time field in seconds.
fn seconds_field(ui: &mut Ui, ms: f64, name: &str) -> (NumericResponse, f64) {
    let mut seconds = ms / 1000.0;
    let response = NumericField::new(&mut seconds, name)
        .unit("s")
        .conversions(SECONDS)
        .range(0.0..=3600.0)
        .step(0.05)
        .decimals(2)
        .id_salt(("action-time", name))
        .show(ui);
    (response, (seconds * 1000.0).round())
}

/// Start, end and length; one moment for `show` and `hide`.
fn timing(ui: &mut Ui, bind: &mut Bind<'_>, node: &Node) {
    let Some((start, end)) = node.data.span() else {
        return;
    };
    let id = node.id.clone();
    if matches!(node.node_type(), NodeType::Show | NodeType::Hide) {
        let (response, at) = property_row(ui, "At", |ui| seconds_field(ui, start, "At"));
        let document = std::sync::Arc::clone(bind.editor.document());
        bind.field(&response, "Change time", || {
            retime(&document, &id, at, at).into_iter().collect()
        });
        return;
    }
    let length = end - start;
    let (start_response, new_start) =
        property_row(ui, "Start", |ui| seconds_field(ui, start, "Start"));
    let (end_response, new_end) = property_row(ui, "End", |ui| seconds_field(ui, end, "End"));
    let (length_response, new_length) =
        property_row(ui, "Length", |ui| seconds_field(ui, length, "Length"));
    let document = std::sync::Arc::clone(bind.editor.document());
    let edits = |start: f64, end: f64| -> Vec<Edit> {
        retime(&document, &id, start, end).into_iter().collect()
    };
    // Moving the start keeps the length; the end and the length move the
    // end only.
    bind.field(&start_response, "Change start", || {
        edits(new_start, new_start + length)
    });
    bind.field(&end_response, "Change end", || {
        edits(start, new_end.max(start))
    });
    bind.field(&length_response, "Change length", || {
        edits(start, start + new_length)
    });
}

/// How a setting of an action type is shown.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Control {
    /// A number, shown multiplied by `scale` with `unit`.
    Number {
        /// The label.
        label: &'static str,
        /// The unit shown after the number.
        unit: &'static str,
        /// What the stored value is multiplied by to be shown.
        scale: f64,
        /// The lowest value that can be shown.
        min: f64,
        /// The highest value that can be shown.
        max: f64,
    },
    /// On or off.
    Flag {
        /// The label.
        label: &'static str,
    },
    /// One of a few words: each stored value with its label.
    Choice {
        /// The label.
        label: &'static str,
        /// Stored value and label of each option.
        options: Vec<(String, String)>,
    },
}

/// The label, unit, display scale and range of a numeric setting.
fn number_setting(ty: NodeType, name: &str) -> (&'static str, &'static str, f64, f64, f64) {
    match name {
        "distance" if matches!(ty, NodeType::TextIn | NodeType::TextOut) => {
            ("Distance", "%", 1.0, 0.0, 1000.0)
        }
        "distance" => ("Distance", "px", 1.0, 0.0, 10_000.0),
        "scale" => ("Scale", "%", 100.0, 0.0, 1000.0),
        "angle" => ("Angle", "°", 1.0, -3600.0, 3600.0),
        "blur" => ("Blur", "px", 1.0, 0.0, 500.0),
        "stagger" => ("Delay", "ms", 1.0, 0.0, 10_000.0),
        _ => ("Value", "", 1.0, -1e6, 1e6),
    }
}

/// The label of a setting that is not a number.
fn setting_label(name: &str) -> &'static str {
    match name {
        "direction" => "Direction",
        "fade" => "Fade",
        "anchor" => "Anchor",
        "split" => "Apply to",
        "effect" => "Effect",
        "order" => "Order",
        _ => "Setting",
    }
}

/// A word from a file as a label: `slide_up` reads "Slide up".
pub(super) fn word_label(word: &str) -> String {
    let spaced = word.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The words a field may hold, from its schema description:
/// `"left" | "right"` gives `left` and `right`.
fn choices(value: &str) -> Option<Vec<String>> {
    let words: Vec<String> = value
        .split(" | ")
        .map(|part| part.trim().trim_matches('"').to_owned())
        .collect();
    (value.contains('"') && words.iter().all(|word| !word.is_empty())).then_some(words)
}

/// How a field of an action type is shown, or `None` for fields the timing
/// and value sections show, and for kinds of value this panel cannot edit.
pub(super) fn control_for(ty: NodeType, field: &FieldInfo) -> Option<Control> {
    if COMMON.contains(&field.name.as_str()) {
        return None;
    }
    match field.value.as_str() {
        "number" => {
            let (label, unit, scale, min, max) = number_setting(ty, &field.name);
            Some(Control::Number {
                label,
                unit,
                scale,
                min,
                max,
            })
        }
        "boolean" => Some(Control::Flag {
            label: setting_label(&field.name),
        }),
        other => choices(other).map(|words| Control::Choice {
            label: setting_label(&field.name),
            options: words
                .into_iter()
                .map(|word| {
                    let label = word_label(&word);
                    (word, label)
                })
                .collect(),
        }),
    }
}

/// The settings of an action type with their controls, in file order.
pub(super) fn settings_of(ty: NodeType) -> Vec<(String, Control)> {
    schema::fields(ty)
        .iter()
        .filter_map(|field| Some((field.name.clone(), control_for(ty, field)?)))
        .collect()
}

/// The section of the type's own settings, when it has any.
fn settings_section(ui: &mut Ui, bind: &mut Bind<'_>, node: &Node) {
    let ty = node.node_type();
    let settings = settings_of(ty);
    if settings.is_empty() {
        return;
    }
    let values = node.to_full_value();
    let mut open = bind.state.open.settings;
    let mut changes: Vec<(String, Value, Option<NumericResponse>)> = Vec::new();
    crate::widgets::section(
        ui,
        "Settings",
        &mut open,
        |_| {},
        |ui| {
            for (name, control) in &settings {
                let current = &values[name.as_str()];
                if let Some(change) = setting_row(ui, &node.id, name, control, current) {
                    changes.push(change);
                }
            }
        },
    );
    bind.state.open.settings = open;
    let id = node.id.clone();
    for (name, value, response) in changes {
        let summary = format!("Change {}", setting_summary(&name));
        let edits = || vec![Edit::set(id.clone(), name.clone(), value.clone())];
        match response {
            Some(response) => bind.field(&response, &summary, edits),
            None => bind.apply(&summary, &edits()),
        }
    }
}

/// The words for a setting in an undo label: "Change direction".
fn setting_summary(name: &str) -> String {
    word_label(name).to_lowercase()
}

/// One setting's row. Returns the new value when it was changed, with the
/// field's response for numbers (which may change over many frames).
fn setting_row(
    ui: &mut Ui,
    id: &NodeId,
    name: &str,
    control: &Control,
    current: &Value,
) -> Option<(String, Value, Option<NumericResponse>)> {
    match control {
        Control::Number {
            label,
            unit,
            scale,
            min,
            max,
        } => {
            let mut shown = current.as_f64().unwrap_or(0.0) * scale;
            let response = property_row(ui, label, |ui| {
                NumericField::new(&mut shown, label)
                    .unit(unit)
                    .range(*min..=*max)
                    .decimals(if *scale > 1.0 { 0 } else { 1 })
                    .id_salt(("setting", id.as_str(), name))
                    .show(ui)
            });
            let changed = response.response.changed() || response.finished;
            changed.then(|| (name.to_owned(), Value::from(shown / scale), Some(response)))
        }
        Control::Flag { label } => {
            let mut on = current.as_bool().unwrap_or(false);
            let changed = property_row(ui, label, |ui| {
                segmented(ui, &mut on, &[(true, "On"), (false, "Off")]).changed()
            });
            changed.then(|| (name.to_owned(), Value::from(on), None))
        }
        Control::Choice { label, options } => {
            let now = current.as_str().unwrap_or_default();
            let mut chosen = options
                .iter()
                .position(|(word, _)| word == now)
                .unwrap_or(0);
            let labelled: Vec<(usize, &str)> = options
                .iter()
                .enumerate()
                .map(|(n, (_, label))| (n, label.as_str()))
                .collect();
            let changed = property_row(ui, label, |ui| {
                if labelled.len() <= 3 {
                    segmented(ui, &mut chosen, &labelled).changed()
                } else {
                    Dropdown::new(&mut chosen, label, &labelled)
                        .show(ui)
                        .changed()
                }
            });
            let word = options.get(chosen).map(|(word, _)| word.clone())?;
            changed.then(|| (name.to_owned(), Value::from(word), None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ty: NodeType) -> Vec<String> {
        settings_of(ty).into_iter().map(|(name, _)| name).collect()
    }

    #[test]
    fn each_type_shows_its_own_settings() {
        assert_eq!(names(NodeType::SlideIn), ["direction", "distance", "fade"]);
        assert_eq!(names(NodeType::GrowIn), ["scale", "fade"]);
        assert_eq!(names(NodeType::SpinOut), ["angle", "fade"]);
        assert_eq!(names(NodeType::RevealIn), ["direction"]);
        assert_eq!(names(NodeType::Resize), ["anchor"]);
        assert_eq!(
            names(NodeType::TextIn),
            [
                "split", "effect", "stagger", "order", "distance", "scale", "blur"
            ]
        );
        for ty in [
            NodeType::FadeIn,
            NodeType::Move,
            NodeType::Opacity,
            NodeType::Show,
            NodeType::Counter,
        ] {
            assert!(names(ty).is_empty(), "{ty}");
        }
    }

    #[test]
    fn settings_get_fitting_controls() {
        let settings = settings_of(NodeType::TextIn);
        let control = |name: &str| {
            settings
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, control)| control.clone())
                .unwrap()
        };
        match control("split") {
            Control::Choice { label, options } => {
                assert_eq!(label, "Apply to");
                let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
                assert_eq!(labels, ["Letters", "Words", "Lines"]);
            }
            other => panic!("{other:?}"),
        }
        match control("scale") {
            Control::Number { unit, scale, .. } => assert_eq!((unit, scale), ("%", 100.0)),
            other => panic!("{other:?}"),
        }
        match control("distance") {
            Control::Number { unit, .. } => assert_eq!(unit, "%", "of the line height"),
            other => panic!("{other:?}"),
        }
        match &settings_of(NodeType::SlideIn)[1].1 {
            Control::Number { unit, .. } => assert_eq!(*unit, "px"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            settings_of(NodeType::SlideIn)[2].1,
            Control::Flag { label: "Fade" }
        );
    }

    #[test]
    fn file_words_read_as_labels() {
        assert_eq!(word_label("slide_up"), "Slide up");
        assert_eq!(word_label("top_left"), "Top left");
        assert_eq!(word_label(""), "");
        assert_eq!(
            choices(r#""left" | "right""#),
            Some(vec!["left".to_owned(), "right".to_owned()])
        );
        assert_eq!(choices("number"), None);
    }
}
