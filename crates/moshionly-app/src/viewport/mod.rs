//! The canvas viewport: an infinite canvas with pan and zoom.
//!
//! The viewport draws what it is given and reports what the user did. It
//! shows each artboard from a texture handed to it and knows nothing about
//! documents or layers:
//!
//! - [`Camera`] holds the pan and zoom and converts between world and screen
//!   coordinates.
//! - [`Viewport`] is the widget. It handles pan and zoom itself and reports
//!   every other gesture as a [`ViewportEvent`] with positions in world
//!   coordinates.
//!
//! Hit testing of layers belongs to the engine, which knows their shapes.

mod camera;
mod widget;

pub use camera::{Camera, MAX_ZOOM, MIN_ZOOM, ZOOM_STOPS, next_zoom_stop, previous_zoom_stop};
pub use widget::{Artboard, Viewport, ViewportEvent, ViewportOutput};
