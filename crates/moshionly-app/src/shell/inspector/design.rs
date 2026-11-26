//! The Design tab: how the selected layers look at rest.
//!
//! Each section shows the properties the selection has. Widgets report what
//! the person did; the edits are made after each section is drawn.

use egui::Ui;
use moshionly_core::build::instances_follow_mains;
use moshionly_core::model::{
    BlendMode, DirectionalBlur, Glow, Gradient, GradientStop, LayerProps, LinearGradient, NodeData,
    Noise, Paint, RadialGradient, Shadow, Stroke, StrokePosition,
};
use moshionly_core::{Color, Edit, Node, NodeId, NodeType, schema};
use serde_json::{Value, json};

use super::{Bind, component, media, numeric, text};
use crate::{
    editor::{
        effects::{self, Effect},
        inspect::{
            Inspected, Shared, artboard_of, resize_all, set_all, shared, stretch_contents, text_of,
            tidy,
        },
        settle::{Frame, is_auto_sized, settle_edit},
    },
    theme::icons,
    widgets::{
        ColorField, ColorResponse, Dropdown, IconButton, MenuItem, NumericResponse, Rgba,
        control_row, pair, popup_frame, property_row, section, segmented_mixed,
        slider_with_value_mixed,
    },
};

/// Sizes an artboard can take with one choice.
const PRESETS: [(&str, f64, f64); 6] = [
    ("Square · 1080 × 1080", 1080.0, 1080.0),
    ("Portrait 4:5 · 1080 × 1350", 1080.0, 1350.0),
    ("Story 9:16 · 1080 × 1920", 1080.0, 1920.0),
    ("Landscape 16:9 · 1920 × 1080", 1920.0, 1080.0),
    ("Classic 4:3 · 1440 × 1080", 1440.0, 1080.0),
    ("4K · 3840 × 2160", 3840.0, 2160.0),
];

/// Blend modes with their names, in menu order.
const BLENDS: [(BlendMode, &str); 16] = [
    (BlendMode::Normal, "Normal"),
    (BlendMode::Darken, "Darken"),
    (BlendMode::Multiply, "Multiply"),
    (BlendMode::ColorBurn, "Color burn"),
    (BlendMode::Lighten, "Lighten"),
    (BlendMode::Screen, "Screen"),
    (BlendMode::ColorDodge, "Color dodge"),
    (BlendMode::Overlay, "Overlay"),
    (BlendMode::SoftLight, "Soft light"),
    (BlendMode::HardLight, "Hard light"),
    (BlendMode::Difference, "Difference"),
    (BlendMode::Exclusion, "Exclusion"),
    (BlendMode::Hue, "Hue"),
    (BlendMode::Saturation, "Saturation"),
    (BlendMode::Color, "Color"),
    (BlendMode::Luminosity, "Luminosity"),
];

/// The kinds of paint a fill can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaintKind {
    Solid,
    Linear,
    Radial,
}

const PAINT_KINDS: [(PaintKind, &str); 3] = [
    (PaintKind::Solid, "Solid"),
    (PaintKind::Linear, "Linear gradient"),
    (PaintKind::Radial, "Radial gradient"),
];

/// Where a stroke sits, with its name.
const POSITIONS: [(StrokePosition, &str); 3] = [
    (StrokePosition::Inside, "Inside"),
    (StrokePosition::Center, "Center"),
    (StrokePosition::Outside, "Outside"),
];

/// The grey a new fill starts with.
const NEW_FILL: Color = Color::rgb(0xD9, 0xD9, 0xD9);

/// Whether a node's type has the property `field`.
pub(super) fn has_field(node: &Node, field: &str) -> bool {
    schema::field_names(node.node_type())
        .iter()
        .any(|name| name == field)
}

/// A document colour as the colour widgets take it.
pub(super) fn rgba(color: Color) -> Rgba {
    Rgba::new(color.r, color.g, color.b, color.a)
}

/// A colour from the colour widgets as a document value.
pub(super) fn hex(color: Rgba) -> Value {
    Value::from(Color::rgba(color.r, color.g, color.b, color.a).to_hex())
}

/// A value as the document's JSON.
fn value_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

pub(super) fn design_tab(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    component::component_section(ui, bind, inspected);
    if inspected.all_of(NodeType::Artboard) {
        artboard_section(ui, bind, inspected);
    }
    layout_section(ui, bind, inspected);
    if !inspected.all_of(NodeType::Artboard) {
        appearance_section(ui, bind, inspected);
    }
    if inspected.any_of(NodeType::Star) {
        star_section(ui, bind, inspected);
    }
    if inspected.any_of(NodeType::Text) {
        text::text_section(ui, bind, inspected);
    }
    if inspected.any_of(NodeType::Image) {
        text::image_section(ui, bind, inspected);
    }
    if inspected.any_of(NodeType::Video) {
        media::video_section(ui, bind, inspected);
    }
    fill_section(ui, bind, inspected);
    stroke_section(ui, bind, inspected);
    if !inspected.all_of(NodeType::Artboard) {
        effects_section(ui, bind, inspected);
    }
}

