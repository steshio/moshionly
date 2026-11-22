//! Where the parts of a timeline are.
//!
//! ```text
//! +---------------------+-----------------------------------+--------+
//! | transport           | ruler                             | zoom   |
//! +---------------------+-----------------------------------+--------+
//! | labels              | tracks                                     |
//! |                     |                                            |
//! +---------------------+--------------------------------------------+
//! ```

use egui::{Rect, pos2};

use super::{scale::TimeScale, state::TimelineState};
use crate::theme::Theme;

/// Room between the left edge of the tracks and time zero, so the first bar
/// and the playhead at zero are not jammed against the label column.
pub const TRACK_PAD: f32 = 16.0;

/// Narrowest the label column can be dragged.
pub const LABELS_MIN: f32 = 140.0;
/// Least width the tracks keep when the label column is dragged wider.
pub const TRACKS_MIN: f32 = 200.0;
/// Width of the zoom controls at the right end of the header.
const ZOOM_WIDTH: f32 = 152.0;

/// The rectangles of a timeline and its time scale, for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineLayout {
    /// The whole timeline.
    pub rect: Rect,
    /// The transport controls, above the label column.
    pub transport: Rect,
    /// The ruler, above the tracks.
    pub ruler: Rect,
    /// The zoom controls, over the right end of the ruler.
    pub zoom: Rect,
    /// The label column.
    pub labels: Rect,
    /// The area the bars are drawn in.
    pub tracks: Rect,
    /// The mapping between time and screen x in the ruler and the tracks.
    pub scale: TimeScale,
    /// The vertical pitch of rows.
    pub row_height: f32,
    /// The height of a bar.
    pub bar_height: f32,
    /// How far the rows are scrolled.
    pub scroll_y: f32,
}

impl TimelineLayout {
    /// Lays a timeline out in `rect` for the given zoom and scroll.
    pub fn compute(rect: Rect, state: &TimelineState, theme: &Theme) -> Self {
        let size = &theme.size;
        let header = size.timeline_header.min(rect.height());
        let label_width = label_width(state.label_width, rect.width());
        let split = rect.left() + label_width;
        let body_top = rect.top() + header;

        let ruler = Rect::from_min_max(pos2(split, rect.top()), pos2(rect.right(), body_top));
        let zoom_width = ZOOM_WIDTH.min(ruler.width());
        Self {
            rect,
            transport: Rect::from_min_max(rect.min, pos2(split, body_top)),
            ruler,
            zoom: Rect::from_min_max(pos2(ruler.right() - zoom_width, ruler.top()), ruler.max),
            labels: Rect::from_min_max(pos2(rect.left(), body_top), pos2(split, rect.bottom())),
            tracks: Rect::from_min_max(pos2(split, body_top), rect.max),
            scale: TimeScale {
                origin: split + TRACK_PAD - state.scroll_x,
                points_per_ms: state.points_per_ms(),
            },
            row_height: size.timeline_row,
            bar_height: size.timeline_bar,
            scroll_y: state.scroll_y,
        }
    }

    /// The width the scene should take when it is zoomed to fit: from time
    /// zero to a little before the zoom controls, so the tab at the end of
    /// the scene stays in reach.
    pub fn fit_width(&self) -> f32 {
        (self.zoom.left() - self.tracks.left() - TRACK_PAD * 2.0).max(1.0)
    }

    /// The top of the row at `index`, on screen.
    pub fn row_top(&self, index: usize) -> f32 {
        self.tracks.top() - self.scroll_y + index as f32 * self.row_height
    }

    /// The index of the row at a screen y, if `y` is at or below the first
    /// row. It may be past the last row.
    pub fn row_at(&self, y: f32) -> Option<usize> {
        let offset = y - self.tracks.top() + self.scroll_y;
        (offset >= 0.0).then(|| (offset / self.row_height) as usize)
    }

    /// The rectangle of a bar from `start` to `end` on the row at `index`.
    /// Very short bars are widened to stay visible and grabbable.
    pub fn bar_rect(&self, index: usize, start: f64, end: f64) -> Rect {
        let left = self.scale.x(start);
        let right = self.scale.x(end).max(left + MIN_BAR_WIDTH);
        let top = self.row_top(index) + (self.row_height - self.bar_height) / 2.0;
        Rect::from_min_max(pos2(left, top), pos2(right, top + self.bar_height))
    }

