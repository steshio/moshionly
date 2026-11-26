//! Segmented control and tabs: pick one of a few options shown side by side.

use egui::{Align2, Color32, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::paint;
use crate::theme::Theme;

/// A segmented control: a grey well with one raised tile on the chosen option.
///
/// Options share the available width equally. Used for the Design / Animate
/// switch of the inspector. Returns a response whose `changed()` is true when
/// the user picked a different option.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    segmented_mixed(ui, value, options, false)
}

/// A [`segmented`] control that can say the layers being edited differ
/// (`mixed`): then no option is raised, and choosing any sets it for all.
pub fn segmented_mixed<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
    mixed: bool,
) -> Response {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let height = theme.size.control + 4.0;
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    if options.is_empty() {
        return response;
    }

    ui.painter()
        .rect_filled(rect, theme.radius.sm + 1.0, c.well);
    let inner = rect.shrink(2.0);
    let width = inner.width() / options.len() as f32;
    for (index, (option, label)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(
            inner.min + Vec2::new(width * index as f32, 0.0),
            Vec2::new(width, inner.height()),
        );
        let cell_response = ui.interact(cell, response.id.with(index), Sense::click());
        let selected = !mixed && *value == *option;
        if selected {
            paint::shadows(
                ui.painter(),
                cell,
                theme.radius.sm,
                &theme.shadow.float[..1],
            );
            paint::panel(
                ui.painter(),
                cell,
                theme.radius.sm,
                if theme.dark { c.well_pressed } else { c.raised },
                Stroke::NONE,
            );
        }
        let (font, ink) = if selected {
            (theme.body_medium(), c.text)
        } else if cell_response.hovered() {
            (theme.body(), c.text)
        } else {
            (theme.body(), c.text_secondary)
        };
        paint::text_in(ui, cell, Align2::CENTER_CENTER, label, font, ink);
        if cell_response.clicked() && !selected {
            *value = *option;
            response.mark_changed();
        }
    }
    response
}

/// A row of text tabs: the chosen one in full-strength text with an accent
/// underline, the others quiet. Tabs are as wide as their labels.
///
/// Returns a response whose `changed()` is true when the user switched tab.
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let height = theme.size.section_header;
    let gap = theme.space.xl;

    let galleys: Vec<_> = options
        .iter()
        .map(|(option, label)| {
            let font = if *value == *option {
                theme.body_strong()
            } else {
                theme.body()
            };
            paint::line(ui, label, font, Color32::PLACEHOLDER)
        })
        .collect();
    let total: f32 = galleys.iter().map(|g| g.size().x).sum::<f32>()
        + gap * options.len().saturating_sub(1) as f32;

    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(total, height), Sense::hover());
    let mut x = rect.left();
    for (index, ((option, label), galley)) in options.iter().zip(galleys).enumerate() {
        let width = galley.size().x;
        let cell = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(width, height));
        let cell_response = ui.interact(
            cell.expand2(Vec2::new(gap / 2.0, 0.0)),
            response.id.with(index),
            Sense::click(),
        );
        let selected = *value == *option;
        cell_response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, *label));
        let ink = if selected || cell_response.hovered() {
            c.text
        } else {
            c.text_secondary
        };
        let pos = Align2::LEFT_CENTER
            .align_size_within_rect(galley.size(), cell)
            .min;
        ui.painter().galley(paint::snap(pos), galley, ink);
        if selected {
            let y = cell.bottom() - 1.0;
            ui.painter()
                .hline(cell.x_range(), y, Stroke::new(2.0, c.accent));
        }
        if cell_response.clicked() && !selected {
            *value = *option;
            response.mark_changed();
        }
        x += width + gap;
    }
    response
}
