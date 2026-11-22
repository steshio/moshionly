//! Where the parts of the window go.
//!
//! ```text
//! +---------------------------------------------------+--------+
//! | top bar                                                    |
//! +---------+-----------------------------+-----------+--------+
//! | layers  |           canvas            | inspector |  chat  |
//! |         |                             |           |        |
//! +---------+-----------------------------+-----------+        |
//! | timeline                                          |        |
//! +---------------------------------------------------+--------+
//! ```
//!
//! [`PanelSizes`] is what the user chose by dragging the splitters; it is
//! saved with the settings. [`ShellLayout::compute`] turns it into rectangles
//! for a window of any size, giving way gracefully when the window is too
//! small for everything.

use std::ops::RangeInclusive;

use egui::{Rect, Vec2, pos2};
use serde::{Deserialize, Serialize};

use crate::theme::{Size, Theme};

/// The least room the canvas keeps when panels are dragged wider or the
/// window gets small.
pub const CANVAS_MIN: Vec2 = Vec2::new(280.0, 160.0);

/// The sizes of the resizable panels, as the user left them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSizes {
    /// Width of the layers panel.
    pub left: f32,
    /// Width of the inspector.
    pub right: f32,
    /// Width of the chat panel.
    pub chat: f32,
    /// Height of the timeline.
    pub timeline: f32,
    /// Whether the chat panel is showing.
    pub chat_open: bool,
}

impl Default for PanelSizes {
    fn default() -> Self {
        let size = &Theme::light().size;
        Self {
            left: size.left_panel,
            right: size.right_panel,
            chat: size.chat_panel,
            timeline: size.timeline,
            chat_open: false,
        }
    }
}

impl PanelSizes {
    /// The sizes with anything unusable replaced: a settings file can hold
    /// any number, including ones that are not numbers.
    pub fn sanitized(self, size: &Size) -> Self {
        let defaults = Self::default();
        let width = |value: f32, default: f32| {
            if value.is_finite() {
                value.clamp(size.panel_min, size.panel_max)
            } else {
                default
            }
        };
        Self {
            left: width(self.left, defaults.left),
            right: width(self.right, defaults.right),
            chat: width(self.chat, defaults.chat),
            timeline: if self.timeline.is_finite() {
                self.timeline.max(size.timeline_min)
            } else {
                defaults.timeline
            },
            chat_open: self.chat_open,
        }
    }
}

/// The rectangle of every part of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayout {
    /// The bar across the top.
    pub top_bar: Rect,
    /// The layers panel.
    pub left: Rect,
    /// The canvas.
    pub canvas: Rect,
    /// The inspector.
    pub right: Rect,
    /// The timeline.
    pub timeline: Rect,
    /// The chat panel, when it is open.
    pub chat: Option<Rect>,
}

/// How far each splitter may be dragged in the current window.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelLimits {
    /// Allowed widths of the layers panel.
    pub left: RangeInclusive<f32>,
    /// Allowed widths of the inspector.
    pub right: RangeInclusive<f32>,
    /// Allowed widths of the chat panel.
    pub chat: RangeInclusive<f32>,
    /// Allowed heights of the timeline.
    pub timeline: RangeInclusive<f32>,
}

