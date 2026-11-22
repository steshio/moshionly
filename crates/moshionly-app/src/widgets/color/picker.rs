//! The colour picker panel: saturation and brightness square, hue and opacity
//! strips, hex and opacity inputs.

use egui::{
    Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, ecolor::Hsva, lerp,
};

use super::{Rgba, field::hex_input};
use crate::{
    theme::Theme,
    widgets::{NumericField, paint},
};

/// Width of the picker panel's content.
pub const PICKER_WIDTH: f32 = 216.0;

/// What changed in the picker this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickerResponse {
    /// The colour changed this frame.
    pub changed: bool,
    /// A drag or a typed edit finished this frame.
    pub finished: bool,
}

/// Draws the picker for `color`.
///
/// The hue is remembered under `id` while the picker is open, so dragging
/// through grey or black does not lose it.
pub fn color_picker(ui: &mut Ui, id: egui::Id, color: &mut Rgba) -> PickerResponse {
    let theme = Theme::of(ui);
    let mut out = PickerResponse::default();

    // Keep hue and saturation stable while the colour passes through greys.
    let mut hsva = ui
        .data(|d| d.get_temp::<Hsva>(id))
        .filter(|stored| rgba_of(*stored) == *color)
        .unwrap_or_else(|| Hsva::from_srgba_unmultiplied([color.r, color.g, color.b, color.a]));
    let before = hsva;

    ui.spacing_mut().item_spacing = Vec2::new(theme.space.md, theme.space.md);
    ui.set_width(PICKER_WIDTH);

    // Saturation (x) and value (y).
    let (square, response) =
        ui.allocate_exact_size(Vec2::new(PICKER_WIDTH, 140.0), Sense::click_and_drag());
    if let Some(pointer) = response.interact_pointer_pos() {
        hsva.s = ((pointer.x - square.left()) / square.width()).clamp(0.0, 1.0);
        hsva.v = 1.0 - ((pointer.y - square.top()) / square.height()).clamp(0.0, 1.0);
    }
    out.finished |= response.drag_stopped() || response.clicked();
    paint_square(ui, square, hsva.h);
    let knob = Pos2::new(
        lerp(square.left()..=square.right(), hsva.s),
        lerp(square.bottom()..=square.top(), hsva.v),
    );
    paint_knob(ui, knob, Color32::from(Hsva { a: 1.0, ..hsva }));

    // Hue.
    let (strip, response) =
        ui.allocate_exact_size(Vec2::new(PICKER_WIDTH, 12.0), Sense::click_and_drag());
    if let Some(pointer) = response.interact_pointer_pos() {
        // Stop just short of 1 so the far end stays red rather than wrapping.
        hsva.h = ((pointer.x - strip.left()) / strip.width()).clamp(0.0, 0.999);
    }
    out.finished |= response.drag_stopped() || response.clicked();
    paint_strip(
        ui,
        strip,
        |t| Color32::from(Hsva::new(t, 1.0, 1.0, 1.0)),
        48,
    );
    paint_knob(
        ui,
        Pos2::new(lerp(strip.left()..=strip.right(), hsva.h), strip.center().y),
        Color32::from(Hsva::new(hsva.h, 1.0, 1.0, 1.0)),
    );

    // Opacity.
    let (strip, response) =
        ui.allocate_exact_size(Vec2::new(PICKER_WIDTH, 12.0), Sense::click_and_drag());
    if let Some(pointer) = response.interact_pointer_pos() {
        hsva.a = ((pointer.x - strip.left()) / strip.width()).clamp(0.0, 1.0);
    }
    out.finished |= response.drag_stopped() || response.clicked();
    let opaque = Hsva { a: 1.0, ..hsva };
    let [r, g, b, _] = opaque.to_srgba_unmultiplied();
    paint::checkerboard(
        &ui.painter().with_clip_rect(strip),
        strip,
        6.0,
        theme.color.checker_light,
        theme.color.checker_dark,
    );
    paint_strip(
        ui,
        strip,
        |t| Color32::from_rgba_unmultiplied(r, g, b, (t * 255.0).round() as u8),
        16,
    );
    paint_knob(
        ui,
        Pos2::new(lerp(strip.left()..=strip.right(), hsva.a), strip.center().y),
        Color32::from(opaque),
    );

    if hsva != before {
        *color = rgba_of(hsva);
        out.changed = true;
    }

    // Hex and opacity inputs.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.md;
        let opacity_width = 64.0;
        let hex = hex_input(
            ui,
            id.with("hex"),
            color,
            PICKER_WIDTH - opacity_width - theme.space.md,
        );
        let mut opacity = color.opacity();
        let numeric = NumericField::new(&mut opacity, "Opacity")
            .unit("%")
            .range(0.0..=100.0)
            .width(opacity_width)
            .id_salt(id.with("opacity"))
            .show(ui);
        if numeric.response.changed() {
            *color = color.with_opacity(opacity);
            out.changed = true;
        }
        if hex.changed {
            out.changed = true;
        }
        out.finished |= hex.finished || numeric.finished;
    });

    // Remember the hue for the next frame. When the colour came from a typed
    // value, derive it afresh.
    let stored = if rgba_of(hsva) == *color {
        hsva
    } else {
        Hsva::from_srgba_unmultiplied([color.r, color.g, color.b, color.a])
    };
    ui.data_mut(|d| d.insert_temp(id, stored));
    out
}

