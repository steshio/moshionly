//! Tooltips with an optional keyboard-shortcut hint.

use egui::{Response, RichText, Ui};

use crate::theme::Theme;

/// Attaches a tooltip to `response`: a short text and, if given, the shortcut
/// that does the same thing, shown as a quiet hint after the text.
pub fn tooltip(response: Response, text: &str, shortcut: Option<&str>) -> Response {
    response.on_hover_ui(|ui| tooltip_contents(ui, text, shortcut))
}

fn tooltip_contents(ui: &mut Ui, text: &str, shortcut: Option<&str>) {
    let theme = Theme::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.md;
        ui.label(
            RichText::new(text)
                .font(theme.small())
                .color(theme.color.text),
        );
        if let Some(shortcut) = shortcut {
            ui.label(
                RichText::new(shortcut)
                    .font(theme.small())
                    .color(theme.color.text_secondary),
            );
        }
    });
}
