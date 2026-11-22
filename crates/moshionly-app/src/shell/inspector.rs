//! The inspector on the right of the window: Design and Animate.

use egui::{RichText, ScrollArea, Ui};

use super::{
    parts::sample_tag,
    sample::{Sample, SampleBlend, SampleEasing},
};
use crate::{
    theme::{Theme, icons},
    widgets::{
        ButtonKind, ColorField, Dropdown, IconButton, ListRow, NumericField, TextButton,
        control_row, divider, empty_state, numeric::format_value, padded, pair, property_row,
        section, section_header, segmented, slider_with_value,
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

/// What the inspector remembers: the tab and which sections are open.
#[derive(Clone, Debug)]
pub struct InspectorState {
    /// The tab showing.
    pub tab: InspectorTab,
    layout_open: bool,
    appearance_open: bool,
    fill_open: bool,
    stroke_open: bool,
    effects_open: bool,
    actions_open: bool,
    timing_open: bool,
}

impl Default for InspectorState {
    fn default() -> Self {
        Self {
            tab: InspectorTab::default(),
            layout_open: true,
            appearance_open: true,
            fill_open: true,
            stroke_open: true,
            effects_open: false,
            actions_open: true,
            timing_open: true,
        }
    }
}

/// Draws the inspector for the sample scene.
pub fn inspector(ui: &mut Ui, state: &mut InspectorState, sample: &mut Sample) {
    let theme = Theme::of(ui);

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

    let Some(layer) = sample.selected().cloned() else {
        empty_state(
            ui,
            icons::CURSOR,
            "Nothing selected",
            "Select a layer on the canvas or in the list to edit it.",
            None,
        );
        return;
    };

    section_header(ui, layer.name, None, sample_tag);
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            match state.tab {
                InspectorTab::Design => design_tab(ui, state, sample),
                InspectorTab::Animate => animate_tab(ui, state, sample),
            }
            divider(ui);
            ui.add_space(theme.space.xl);
        });
}

fn design_tab(ui: &mut Ui, state: &mut InspectorState, sample: &mut Sample) {
    let design = &mut sample.design;
    section(
        ui,
        "Layout",
        &mut state.layout_open,
        |_| {},
        |ui| {
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut design.x, "X")
                            .label("X")
                            .silent_unit("px")
                            .show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut design.y, "Y")
                            .label("Y")
                            .silent_unit("px")
                            .show(ui);
                    },
                );
            });
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut design.width, "Width")
                            .label("W")
                            .silent_unit("px")
                            .range(0.0..=f64::INFINITY)
                            .show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut design.height, "Height")
                            .label("H")
                            .silent_unit("px")
                            .range(0.0..=f64::INFINITY)
                            .show(ui);
                    },
                );
            });
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut design.rotation, "Rotation")
                            .icon(icons::ANGLE)
                            .unit("°")
                            .show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut design.corner_radius, "Corner radius")
                            .icon(icons::CORNERS_OUT)
                            .silent_unit("px")
                            .range(0.0..=f64::INFINITY)
                            .show(ui);
                    },
                );
            });
        },
    );
    section(
        ui,
        "Appearance",
        &mut state.appearance_open,
        |_| {},
        |ui| {
            property_row(ui, "Opacity", |ui| {
                slider_with_value(ui, &mut design.opacity, "Opacity", 0.0..=100.0, "%");
            });
            property_row(ui, "Blend", |ui| {
                Dropdown::new(&mut design.blend, "Blend mode", &SampleBlend::OPTIONS).show(ui);
            });
        },
    );
    section(
        ui,
        "Fill",
        &mut state.fill_open,
        |ui| {
            ui.add(
                IconButton::new(icons::PLUS)
                    .dimmed(true)
                    .tooltip("Add fill"),
            );
        },
        |ui| {
            control_row(ui, |ui| {
                ColorField::new(&mut design.fill, "Fill").show(ui);
            });
        },
    );
    section(
        ui,
        "Stroke",
        &mut state.stroke_open,
        |ui| {
            ui.add(
                IconButton::new(icons::PLUS)
                    .dimmed(true)
                    .tooltip("Add stroke"),
            );
        },
        |ui| {
            control_row(ui, |ui| {
                ColorField::new(&mut design.stroke, "Stroke").show(ui);
            });
            property_row(ui, "Width", |ui| {
                NumericField::new(&mut design.stroke_width, "Stroke width")
                    .icon(icons::LINE_SEGMENT)
                    .silent_unit("px")
                    .range(0.0..=f64::INFINITY)
                    .step(0.5)
                    .show(ui);
            });
        },
    );
    section(
        ui,
        "Effects",
        &mut state.effects_open,
        |ui| {
            ui.add(
                IconButton::new(icons::PLUS)
                    .dimmed(true)
                    .tooltip("Add shadow or blur"),
            );
        },
        |ui| {
            let theme = Theme::of(ui);
            ui.add_space(theme.space.xs);
            ui.label(
                RichText::new("No shadow or blur on this layer.")
                    .font(theme.body())
                    .color(theme.color.text_secondary),
            );
            ui.add_space(theme.space.xs);
        },
    );
}

