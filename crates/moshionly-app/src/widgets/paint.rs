//! Small painting helpers shared by the widgets.

use std::sync::Arc;

use egui::{
    Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2,
    epaint::Shadow,
    text::{LayoutJob, TextWrapping},
};

use crate::theme::corner;

/// Lays out one line of text without wrapping.
pub fn line(ui: &Ui, text: &str, font: FontId, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), font, color)
}

/// Lays out one line of text, cut off with an ellipsis if it is wider than
/// `max_width`.
pub fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.painter().layout_job(job)
}

/// Paints a galley so that its box is placed in `rect` according to `align`.
pub fn galley_in(painter: &Painter, rect: Rect, align: Align2, galley: Arc<Galley>) {
    let pos = align.align_size_within_rect(galley.size(), rect).min;
    painter.galley(snap(pos), galley, Color32::PLACEHOLDER);
}

/// Paints one line of text aligned inside `rect`, elided to fit.
pub fn text_in(ui: &Ui, rect: Rect, align: Align2, text: &str, font: FontId, color: Color32) {
    let galley = elided(ui, text, font, color, rect.width());
    galley_in(ui.painter(), rect, align, galley);
}

/// Rounds a position to whole points so text and hairlines stay crisp.
pub fn snap(pos: Pos2) -> Pos2 {
    Pos2::new(pos.x.round(), pos.y.round())
}

/// Paints a filled, optionally outlined, rounded rectangle.
pub fn panel(painter: &Painter, rect: Rect, radius: f32, fill: Color32, outline: Stroke) {
    painter.rect(rect, corner(radius), fill, outline, StrokeKind::Inside);
}

/// Paints a stack of shadows behind a rounded rectangle.
pub fn shadows(painter: &Painter, rect: Rect, radius: f32, shadows: &[Shadow]) {
    for shadow in shadows {
        painter.add(shadow.as_shape(rect, corner(radius)));
    }
}

/// Paints a small solid triangle pointing down (`open`) or right (closed),
/// used as the expand arrow of rows and sections.
pub fn disclosure(painter: &Painter, center: Pos2, open: bool, color: Color32) {
    let r = 3.0;
    let points = if open {
        vec![
            center + Vec2::new(-r - 0.5, -r * 0.5),
            center + Vec2::new(r + 0.5, -r * 0.5),
            center + Vec2::new(0.0, r * 0.8),
        ]
    } else {
        vec![
            center + Vec2::new(-r * 0.5, -r - 0.5),
            center + Vec2::new(-r * 0.5, r + 0.5),
            center + Vec2::new(r * 0.8, 0.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Paints a checkerboard, the conventional backdrop for transparency.
pub fn checkerboard(painter: &Painter, rect: Rect, cell: f32, light: Color32, dark: Color32) {
    painter.rect_filled(rect, 0.0, light);
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    for row in 0..rows {
        for col in 0..cols {
            if (row + col) % 2 == 1 {
                let min = rect.min + Vec2::new(col as f32 * cell, row as f32 * cell);
                painter.rect_filled(Rect::from_min_size(min, Vec2::splat(cell)), 0.0, dark);
            }
        }
    }
}
