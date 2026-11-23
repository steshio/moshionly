//! The inspector on the right of the window: Design and Animate.
//!
//! The Design tab edits how the selected layers look at rest. Every change
//! goes through the editor as an edit; a value scrubbed or typed into a
//! field is one undo step.

use egui::{RichText, ScrollArea, Ui};
use moshionly_core::{Edit, Family, NodeType};

use crate::{
    editor::{
        Editor,
        describe::{display_name, type_label},
        inspect::{Inspected, Shared, artboard_of, resize_all, set_all, tidy},
        timeline::action_icon,
    },
    theme::{Theme, icons},
    widgets::{
        ListRow, NumericField, NumericResponse, control_row, divider, empty_state,
        numeric::format_value, padded, pair, section, section_header, segmented,
    },
};

/// The two faces of the inspector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InspectorTab {
    /// How the selected layer looks at rest.
    #[default]
    Design,
    /// What the selected layer does over time.
    Animate,
}

/// What the inspector remembers: the tab, which sections are open and the
/// field being edited.
#[derive(Clone, Debug)]
pub struct InspectorState {
    /// The tab showing.
    pub tab: InspectorTab,
    /// Changing the width changes the height in proportion, and back.
    pub keep_proportions: bool,
    artboard_open: bool,
    layout_open: bool,
    actions_open: bool,
    /// A field's change is being recorded as one undo step.
    editing: bool,
}

impl Default for InspectorState {
    fn default() -> Self {
        Self {
            tab: InspectorTab::default(),
            keep_proportions: false,
            artboard_open: true,
            layout_open: true,
            actions_open: true,
            editing: false,
        }
    }
}

/// What happened in the inspector.
#[derive(Debug, Default)]
pub(super) struct InspectorOutput {
    /// An edit was refused: why, in a few words.
    pub error: Option<String>,
}

/// The inspector's view of the editor for one frame.
struct Bind<'a> {
    editor: &'a mut Editor,
    state: &'a mut InspectorState,
    out: &'a mut InspectorOutput,
}

impl Bind<'_> {
    /// Applies what a field did. The first change opens an undo step that
    /// the end of the edit closes, so a scrub is one step however many
    /// values it passes through.
    fn field(
        &mut self,
        response: &NumericResponse,
        summary: &str,
        edits: impl FnOnce() -> Vec<Edit>,
    ) {
        if response.response.changed() {
            if !self.state.editing {
                self.editor.begin_gesture(summary);
                self.state.editing = true;
            }
            self.apply(summary, &edits());
        }
        if response.finished && self.state.editing {
            self.editor.commit_gesture();
            self.state.editing = false;
        }
    }

    /// Applies edits, keeping the reason when they are refused.
    fn apply(&mut self, summary: &str, edits: &[Edit]) {
        if let Err(error) = self.editor.apply(summary, edits) {
            self.out.error = Some(error.problem.message);
        }
    }
}

/// Draws the inspector for the selection.
pub(super) fn inspector(
    ui: &mut Ui,
    state: &mut InspectorState,
    editor: &mut Editor,
) -> InspectorOutput {
    let theme = Theme::of(ui);
    let mut out = InspectorOutput::default();

    ui.add_space(theme.space.md);
    padded(ui, |ui| {
        segmented(
            ui,
            &mut state.tab,
            &[
                (InspectorTab::Design, "Design"),
                (InspectorTab::Animate, "Animate"),
            ],
        );
    });
    ui.add_space(theme.space.md);
    divider(ui);

    let document = std::sync::Arc::clone(editor.document());
    let ids = editor.selection.ids().to_vec();
    let Some(first) = ids.first().and_then(|id| document.get(id)) else {
        empty_state(
            ui,
            icons::CURSOR,
            "Nothing selected",
            "Select a layer on the canvas or in the list to edit it.",
            None,
        );
        return out;
    };
    let title = if ids.len() == 1 {
        display_name(first)
    } else {
        format!("{} layers", ids.len())
    };
    section_header(ui, &title, None, |ui| {
        if ids.len() == 1 {
            ui.label(
                RichText::new(type_label(first.node_type()))
                    .font(theme.small())
                    .color(theme.color.text_tertiary),
            );
        }
    });

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut bind = Bind {
                editor,
                state,
                out: &mut out,
            };
            let inspected = Inspected::new(&document, &ids);
            match bind.state.tab {
                InspectorTab::Design => design_tab(ui, &mut bind, &inspected),
                InspectorTab::Animate => animate_tab(ui, &mut bind, &inspected),
            }
            divider(ui);
            ui.add_space(theme.space.xl);
        });
    out
}

