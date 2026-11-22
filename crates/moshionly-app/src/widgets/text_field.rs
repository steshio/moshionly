//! Text field and search field.

use egui::{Color32, Frame, Rect, Response, Sense, Stroke, TextEdit, Ui, Vec2};

use super::paint;
use crate::theme::{Theme, icons};

/// A single-line text field on a grey well, with an optional leading icon and
/// placeholder text. It takes the available width unless [`width`](Self::width)
/// is set.
#[must_use = "call show()"]
pub struct TextField<'a> {
    text: &'a mut String,
    hint: &'a str,
    name: Option<&'a str>,
    icon: Option<&'a str>,
    width: Option<f32>,
    clearable: bool,
    id: Option<egui::Id>,
}

/// What a [`TextField`] reports back.
pub struct TextFieldResponse {
    /// The response of the text input itself. `changed()` is true when the
    /// text was edited, `lost_focus()` when editing ended.
    pub response: Response,
    /// True on the frame the user pressed Enter in the field.
    pub submitted: bool,
}

impl<'a> TextField<'a> {
    /// A field editing `text`.
    pub fn new(text: &'a mut String) -> Self {
        Self {
            text,
            hint: "",
            name: None,
            icon: None,
            width: None,
            clearable: false,
            id: None,
        }
    }

    /// Placeholder shown while the field is empty.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// The name screen readers announce. Defaults to the placeholder.
    pub fn name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// A leading icon.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A fixed width instead of the available width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Shows a small clear button while the field has text.
    pub fn clearable(mut self, clearable: bool) -> Self {
        self.clearable = clearable;
        self
    }

    /// A stable id, needed to move keyboard focus to the field from outside.
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Draws the field.
    pub fn show(self, ui: &mut Ui) -> TextFieldResponse {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(width, theme.size.control), Sense::hover());

        let id = self.id.unwrap_or_else(|| ui.next_auto_id());
        let focused = ui.memory(|m| m.has_focus(id));
        let hovered = ui.rect_contains_pointer(rect);
        let fill = if hovered && !focused {
            c.well_hover
        } else {
            c.well
        };
        let outline = if focused {
            Stroke::new(1.0, c.focus_ring)
        } else {
            Stroke::NONE
        };
        paint::panel(ui.painter(), rect, theme.radius.sm, fill, outline);

        let mut left = rect.left() + theme.space.md;
        if let Some(icon) = self.icon {
            icons::paint(
                ui.painter(),
                egui::pos2(left + theme.size.icon / 2.0 - 1.0, rect.center().y),
                icon,
                theme.size.icon - 2.0,
                c.text_secondary,
            );
            left += theme.size.icon + theme.space.xs;
        }
        let show_clear = self.clearable && !self.text.is_empty();
        let right = if show_clear {
            rect.right() - theme.size.control
        } else {
            rect.right() - theme.space.md
        };

        let text_height = ui.text_style_height(&egui::TextStyle::Body);
        let edit_rect = Rect::from_min_max(
            egui::pos2(left, rect.center().y - text_height / 2.0),
            egui::pos2(right, rect.center().y + text_height / 2.0),
        );
        let response = ui.put(
            edit_rect,
            TextEdit::singleline(self.text)
                .id(id)
                .hint_text(egui::RichText::new(self.hint).color(c.text_tertiary))
                .frame(Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(theme.body())
                .text_color(c.text)
                .background_color(Color32::TRANSPARENT)
                .desired_width(edit_rect.width()),
        );
        let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

        let name = self.name.unwrap_or(if self.hint.is_empty() {
            "Text"
        } else {
            self.hint
        });
        let mut response = response.accessible_name(name);
        if show_clear {
            let clear_rect = Rect::from_min_max(
                egui::pos2(rect.right() - theme.size.control, rect.top()),
                rect.max,
            );
            let clear = ui.interact(clear_rect, id.with("clear"), Sense::click());
            let ink = if clear.hovered() {
                c.text
            } else {
                c.text_secondary
            };
            icons::paint(ui.painter(), clear_rect.center(), icons::X, 12.0, ink);
            if clear.clicked() {
                self.text.clear();
                response.mark_changed();
            }
        }

        TextFieldResponse {
            response,
            submitted,
        }
    }
}

/// A search field: a text field with a magnifying glass, a "Search"
/// placeholder and a clear button.
pub fn search_field(ui: &mut Ui, query: &mut String) -> TextFieldResponse {
    TextField::new(query)
        .hint("Search")
        .icon(icons::MAGNIFYING_GLASS)
        .clearable(true)
        .show(ui)
}
