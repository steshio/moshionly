//! Colour swatch and the colour field of the inspector.

use egui::{
    Align2, Color32, Frame, Key, Popup, PopupCloseBehavior, Rect, Response, Sense, Stroke,
    StrokeKind, TextEdit, Ui, Vec2, WidgetInfo, text::CCursorRange, text_edit::TextEditState,
};

use super::{
    Rgba,
    picker::{PickerResponse, color_picker},
};
use crate::{
    theme::Theme,
    widgets::{NumericField, menu::popup_frame, paint},
};

/// Paints a colour chip of `size` points: the colour over a checkerboard when
/// it is translucent, with a hairline so pale colours stay visible.
pub fn paint_swatch(ui: &Ui, rect: Rect, color: Rgba) {
    let theme = Theme::of(ui);
    let radius = theme.radius.xs;
    if color.a < 255 {
        let painter = ui.painter().with_clip_rect(rect.shrink(0.5));
        paint::checkerboard(
            &painter,
            rect,
            rect.width() / 4.0,
            theme.color.checker_light,
            theme.color.checker_dark,
        );
    }
    ui.painter().rect_filled(rect, radius, color.to_color32());
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme.color.border_strong.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
}

/// A clickable colour chip. Returns its response; pair it with a popup or use
/// [`ColorField`], which does that for you.
pub fn color_swatch(ui: &mut Ui, color: Rgba, name: &str) -> Response {
    let theme = Theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme.size.icon), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
    if ui.is_rect_visible(rect) {
        paint_swatch(ui, rect, color);
    }
    response
}

/// What a colour control reports back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorResponse {
    /// The colour changed this frame.
    pub changed: bool,
    /// An edit finished this frame: use it to close an undo step.
    pub finished: bool,
}

impl From<PickerResponse> for ColorResponse {
    fn from(picker: PickerResponse) -> Self {
        Self {
            changed: picker.changed,
            finished: picker.finished,
        }
    }
}

/// The colour field of the inspector: a swatch that opens the picker, the hex
/// value, and the opacity in percent.
#[must_use = "call show()"]
pub struct ColorField<'a> {
    color: &'a mut Rgba,
    name: &'a str,
    width: Option<f32>,
    id_salt: Option<egui::IdSalt>,
}

impl<'a> ColorField<'a> {
    /// A field editing `color`. `name` is what screen readers announce, for
    /// example "Fill".
    pub fn new(color: &'a mut Rgba, name: &'a str) -> Self {
        Self {
            color,
            name,
            width: None,
            id_salt: None,
        }
    }

    /// A fixed width instead of the available width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Tells this field apart from others with the same name in one panel,
    /// such as the stops of a gradient. Without it the field's widgets are
    /// named after `name`, and two fields with one name would share their
    /// state.
    pub fn id_salt(mut self, salt: impl egui::AsIdSalt) -> Self {
        self.id_salt = Some(egui::IdSalt::new(salt));
        self
    }

    /// Draws the field and its picker popup.
    pub fn show(self, ui: &mut Ui) -> ColorResponse {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let mut out = ColorResponse::default();
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let id = match self.id_salt {
            Some(salt) => ui.make_persistent_id(("color-field", salt)),
            None => ui.make_persistent_id(("color-field", self.name)),
        };
        let (_, rect) = ui.allocate_space(Vec2::new(width, theme.size.control));
        ui.painter().rect_filled(rect, theme.radius.sm, c.well);

        // Swatch.
        let swatch_rect = Rect::from_center_size(
            egui::pos2(
                rect.left() + theme.space.xs + theme.size.icon / 2.0,
                rect.center().y,
            ),
            Vec2::splat(theme.size.icon),
        );
        let swatch = ui.interact(swatch_rect.expand(2.0), id.with("swatch"), Sense::click());
        swatch.widget_info(|| {
            WidgetInfo::labeled(
                egui::Role::Button,
                ui.is_enabled(),
                format!("{} colour", self.name),
            )
        });
        paint_swatch(ui, swatch_rect, *self.color);

        // Opacity on the right, behind a hairline.
        let opacity_width = 52.0;
        let opacity_rect = Rect::from_min_max(
            egui::pos2(rect.right() - opacity_width, rect.top()),
            rect.max,
        );
        ui.painter().vline(
            opacity_rect.left(),
            rect.y_range().shrink(4.0),
            Stroke::new(1.0, c.border_strong.gamma_multiply(0.7)),
        );
        let mut opacity = self.color.opacity();
        let numeric = ui
            .scope_builder(egui::UiBuilder::new().max_rect(opacity_rect), |ui| {
                // The field paints its own well; make it blend into ours.
                NumericField::new(&mut opacity, "Opacity")
                    .unit("%")
                    .range(0.0..=100.0)
                    .width(opacity_width)
                    .id_salt(id.with("opacity"))
                    .show(ui)
            })
            .inner;
        if numeric.response.changed() {
            *self.color = self.color.with_opacity(opacity);
            out.changed = true;
        }
        out.finished |= numeric.finished;

        // Hex in the middle.
        let hex_rect = Rect::from_min_max(
            egui::pos2(swatch_rect.right() + theme.space.sm, rect.top()),
            egui::pos2(opacity_rect.left() - theme.space.xs, rect.bottom()),
        );
        let hex = hex_text(ui, id.with("hex"), hex_rect, self.color);
        out.changed |= hex.changed;
        out.finished |= hex.finished;

        // Picker popup.
        Popup::from_toggle_button_response(&swatch)
            .frame(popup_frame(ui).inner_margin(theme.space.lg as i8))
            .gap(theme.space.sm)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                let picker = color_picker(ui, id.with("picker"), self.color);
                out.changed |= picker.changed;
                out.finished |= picker.finished;
            });
        out
    }
}

