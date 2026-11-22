//! Time on screen: the maths that turns milliseconds into points and back,
//! picks ruler ticks for a zoom level, and writes times as text.
//!
//! Times are milliseconds from the start of the scene, as in a document.
//! Horizontal positions are egui points.

/// The most zoomed-out view: this many points per second.
pub const MIN_POINTS_PER_SECOND: f32 = 4.0;
/// The most zoomed-in view: one point is a quarter of a millisecond.
pub const MAX_POINTS_PER_SECOND: f32 = 4000.0;

/// The mapping between time and horizontal position for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeScale {
    /// The screen x of time zero.
    pub origin: f32,
    /// Points per millisecond.
    pub points_per_ms: f32,
}

impl TimeScale {
    /// The screen x of a time.
    pub fn x(&self, time: f64) -> f32 {
        self.origin + (time * f64::from(self.points_per_ms)) as f32
    }

    /// The time at a screen x.
    pub fn time(&self, x: f32) -> f64 {
        f64::from(x - self.origin) / f64::from(self.points_per_ms)
    }

    /// How wide a span of time is on screen.
    pub fn width(&self, duration: f64) -> f32 {
        (duration * f64::from(self.points_per_ms)) as f32
    }

    /// How much time a width on screen covers.
    pub fn duration(&self, width: f32) -> f64 {
        f64::from(width) / f64::from(self.points_per_ms)
    }
}

/// Keeps a zoom level within the limits.
pub fn clamp_zoom(points_per_second: f32) -> f32 {
    if points_per_second.is_finite() {
        points_per_second.clamp(MIN_POINTS_PER_SECOND, MAX_POINTS_PER_SECOND)
    } else {
        MIN_POINTS_PER_SECOND
    }
}

/// The zoom at which `duration` milliseconds fill `width` points.
pub fn zoom_to_fit(duration: f64, width: f32) -> f32 {
    if duration <= 0.0 || width <= 0.0 {
        return clamp_zoom(f32::NAN);
    }
    clamp_zoom((f64::from(width) / duration * 1000.0) as f32)
}

/// The scroll offset that keeps the time under `anchor` (a distance in points
/// from where time zero is drawn when not scrolled) in place while the zoom
/// changes from `old` to `new` points per second.
pub fn scroll_after_zoom(scroll: f32, anchor: f32, old: f32, new: f32) -> f32 {
    // The time under the anchor, in points at the old zoom, rescaled.
    (scroll + anchor) * (new / old) - anchor
}

/// A zoom level as a slider position from 0 to 1. The slider is logarithmic,
/// so each step feels the same.
pub fn zoom_to_slider(points_per_second: f32) -> f32 {
    let (min, max) = (MIN_POINTS_PER_SECOND.ln(), MAX_POINTS_PER_SECOND.ln());
    ((clamp_zoom(points_per_second).ln() - min) / (max - min)).clamp(0.0, 1.0)
}

/// The zoom level for a slider position from 0 to 1.
pub fn slider_to_zoom(position: f32) -> f32 {
    let (min, max) = (MIN_POINTS_PER_SECOND.ln(), MAX_POINTS_PER_SECOND.ln());
    clamp_zoom((min + (max - min) * position.clamp(0.0, 1.0)).exp())
}

/// Slack for comparing widths, so that a zoom such as 0.2 points per
/// millisecond, which is not exact in binary, lands on the step it means.
const ROUNDING: f64 = 1e-4;

/// The spacing of ruler ticks, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ticks {
    /// Between labelled ticks.
    pub major: f64,
    /// Between the small ticks in between. Divides `major` evenly.
    pub minor: f64,
}

/// Tick spacings to choose from: `(major, minor)` in milliseconds.
const TICK_STEPS: [(f64, f64); 19] = [
    (1.0, 1.0),
    (2.0, 1.0),
    (5.0, 1.0),
    (10.0, 2.0),
    (20.0, 5.0),
    (50.0, 10.0),
    (100.0, 20.0),
    (200.0, 50.0),
    (500.0, 100.0),
    (1_000.0, 200.0),
    (2_000.0, 500.0),
    (5_000.0, 1_000.0),
    (10_000.0, 2_000.0),
    (15_000.0, 5_000.0),
    (30_000.0, 5_000.0),
    (60_000.0, 10_000.0),
    (120_000.0, 30_000.0),
    (300_000.0, 60_000.0),
    (600_000.0, 120_000.0),
];

