//! The numeric field widget.

use std::ops::RangeInclusive;

use egui::{
    Align2, Color32, CursorIcon, Frame, Key, Modifiers, Rect, Response, Sense, Stroke, TextEdit,
    Ui, Vec2, text::CCursorRange, text_edit::TextEditState,
};

use super::expr::{ExprContext, evaluate, format_value};
use crate::{
    theme::{Theme, icons},
    widgets::paint,
};

/// A field that edits one number.
///
/// - **Type** a number or a calculation such as `100+20` or `50%*2` and press
///   Enter. Escape cancels. See [`evaluate`] for the language.
/// - **Drag** left and right on the field to scrub the value. Hold Shift for
///   ten times the step, Alt for a tenth.
/// - **Arrow up and down** step the value while typing, with the same
///   modifiers.
///
/// The field takes the available width unless [`width`](Self::width) is set.
#[must_use = "call show()"]
pub struct NumericField<'a> {
    value: &'a mut f64,
    name: &'a str,
    label: Option<&'a str>,
    icon: Option<&'a str>,
    unit: &'a str,
    show_unit: bool,
    conversions: &'a [(&'a str, f64)],
    range: RangeInclusive<f64>,
    step: f64,
    decimals: usize,
    width: Option<f32>,
    mixed: bool,
    id_salt: Option<egui::IdSalt>,
}

/// What a [`NumericField`] reports back.
pub struct NumericResponse {
    /// The field's response. `changed()` is true on every frame the value
    /// changed, including each step of a scrub.
    pub response: Response,
    /// True on the frame an edit finished: Enter or focus loss after typing,
    /// or the end of a scrub. Use it to close an undo step.
    pub finished: bool,
    /// True while the user is scrubbing or typing.
    pub active: bool,
}

/// The typing state of one field, kept in egui's temporary memory.
#[derive(Clone, Default)]
struct Editing {
    buffer: String,
    select_all: bool,
}

impl<'a> NumericField<'a> {
    /// A field editing `value`. `name` is what screen readers announce, for
    /// example "Width".
    pub fn new(value: &'a mut f64, name: &'a str) -> Self {
        Self {
            value,
            name,
            label: None,
            icon: None,
            unit: "",
            show_unit: true,
            conversions: &[],
            range: f64::NEG_INFINITY..=f64::INFINITY,
            step: 1.0,
            decimals: 2,
            width: None,
            mixed: false,
            id_salt: None,
        }
    }