/// An artboard's size, length and background.
fn artboard_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids();
    let size = inspected.read(|node| node.layer().map(|l| (l.width, l.height)));
    let duration = inspected.number(|node| Some(artboard_of(node)?.duration));
    let background = inspected.read(|node| Some(artboard_of(node)?.fill.clone()));
    let mut preset = size
        .as_ref()
        .filter(|size| !size.is_mixed())
        .and_then(|size| {
            PRESETS
                .iter()
                .position(|(_, w, h)| (*w, *h) == *size.value())
        })
        .map_or(0, |at| at + 1);
    let options: Vec<(usize, &str)> = std::iter::once((0, "Custom size"))
        .chain(
            PRESETS
                .iter()
                .enumerate()
                .map(|(n, (name, ..))| (n + 1, *name)),
        )
        .collect();
    let mut chosen = None;
    let mut seconds_change = None;
    let mut color_change: Option<(ColorResponse, Rgba)> = None;
    let mut toggle_background = false;
    let solid = background
        .as_ref()
        .and_then(|b| b.value().as_ref().and_then(Paint::as_solid));
    let has_background = background.as_ref().is_some_and(|b| b.value().is_some());
    // Artboards with different backgrounds show one mixed colour.
    let mixed_background = background.as_ref().is_some_and(|b| b.is_mixed());
    let size_mixed = size.as_ref().is_some_and(|size| size.is_mixed());
    let mut open = bind.state.open.artboard;
    section(
        ui,
        "Artboard",
        &mut open,
        |_| {},
        |ui| {
            property_row(ui, "Size", |ui| {
                if Dropdown::new(&mut preset, "Artboard size", &options)
                    .mixed(size_mixed)
                    .show(ui)
                    .changed()
                {
                    chosen = Some(preset);
                }
            });
            if let Some(duration) = &duration {
                let mut seconds = duration.value() / 1000.0;
                let units: &[(&str, f64)] = &[("ms", 0.001)];
                let response = property_row(ui, "Duration", |ui| {
                    crate::widgets::NumericField::new(&mut seconds, "Duration")
                        .icon(icons::TIMER)
                        .unit("s")
                        .conversions(units)
                        .range(0.001..=3600.0)
                        .step(0.1)
                        .decimals(2)
                        .mixed(duration.is_mixed())
                        .show(ui)
                });
                seconds_change = Some((response, seconds));
            }
            property_row(ui, "Background", |ui| {
                ui.horizontal(|ui| {
                    if solid.is_some() || mixed_background {
                        let mut picked = rgba(solid.unwrap_or(Color::WHITE));
                        let response = ColorField::new(&mut picked, "Background")
                            .width((ui.available_width() - 32.0).max(80.0))
                            .mixed(mixed_background)
                            .show(ui);
                        color_change = Some((response, picked));
                    } else if has_background {
                        ui.label("Gradient");
                    }
                    let (icon, tip) = if has_background {
                        (icons::MINUS, "Remove background")
                    } else {
                        (icons::PLUS, "Add background")
                    };
                    toggle_background = ui
                        .add(IconButton::new(icon).dimmed(true).tooltip(tip))
                        .clicked();
                });
            });
        },
    );
    bind.state.open.artboard = open;
    if let Some(chosen) = chosen
        && let Some((_, w, h)) = chosen.checked_sub(1).and_then(|at| PRESETS.get(at))
    {
        let edits: Vec<Edit> = ids
            .iter()
            .map(|id| {
                Edit::update(
                    id.clone(),
                    [("width", Value::from(*w)), ("height", Value::from(*h))],
                )
            })
            .collect();
        bind.apply("Resize artboard", &edits);
    }
    if let Some((response, seconds)) = seconds_change {
        bind.field(&response, "Change duration", || {
            set_all(&ids, "duration", (seconds * 1000.0).round().max(1.0))
        });
    }
    if let Some((response, color)) = color_change {
        bind.color(&response, "Change background", || {
            set_all(&ids, "fill", hex(color))
        });
    }
    if toggle_background {
        let value = if has_background {
            Value::Null
        } else {
            hex(rgba(Color::WHITE))
        };
        bind.apply("Change background", &set_all(&ids, "fill", value));
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
    Radius,
}