/// A standalone hex input on its own well, used inside the picker.
pub(super) fn hex_input(ui: &mut Ui, id: egui::Id, color: &mut Rgba, width: f32) -> ColorResponse {
    let theme = Theme::of(ui);
    let (_, rect) = ui.allocate_space(Vec2::new(width, theme.size.control));
    ui.painter()
        .rect_filled(rect, theme.radius.sm, theme.color.well);
    let prefix = Rect::from_min_size(rect.min, Vec2::new(theme.space.xl + 4.0, rect.height()));
    paint::text_in(
        ui,
        prefix,
        Align2::CENTER_CENTER,
        "#",
        theme.body(),
        theme.color.text_secondary,
    );
    let text_rect = Rect::from_min_max(
        egui::pos2(prefix.right(), rect.top()),
        egui::pos2(rect.right() - theme.space.sm, rect.bottom()),
    );
    hex_text(ui, id, text_rect, color)
}

/// The typing state of a hex field, kept in egui's temporary memory.
#[derive(Clone, Default)]
struct HexEditing {
    buffer: String,
    select_all: bool,
}

/// Hex digits that become an editor when clicked. Enter or focus loss
/// applies a valid colour, Escape or an invalid one keeps the old colour.
/// Alpha is only replaced when eight digits are typed.
fn hex_text(ui: &mut Ui, id: egui::Id, rect: Rect, color: &mut Rgba) -> ColorResponse {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut out = ColorResponse::default();
    let response = ui.interact(rect, id, Sense::click());
    let mut editing: Option<HexEditing> = ui.data(|d| d.get_temp(id));

    if editing.is_none() && response.clicked() {
        editing = Some(HexEditing {
            buffer: color.hex_rgb(),
            select_all: true,
        });
    }

    let Some(mut edit) = editing else {
        response
            .widget_info(|| WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Hex colour"));
        paint::text_in(
            ui,
            rect,
            Align2::LEFT_CENTER,
            &color.hex_rgb(),
            theme.body(),
            c.text,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        return out;
    };

    let edit_id = id.with("edit");
    let text_height = ui.text_style_height(&egui::TextStyle::Body);
    let edit_rect = Rect::from_min_max(
        egui::pos2(rect.left(), rect.center().y - text_height / 2.0),
        egui::pos2(rect.right(), rect.center().y + text_height / 2.0),
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
                .char_limit(9)
                .desired_width(edit_rect.width()),
        )
        .accessible_name("Hex colour");
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
        if !cancelled
            && let Some(next) = apply_hex(*color, &edit.buffer)
            && next != *color
        {
            *color = next;
            out.changed = true;
        }
        out.finished = true;
        ui.data_mut(|d| d.remove::<HexEditing>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, edit));
    }
    out
}

/// The colour after typing `text` into a hex field showing `current`.
///
/// Three or six digits replace the colour and keep the current alpha; four or
/// eight digits replace the alpha too. Returns `None` for invalid text.
pub fn apply_hex(current: Rgba, text: &str) -> Option<Rgba> {
    let typed = Rgba::from_hex(text).ok()?;
    let digits = text.trim().trim_start_matches('#').chars().count();
    Some(if digits == 3 || digits == 6 {
        Rgba {
            a: current.a,
            ..typed
        }
    } else {
        typed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_six_digits_keeps_the_alpha() {
        let current = Rgba::new(1, 2, 3, 128);
        assert_eq!(
            apply_hex(current, "FF0000"),
            Some(Rgba::new(255, 0, 0, 128))
        );
        assert_eq!(apply_hex(current, "#0f0"), Some(Rgba::new(0, 255, 0, 128)));
    }

    #[test]
    fn typing_eight_digits_replaces_the_alpha() {
        let current = Rgba::new(1, 2, 3, 128);
        assert_eq!(
            apply_hex(current, "#FF000040"),
            Some(Rgba::new(255, 0, 0, 0x40))
        );
        assert_eq!(apply_hex(current, "f00f"), Some(Rgba::new(255, 0, 0, 255)));
    }

    #[test]
    fn invalid_text_changes_nothing() {
        let current = Rgba::new(1, 2, 3, 128);
        assert_eq!(apply_hex(current, "nope"), None);
        assert_eq!(apply_hex(current, ""), None);
        assert_eq!(apply_hex(current, "#12345"), None);
    }
}
