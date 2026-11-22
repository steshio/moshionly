//! What the timeline remembers between frames: how it is zoomed and
//! scrolled, and the drag in progress.

use super::{edit::Original, model::Edge};
use crate::{key::Key, theme::Theme};

/// The view state of a timeline. Keep one per timeline in the app.
///
/// This is how the timeline is looked at, not what it shows: the widget
/// changes it freely, while the data it is given is never touched.
#[derive(Clone, Debug)]
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
    /// The drag in progress.
    pub(super) drag: Option<Drag>,
    /// Where the snapping guide is, while something is snapped to a target.
    pub(super) guide: Option<f64>,
    /// Whether the drag in progress has changed anything yet.
    pub(super) drag_changed: bool,
}

/// A drag on the timeline, remembered from the frame it began.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Drag {
    /// Moving bars along the timeline.
    Bars {
        /// The start and end, when the drag began, of the bar under the
        /// pointer (or of the summary bar): the ends that snap.
        span: (f64, f64),
        /// The time under the pointer when the drag began.
        grab_time: f64,
        /// Every bar being moved, with its times when the drag began.
        originals: Vec<Original>,
    },
    /// Resizing one bar from an end.
    Edge {
        /// The bar.
        key: Key,
        /// The end being dragged.
        edge: Edge,
        /// The bar's start and end when the drag began.
        original: (f64, f64),
        /// How far the pointer was from the end when it grabbed it, in
        /// milliseconds, so the end does not jump to the pointer.
        grab_offset: f64,
    },
    /// Dragging the end of the scene.
    Duration {
        /// How far the pointer was from the marker when it grabbed it.
        grab_offset: f64,
    },
    /// A rubber-band selection.
    Marquee {
        /// The time where the band began.
        time: f64,
        /// How far below the top of the first row it began, in points.
        y: f32,
        /// The selection to add to (empty unless Shift or ⌘ was held).
        base: Vec<Key>,
    },
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            points_per_second: 200.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            label_width: Theme::light().size.timeline_labels,
            fit_requested: true,
            drag: None,
            guide: None,
            drag_changed: false,
        }
    }
}

impl TimelineState {
    /// A state at a fixed zoom, instead of one that fits the scene the first
    /// time it is shown.
    pub fn with_zoom(points_per_second: f32) -> Self {
        Self {
            points_per_second,
            fit_requested: false,
            ..Self::default()
        }
    }

    /// Points per millisecond at the current zoom.
    pub fn points_per_ms(&self) -> f32 {
        self.points_per_second / 1000.0
    }

    /// Whether the user is in the middle of dragging something in the
    /// timeline.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
}