/// Position, size, rotation and corner radius.
fn layout_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids();
    let artboards = inspected.all_of(NodeType::Artboard);
    let rotatable = inspected.ids_where(|node| node.node_type() != NodeType::Artboard);
    let rounded = inspected.ids_where(|node| has_field(node, "corner_radius"));
    // Every layer's box as drawn at rest: text that hugs its words shows,
    // and is sized from, the box its words take, not the hint in the file.
    let framed: Vec<(&Node, Frame)> = inspected
        .nodes()
        .iter()
        .filter_map(|node| Some((*node, bind.editor.frame_at_rest(&node.id)?)))
        .collect();
    // Editing such text first makes that box its own, in the same step.
    let settle: Vec<Edit> = framed
        .iter()
        .filter(|(node, _)| text_of(node).is_some_and(is_auto_sized))
        .filter_map(|(node, frame)| settle_edit(node, *frame))
        .collect();
    let part = |read: fn(&Frame) -> f64| shared(framed.iter().map(|(_, frame)| tidy(read(frame))));
    let (Some(x), Some(y), Some(w), Some(h)) = (
        part(|f| f.x),
        part(|f| f.y),
        part(|f| f.width),
        part(|f| f.height),
    ) else {
        return;
    };
    let settled = |more: Vec<Edit>| [settle.clone(), more].concat();
    let rotation = inspected.number(|node| {
        (node.node_type() != NodeType::Artboard).then(|| node.layer().map(|l| l.rotation))?
    });
    let radius = shared(
        inspected
            .nodes()
            .iter()
            .filter_map(|node| node.data.corner_radius())
            .map(|r| r.radii()),
    )
    .map(|radii| match radii {
        // Four different corners show as mixed.
        crate::editor::inspect::Shared::Same(r) if r.iter().all(|c| *c == r[0]) => {
            crate::editor::inspect::Shared::Same(r[0])
        }
        other => crate::editor::inspect::Shared::Mixed(other.value()[0]),
    });
    let keep = bind.state.keep_proportions;
    let mut open = bind.state.open.layout;
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
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        if let Some(rotation) = &rotation {
                            first = Some(numeric(ui, rotation, "Rotation", |f| {
                                f.icon(icons::ANGLE).unit("°")
                            }));
                        }
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
                            .add(IconButton::new(icon).selected(keep).tooltip(tip))
                            .clicked();
                    },
                );
            });
            fields.extend(first.take().map(|(r, v)| (r, v, Field::Rotation)));
            if let Some(radius) = &radius {
                control_row(ui, |ui| {
                    pair(
                        ui,
                        |ui| {
                            first = Some(numeric(ui, radius, "Corner radius", |f| {
                                f.icon(icons::CORNERS_OUT)
                                    .silent_unit("px")
                                    .range(0.0..=f64::INFINITY)
                            }));
                        },
                        |_| {},
                    );
                });
                fields.extend(first.take().map(|(r, v)| (r, v, Field::Radius)));
            }
        },
    );
    bind.state.open.layout = open;
    if flip_keep {
        bind.state.keep_proportions = !keep;
    }
    for (response, value, field) in fields {
        match field {
            Field::X => bind.field(&response, "Move", || {
                settled(set_all(&ids, "x", tidy(value)))
            }),
            Field::Y => bind.field(&response, "Move", || {
                settled(set_all(&ids, "y", tidy(value)))
            }),
            Field::Width | Field::Height => {
                let height = matches!(field, Field::Height);
                // A group that does not clip stretches what is in it; the
                // text in it is settled first, so it moves with its words.
                let inside = response.response.changed().then(|| {
                    let edits = bind.editor.settle_edits(&ids, true);
                    let mut working = (**bind.editor.document()).clone();
                    let ok = working.apply(&edits).is_ok();
                    (edits, working, ok)
                });
                // Instances the size of a resized main when the edit began
                // follow it.
                let before = bind.began();
                bind.field(&response, "Resize", || {
                    let mut edits = match inside {
                        Some((settle, working, true)) => [
                            settle,
                            resize_all(&framed, value, height, keep),
                            stretch_contents(&working, &framed, value, height, keep),
                        ]
                        .concat(),
                        _ => settled(resize_all(&framed, value, height, keep)),
                    };
                    let follow = instances_follow_mains(&before, &edits);
                    edits.extend(follow);
                    edits
                });
            }
            Field::Rotation => bind.field(&response, "Rotate", || {
                settled(set_all(&rotatable, "rotation", tidy(normal_angle(value))))
            }),
            Field::Radius => bind.field(&response, "Change corner radius", || {
                set_all(&rounded, "corner_radius", tidy(value.max(0.0)))
            }),
        }
    }
}

/// A field for a width or a height.
fn size<'a>(
    field: crate::widgets::NumericField<'a>,
    label: &'a str,
) -> crate::widgets::NumericField<'a> {
    field
        .label(label)
        .silent_unit("px")
        .range(0.0..=f64::INFINITY)
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

/// Opacity and blend mode.
fn appearance_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids_where(|node| node.node_type() != NodeType::Artboard);
    let opacity = inspected.number(|node| {
        (node.node_type() != NodeType::Artboard).then(|| node.layer().map(|l| l.opacity))?
    });
    let blend = inspected.read(|node| {
        (node.node_type() != NodeType::Artboard).then(|| node.layer().map(|l| l.blend_mode))?
    });
    let (Some(opacity), Some(blend)) = (opacity, blend) else {
        return;
    };
    let mut value = *opacity.value();
    let mut mode = *blend.value();
    let mut slid = None;
    let mut blended = false;
    let mut open = bind.state.open.appearance;
    section(
        ui,
        "Appearance",
        &mut open,
        |_| {},
        |ui| {
            property_row(ui, "Opacity", |ui| {
                slid = Some(slider_with_value_mixed(
                    ui,
                    &mut value,
                    "Opacity",
                    0.0..=100.0,
                    "%",
                    opacity.is_mixed(),
                ));
            });
            property_row(ui, "Blend", |ui| {
                blended = Dropdown::new(&mut mode, "Blend mode", &BLENDS)
                    .mixed(blend.is_mixed())
                    .show(ui)
                    .changed();
            });
        },
    );
    bind.state.open.appearance = open;
    if let Some(response) = slid {
        bind.field(&response, "Change opacity", || {
            set_all(&ids, "opacity", tidy(value.clamp(0.0, 100.0)))
        });
    }
    if blended {
        bind.apply(
            "Change blend mode",
            &set_all(&ids, "blend_mode", value_of(&mode)),
        );
    }
}

