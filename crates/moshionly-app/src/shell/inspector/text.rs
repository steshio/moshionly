//! The Design tab's sections for text and images.

use egui::{RichText, Ui};
use moshionly_core::model::{AutoSize, ImageFit, TextAlign, TextCase};
use moshionly_core::{NodeType, io};
use serde_json::{Value, json};

use super::{Bind, numeric};
use crate::{
    editor::inspect::{Inspected, image_of, set_all, text_of, tidy},
    theme::{Theme, icons},
    widgets::{
        ButtonKind, Dropdown, NumericResponse, TextButton, control_row, pair, property_row,
        section, segmented,
    },
};

const ALIGNS: [(TextAlign, &str); 3] = [
    (TextAlign::Left, "Left"),
    (TextAlign::Center, "Center"),
    (TextAlign::Right, "Right"),
];

const CASES: [(TextCase, &str); 4] = [
    (TextCase::Original, "As typed"),
    (TextCase::Upper, "UPPER CASE"),
    (TextCase::Lower, "lower case"),
    (TextCase::Title, "Title Case"),
];

const SIZING: [(AutoSize, &str); 3] = [
    (AutoSize::WidthAndHeight, "Auto width"),
    (AutoSize::Height, "Auto height"),
    (AutoSize::Fixed, "Fixed size"),
];

const FITS: [(ImageFit, &str); 3] = [
    (ImageFit::Cover, "Fill"),
    (ImageFit::Contain, "Fit"),
    (ImageFit::Stretch, "Stretch"),
];

/// Names of the common weights, for the weight menu.
fn weight_name(weight: u16) -> String {
    let name = match weight {
        100 => "Thin",
        200 => "Extra light",
        300 => "Light",
        400 => "Regular",
        500 => "Medium",
        600 => "Semibold",
        700 => "Bold",
        800 => "Extra bold",
        900 => "Black",
        _ => return weight.to_string(),
    };
    format!("{name} · {weight}")
}

/// A value as the document's JSON.
fn value_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// A change to text.
enum TextChange {
    Family(String),
    Weight(u16),
    Number(NumericResponse, &'static str, &'static str, f64),
    Align(TextAlign),
    Case(TextCase),
    Sizing(AutoSize),
}

/// Font, size, spacing, alignment and case of the selected text.
pub(super) fn text_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids_where(|node| node.node_type() == NodeType::Text);
    let read = |field: fn(&moshionly_core::model::TextLayer) -> f64| {
        inspected.number(|node| text_of(node).map(field))
    };
    let (Some(size), Some(line), Some(spacing)) = (
        read(|t| t.font_size),
        read(|t| t.line_height),
        read(|t| t.letter_spacing),
    ) else {
        return;
    };
    let family = inspected.read(|node| text_of(node).map(|t| t.font.family.clone()));
    let weight = inspected.read(|node| text_of(node).map(|t| t.font.weight));
    let align = inspected.read(|node| text_of(node).map(|t| t.align));
    let case = inspected.read(|node| text_of(node).map(|t| t.case));
    let sizing = inspected.read(|node| text_of(node).map(|t| t.auto_size));

    // The families the renderer can draw, looked up once.
    if bind.state.families.is_empty() {
        bind.state.families = bind.editor.renderer().fonts().families();
    }
    let families = &bind.state.families;
    let current_family = family.as_ref().map_or("Inter", |f| f.value().as_str());
    let mut names: Vec<&str> = families.iter().map(|f| f.name.as_str()).collect();
    if !names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(current_family))
    {
        // A family the file asks for that is not installed still shows.
        names.insert(0, current_family);
    }
    let family_options: Vec<(usize, &str)> = names.iter().copied().enumerate().collect();
    let mut family_index = names
        .iter()
        .position(|name| name.eq_ignore_ascii_case(current_family))
        .unwrap_or(0);
    let mut weights: Vec<u16> = families
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(current_family))
        .map(|f| f.weights.clone())
        .unwrap_or_default();
    let current_weight = weight.as_ref().map_or(400, |w| *w.value());
    if !weights.contains(&current_weight) {
        weights.push(current_weight);
        weights.sort_unstable();
    }
    let weight_names: Vec<String> = weights.iter().map(|w| weight_name(*w)).collect();
    let weight_options: Vec<(u16, &str)> = weights
        .iter()
        .copied()
        .zip(weight_names.iter().map(String::as_str))
        .collect();
    let mut weight_value = current_weight;
    let mut align_value = align.as_ref().map_or(TextAlign::Left, |a| *a.value());
    let mut case_value = case.as_ref().map_or(TextCase::Original, |c| *c.value());
    let mut sizing_value = sizing
        .as_ref()
        .map_or(AutoSize::WidthAndHeight, |s| *s.value());

    let mut changes: Vec<TextChange> = Vec::new();
    let mut open = bind.state.open.text;
    section(
        ui,
        "Text",
        &mut open,
        |_| {},
        |ui| {
            control_row(ui, |ui| {
                if Dropdown::new(&mut family_index, "Font", &family_options)
                    .icon(icons::TEXT_AA)
                    .show(ui)
                    .changed()
                    && let Some(name) = names.get(family_index)
                {
                    changes.push(TextChange::Family((*name).to_owned()));
                }
            });
            let mut weighed = false;
            let mut sized = None;
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        weighed = Dropdown::new(&mut weight_value, "Weight", &weight_options)
                            .show(ui)
                            .changed();
                    },
                    |ui| {
                        sized = Some(numeric(ui, &size, "Font size", |f| {
                            f.icon(icons::TEXT_T).silent_unit("px").range(1.0..=4096.0)
                        }));
                    },
                );
            });
            if weighed {
                changes.push(TextChange::Weight(weight_value));
            }
            if let Some((response, value)) = sized {
                changes.push(TextChange::Number(
                    response,
                    "font_size",
                    "Change font size",
                    value,
                ));
            }
            let (mut first, mut second) = (None, None);
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        first = Some(numeric(ui, &line, "Line height", |f| {
                            f.icon(icons::ROWS).unit("%").range(1.0..=1000.0)
                        }));
                    },
                    |ui| {
                        second = Some(numeric(ui, &spacing, "Letter spacing", |f| {
                            f.icon(icons::ARROWS_HORIZONTAL).unit("%").step(0.5)
                        }));
                    },
                );
            });
            if let Some((response, value)) = first {
                changes.push(TextChange::Number(
                    response,
                    "line_height",
                    "Change line height",
                    value,
                ));
            }
            if let Some((response, value)) = second {
                changes.push(TextChange::Number(
                    response,
                    "letter_spacing",
                    "Change letter spacing",
                    value,
                ));
            }
            control_row(ui, |ui| {
                if segmented(ui, &mut align_value, &ALIGNS).changed() {
                    changes.push(TextChange::Align(align_value));
                }
            });
            property_row(ui, "Case", |ui| {
                if Dropdown::new(&mut case_value, "Letter case", &CASES)
                    .show(ui)
                    .changed()
                {
                    changes.push(TextChange::Case(case_value));
                }
            });
            property_row(ui, "Resizing", |ui| {
                if Dropdown::new(&mut sizing_value, "Text resizing", &SIZING)
                    .show(ui)
                    .changed()
                {
                    changes.push(TextChange::Sizing(sizing_value));
                }
            });
        },
    );
    bind.state.open.text = open;
    for change in changes {
        match change {
            TextChange::Family(name) => {
                bind.apply(
                    "Change font",
                    &set_all(&ids, "font", json!({ "family": name })),
                );
            }
            TextChange::Weight(weight) => {
                bind.apply(
                    "Change font",
                    &set_all(&ids, "font", json!({ "weight": weight })),
                );
            }
            TextChange::Number(response, field, summary, value) => {
                bind.field(&response, summary, || set_all(&ids, field, tidy(value)));
            }
            TextChange::Align(align) => {
                bind.apply("Align text", &set_all(&ids, "align", value_of(&align)));
            }
            TextChange::Case(case) => {
                bind.apply("Change case", &set_all(&ids, "case", value_of(&case)));
            }
            TextChange::Sizing(sizing) => {
                bind.apply(
                    "Change resizing",
                    &set_all(&ids, "auto_size", value_of(&sizing)),
                );
            }
        }
    }
}