/// The tick spacing for a zoom level: the finest one whose labels are at
/// least `min_label_spacing` points apart.
pub fn ticks(points_per_ms: f32, min_label_spacing: f32) -> Ticks {
    let (major, minor) = TICK_STEPS
        .into_iter()
        .find(|(major, _)| {
            *major * f64::from(points_per_ms) >= f64::from(min_label_spacing) - ROUNDING
        })
        .unwrap_or(TICK_STEPS[TICK_STEPS.len() - 1]);
    Ticks { major, minor }
}

/// The step of the time grid that dragged things snap to at a zoom level:
/// the largest round number of milliseconds that is no wider than
/// `max_step_width` points, and at least one millisecond.
pub fn grid_step(points_per_ms: f32, max_step_width: f32) -> f64 {
    const STEPS: [f64; 10] = [
        1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0,
    ];
    STEPS
        .into_iter()
        .rev()
        .find(|step| *step * f64::from(points_per_ms) <= f64::from(max_step_width) + ROUNDING)
        .unwrap_or(1.0)
}

/// The label of a ruler tick: seconds with as few decimals as the tick
/// spacing needs (`0s`, `0.5s`, `1s`), or minutes and seconds from one minute
/// on (`1:30`).
pub fn tick_label(time: f64, major: f64) -> String {
    let ms = time.round() as i64;
    if ms >= 60_000 && major >= 1_000.0 {
        let seconds = ms / 1_000;
        return format!("{}:{:02}", seconds / 60, seconds % 60);
    }
    let step = major.round().max(1.0) as i64;
    let decimals = if step % 1_000 == 0 {
        0
    } else if step % 100 == 0 {
        1
    } else if step % 10 == 0 {
        2
    } else {
        3
    };
    let text = format!("{:.*}", decimals, ms as f64 / 1_000.0);
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("{text}s")
}