/// The points of a star.
fn star_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids_where(|node| node.node_type() == NodeType::Star);
    let star = |node: &Node| match &node.data {
        NodeData::Star(star) => Some((f64::from(star.points), star.inner_radius)),
        _ => None,
    };
    let (Some(points), Some(inner)) = (
        inspected.number(|node| star(node).map(|s| s.0)),
        inspected.number(|node| star(node).map(|s| s.1 * 100.0)),
    ) else {
        return;
    };
    let mut fields = (None, None);
    let mut open = bind.state.open.shape;
    section(
        ui,
        "Star",
        &mut open,
        |_| {},
        |ui| {
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        fields.0 = Some(numeric(ui, &points, "Points", |f| {
                            f.icon(icons::STAR).range(3.0..=64.0).decimals(0)
                        }));
                    },
                    |ui| {
                        fields.1 = Some(numeric(ui, &inner, "Inner radius", |f| {
                            f.icon(icons::CIRCLE_DASHED).unit("%").range(0.0..=100.0)
                        }));
                    },
                );
            });
        },
    );
    bind.state.open.shape = open;
    if let Some((response, value)) = fields.0 {
        bind.field(&response, "Change points", || {
            set_all(&ids, "points", value.round().clamp(3.0, 64.0) as u64)
        });
    }
    if let Some((response, value)) = fields.1 {
        bind.field(&response, "Change inner radius", || {
            set_all(
                &ids,
                "inner_radius",
                (value.clamp(0.0, 100.0) / 100.0 * 1000.0).round() / 1000.0,
            )
        });
    }
}

/// A change to a fill.
enum FillChange {
    Add,
    Remove,
    Kind(PaintKind),
    Solid(ColorResponse, Rgba),
    Stop(ColorResponse, usize, Rgba),
}

/// The paint inside shapes, text and groups.
fn fill_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected
        .ids_where(|node| has_field(node, "fill") && node.node_type() != NodeType::Artboard);
    if ids.is_empty() {
        return;
    }
    let fills = shared(
        inspected
            .nodes()
            .iter()
            .filter(|node| ids.contains(&node.id))
            .map(|node| node.data.fill().cloned()),
    );
    let Some(fills) = fills else {
        return;
    };
    let filled: Vec<&Node> = inspected
        .nodes()
        .iter()
        .copied()
        .filter(|node| ids.contains(&node.id))
        .collect();
    // Layers without a fill: "Add fill" gives them one and leaves the
    // others' alone.
    let unfilled: Vec<moshionly_core::NodeId> = filled
        .iter()
        .filter(|node| node.data.fill().is_none())
        .map(|node| node.id.clone())
        .collect();
    let all_solid = filled
        .iter()
        .all(|node| matches!(node.data.fill(), Some(Paint::Solid(_))));
    // Fills that differ only in colour show one mixed colour; any other
    // difference shows only the kind, as mixed, until one is chosen.
    let mixed = fills.is_mixed();
    let current = if mixed {
        filled.iter().find_map(|node| node.data.fill().cloned())
    } else {
        fills.value().clone()
    };
    let mut changes: Vec<FillChange> = Vec::new();
    let mut toggled = None;
    let mut open = bind.state.open.fill;
    section(
        ui,
        "Fill",
        &mut open,
        |ui| {
            let (icon, tip, change) = if unfilled.is_empty() {
                (icons::MINUS, "Remove fill", FillChange::Remove)
            } else {
                (icons::PLUS, "Add fill", FillChange::Add)
            };
            if ui
                .add(IconButton::new(icon).dimmed(true).tooltip(tip))
                .clicked()
            {
                toggled = Some(change);
            }
        },
        |ui| match &current {
            Some(paint) if !mixed || all_solid => {
                paint_rows(ui, "Fill", paint, mixed, &mut changes);
            }
            Some(_) => {
                kind_row(ui, None, &mut changes);
                quiet_note(ui, "The selected layers have different fills.");
            }
            None => {}
        },
    );
    bind.state.open.fill = open;
    changes.extend(toggled);
    for change in changes {
        match change {
            FillChange::Add => {
                bind.apply("Add fill", &set_all(&unfilled, "fill", hex(rgba(NEW_FILL))))
            }
            FillChange::Remove => bind.apply("Remove fill", &set_all(&ids, "fill", Value::Null)),
            FillChange::Kind(kind) => {
                let from = current.clone().unwrap_or(Paint::Solid(NEW_FILL));
                bind.apply(
                    "Change fill",
                    &set_all(&ids, "fill", value_of(&converted(&from, kind))),
                );
            }
            FillChange::Solid(response, color) => {
                bind.color(&response, "Change fill", || {
                    set_all(&ids, "fill", hex(color))
                });
            }
            FillChange::Stop(response, at, color) => {
                if let Some(Paint::Gradient(gradient)) = &current {
                    let changed = with_stop(gradient, at, color);
                    bind.color(&response, "Change fill", || {
                        set_all(&ids, "fill", value_of(&Paint::Gradient(changed)))
                    });
                }
            }
        }
    }
}

/// The kind of a paint.
fn kind_of(paint: &Paint) -> PaintKind {
    match paint {
        Paint::Solid(_) => PaintKind::Solid,
        Paint::Gradient(Gradient::LinearGradient(_)) => PaintKind::Linear,
        Paint::Gradient(Gradient::RadialGradient(_)) => PaintKind::Radial,
    }
}

/// The row choosing a paint's kind; `None` when the layers differ.
fn kind_row(ui: &mut Ui, kind: Option<PaintKind>, changes: &mut Vec<FillChange>) {
    let mut chosen = kind.unwrap_or(PaintKind::Solid);
    control_row(ui, |ui| {
        if Dropdown::new(&mut chosen, "Paint", &PAINT_KINDS)
            .icon(icons::PAINT_BUCKET)
            .mixed(kind.is_none())
            .show(ui)
            .changed()
        {
            changes.push(FillChange::Kind(chosen));
        }
    });
}

/// A line of quiet text in a section.
fn quiet_note(ui: &mut Ui, text: &str) {
    let theme = crate::theme::Theme::of(ui);
    ui.add_space(theme.space.xs);
    ui.label(
        egui::RichText::new(text)
            .font(theme.body())
            .color(theme.color.text_secondary),
    );
    ui.add_space(theme.space.xs);
}

