//! What the timeline remembers between frames: how it is zoomed and
//! scrolled.

use crate::theme::Theme;

/// The view state of a timeline. Keep one per timeline in the app.
///
/// This is how the timeline is looked at, not what it shows: the widget
/// changes it freely, while the data it is given is never touched.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineState {
    /// Zoom: points per second of time.
    pub points_per_second: f32,
    /// Horizontal scroll, in points.
    pub scroll_x: f32,
    /// Vertical scroll, in points.
    pub scroll_y: f32,
    /// Width of the label column.
    pub label_width: f32,
    /// Set to zoom so the whole scene fits when the timeline is next shown.
    pub fit_requested: bool,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            points_per_second: 200.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            label_width: Theme::light().size.timeline_labels,
            fit_requested: true,
        }
    }
}

impl TimelineState {
    /// Points per millisecond at the current zoom.
    pub fn points_per_ms(&self) -> f32 {
        self.points_per_second / 1000.0
    }
}