    /// A short quiet label inside the field, before the value: `X`, `W`.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// A quiet icon inside the field, before the value.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The unit of the value, shown after it: `%`, `°`, `ms`.
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    /// A unit that is understood when typed but not displayed, for example
    /// `px` on a size field.
    pub fn silent_unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self.show_unit = false;
        self
    }

    /// Other units that can be typed, with the factor that converts them into
    /// the field's unit: `&[("s", 1000.0)]` on a millisecond field.
    pub fn conversions(mut self, conversions: &'a [(&'a str, f64)]) -> Self {
        self.conversions = conversions;
        self
    }

    /// The allowed range. Values outside it are clamped.
    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.range = range;
        self
    }

    /// How much one arrow press or one point of scrubbing changes the value.
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// The most decimal places shown.
    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    /// A fixed width instead of the available width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Shows "Mixed" instead of the value, for a selection whose members
    /// differ. Typing or scrubbing still sets the value.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// A stable id for the field. Give one when fields come and go around it.
    pub fn id_salt(mut self, salt: impl egui::AsIdSalt) -> Self {
        self.id_salt = Some(egui::IdSalt::new(salt));
        self
    }

    /// Draws the field and applies what the user did to the value.
    pub fn show(self, ui: &mut Ui) -> NumericResponse {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let id = match self.id_salt {
            Some(salt) => ui.make_persistent_id(("numeric", salt)),
            None => ui.next_auto_id(),
        };
        let edit_id = id.with("edit");
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let (_, rect) = ui.allocate_space(Vec2::new(width, theme.size.control));
        let mut response = ui.interact(rect, id, Sense::click_and_drag());

        let mut editing: Option<Editing> = ui.data(|d| d.get_temp(id));
        let mut changed = false;
        let mut finished = false;
        let multiplier = step_multiplier(ui.input(|i| i.modifiers));

        if editing.is_none() && response.clicked() {
            editing = Some(Editing {
                buffer: if self.mixed {
                    String::new()
                } else {
                    format_value(*self.value, self.decimals)
                },
                select_all: true,
            });
        }

        // Background.
        let hovered = response.hovered() || ui.rect_contains_pointer(rect);
        let scrubbing = response.dragged() && editing.is_none();
        let fill = if editing.is_some() {
            c.well
        } else if scrubbing {
            c.well_pressed
        } else if hovered {
            c.well_hover
        } else {
            c.well
        };
        let outline = if editing.is_some() {
            Stroke::new(1.0, c.focus_ring)
        } else {
            Stroke::NONE
        };
        paint::panel(ui.painter(), rect, theme.radius.sm, fill, outline);

        // Prefix: a label or an icon.
        let mut left = rect.left() + theme.space.md;
        if let Some(icon) = self.icon {
            icons::paint(
                ui.painter(),
                egui::pos2(left + 6.0, rect.center().y),
                icon,
                theme.size.icon - 2.0,
                c.text_secondary,
            );
            left += theme.size.icon + theme.space.xxs;
        } else if let Some(label) = self.label {
            let galley = paint::line(ui, label, theme.body(), c.text_secondary);
            let label_width = galley.size().x;
            let pos = Align2::LEFT_CENTER
                .align_size_within_rect(
                    galley.size(),
                    Rect::from_min_max(egui::pos2(left, rect.top()), rect.max),
                )
                .min;
            ui.painter()
                .galley(paint::snap(pos), galley, c.text_secondary);
            left += label_width.max(8.0) + theme.space.sm;
        }
        let content = Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(rect.right() - theme.space.sm, rect.bottom()),
        );

        let context = ExprContext {
            current: *self.value,
            unit: self.unit,
            conversions: self.conversions,
        };

        if let Some(mut edit) = editing {
            // Arrow keys step the value while typing.
            if ui.memory(|m| m.has_focus(edit_id)) {
                // The modifiers are read from the key event itself, so a
                // shortcut sent in one go (Shift+Down) is seen as a whole.
                let steps: f64 = ui.input_mut(|i| {
                    let mut steps = 0.0;
                    i.events.retain(|event| match event {
                        egui::Event::Key {
                            key: key @ (Key::ArrowUp | Key::ArrowDown),
                            pressed,
                            modifiers,
                            ..
                        } => {
                            if *pressed {
                                let sign = if *key == Key::ArrowUp { 1.0 } else { -1.0 };
                                steps += sign * step_multiplier(*modifiers);
                            }
                            false
                        }
                        _ => true,
                    });
                    steps
                });
                if steps != 0.0 {
                    let base = evaluate(&edit.buffer, &context).unwrap_or(*self.value);
                    let next =
                        step_value(base, steps.signum(), self.step * steps.abs(), &self.range);
                    if next != *self.value || self.mixed {
                        *self.value = next;
                        changed = true;
                    }
                    edit.buffer = format_value(next, self.decimals);
                    edit.select_all = true;
                }
            }

            let text_height = ui.text_style_height(&egui::TextStyle::Body);
            let edit_rect = Rect::from_min_max(
                egui::pos2(content.left(), rect.center().y - text_height / 2.0),
                egui::pos2(content.right(), rect.center().y + text_height / 2.0),
            );
            let text_response = ui
                .put(
                    edit_rect,
                    TextEdit::singleline(&mut edit.buffer)
                        .id(edit_id)
                        .frame(Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .font(theme.body())
                        .text_color(c.text)
                        .background_color(Color32::TRANSPARENT)
                        .desired_width(edit_rect.width()),
                )
                .accessible_name(self.name);

            if edit.select_all {
                text_response.request_focus();
                let mut state = TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::select_all_str(&edit.buffer)));
                state.store(ui.ctx(), edit_id);
                edit.select_all = false;
            }

            if text_response.lost_focus() {
                let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
                if !cancelled && let Ok(typed) = evaluate(&edit.buffer, &context) {
                    let next = clamp(typed, &self.range);
                    if next != *self.value || self.mixed {
                        *self.value = next;
                        changed = true;
                    }
                }
                finished = true;
                ui.data_mut(|d| d.remove::<Editing>(id));
            } else {
                ui.data_mut(|d| d.insert_temp(id, edit));
            }
        } else {
            // Scrubbing.
            let scrub_id = id.with("scrub");
            if response.drag_started() {
                ui.data_mut(|d| d.insert_temp(scrub_id, *self.value));
            }
            if response.dragged()
                && let Some(delta) = response.total_drag_delta()
            {
                let start = ui.data(|d| d.get_temp(scrub_id)).unwrap_or(*self.value);
                let next = scrub_value(
                    start,
                    f64::from(delta.x),
                    self.step * multiplier,
                    &self.range,
                );
                if next != *self.value {
                    *self.value = next;
                    changed = true;
                }
            }
            if response.drag_stopped() {
                finished = true;
                ui.data_mut(|d| d.remove::<f64>(scrub_id));
            }
            if hovered || scrubbing {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }

            // Value and unit.
            let (text, ink) = if self.mixed {
                ("Mixed".to_owned(), c.text_secondary)
            } else {
                (format_value(*self.value, self.decimals), c.text)
            };
            let value_galley = paint::elided(ui, &text, theme.body(), ink, content.width());
            let value_width = value_galley.size().x;
            paint::galley_in(ui.painter(), content, Align2::LEFT_CENTER, value_galley);
            if self.show_unit && !self.unit.is_empty() && !self.mixed {
                // Symbols hug the number, words get a little air.
                let gap = if self.unit.chars().all(char::is_alphabetic) {
                    theme.space.xxs + 1.0
                } else {
                    0.5
                };
                let unit_rect = Rect::from_min_max(
                    egui::pos2(content.left() + value_width + gap, content.top()),
                    content.max,
                );
                if unit_rect.width() > 4.0 {
                    paint::text_in(
                        ui,
                        unit_rect,
                        Align2::LEFT_CENTER,
                        self.unit,
                        theme.body(),
                        c.text_secondary,
                    );
                }
            }
            response.widget_info(|| {
                let mut info = egui::WidgetInfo::drag_value(ui.is_enabled(), *self.value);
                if self.mixed {
                    // Screen readers say "Mixed", not the first layer's value.
                    info.value = None;
                    info.current_text_value = Some("Mixed".to_owned());
                }
                info
            });
            response = response.accessible_name(self.name);
        }

        if changed {
            response.mark_changed();
        }
        let active = ui.data(|d| d.get_temp::<Editing>(id)).is_some() || response.dragged();
        NumericResponse {
            response,
            finished,
            active,
        }
    }
}