/// The rows that edit a paint: its kind, and its colour or the colours of
/// its gradient stops. `mixed` says the layers have solid paints of
/// different colours.
fn paint_rows(ui: &mut Ui, name: &str, paint: &Paint, mixed: bool, changes: &mut Vec<FillChange>) {
    kind_row(ui, Some(kind_of(paint)), changes);
    match paint {
        Paint::Solid(color) => {
            let mut picked = rgba(*color);
            let response = control_row(ui, |ui| {
                ColorField::new(&mut picked, name).mixed(mixed).show(ui)
            });
            changes.push(FillChange::Solid(response, picked));
        }
        Paint::Gradient(gradient) => {
            for (at, stop) in gradient.stops().iter().enumerate() {
                let mut picked = rgba(stop.color);
                let label = format!("Stop {} · {}%", at + 1, (stop.offset * 100.0).round());
                // Each stop has widgets of its own: one name for all of them
                // would make them share a picker and a hex field.
                let stop_name = format!("Gradient stop {}", at + 1);
                let response = property_row(ui, &label, |ui| {
                    ColorField::new(&mut picked, &stop_name)
                        .id_salt((name, "gradient-stop", at))
                        .show(ui)
                });
                changes.push(FillChange::Stop(response, at, picked));
            }
        }
    }
}

/// `paint` turned into a paint of `kind`: a gradient starts from the solid
/// colour and fades it out; a solid takes the first stop's colour.
fn converted(paint: &Paint, kind: PaintKind) -> Paint {
    let stops = match paint {
        Paint::Solid(color) => vec![
            GradientStop::new(0.0, *color),
            GradientStop::new(1.0, color.with_alpha(0)),
        ],
        Paint::Gradient(gradient) => gradient.stops().to_vec(),
    };
    match kind {
        PaintKind::Solid => Paint::Solid(stops.first().map_or(NEW_FILL, |s| s.color)),
        PaintKind::Linear => Paint::Gradient(Gradient::LinearGradient(LinearGradient {
            start: [0.5, 0.0],
            end: [0.5, 1.0],
            stops,
        })),
        PaintKind::Radial => Paint::Gradient(Gradient::RadialGradient(RadialGradient {
            center: [0.5, 0.5],
            radius: [0.5, 0.5],
            rotation: 0.0,
            stops,
        })),
    }
}

/// `gradient` with the colour of stop `at` changed.
fn with_stop(gradient: &Gradient, at: usize, color: Rgba) -> Gradient {
    let mut changed = gradient.clone();
    let stops = match &mut changed {
        Gradient::LinearGradient(g) => &mut g.stops,
        Gradient::RadialGradient(g) => &mut g.stops,
    };
    if let Some(stop) = stops.get_mut(at) {
        stop.color = Color::rgba(color.r, color.g, color.b, color.a);
    }
    changed
}

/// A change to a stroke.
enum StrokeChange {
    Add,
    Remove,
    Color(ColorResponse, Rgba),
    Width(NumericResponse, f64),
    Position(StrokePosition),
}

/// The outline of shapes, text, groups and images.
fn stroke_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids_where(|node| has_field(node, "stroke"));
    if ids.is_empty() {
        return;
    }
    let strokes = shared(
        inspected
            .nodes()
            .iter()
            .filter(|node| ids.contains(&node.id))
            .map(|node| node.data.stroke().cloned()),
    );
    if strokes.is_none() {
        return;
    }
    // A stroke's colour, width and position are changed on the layers that
    // have one; "Add stroke" gives one to the layers that have none.
    let stroked =
        inspected.ids_where(|node| ids.contains(&node.id) && node.data.stroke().is_some());
    let unstroked: Vec<moshionly_core::NodeId> = ids
        .iter()
        .filter(|id| !stroked.contains(id))
        .cloned()
        .collect();
    let current: Option<Stroke> = inspected
        .nodes()
        .iter()
        .find_map(|node| node.data.stroke().cloned());
    let colors = inspected.read(|node| node.data.stroke()?.paint.as_solid());
    let positions = inspected.read(|node| node.data.stroke().map(|s| s.position));
    let widths = inspected.number(|node| node.data.stroke().map(|s| s.width));
    let mut changes: Vec<StrokeChange> = Vec::new();
    let mut toggled = None;
    let mut open = bind.state.open.stroke;
    section(
        ui,
        "Stroke",
        &mut open,
        |ui| {
            let (icon, tip, change) = if unstroked.is_empty() {
                (icons::MINUS, "Remove stroke", StrokeChange::Remove)
            } else {
                (icons::PLUS, "Add stroke", StrokeChange::Add)
            };
            if ui
                .add(IconButton::new(icon).dimmed(true).tooltip(tip))
                .clicked()
            {
                toggled = Some(change);
            }
        },
        |ui| {
            let Some(stroke) = &current else {
                return;
            };
            if let Some(colors) = &colors {
                let mut picked = rgba(*colors.value());
                let response = control_row(ui, |ui| {
                    ColorField::new(&mut picked, "Stroke")
                        .mixed(colors.is_mixed())
                        .show(ui)
                });
                changes.push(StrokeChange::Color(response, picked));
            }
            if let Some(widths) = &widths {
                let (response, value) = property_row(ui, "Width", |ui| {
                    numeric(ui, widths, "Stroke width", |f| {
                        f.icon(icons::LINE_SEGMENT)
                            .silent_unit("px")
                            .range(0.0..=f64::INFINITY)
                            .step(0.5)
                    })
                });
                changes.push(StrokeChange::Width(response, value));
            }
            let mut position = stroke.position;
            property_row(ui, "Position", |ui| {
                if Dropdown::new(&mut position, "Stroke position", &POSITIONS)
                    .mixed(positions.as_ref().is_some_and(|p| p.is_mixed()))
                    .show(ui)
                    .changed()
                {
                    changes.push(StrokeChange::Position(position));
                }
            });
        },
    );
    bind.state.open.stroke = open;
    changes.extend(toggled);
    for change in changes {
        match change {
            StrokeChange::Add => bind.apply(
                "Add stroke",
                &set_all(
                    &unstroked,
                    "stroke",
                    value_of(&Stroke::solid(Color::BLACK, 1.0)),
                ),
            ),
            StrokeChange::Remove => {
                bind.apply("Remove stroke", &set_all(&ids, "stroke", Value::Null))
            }
            StrokeChange::Color(response, color) => bind.color(&response, "Change stroke", || {
                set_all(&stroked, "stroke", json!({ "paint": hex(color) }))
            }),
            StrokeChange::Width(response, width) => bind.field(&response, "Change stroke", || {
                set_all(&stroked, "stroke", json!({ "width": tidy(width.max(0.0)) }))
            }),
            StrokeChange::Position(position) => bind.apply(
                "Change stroke",
                &set_all(
                    &stroked,
                    "stroke",
                    json!({ "position": value_of(&position) }),
                ),
            ),
        }
    }
}

