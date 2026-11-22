//! Property rows: the label-and-controls lines of the inspector.

use egui::{Align, Align2, Layout, Rect, Sense, Ui, UiBuilder, Vec2};

use super::paint;
use crate::theme::Theme;

/// Width of the label column of a [`property_row`].
pub const LABEL_WIDTH: f32 = 72.0;

/// One inspector row: a quiet label on the left and controls on the right.
///
/// The row has the standard row pitch; controls are centred in it and laid
/// out left to right in the space after the label column.
pub fn property_row<R>(ui: &mut Ui, label: &str, controls: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.row),
        Sense::hover(),
    );
    let label_rect = Rect::from_min_size(rect.min, Vec2::new(LABEL_WIDTH, rect.height()));
    paint::text_in(
        ui,
        label_rect,
        Align2::LEFT_CENTER,
        label,
        theme.body(),
        theme.color.text_secondary,
    );
    let controls_rect =
        Rect::from_min_max(egui::pos2(rect.left() + LABEL_WIDTH, rect.top()), rect.max);
    controls_in(ui, controls_rect, controls)
}

/// One inspector row without a label: controls use the full width.
pub fn control_row<R>(ui: &mut Ui, controls: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.row),
        Sense::hover(),
    );
    controls_in(ui, rect, controls)
}

/// Lays out two controls side by side, each taking half the available width.
///
/// Use it inside a row for pairs such as X and Y or width and height.
pub fn pair(ui: &mut Ui, first: impl FnOnce(&mut Ui), second: impl FnOnce(&mut Ui)) {
    let theme = Theme::of(ui);
    let gap = theme.space.md;
    let rect = ui.available_rect_before_wrap();
    let half = ((rect.width() - gap) / 2.0).max(0.0);
    let height = theme.size.control;
    let top = rect.center().y - height / 2.0;
    let left = Rect::from_min_size(egui::pos2(rect.left(), top), Vec2::new(half, height));
    let right = Rect::from_min_size(
        egui::pos2(rect.left() + half + gap, top),
        Vec2::new(half, height),
    );
    controls_in(ui, left, first);
    controls_in(ui, right, second);
    ui.allocate_rect(left.union(right), Sense::hover());
}

fn controls_in<R>(ui: &mut Ui, rect: Rect, controls: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = Theme::of(ui);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = theme.space.md;
    controls(&mut child)
}
