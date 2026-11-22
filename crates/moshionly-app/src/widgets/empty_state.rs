//! Empty-state block: what a panel shows when it has nothing to list.

use egui::{Align2, Response, Sense, Ui, Vec2};

use super::{TextButton, paint};
use crate::theme::{Theme, icons};

/// A centred icon, headline and explanation, with an optional action button.
///
/// Fills the available width and is as tall as its content. Returns the
/// button's response when `action` is given.
pub fn empty_state(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    body: &str,
    action: Option<&str>,
) -> Option<Response> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let width = ui.available_width();
    let text_width = (width - theme.space.xl * 2.0).clamp(80.0, 260.0);

    let mut job =
        egui::text::LayoutJob::simple(body.to_owned(), theme.body(), c.text_secondary, text_width);
    job.halign = egui::Align::Center;
    let body_galley = ui.painter().layout_job(job);

    let badge = 40.0;
    let height = theme.space.xxl
        + badge
        + theme.space.lg
        + theme.type_scale.title * 1.4
        + theme.space.xs
        + body_galley.size().y
        + theme.space.lg;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());

    let mut y = rect.top() + theme.space.xxl;
    let badge_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, y + badge / 2.0),
        Vec2::splat(badge),
    );
    ui.painter()
        .rect_filled(badge_rect, theme.radius.md, c.well);
    icons::paint(
        ui.painter(),
        badge_rect.center(),
        icon,
        theme.size.icon_large,
        c.text_secondary,
    );
    y += badge + theme.space.lg;

    let title_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), y),
        Vec2::new(width, theme.type_scale.title * 1.4),
    );
    paint::text_in(
        ui,
        title_rect,
        Align2::CENTER_CENTER,
        title,
        theme.title(),
        c.text,
    );
    y += title_rect.height() + theme.space.xs;

    ui.painter().galley(
        paint::snap(egui::pos2(rect.center().x, y)),
        body_galley,
        c.text_secondary,
    );

    action.map(|label| {
        ui.vertical_centered(|ui| ui.add(TextButton::new(label)))
            .inner
    })
}