/// Where the selected images come from, and how they fill their box.
pub(super) fn image_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids_where(|node| node.node_type() == NodeType::Image);
    let source = inspected.read(|node| image_of(node).map(|i| i.source.clone()));
    let fit = inspected.read(|node| image_of(node).map(|i| i.fit));
    let (Some(source), Some(fit)) = (source, fit) else {
        return;
    };
    let shown = if source.is_mixed() {
        "Several images".to_owned()
    } else {
        source_name(source.value())
    };
    let mut fit_value = *fit.value();
    let mut fitted = false;
    let mut replace = false;
    let mut open = bind.state.open.image;
    section(
        ui,
        "Image",
        &mut open,
        |_| {},
        |ui| {
            let theme = Theme::of(ui);
            property_row(ui, "Source", |ui| {
                ui.label(
                    RichText::new(&shown)
                        .font(theme.body())
                        .color(theme.color.text_secondary),
                )
                .on_hover_text(source.value());
            });
            property_row(ui, "Fill", |ui| {
                fitted = Dropdown::new(&mut fit_value, "Image fill", &FITS)
                    .show(ui)
                    .changed();
            });
            control_row(ui, |ui| {
                replace = ui
                    .add(
                        TextButton::new("Replace image…")
                            .kind(ButtonKind::Quiet)
                            .icon(icons::IMAGE),
                    )
                    .clicked();
            });
        },
    );
    bind.state.open.image = open;
    if fitted {
        bind.apply(
            "Change image fill",
            &set_all(&ids, "fit", value_of(&fit_value)),
        );
    }
    if replace {
        bind.out.replace_image = true;
    }
}

/// A short name for an image source: the file name, or "Embedded image"
/// for a `data:` URI.
fn source_name(source: &str) -> String {
    match io::resolve_asset(source, None) {
        Ok(io::Asset::Data { .. }) => "Embedded image".to_owned(),
        Ok(io::Asset::File(path)) => path.file_name().map_or_else(
            || source.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
        Err(_) => source.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_and_sources_have_readable_names() {
        assert_eq!(weight_name(700), "Bold · 700");
        assert_eq!(weight_name(450), "450");
        assert_eq!(source_name("assets/hero.png"), "hero.png");
        assert_eq!(source_name("data:image/png;base64,AAAA"), "Embedded image");
        assert_eq!(value_of(&TextAlign::Center), json!("center"));
        assert_eq!(
            value_of(&AutoSize::WidthAndHeight),
            json!("width_and_height")
        );
        assert_eq!(value_of(&ImageFit::Contain), json!("contain"));
    }
}