/// A change to one of the selected layers' effects.
enum EffectChange {
    /// The effect was chosen in the add menu.
    Add(Effect),
    /// The effect's remove button was clicked.
    Remove(Effect),
    /// The colour of a shadow or a glow moved.
    Color(Effect, ColorResponse, Rgba),
    /// A number field of the effect moved: the part (`None` for the
    /// blur, which is one number) and the number shown.
    Number(Effect, Option<&'static str>, NumericResponse, f64),
    /// Whether grain moves was chosen.
    Moving(bool),
}

/// Grain that moves or stays still, as the Noise rows offer it.
const GRAIN: [(bool, &str); 2] = [(true, "Moving"), (false, "Still")];

/// The effects of the selected layers: shadow, blur, glow, noise and
/// directional blur, one block each, with an add menu of those the
/// selection does not have yet. Values are shown and changed across the
/// layers that have the effect; each keeps its own other parts.
fn effects_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let layers = effects::layers(inspected.nodes());
    if layers.is_empty() {
        return;
    }
    let addable = effects::addable(&layers);
    let present = effects::present(&layers);
    let mut changes: Vec<EffectChange> = Vec::new();
    let mut added = None;
    let mut open = bind.state.open.effects;
    section(
        ui,
        "Effects",
        &mut open,
        |ui| {
            if addable.is_empty() {
                return;
            }
            let response = ui.add(
                IconButton::new(icons::PLUS)
                    .dimmed(true)
                    .tooltip("Add effect"),
            );
            added = add_effect_menu(ui, &response, &addable);
        },
        |ui| {
            for effect in &present {
                if effect_header(ui, *effect) {
                    changes.push(EffectChange::Remove(*effect));
                }
                match effect {
                    Effect::Shadow => shadow_rows(ui, &layers, &mut changes),
                    Effect::Blur => blur_rows(ui, &layers, &mut changes),
                    Effect::Glow => glow_rows(ui, &layers, &mut changes),
                    Effect::Noise => noise_rows(ui, &layers, &mut changes),
                    Effect::DirectionalBlur => directional_blur_rows(ui, &layers, &mut changes),
                }
            }
            if present.is_empty() {
                quiet_note(ui, "No effects.");
            }
        },
    );
    bind.state.open.effects = open;
    changes.extend(added.map(EffectChange::Add));
    // A typed value is kept when its field loses focus, which can be on
    // the very click that removes its effect. The value changes are worked
    // out from the layers as they were at the start of the frame, so they
    // go first and the removal last, or it would be written back.
    changes.sort_by_key(|change| matches!(change, EffectChange::Remove(_)));
    for change in changes {
        match change {
            EffectChange::Add(effect) => {
                // What was added is shown, even if the section was closed.
                bind.state.open.effects = true;
                bind.apply(
                    &format!("Add {}", effect.lowercase()),
                    &effects::add(effect, &layers),
                );
            }
            EffectChange::Remove(effect) => bind.apply(
                &format!("Remove {}", effect.lowercase()),
                &effects::remove(effect, &layers),
            ),
            EffectChange::Color(effect, response, color) => {
                bind.color(&response, &format!("Change {}", effect.lowercase()), || {
                    effects::set_part(effect, &layers, Some("color"), hex(color))
                });
            }
            EffectChange::Number(effect, part, response, shown) => {
                let value = effects::stored_part(effect, part, shown);
                let changed = response.response.changed();
                // A blur of 0 is no blur: its block goes away at once, and
                // its field is not there to say when a scrub ends. So the
                // step ends on the frame the blur reaches 0.
                let gone = changed && effect == Effect::Blur && value == 0.0;
                let summary = format!("Change {}", effect.lowercase());
                bind.continuous(changed, response.finished || gone, &summary, || {
                    effects::set_part(effect, &layers, part, json!(value))
                });
            }
            EffectChange::Moving(moving) => bind.apply(
                "Change noise",
                &effects::set_part(Effect::Noise, &layers, Some("animated"), json!(moving)),
            ),
        }
    }
}