fn design_tab(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    if inspected.all_of(NodeType::Artboard) {
        artboard_section(ui, bind, inspected);
    }
    layout_section(ui, bind, inspected);
}

/// An artboard's size and length.
fn artboard_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids();
    let mut open = bind.state.artboard_open;
    section(
        ui,
        "Artboard",
        &mut open,
        |_| {},
        |ui| {
            let duration = inspected.number(|node| Some(artboard_of(node)?.duration));
            if let Some(duration) = duration {
                let mut seconds = duration.value() / 1000.0;
                let seconds_units: &[(&str, f64)] = &[("ms", 0.001)];
                let response = control_row(ui, |ui| {
                    NumericField::new(&mut seconds, "Duration")
                        .icon(icons::TIMER)
                        .unit("s")
                        .conversions(seconds_units)
                        .range(0.001..=3600.0)
                        .step(0.1)
                        .decimals(2)
                        .mixed(duration.is_mixed())
                        .show(ui)
                });
                bind.field(&response, "Change duration", || {
                    set_all(&ids, "duration", (seconds * 1000.0).round().max(1.0))
                });
            }
        },
    );
    bind.state.artboard_open = open;
}

/// Position, size and rotation.
fn layout_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids();
    let artboards = inspected.all_of(NodeType::Artboard);
    let rotatable = inspected.ids_where(|node| node.node_type() != NodeType::Artboard);
    let number = |read: fn(&moshionly_core::model::LayerProps) -> f64| {
        inspected.number(|node| Some(read(node.layer()?)))
    };
    let (Some(x), Some(y), Some(w), Some(h)) = (
        number(|l| l.x),
        number(|l| l.y),
        number(|l| l.width),
        number(|l| l.height),
    ) else {
        return;
    };
    let rotation = inspected.number(|node| {
        (node.node_type() != NodeType::Artboard).then(|| node.layer().map(|l| l.rotation))?
    });
    let keep = bind.state.keep_proportions;
    let mut open = bind.state.layout_open;
    let mut fields: Vec<(NumericResponse, f64, Field)> = Vec::new();
    let mut flip_keep = false;
    section(
        ui,
        if artboards {
            "Position and size"
        } else {
            "Layout"
        },
        &mut open,
        |_| {},
        |ui| {
            let (mut first, mut second) = (None, None);
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| first = Some(numeric(ui, &x, "X", |f| f.label("X").silent_unit("px"))),
                    |ui| second = Some(numeric(ui, &y, "Y", |f| f.label("Y").silent_unit("px"))),
                );
            });
            fields.extend(first.take().map(|(r, v)| (r, v, Field::X)));
            fields.extend(second.take().map(|(r, v)| (r, v, Field::Y)));
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| first = Some(numeric(ui, &w, "Width", |f| size(f, "W"))),
                    |ui| second = Some(numeric(ui, &h, "Height", |f| size(f, "H"))),
                );
            });
            fields.extend(first.take().map(|(r, v)| (r, v, Field::Width)));
            fields.extend(second.take().map(|(r, v)| (r, v, Field::Height)));
            if let Some(rotation) = &rotation {
                control_row(ui, |ui| {
                    pair(
                        ui,
                        |ui| {
                            first = Some(numeric(ui, rotation, "Rotation", |f| {
                                f.icon(icons::ANGLE).unit("°")
                            }));
                        },
                        |ui| {
                            let icon = if keep {
                                icons::LINK_SIMPLE
                            } else {
                                icons::LINK_SIMPLE_BREAK
                            };
                            let tip = if keep {
                                "Width and height change together"
                            } else {
                                "Keep proportions"
                            };
                            flip_keep = ui
                                .add(
                                    crate::widgets::IconButton::new(icon)
                                        .selected(keep)
                                        .tooltip(tip),
                                )
                                .clicked();
                        },
                    );
                });
                fields.extend(first.take().map(|(r, v)| (r, v, Field::Rotation)));
            }
        },
    );
    bind.state.layout_open = open;
    if flip_keep {
        bind.state.keep_proportions = !keep;
    }
    for (response, value, field) in fields {
        match field {
            Field::X => bind.field(&response, "Move", || set_all(&ids, "x", tidy(value))),
            Field::Y => bind.field(&response, "Move", || set_all(&ids, "y", tidy(value))),
            Field::Width => bind.field(&response, "Resize", || {
                resize_all(inspected.nodes(), value, false, keep)
            }),
            Field::Height => bind.field(&response, "Resize", || {
                resize_all(inspected.nodes(), value, true, keep)
            }),
            Field::Rotation => bind.field(&response, "Rotate", || {
                set_all(&rotatable, "rotation", tidy(normal_angle(value)))
            }),
        }
    }
}