fn rgba_of(hsva: Hsva) -> Rgba {
    let [r, g, b, a] = hsva.to_srgba_unmultiplied();
    Rgba::new(r, g, b, a)
}

/// Paints the saturation and value square for one hue as a colour mesh.
fn paint_square(ui: &Ui, rect: Rect, hue: f32) {
    const STEPS: u32 = 24;
    let mut mesh = Mesh::default();
    for row in 0..=STEPS {
        for col in 0..=STEPS {
            let s = col as f32 / STEPS as f32;
            let v = 1.0 - row as f32 / STEPS as f32;
            let pos = Pos2::new(
                lerp(rect.left()..=rect.right(), s),
                lerp(rect.top()..=rect.bottom(), 1.0 - v),
            );
            mesh.colored_vertex(pos, Color32::from(Hsva::new(hue, s, v, 1.0)));
            if row < STEPS && col < STEPS {
                let i = row * (STEPS + 1) + col;
                mesh.add_triangle(i, i + 1, i + STEPS + 1);
                mesh.add_triangle(i + 1, i + STEPS + 1, i + STEPS + 2);
            }
        }
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.add(Shape::mesh(mesh));
    round_off(ui, rect);
}

/// Paints a horizontal gradient strip from a colour function of 0 to 1.
fn paint_strip(ui: &Ui, rect: Rect, color_at: impl Fn(f32) -> Color32, steps: u32) {
    let mut mesh = Mesh::default();
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = lerp(rect.left()..=rect.right(), t);
        let color = color_at(t);
        mesh.colored_vertex(Pos2::new(x, rect.top()), color);
        mesh.colored_vertex(Pos2::new(x, rect.bottom()), color);
        if step < steps {
            let i = step * 2;
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i + 1, i + 2, i + 3);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    round_off(ui, rect);
}

/// Rounds the corners of a gradient painted as a square mesh, and outlines
/// it. The picker lives on a raised surface, so the corners are covered with
/// that colour.
fn round_off(ui: &Ui, rect: Rect) {
    let theme = Theme::of(ui);
    let radius = theme.radius.xs;
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.5, theme.color.raised),
        StrokeKind::Outside,
    );
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme.color.border),
        StrokeKind::Inside,
    );
}

/// Paints the round marker of a picker control.
fn paint_knob(ui: &Ui, center: Pos2, fill: Color32) {
    let painter = ui.painter();
    painter.circle_filled(center, 7.0, Color32::from_black_alpha(40));
    painter.circle_filled(center, 6.0, Color32::WHITE);
    painter.circle_filled(center, 4.0, fill);
}