/// A time as a readout that keeps its width while it counts:
/// minutes, seconds and hundredths, `0:01.25`.
pub fn timecode(time: f64) -> String {
    let centiseconds = (time.max(0.0) / 10.0).round() as i64;
    let seconds = centiseconds / 100;
    format!(
        "{}:{:02}.{:02}",
        seconds / 60,
        seconds % 60,
        centiseconds % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 200 points per second, with time zero at x = 264.
    fn scale() -> TimeScale {
        TimeScale {
            origin: 264.0,
            points_per_ms: 0.2,
        }
    }

    #[test]
    fn time_maps_to_pixels_and_back() {
        let s = scale();
        assert_eq!(s.x(0.0), 264.0);
        assert!((s.x(1000.0) - 464.0).abs() < 1e-3);
        assert!((s.x(-500.0) - 164.0).abs() < 1e-3);
        assert!((s.time(464.0) - 1000.0).abs() < 1e-3);
        assert!((s.width(250.0) - 50.0).abs() < 1e-3);
        assert!((s.duration(50.0) - 250.0).abs() < 1e-3);
        for time in [0.0, 16.0, 333.0, 4000.0, 59_999.0, 600_000.0] {
            assert!((s.time(s.x(time)) - time).abs() < 0.2, "{time}");
        }
    }

    #[test]
    fn zooming_keeps_the_time_under_the_pointer() {
        // Scrolled by 100 points, pointer 300 points into the track.
        let (scroll, anchor, old, new) = (100.0, 300.0, 200.0, 500.0);
        let time_before = f64::from(scroll + anchor) / f64::from(old) * 1000.0;
        let scrolled = scroll_after_zoom(scroll, anchor, old, new);
        let time_after = f64::from(scrolled + anchor) / f64::from(new) * 1000.0;
        assert!((time_before - time_after).abs() < 1e-2);
        // Zooming at the very start of an unscrolled track does not scroll.
        assert_eq!(scroll_after_zoom(0.0, 0.0, 200.0, 800.0), 0.0);
    }

    #[test]
    fn zoom_is_limited_and_survives_nonsense() {
        assert_eq!(clamp_zoom(1e9), MAX_POINTS_PER_SECOND);
        assert_eq!(clamp_zoom(0.0), MIN_POINTS_PER_SECOND);
        assert_eq!(clamp_zoom(f32::NAN), MIN_POINTS_PER_SECOND);
        assert_eq!(zoom_to_fit(4000.0, 800.0), 200.0);
        assert_eq!(zoom_to_fit(0.0, 800.0), MIN_POINTS_PER_SECOND);
        assert_eq!(zoom_to_fit(4000.0, 0.0), MIN_POINTS_PER_SECOND);
        assert_eq!(zoom_to_fit(1.0, 100_000.0), MAX_POINTS_PER_SECOND);
    }

    #[test]
    fn the_zoom_slider_is_logarithmic_and_reversible() {
        assert_eq!(zoom_to_slider(MIN_POINTS_PER_SECOND), 0.0);
        assert_eq!(zoom_to_slider(MAX_POINTS_PER_SECOND), 1.0);
        for zoom in [4.0, 37.0, 200.0, 1234.0, 4000.0] {
            let back = slider_to_zoom(zoom_to_slider(zoom));
            assert!((back - zoom).abs() / zoom < 1e-3, "{zoom} became {back}");
        }
        // Halfway along the slider is the geometric mean of the limits.
        let middle = slider_to_zoom(0.5);
        let mean = (MIN_POINTS_PER_SECOND * MAX_POINTS_PER_SECOND).sqrt();
        assert!((middle - mean).abs() < 0.1);
    }

    #[test]
    fn ticks_adapt_to_the_zoom() {
        // 200 points per second: a label every 500 ms is 100 points apart.
        assert_eq!(
            ticks(0.2, 72.0),
            Ticks {
                major: 500.0,
                minor: 100.0
            }
        );
        // Zoomed far in.
        assert_eq!(
            ticks(4.0, 72.0),
            Ticks {
                major: 20.0,
                minor: 5.0
            }
        );
        // Zoomed far out.
        assert_eq!(ticks(0.004, 72.0).major, 30_000.0);
        // Beyond the table the coarsest spacing is used.
        assert_eq!(ticks(1e-6, 72.0).major, 600_000.0);
        // Minor ticks always divide major ones evenly.
        for (major, minor) in TICK_STEPS {
            assert_eq!(major % minor, 0.0, "{major} / {minor}");
        }
    }

    #[test]
    fn the_snapping_grid_follows_the_zoom() {
        assert_eq!(grid_step(0.2, 10.0), 50.0);
        assert_eq!(grid_step(1.0, 10.0), 10.0);
        assert_eq!(grid_step(4.0, 10.0), 2.0);
        assert_eq!(grid_step(0.004, 10.0), 1_000.0);
        // Never finer than a millisecond.
        assert_eq!(grid_step(100.0, 10.0), 1.0);
    }

    #[test]
    fn tick_labels_use_as_few_decimals_as_needed() {
        assert_eq!(tick_label(0.0, 500.0), "0s");
        assert_eq!(tick_label(500.0, 500.0), "0.5s");
        assert_eq!(tick_label(1000.0, 500.0), "1s");
        assert_eq!(tick_label(1500.0, 500.0), "1.5s");
        assert_eq!(tick_label(3000.0, 1000.0), "3s");
        assert_eq!(tick_label(250.0, 50.0), "0.25s");
        assert_eq!(tick_label(1200.0, 20.0), "1.2s");
        assert_eq!(tick_label(15.0, 5.0), "0.015s");
        assert_eq!(tick_label(30_000.0, 10_000.0), "30s");
        assert_eq!(tick_label(60_000.0, 10_000.0), "1:00");
        assert_eq!(tick_label(90_000.0, 30_000.0), "1:30");
        assert_eq!(tick_label(600_000.0, 60_000.0), "10:00");
    }

    #[test]
    fn timecodes_keep_their_shape() {
        assert_eq!(timecode(0.0), "0:00.00");
        assert_eq!(timecode(1250.0), "0:01.25");
        assert_eq!(timecode(4000.0), "0:04.00");
        assert_eq!(timecode(59_994.0), "0:59.99");
        assert_eq!(timecode(59_996.0), "1:00.00");
        assert_eq!(timecode(754_320.0), "12:34.32");
        assert_eq!(timecode(-20.0), "0:00.00");
    }
}
