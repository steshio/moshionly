//! Splitters: the draggable edges between panels.

use std::ops::RangeInclusive;

use egui::{CursorIcon, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::theme::Theme;

/// Which way a panel grows when its splitter is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grow {
    /// The panel is on the left of the splitter: dragging right widens it.
    Right,
    /// The panel is on the right of the splitter: dragging left widens it.
    Left,
    /// The panel is below the splitter: dragging up makes it taller.
    Up,
}

/// A draggable panel edge.
///
/// `edge` is the line between two panels (a rect of zero or one point
/// thickness). The splitter senses a few points either side of it, shows a
/// resize cursor, highlights the line while active, and changes `size` when
/// dragged. A double click resets `size` to `default`.
///
/// Returns the response; `changed()` is true while the size is changing and
/// `drag_stopped()` when it is time to remember the new size.
pub fn splitter(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    edge: Rect,
    grow: Grow,
    size: &mut f32,
    range: RangeInclusive<f32>,
    default: f32,
) -> Response {
    let theme = Theme::of(ui);
    let grab = theme.size.splitter_grab;
    let vertical_line = matches!(grow, Grow::Left | Grow::Right);
    let hit = if vertical_line {
        edge.expand2(Vec2::new(grab / 2.0, 0.0))
    } else {
        edge.expand2(Vec2::new(0.0, grab / 2.0))
    };
    let id = ui.make_persistent_id(("splitter", egui::IdSalt::new(id_salt)));
    let mut response = ui.interact(hit, id, Sense::click_and_drag());

    // The size is measured from where the drag began, so the edge stays under
    // the pointer even after the pointer has gone past a limit and come back.
    if response.drag_started() {
        ui.data_mut(|d| d.insert_temp(id, *size));
    }
    if response.double_clicked() {
        *size = default.clamp(*range.start(), *range.end());
        response.mark_changed();
    } else if response.dragged()
        && let Some(delta) = response.total_drag_delta()
    {
        let start = ui.data(|d| d.get_temp(id)).unwrap_or(*size);
        let next = resized(start, delta, grow, &range);
        if next != *size {
            *size = next;
            response.mark_changed();
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<f32>(id));
    }

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(if vertical_line {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        });
        let stroke = Stroke::new(
            if response.dragged() { 2.0 } else { 1.0 },
            theme.color.accent,
        );
        if vertical_line {
            ui.painter().vline(edge.center().x, edge.y_range(), stroke);
        } else {
            ui.painter().hline(edge.x_range(), edge.center().y, stroke);
        }
    }
    response
}

/// The size of a panel that was `size` when its splitter began to move, after
/// the splitter has moved by `delta`.
pub fn resized(size: f32, delta: Vec2, grow: Grow, range: &RangeInclusive<f32>) -> f32 {
    let change = match grow {
        Grow::Right => delta.x,
        Grow::Left => -delta.x,
        Grow::Up => -delta.y,
    };
    (size + change).clamp(*range.start(), *range.end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_direction_grows_the_right_way() {
        let range = 100.0..=400.0;
        let d = Vec2::new(10.0, -20.0);
        assert_eq!(resized(200.0, d, Grow::Right, &range), 210.0);
        assert_eq!(resized(200.0, d, Grow::Left, &range), 190.0);
        assert_eq!(resized(200.0, d, Grow::Up, &range), 220.0);
    }

    #[test]
    fn sizes_stay_in_range() {
        let range = 100.0..=400.0;
        assert_eq!(
            resized(390.0, Vec2::new(50.0, 0.0), Grow::Right, &range),
            400.0
        );
        assert_eq!(
            resized(110.0, Vec2::new(50.0, 0.0), Grow::Left, &range),
            100.0
        );
    }
}