/// How much the modifier keys scale a step: Shift ten times, Alt a tenth.
fn step_multiplier(modifiers: Modifiers) -> f64 {
    if modifiers.shift {
        10.0
    } else if modifiers.alt {
        0.1
    } else {
        1.0
    }
}

fn clamp(value: f64, range: &RangeInclusive<f64>) -> f64 {
    value.clamp(*range.start(), *range.end())
}

/// Rounds away the binary noise a sum of steps picks up (0.1 + 0.2).
fn tidy(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

/// The value after one arrow press: `direction` is +1 or -1.
///
/// The step is added as is, so 12.3 becomes 13.3, and the result is clamped
/// to `range`.
pub fn step_value(value: f64, direction: f64, step: f64, range: &RangeInclusive<f64>) -> f64 {
    if !value.is_finite() {
        return clamp(0.0, range);
    }
    clamp(tidy(value + direction * step), range)
}

/// The value while scrubbing: the value at the start of the drag plus one
/// `step` per whole point of horizontal movement, clamped to `range`.
pub fn scrub_value(start: f64, delta_points: f64, step: f64, range: &RangeInclusive<f64>) -> f64 {
    clamp(tidy(start + delta_points.round() * step), range)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANY: RangeInclusive<f64> = f64::NEG_INFINITY..=f64::INFINITY;

    #[test]
    fn stepping_adds_the_step() {
        assert_eq!(step_value(12.0, 1.0, 1.0, &ANY), 13.0);
        assert_eq!(step_value(12.0, -1.0, 1.0, &ANY), 11.0);
        assert_eq!(step_value(12.3, 1.0, 1.0, &ANY), 13.3);
        assert_eq!(step_value(41.0, -1.0, 10.0, &ANY), 31.0);
        assert_eq!(step_value(0.2, 1.0, 0.1, &ANY), 0.3);
        assert_eq!(step_value(-0.5, -1.0, 1.0, &ANY), -1.5);
    }

    #[test]
    fn stepping_respects_the_range() {
        let range = 0.0..=100.0;
        assert_eq!(step_value(100.0, 1.0, 1.0, &range), 100.0);
        assert_eq!(step_value(0.0, -1.0, 10.0, &range), 0.0);
        assert_eq!(step_value(95.0, 1.0, 10.0, &range), 100.0);
        assert_eq!(step_value(f64::NAN, 1.0, 1.0, &range), 0.0);
    }

    #[test]
    fn scrubbing_is_relative_to_the_start_of_the_drag() {
        assert_eq!(scrub_value(50.0, 12.0, 1.0, &ANY), 62.0);
        assert_eq!(scrub_value(50.0, -12.4, 1.0, &ANY), 38.0);
        assert_eq!(scrub_value(50.0, 3.0, 10.0, &ANY), 80.0);
        assert_eq!(scrub_value(1.0, 3.0, 0.1, &ANY), 1.3);
        assert_eq!(scrub_value(12.3, 2.0, 1.0, &ANY), 14.3);
        assert_eq!(scrub_value(50.0, 500.0, 1.0, &(0.0..=100.0)), 100.0);
    }
}