/// The menu of effects that opens under the add button. Returns the
/// effect chosen.
fn add_effect_menu(ui: &Ui, response: &egui::Response, addable: &[Effect]) -> Option<Effect> {
    let theme = crate::theme::Theme::of(ui);
    let mut chosen = None;
    egui::Popup::menu(response)
        .frame(popup_frame(ui))
        .gap(theme.space.xs)
        .width(theme.size.menu_width)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::new(0.0, 1.0);
            for effect in addable {
                let name = format!("Add {}", effect.lowercase());
                let item = MenuItem::new(effect.label())
                    .icon(effect.icon())
                    .accessible_name(&name);
                if ui.add(item).clicked() {
                    chosen = Some(*effect);
                    // Closed here too, for a choice made without a pointer.
                    ui.close();
                }
            }
        });
    chosen
}

/// The first line of an effect's block: its icon and name, and a button
/// that removes it. Returns whether that button was clicked.
fn effect_header(ui: &mut Ui, effect: Effect) -> bool {
    let theme = crate::theme::Theme::of(ui);
    let remove = format!("Remove {}", effect.lowercase());
    control_row(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(theme.size.icon, theme.size.control),
            egui::Sense::hover(),
        );
        icons::paint(
            ui.painter(),
            rect.center(),
            effect.icon(),
            theme.size.icon,
            theme.color.text_secondary,
        );
        ui.label(
            egui::RichText::new(effect.label())
                .font(theme.body())
                .color(theme.color.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(IconButton::new(icons::MINUS).dimmed(true).tooltip(&remove))
                .clicked()
        })
        .inner
    })
}

/// A part of an effect read from every layer that has the effect: the
/// same everywhere, or mixed.
fn effect_part(
    layers: &[(&NodeId, &LayerProps)],
    read: impl Fn(&LayerProps) -> Option<f64>,
) -> Option<Shared<f64>> {
    shared(layers.iter().filter_map(|(_, layer)| read(layer)))
}

/// One number field of an effect: the part it edits (`None` for the blur,
/// which is one number), its value across the layers that have the
/// effect, its name and how it looks.
struct EffectField {
    effect: Effect,
    part: Option<&'static str>,
    value: Shared<f64>,
    name: &'static str,
    look: NumberLook,
}