impl ShellLayout {
    /// Lays out `window` for the given panel sizes.
    ///
    /// When the window is too narrow for the panels at their chosen widths,
    /// the chat panel gives way first, then the inspector, then the layers
    /// panel, each down to the minimum panel width; after that the canvas
    /// shrinks. The stored sizes are not changed, so the panels return to
    /// their widths when the window grows again.
    pub fn compute(window: Rect, sizes: &PanelSizes, size: &Size) -> Self {
        let top = size.top_bar.min(window.height());
        let top_bar = Rect::from_min_max(window.min, pos2(window.right(), window.top() + top));
        let body = Rect::from_min_max(pos2(window.left(), top_bar.bottom()), window.max);

        let chat_wanted = if sizes.chat_open { sizes.chat } else { 0.0 };
        let chat_min = if sizes.chat_open { size.panel_min } else { 0.0 };
        let [left, right, chat] = fit(
            [sizes.left, sizes.right, chat_wanted],
            [size.panel_min, size.panel_min, chat_min],
            (body.width() - CANVAS_MIN.x).max(0.0),
        );

        let chat = chat.min(body.width());
        let columns = Rect::from_min_max(body.min, pos2(body.right() - chat, body.bottom()));
        let chat = sizes
            .chat_open
            .then(|| Rect::from_min_max(pos2(columns.right(), body.top()), body.max));

        let timeline_height = sizes
            .timeline
            .min((columns.height() - CANVAS_MIN.y).max(size.timeline_min))
            .min(columns.height());
        let timeline = Rect::from_min_max(
            pos2(columns.left(), columns.bottom() - timeline_height),
            columns.max,
        );
        let upper = Rect::from_min_max(columns.min, pos2(columns.right(), timeline.top()));

        let left = left.min(upper.width());
        let right = right.min(upper.width() - left);
        let left_rect = Rect::from_min_max(upper.min, pos2(upper.left() + left, upper.bottom()));
        let right_rect = Rect::from_min_max(pos2(upper.right() - right, upper.top()), upper.max);
        let canvas = Rect::from_min_max(
            pos2(left_rect.right(), upper.top()),
            pos2(right_rect.left(), upper.bottom()),
        );

        Self {
            top_bar,
            left: left_rect,
            canvas,
            right: right_rect,
            timeline,
            chat,
        }
    }

    /// How far each splitter may be dragged in `window`, given the other
    /// panels at their current sizes. A panel can grow until the canvas is
    /// down to [`CANVAS_MIN`].
    pub fn limits(window: Rect, sizes: &PanelSizes, size: &Size) -> PanelLimits {
        let layout = Self::compute(window, sizes, size);
        let spare = (layout.canvas.width() - CANVAS_MIN.x).max(0.0);
        let width = |current: f32| {
            let max = (current + spare).clamp(size.panel_min, size.panel_max);
            size.panel_min..=max
        };
        let body_height = window.height() - layout.top_bar.height();
        let timeline_max = (body_height - CANVAS_MIN.y).max(size.timeline_min);
        PanelLimits {
            left: width(layout.left.width()),
            right: width(layout.right.width()),
            chat: width(layout.chat.map_or(sizes.chat, |r| r.width())),
            timeline: size.timeline_min..=timeline_max,
        }
    }
}