fn animate_tab(ui: &mut Ui, state: &mut InspectorState, sample: &mut Sample) {
    let theme = Theme::of(ui);
    let Some(layer_key) = sample.selected_layer.clone() else {
        return;
    };
    let own: Vec<usize> = (0..sample.actions.len())
        .filter(|index| sample.actions[*index].layer == layer_key)
        .collect();

    divider(ui);
    section_header(ui, "Actions", Some(&mut state.actions_open), |ui| {
        ui.add(
            IconButton::new(icons::PLUS)
                .dimmed(true)
                .tooltip("Add action"),
        );
    });
    if state.actions_open {
        if own.is_empty() {
            empty_state(
                ui,
                icons::LIGHTNING,
                "No actions yet",
                "Add an action to make this layer move, fade or change.",
                Some("Add action"),
            );
        }
        let mut clicked = None;
        for index in &own {
            let action = &sample.actions[*index];
            let selected = sample.selected_actions.contains(&action.key);
            let span = time_span(action.start, action.end);
            let row = ListRow::new(action.name)
                .icon(action.icon)
                .selected(selected)
                .id_salt(action.key.as_str())
                .show(ui, |ui, _| {
                    ui.add_space(theme.space.md);
                    ui.label(
                        RichText::new(span)
                            .font(theme.small())
                            .color(theme.color.text_secondary),
                    );
                });
            if row.response.clicked() {
                clicked = Some(action.key.clone());
            }
        }
        if let Some(key) = clicked {
            sample.selected_actions = vec![key];
        }
        ui.add_space(theme.space.md);
    }

    // The timing of the selected action, when exactly one of this layer's
    // actions is selected.
    let mut chosen = own
        .iter()
        .filter(|index| {
            sample
                .selected_actions
                .contains(&sample.actions[**index].key)
        })
        .copied();
    let (Some(index), None) = (chosen.next(), chosen.next()) else {
        return;
    };
    let action = &mut sample.actions[index];
    let easing = &mut sample.easing;
    let intensity = &mut sample.intensity;
    section(
        ui,
        "Timing",
        &mut state.timing_open,
        |_| {},
        |ui| {
            // Moving the start keeps the length; the length moves the end.
            let (mut start, mut duration) = (action.start, action.end - action.start);
            let seconds: &[(&str, f64)] = &[("s", 1000.0)];
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut start, "Start")
                            .icon(icons::TIMER)
                            .unit("ms")
                            .conversions(seconds)
                            .range(0.0..=f64::INFINITY)
                            .step(10.0)
                            .decimals(0)
                            .show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut duration, "Duration")
                            .icon(icons::ARROWS_HORIZONTAL)
                            .unit("ms")
                            .conversions(seconds)
                            .range(10.0..=f64::INFINITY)
                            .step(10.0)
                            .decimals(0)
                            .show(ui);
                    },
                );
            });
            action.start = start;
            action.end = start + duration;
            property_row(ui, "Easing", |ui| {
                Dropdown::new(easing, "Easing", &SampleEasing::OPTIONS)
                    .icon(icons::WAVE_SINE)
                    .show(ui);
            });
            property_row(ui, "Intensity", |ui| {
                slider_with_value(ui, intensity, "Intensity", 0.0..=100.0, "");
            });
            control_row(ui, |ui| {
                ui.add(
                    TextButton::new("Remove action")
                        .kind(ButtonKind::Quiet)
                        .icon(icons::MINUS),
                );
            });
        },
    );
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