/// How an effect's number field looks: its label or icon, unit and range.
#[derive(Clone, Copy)]
enum NumberLook {
    /// A blur's softness in pixels, 0 or more, marked with a drop.
    Softness,
    /// Pixels, 0 or more, with a short label.
    Length(&'static str),
    /// Pixels that may be negative, with a short label.
    Offset(&'static str),
    /// Percent from 0 to 100, with a short label.
    Percent(&'static str),
    /// Degrees, with a short label.
    Angle(&'static str),
}

/// Two number fields side by side, each shown when the layers have its
/// value; what they did is added to `changes`.
fn number_pair(
    ui: &mut Ui,
    changes: &mut Vec<EffectChange>,
    first: Option<EffectField>,
    second: Option<EffectField>,
) {
    let mut out = (None, None);
    control_row(ui, |ui| {
        pair(
            ui,
            |ui| out.0 = first.map(|field| effect_number(ui, field)),
            |ui| out.1 = second.map(|field| effect_number(ui, field)),
        );
    });
    changes.extend(out.0);
    changes.extend(out.1);
}

/// One number field of an effect, and what it did.
fn effect_number(ui: &mut Ui, field: EffectField) -> EffectChange {
    let (response, shown) = numeric(ui, &field.value, field.name, |f| match field.look {
        NumberLook::Softness => f
            .icon(icons::DROP)
            .silent_unit("px")
            .range(0.0..=f64::INFINITY),
        NumberLook::Length(label) => f.label(label).silent_unit("px").range(0.0..=f64::INFINITY),
        NumberLook::Offset(label) => f.label(label).silent_unit("px"),
        NumberLook::Percent(label) => f.label(label).unit("%").range(0.0..=100.0).decimals(0),
        NumberLook::Angle(label) => f.label(label).unit("°"),
    });
    EffectChange::Number(field.effect, field.part, response, shown)
}

/// A shadow's colour, offset and blur.
fn shadow_rows(ui: &mut Ui, layers: &[(&NodeId, &LayerProps)], changes: &mut Vec<EffectChange>) {
    let shadows: Vec<&Shadow> = layers
        .iter()
        .filter_map(|(_, layer)| layer.shadow.as_ref())
        .collect();
    let Some(colors) = shared(shadows.iter().map(|shadow| shadow.color)) else {
        return;
    };
    let mut picked = rgba(*colors.value());
    let response = control_row(ui, |ui| {
        ColorField::new(&mut picked, "Shadow colour")
            .mixed(colors.is_mixed())
            .show(ui)
    });
    changes.push(EffectChange::Color(Effect::Shadow, response, picked));
    let field = |part, read: fn(&Shadow) -> f64, name, look| {
        Some(EffectField {
            effect: Effect::Shadow,
            part: Some(part),
            value: shared(shadows.iter().map(|shadow| read(shadow)))?,
            name,
            look,
        })
    };
    number_pair(
        ui,
        changes,
        field("x", |s| s.x, "Shadow X", NumberLook::Offset("X")),
        field("y", |s| s.y, "Shadow Y", NumberLook::Offset("Y")),
    );
    number_pair(
        ui,
        changes,
        field("blur", |s| s.blur, "Shadow blur", NumberLook::Softness),
        None,
    );
}

/// The blur of the whole layer.
fn blur_rows(ui: &mut Ui, layers: &[(&NodeId, &LayerProps)], changes: &mut Vec<EffectChange>) {
    let blur = effect_part(layers, |layer| (layer.blur > 0.0).then_some(layer.blur));
    let field = blur.map(|value| EffectField {
        effect: Effect::Blur,
        part: None,
        value,
        name: "Layer blur",
        look: NumberLook::Softness,
    });
    number_pair(ui, changes, field, None);
}

/// A glow's colour, softness and spread.
fn glow_rows(ui: &mut Ui, layers: &[(&NodeId, &LayerProps)], changes: &mut Vec<EffectChange>) {
    let Some(colors) = shared(layers.iter().filter_map(|(_, l)| Some(l.glow?.color))) else {
        return;
    };
    let mut picked = rgba(*colors.value());
    let response = control_row(ui, |ui| {
        ColorField::new(&mut picked, "Glow colour")
            .mixed(colors.is_mixed())
            .show(ui)
    });
    changes.push(EffectChange::Color(Effect::Glow, response, picked));
    let field = |part, read: fn(&Glow) -> f64, name, look| {
        Some(EffectField {
            effect: Effect::Glow,
            part: Some(part),
            value: effect_part(layers, |layer| layer.glow.as_ref().map(read))?,
            name,
            look,
        })
    };
    number_pair(
        ui,
        changes,
        field(
            "radius",
            |g| g.radius,
            "Glow radius",
            NumberLook::Length("Radius"),
        ),
        field(
            "spread",
            |g| g.spread,
            "Glow spread",
            NumberLook::Length("Spread"),
        ),
    );
}

/// Grain's strength and size, and whether it moves.
fn noise_rows(ui: &mut Ui, layers: &[(&NodeId, &LayerProps)], changes: &mut Vec<EffectChange>) {
    let field = |part, read: fn(&Noise) -> f64, name, look| {
        Some(EffectField {
            effect: Effect::Noise,
            part: Some(part),
            value: effect_part(layers, |layer| layer.noise.as_ref().map(read))?,
            name,
            look,
        })
    };
    number_pair(
        ui,
        changes,
        field(
            "amount",
            |n| n.amount * 100.0,
            "Noise amount",
            NumberLook::Percent("Amount"),
        ),
        field("size", |n| n.size, "Noise size", NumberLook::Length("Size")),
    );
    let Some(moving) = shared(layers.iter().filter_map(|(_, l)| Some(l.noise?.animated))) else {
        return;
    };
    let mut chosen = *moving.value();
    property_row(ui, "Grain", |ui| {
        if segmented_mixed(ui, &mut chosen, &GRAIN, moving.is_mixed()).changed() {
            changes.push(EffectChange::Moving(chosen));
        }
    });
}

/// A directional blur's strength and angle.
fn directional_blur_rows(
    ui: &mut Ui,
    layers: &[(&NodeId, &LayerProps)],
    changes: &mut Vec<EffectChange>,
) {
    let field = |part, read: fn(&DirectionalBlur) -> f64, name, look| {
        Some(EffectField {
            effect: Effect::DirectionalBlur,
            part: Some(part),
            value: effect_part(layers, |layer| layer.directional_blur.as_ref().map(read))?,
            name,
            look,
        })
    };
    number_pair(
        ui,
        changes,
        field(
            "amount",
            |b| b.amount,
            "Directional blur amount",
            NumberLook::Length("Amount"),
        ),
        field(
            "angle",
            |b| b.angle,
            "Directional blur angle",
            NumberLook::Angle("Angle"),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_stay_within_a_half_turn() {
        assert_eq!(normal_angle(0.0), 0.0);
        assert_eq!(normal_angle(190.0), -170.0);
        assert_eq!(normal_angle(-190.0), 170.0);
        assert_eq!(normal_angle(180.0), 180.0);
        assert_eq!(normal_angle(720.0 + 45.0), 45.0);
    }

    #[test]
    fn paints_convert_between_kinds() {
        let red = Paint::Solid(Color::rgb(255, 0, 0));
        let Paint::Gradient(gradient) = converted(&red, PaintKind::Linear) else {
            panic!("a gradient");
        };
        assert_eq!(gradient.stops().len(), 2);
        assert_eq!(gradient.stops()[1].color, Color::rgba(255, 0, 0, 0));
        let back = converted(&Paint::Gradient(gradient.clone()), PaintKind::Solid);
        assert_eq!(back, red);
        let Paint::Gradient(Gradient::RadialGradient(radial)) =
            converted(&Paint::Gradient(gradient.clone()), PaintKind::Radial)
        else {
            panic!("a radial gradient");
        };
        assert_eq!(radial.stops.len(), 2);
        let changed = with_stop(&gradient, 1, Rgba::rgb(0, 0, 255));
        assert_eq!(changed.stops()[1].color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn values_are_written_as_the_file_has_them() {
        assert_eq!(value_of(&BlendMode::ColorBurn), json!("color_burn"));
        assert_eq!(value_of(&StrokePosition::Inside), json!("inside"));
        assert_eq!(hex(Rgba::new(0x0D, 0x99, 0xFF, 0x80)), json!("#0D99FF80"));
        assert_eq!(rgba(Color::rgb(1, 2, 3)), Rgba::rgb(1, 2, 3));
        let mut document = moshionly_core::Document::new("Test");
        let (artboard, edits) =
            moshionly_core::build::add_artboard(&document, "A", 10.0, 10.0).unwrap();
        document.apply(&edits).unwrap();
        let node = document.get(&artboard).unwrap();
        assert!(has_field(node, "fill"));
        assert!(!has_field(node, "stroke"));
    }
}
