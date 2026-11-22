//! Slider, alone or paired with a numeric field.

use std::ops::RangeInclusive;

use egui::{Rect, Response, Sense, Stroke, Ui, Vec2, WidgetInfo};

use super::{NumericField, numeric::NumericResponse};
use crate::theme::Theme;

/// A horizontal slider: a thin track, filled up to the value, with a round
/// knob.
///
/// Takes the available width unless [`width`](Self::width) is set. Use
/// [`slider_with_value`] when the number should be visible and typeable.
#[must_use = "call show()"]
pub struct Slider<'a> {
    value: &'a mut f64,
    name: &'a str,
    range: RangeInclusive<f64>,
    step: f64,
    width: Option<f32>,
}

impl<'a> Slider<'a> {
    /// A slider editing `value` within `range`. `name` is what screen readers
    /// announce.
    pub fn new(value: &'a mut f64, name: &'a str, range: RangeInclusive<f64>) -> Self {
        Self {
            value,
            name,
            range,
            step: 1.0,
            width: None,
        }
    }

    /// The value snaps to multiples of `step` (1 by default; 0 disables).
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// A fixed width instead of the available width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Draws the slider. `changed()` is true while the value moves and
    /// `drag_stopped()` when the gesture ends.
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme.size.control),
            Sense::click_and_drag(),
        );
        let knob = 6.0;
        let track = Rect::from_min_max(
            egui::pos2(rect.left() + knob, rect.center().y - 1.5),
            egui::pos2(rect.right() - knob, rect.center().y + 1.5),
        );
        let (min, max) = (*self.range.start(), *self.range.end());

        if let Some(pointer) = response.interact_pointer_pos()
            && track.width() > 0.0
            && max > min
        {
            let t = f64::from(((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
            let next = slider_value(t, min, max, self.step);
            if next != *self.value {
                *self.value = next;
                response.mark_changed();
            }
        }

        let t = if max > min {
            ((*self.value - min) / (max - min)).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        if ui.is_rect_visible(rect) {
            let x = track.left() + track.width() * t;
            ui.painter().rect_filled(track, 2.0, c.well_pressed);
            ui.painter().rect_filled(
                Rect::from_min_max(track.min, egui::pos2(x, track.bottom())),
                2.0,
                c.accent,
            );
            let center = egui::pos2(x, rect.center().y);
            let active = response.dragged() || response.hovered();
            ui.painter().circle(
                center,
                knob,
                c.handle_fill,
                Stroke::new(
                    if active { 1.5 } else { 1.0 },
                    if active { c.accent } else { c.border_strong },
                ),
            );
        }
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), *self.value, self.name));
        response
    }
}

/// The value for a slider position `t` from 0 to 1, snapped to `step`.
pub fn slider_value(t: f64, min: f64, max: f64, step: f64) -> f64 {
    let raw = min + (max - min) * t.clamp(0.0, 1.0);
    let snapped = if step > 0.0 {
        min + ((raw - min) / step).round() * step
    } else {
        raw
    };
    snapped.clamp(min, max)
}

/// A slider followed by a compact numeric field showing the same value, the
/// usual control for opacity or an easing's intensity.
///
/// Returns the numeric field's report, with `changed()` also true when the
/// slider moved and `finished` true when either gesture ended.
pub fn slider_with_value(
    ui: &mut Ui,
    value: &mut f64,
    name: &str,
    range: RangeInclusive<f64>,
    unit: &str,
) -> NumericResponse {
    let theme = Theme::of(ui);
    let field_width = 56.0;
    let slider_width = (ui.available_width() - field_width - theme.space.md).max(40.0);
    let slider = Slider::new(value, name, range.clone())
        .width(slider_width)
        .show(ui);
    let mut numeric = NumericField::new(value, name)
        .unit(unit)
        .range(range)
        .width(field_width)
        .show(ui);
    if slider.changed() {
        numeric.response.mark_changed();
    }
    numeric.finished |= slider.drag_stopped();
    numeric.active |= slider.dragged();
    numeric
}

#[cfg(test)]
mod tests {
    use super::slider_value;

    #[test]
    fn position_maps_to_the_range() {
        assert_eq!(slider_value(0.0, 0.0, 100.0, 1.0), 0.0);
        assert_eq!(slider_value(1.0, 0.0, 100.0, 1.0), 100.0);
        assert_eq!(slider_value(0.5, 0.0, 100.0, 1.0), 50.0);
        assert_eq!(slider_value(0.5, -50.0, 50.0, 1.0), 0.0);
    }

    #[test]
    fn values_snap_to_the_step_and_stay_in_range() {
        assert_eq!(slider_value(0.333, 0.0, 100.0, 5.0), 35.0);
        assert_eq!(slider_value(0.333, 0.0, 1.0, 0.0), 0.333);
        assert_eq!(slider_value(2.0, 0.0, 100.0, 1.0), 100.0);
        assert_eq!(slider_value(-1.0, 0.0, 100.0, 1.0), 0.0);
        assert_eq!(slider_value(0.99, 0.0, 10.0, 4.0), 8.0);
    }
}
