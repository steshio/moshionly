//! The timeline: actions as bars over time.
//!
//! Moshionly has no keyframes. Every action is one bar with a start and an
//! end, on its own row, and rows are grouped under the layer the action
//! targets. This module is that view as a self-contained widget:
//!
//! - [`Group`] and [`Bar`] are the plain data it shows.
//! - [`Timeline`] is the widget. It never changes the data: what the user
//!   does comes back as a list of [`TimelineEvent`]s for the app to apply.
//! - [`TimelineState`] is the zoom and scroll, which the widget does own.
//! - [`TimeScale`] and the functions beside it are the time-to-pixel maths,
//!   the ruler's ticks and the way times are written.
//! - [`Snapper`] is the snapping rule, and [`moved`], [`resized`] and their
//!   neighbours are the rules of editing bars, all as plain functions.

mod bars;
mod edit;
mod header;
mod layout;
mod model;
mod scale;
mod snap;
mod state;
mod widget;

pub use edit::{
    MIN_ACTION, MIN_SCENE, Original, clamp_move, marquee_hits, merged, moved, resized, scene_end,
};
pub use layout::{LABELS_MIN, MIN_BAR_WIDTH, TRACK_PAD, TRACKS_MIN, TimelineLayout, label_width};
pub use model::{Bar, BarCommand, Edge, Group, Row, TimelineEvent, last_end, rows, rows_of};
pub use scale::{
    MAX_POINTS_PER_SECOND, MIN_POINTS_PER_SECOND, Ticks, TimeScale, clamp_zoom, grid_step,
    scroll_after_zoom, slider_to_zoom, tick_label, ticks, timecode, zoom_to_fit, zoom_to_slider,
};
pub use snap::{Snapped, Snapper};
pub use state::TimelineState;
pub use widget::{
    Timeline, TimelineOutput, edge_zone, scroll_to_follow, scroll_to_reveal, selection_after_click,
};