/// Shrinks `wanted` sizes until they fit in `available`, taking from the last
/// entry first and never going below the matching entry of `min`.
fn fit<const N: usize>(wanted: [f32; N], min: [f32; N], available: f32) -> [f32; N] {
    let mut sizes = wanted;
    for i in 0..N {
        sizes[i] = sizes[i].max(min[i]);
    }
    let mut excess = sizes.iter().sum::<f32>() - available;
    for i in (0..N).rev() {
        if excess <= 0.0 {
            break;
        }
        let give = (sizes[i] - min[i]).min(excess).max(0.0);
        sizes[i] -= give;
        excess -= give;
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(width, height))
    }

    fn size() -> &'static Size {
        &Theme::light().size
    }

    fn area(rect: Rect) -> f32 {
        rect.width() * rect.height()
    }

    #[test]
    fn the_parts_tile_the_window() {
        for chat_open in [false, true] {
            let sizes = PanelSizes {
                chat_open,
                ..PanelSizes::default()
            };
            let w = window(1440.0, 900.0);
            let l = ShellLayout::compute(w, &sizes, size());
            let total = area(l.top_bar)
                + area(l.left)
                + area(l.canvas)
                + area(l.right)
                + area(l.timeline)
                + l.chat.map_or(0.0, area);
            assert!((total - area(w)).abs() < 0.5, "chat_open={chat_open}");
            assert_eq!(l.chat.is_some(), chat_open);
            assert_eq!(l.top_bar.height(), size().top_bar);
            assert_eq!(l.left.width(), sizes.left);
            assert_eq!(l.right.width(), sizes.right);
            assert_eq!(l.timeline.height(), sizes.timeline);
            assert_eq!(l.canvas.left(), l.left.right());
            assert_eq!(l.canvas.right(), l.right.left());
            assert_eq!(l.canvas.bottom(), l.timeline.top());
        }
    }

    #[test]
    fn the_timeline_runs_under_the_three_columns_but_not_the_chat() {
        let sizes = PanelSizes {
            chat_open: true,
            ..PanelSizes::default()
        };
        let l = ShellLayout::compute(window(1600.0, 900.0), &sizes, size());
        let chat = l.chat.unwrap();
        assert_eq!(l.timeline.left(), 0.0);
        assert_eq!(l.timeline.right(), chat.left());
        assert_eq!(chat.top(), l.top_bar.bottom());
        assert_eq!(chat.bottom(), 900.0);
    }

    #[test]
    fn a_narrow_window_squeezes_chat_first_and_keeps_the_canvas() {
        let sizes = PanelSizes {
            left: 300.0,
            right: 300.0,
            chat: 400.0,
            chat_open: true,
            ..PanelSizes::default()
        };
        // Room for everything but 100 points.
        let l = ShellLayout::compute(window(1180.0, 800.0), &sizes, size());
        assert_eq!(l.chat.unwrap().width(), 300.0);
        assert_eq!(l.left.width(), 300.0);
        assert_eq!(l.right.width(), 300.0);
        assert_eq!(l.canvas.width(), CANVAS_MIN.x);

        // Much narrower: every panel at its minimum, the canvas takes the rest.
        let l = ShellLayout::compute(window(800.0, 800.0), &sizes, size());
        assert_eq!(l.chat.unwrap().width(), size().panel_min);
        assert_eq!(l.left.width(), size().panel_min);
        assert_eq!(l.right.width(), size().panel_min);
        assert_eq!(l.canvas.width(), 200.0);
    }

    #[test]
    fn a_tiny_window_never_produces_negative_rects() {
        for (w, h) in [(0.0, 0.0), (50.0, 20.0), (300.0, 100.0), (500.0, 300.0)] {
            let sizes = PanelSizes {
                chat_open: true,
                ..PanelSizes::default()
            };
            let l = ShellLayout::compute(window(w, h), &sizes, size());
            for rect in [
                l.top_bar,
                l.left,
                l.canvas,
                l.right,
                l.timeline,
                l.chat.unwrap(),
            ] {
                assert!(rect.width() >= 0.0 && rect.height() >= 0.0, "{w}x{h}");
            }
        }
    }

    #[test]
    fn a_short_window_shrinks_the_timeline() {
        let sizes = PanelSizes {
            timeline: 600.0,
            ..PanelSizes::default()
        };
        let l = ShellLayout::compute(window(1440.0, 700.0), &sizes, size());
        assert_eq!(l.canvas.height(), CANVAS_MIN.y);
        assert_eq!(l.timeline.height(), 700.0 - size().top_bar - CANVAS_MIN.y);
    }

    #[test]
    fn limits_stop_a_panel_where_the_canvas_would_get_too_small() {
        let sizes = PanelSizes::default();
        let w = window(1000.0, 800.0);
        let limits = ShellLayout::limits(w, &sizes, size());
        let canvas = 1000.0 - sizes.left - sizes.right;
        assert_eq!(*limits.left.start(), size().panel_min);
        assert_eq!(*limits.left.end(), sizes.left + canvas - CANVAS_MIN.x);
        assert_eq!(
            *limits.timeline.end(),
            800.0 - size().top_bar - CANVAS_MIN.y
        );
        // In a huge window the fixed maximum applies.
        let limits = ShellLayout::limits(window(4000.0, 2000.0), &sizes, size());
        assert_eq!(*limits.right.end(), size().panel_max);
    }

    #[test]
    fn odd_saved_sizes_are_repaired() {
        let odd = PanelSizes {
            left: f32::NAN,
            right: -40.0,
            chat: 9000.0,
            timeline: f32::INFINITY,
            chat_open: true,
        };
        let fixed = odd.sanitized(size());
        assert_eq!(fixed.left, PanelSizes::default().left);
        assert_eq!(fixed.right, size().panel_min);
        assert_eq!(fixed.chat, size().panel_max);
        assert_eq!(fixed.timeline, PanelSizes::default().timeline);
        assert!(fixed.chat_open);
    }

    #[test]
    fn fit_takes_from_the_last_entry_first() {
        assert_eq!(
            fit([300.0, 300.0, 400.0], [200.0; 3], 900.0),
            [300.0, 300.0, 300.0]
        );
        assert_eq!(
            fit([300.0, 300.0, 400.0], [200.0; 3], 650.0),
            [250.0, 200.0, 200.0]
        );
        assert_eq!(
            fit([300.0, 300.0, 0.0], [200.0, 200.0, 0.0], 100.0),
            [200.0, 200.0, 0.0]
        );
    }
}