/// The layout fields.
#[derive(Clone, Copy)]
enum Field {
    X,
    Y,
    Width,
    Height,
    Rotation,
}

/// A field for a width or a height.
fn size<'a>(field: NumericField<'a>, label: &'a str) -> NumericField<'a> {
    field
        .label(label)
        .silent_unit("px")
        .range(0.0..=f64::INFINITY)
}

/// Shows a numeric field for a value shared by the selection, set up by
/// `configure`. Returns what it did and the value it holds now.
fn numeric(
    ui: &mut Ui,
    value: &Shared<f64>,
    name: &str,
    configure: impl FnOnce(NumericField<'_>) -> NumericField<'_>,
) -> (NumericResponse, f64) {
    let mut number = *value.value();
    let field = NumericField::new(&mut number, name)
        .mixed(value.is_mixed())
        .id_salt(name);
    let response = configure(field).show(ui);
    (response, number)
}

/// An angle in degrees brought into the range -180 to 180.
pub fn normal_angle(degrees: f64) -> f64 {
    let turned = degrees.rem_euclid(360.0);
    if turned > 180.0 {
        turned - 360.0
    } else {
        turned
    }
}

fn animate_tab(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let theme = Theme::of(ui);
    let Some(id) = inspected.nodes().first().map(|node| node.id.clone()) else {
        return;
    };
    let document = std::sync::Arc::clone(bind.editor.document());
    let artboard = document.artboard_of(&id).map(|node| node.id.clone());
    let actions: Vec<&moshionly_core::Node> = artboard
        .iter()
        .flat_map(|artboard| document.walk(artboard, Family::Animation))
        .map(|(_, node)| node)
        .filter(|node| node.target() == Some(&id))
        .collect();

    divider(ui);
    section_header(ui, "Actions", Some(&mut bind.state.actions_open), |_| {});
    if !bind.state.actions_open {
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

    #[test]
    fn angles_stay_within_a_half_turn() {
        assert_eq!(normal_angle(0.0), 0.0);
        assert_eq!(normal_angle(190.0), -170.0);
        assert_eq!(normal_angle(-190.0), 170.0);
        assert_eq!(normal_angle(180.0), 180.0);
        assert_eq!(normal_angle(720.0 + 45.0), 45.0);
    }
}
