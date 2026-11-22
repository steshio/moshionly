//! Section headers and sections: the titled blocks of the side panels.

use egui::{Align, Align2, Layout, Rect, Response, Sense, Ui, UiBuilder, Vec2};

use super::paint;
use crate::theme::Theme;

/// Paints a full-width hairline, the separator between sections.
pub fn divider(ui: &mut Ui) {
    let theme = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, theme.hairline());
}

/// A section header: a semibold title on the left and room for small actions
/// (usually [`IconButton`](super::IconButton)s) on the right.
///
/// With `open` set, the header also shows an expand arrow and toggles the
/// flag when its title is clicked. Actions are laid out right to left, so add
/// the outermost one first.
pub fn section_header(
    ui: &mut Ui,
    title: &str,
    open: Option<&mut bool>,
    trailing: impl FnOnce(&mut Ui),
) -> Response {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let pad = theme.space.xl;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.section_header),
        Sense::hover(),
    );

    // Trailing actions first, so the title knows how much room is left.
    let actions_rect = Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top()),
        egui::pos2(rect.right() - pad + theme.space.xs, rect.bottom()),
    );
    let mut actions = ui.new_child(
        UiBuilder::new()
            .max_rect(actions_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    actions.spacing_mut().item_spacing.x = theme.space.xxs;
    trailing(&mut actions);
    let actions_left = if actions.min_rect().width() > 0.0 {
        actions.min_rect().left() - theme.space.sm
    } else {
        rect.right() - pad
    };

    let title_rect = Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top()),
        egui::pos2(actions_left.max(rect.left() + pad), rect.bottom()),
    );
    let collapsible = open.is_some();
    let sense = if collapsible {
        Sense::click()
    } else {
        Sense::hover()
    };
    let mut response = ui.interact(title_rect, ui.make_persistent_id(("section", title)), sense);

    let mut text_rect = title_rect;
    if let Some(open) = open {
        if response.clicked() {
            *open = !*open;
            response.mark_changed();
        }
        // The arrow hangs in the panel's left padding so titles stay aligned.
        let arrow = egui::pos2(rect.left() + pad / 2.0, rect.center().y);
        let ink = if response.hovered() {
            c.text
        } else {
            c.text_tertiary
        };
        paint::disclosure(ui.painter(), arrow, *open, ink);
        text_rect = title_rect;
    }
    paint::text_in(
        ui,
        text_rect,
        Align2::LEFT_CENTER,
        title,
        theme.body_strong(),
        c.text,
    );
    response
}

/// A whole section: a divider, a collapsible header and, when open, `body`
/// inside the panel's side padding.
///
/// `id_salt` keeps sections with the same title apart. Returns the header's
/// response.
pub fn section(
    ui: &mut Ui,
    title: &str,
    open: &mut bool,
    trailing: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) -> Response {
    let theme = Theme::of(ui);
    divider(ui);
    let response = section_header(ui, title, Some(open), trailing);
    if *open {
        padded(ui, body);
        ui.add_space(theme.space.md);
    }
    response
}

/// Runs `body` inside the panel's side padding, top to bottom.
pub fn padded<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = Theme::of(ui);
    let pad = theme.space.xl;
    let outer = ui.available_rect_before_wrap();
    let inner = Rect::from_min_max(
        egui::pos2(outer.left() + pad, outer.top()),
        egui::pos2(outer.right() - pad, outer.bottom()),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 0.0;
    let result = body(&mut child);
    let used = child.min_rect();
    ui.allocate_rect(
        Rect::from_min_max(outer.min, egui::pos2(outer.right(), used.bottom())),
        Sense::hover(),
    );
    result
}