    /// The whole row at `index`, across the label column and the tracks.
    pub fn row_rect(&self, index: usize) -> Rect {
        let top = self.row_top(index);
        Rect::from_min_max(
            pos2(self.rect.left(), top),
            pos2(self.rect.right(), top + self.row_height),
        )
    }
}

/// Narrowest a bar is drawn, however short its action.
pub const MIN_BAR_WIDTH: f32 = 4.0;

/// The label column's width for a timeline `total` points wide: the wanted
/// width, kept between the column's minimum and what leaves the tracks their
/// minimum.
pub fn label_width(wanted: f32, total: f32) -> f32 {
    let max = (total - TRACKS_MIN).max(LABELS_MIN);
    wanted.clamp(LABELS_MIN, max).min(total.max(0.0))
}

#[cfg(test)]
mod tests {
    use egui::vec2;

    use super::*;

    fn layout(state: &TimelineState) -> TimelineLayout {
        let rect = Rect::from_min_size(pos2(0.0, 652.0), vec2(1440.0, 248.0));
        TimelineLayout::compute(rect, state, Theme::light())
    }

    #[test]
    fn the_parts_tile_the_timeline() {
        let state = TimelineState::default();
        let l = layout(&state);
        let size = &Theme::light().size;
        assert_eq!(l.transport.width(), state.label_width);
        assert_eq!(l.transport.height(), size.timeline_header);
        assert_eq!(l.ruler.left(), l.transport.right());
        assert_eq!(l.ruler.right(), 1440.0);
        assert_eq!(l.labels.top(), l.transport.bottom());
        assert_eq!(l.tracks.min, pos2(l.labels.right(), l.ruler.bottom()));
        assert_eq!(l.tracks.max, l.rect.max);
        assert!(l.ruler.contains_rect(l.zoom));
    }

    #[test]
    fn time_zero_sits_a_little_inside_the_tracks_and_scrolls() {
        let mut state = TimelineState::default();
        let l = layout(&state);
        assert_eq!(l.scale.x(0.0), l.tracks.left() + TRACK_PAD);
        // 200 points per second.
        assert!((l.scale.x(1000.0) - (l.tracks.left() + TRACK_PAD + 200.0)).abs() < 1e-3);
        state.scroll_x = 120.0;
        let scrolled = layout(&state);
        assert_eq!(scrolled.scale.x(0.0), l.scale.x(0.0) - 120.0);
    }

    #[test]
    fn rows_stack_from_the_top_of_the_tracks() {
        let mut state = TimelineState::default();
        let l = layout(&state);
        assert_eq!(l.row_top(0), l.tracks.top());
        assert_eq!(l.row_top(3), l.tracks.top() + 3.0 * l.row_height);
        assert_eq!(l.row_at(l.tracks.top() + 1.0), Some(0));
        assert_eq!(l.row_at(l.row_top(3) + l.row_height - 1.0), Some(3));
        assert_eq!(l.row_at(l.tracks.top() - 5.0), None);
        state.scroll_y = 40.0;
        let scrolled = layout(&state);
        assert_eq!(scrolled.row_top(0), l.row_top(0) - 40.0);
        assert_eq!(scrolled.row_at(l.tracks.top() + 1.0), Some(1));
    }

    #[test]
    fn bars_are_centred_in_their_rows() {
        let state = TimelineState::default();
        let l = layout(&state);
        let bar = l.bar_rect(2, 500.0, 1500.0);
        assert!((bar.left() - l.scale.x(500.0)).abs() < 1e-3);
        assert!((bar.width() - 200.0).abs() < 1e-3);
        assert_eq!(bar.height(), l.bar_height);
        let row = l.row_rect(2);
        assert_eq!(bar.center().y, row.center().y);
        assert_eq!(row.left(), l.rect.left());
        assert_eq!(row.right(), l.rect.right());
        // An instant action still has a bar to grab.
        assert_eq!(l.bar_rect(0, 800.0, 800.0).width(), MIN_BAR_WIDTH);
    }

    #[test]
    fn the_label_column_keeps_room_for_the_tracks() {
        assert_eq!(label_width(248.0, 1440.0), 248.0);
        assert_eq!(label_width(20.0, 1440.0), LABELS_MIN);
        assert_eq!(label_width(2000.0, 1440.0), 1440.0 - TRACKS_MIN);
        // Too narrow for both minimums: the labels keep theirs.
        assert_eq!(label_width(248.0, 300.0), LABELS_MIN);
        // Narrower than the label column itself.
        assert_eq!(label_width(248.0, 100.0), 100.0);
        assert_eq!(label_width(248.0, -5.0), 0.0);
    }
}
