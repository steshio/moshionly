//! The rules of editing bars: moving, resizing and rubber-band selection.
//!
//! These are pure functions over times and rectangles. The widget calls them
//! while a drag is in progress, always starting from the times the bars had
//! when the drag began, so a drag can go out past a limit and come back
//! without the bars drifting.

use egui::Rect;

use super::model::Edge;
use crate::key::Key;

/// The shortest an action can be made by resizing, in milliseconds.
pub const MIN_ACTION: f64 = 10.0;
/// The shortest a scene can be made by dragging its end, in milliseconds.
pub const MIN_SCENE: f64 = 100.0;

/// A bar's times when a drag began: key, start, end.
pub type Original = (Key, f64, f64);

/// How far a group of bars can actually move when asked to move by `delta`
/// milliseconds: none of them may start before zero.
pub fn clamp_move(originals: &[Original], delta: f64) -> f64 {
    let earliest = originals
        .iter()
        .map(|(_, start, _)| *start)
        .fold(f64::INFINITY, f64::min);
    if earliest.is_finite() {
        delta.max(-earliest)
    } else {
        delta
    }
}

/// The bars' times after moving them all by `delta` milliseconds. Lengths
/// are kept. `delta` should already be clamped with [`clamp_move`].
pub fn moved(originals: &[Original], delta: f64) -> Vec<Original> {
    originals
        .iter()
        .map(|(key, start, end)| (key.clone(), (start + delta).round(), (end + delta).round()))
        .collect()
}

/// A bar's start and end after dragging one of its ends to `time`. The other
/// end stays put, the bar keeps at least [`MIN_ACTION`], and it cannot start
/// before zero.
pub fn resized(original: (f64, f64), edge: Edge, time: f64) -> (f64, f64) {
    let (start, end) = original;
    match edge {
        Edge::Start => {
            let latest = (end - MIN_ACTION).max(0.0);
            (time.clamp(0.0, latest).round(), end)
        }
        Edge::End => (start, time.max(start + MIN_ACTION).round()),
    }
}

/// The scene's length after dragging its end to `time`.
pub fn scene_end(time: f64) -> f64 {
    time.max(MIN_SCENE).round()
}

/// The keys of the bars a rubber band touches, in the order given.
pub fn marquee_hits(bars: &[(Key, Rect)], marquee: Rect) -> Vec<Key> {
    bars.iter()
        .filter(|(_, rect)| rect.intersects(marquee))
        .map(|(key, _)| key.clone())
        .collect()
}

/// The selection while a rubber band is being dragged: what was selected
/// before it began (kept when Shift or ⌘ is held) plus what it touches.
pub fn merged(base: &[Key], hits: Vec<Key>) -> Vec<Key> {
    let mut selection = base.to_vec();
    for key in hits {
        if !selection.contains(&key) {
            selection.push(key);
        }
    }
    selection
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    fn originals() -> Vec<Original> {
        vec![
            (Key::new("a"), 200.0, 1100.0),
            (Key::new("b"), 900.0, 1500.0),
        ]
    }

    #[test]
    fn bars_move_together_and_keep_their_length() {
        let after = moved(&originals(), 250.0);
        assert_eq!(
            after,
            [
                (Key::new("a"), 450.0, 1350.0),
                (Key::new("b"), 1150.0, 1750.0)
            ]
        );
    }

    #[test]
    fn no_bar_moves_before_time_zero() {
        // The earliest bar starts at 200, so the group can go back 200.
        assert_eq!(clamp_move(&originals(), -500.0), -200.0);
        assert_eq!(clamp_move(&originals(), -150.0), -150.0);
        assert_eq!(clamp_move(&originals(), 800.0), 800.0);
        let after = moved(&originals(), clamp_move(&originals(), -9999.0));
        assert_eq!(after[0], (Key::new("a"), 0.0, 900.0));
        assert_eq!(after[1], (Key::new("b"), 700.0, 1300.0));
        assert_eq!(clamp_move(&[], -50.0), -50.0);
    }

    #[test]
    fn moving_rounds_to_whole_milliseconds() {
        let after = moved(&[(Key::new("a"), 100.0, 433.0)], 16.6);
        assert_eq!(after, [(Key::new("a"), 117.0, 450.0)]);
    }

    #[test]
    fn resizing_moves_one_end_only() {
        assert_eq!(
            resized((200.0, 1100.0), Edge::Start, 350.0),
            (350.0, 1100.0)
        );
        assert_eq!(resized((200.0, 1100.0), Edge::End, 1600.2), (200.0, 1600.0));
    }

    #[test]
    fn a_bar_cannot_be_turned_inside_out_or_start_before_zero() {
        assert_eq!(
            resized((200.0, 1100.0), Edge::Start, 5000.0),
            (1100.0 - MIN_ACTION, 1100.0)
        );
        assert_eq!(resized((200.0, 1100.0), Edge::Start, -300.0), (0.0, 1100.0));
        assert_eq!(
            resized((200.0, 1100.0), Edge::End, -50.0),
            (200.0, 200.0 + MIN_ACTION)
        );
        // A bar already shorter than the minimum, at zero.
        assert_eq!(resized((0.0, 4.0), Edge::Start, 100.0), (0.0, 4.0));
    }

    #[test]
    fn the_scene_keeps_a_minimum_length() {
        assert_eq!(scene_end(5230.4), 5230.0);
        assert_eq!(scene_end(-10.0), MIN_SCENE);
    }

    #[test]
    fn a_rubber_band_selects_what_it_touches() {
        let rect =
            |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        let bars = vec![
            (Key::new("a"), rect(0.0, 0.0, 100.0, 20.0)),
            (Key::new("b"), rect(150.0, 26.0, 300.0, 46.0)),
            (Key::new("c"), rect(400.0, 52.0, 500.0, 72.0)),
        ];
        // From the middle of a's row down into b.
        let hits = marquee_hits(&bars, rect(80.0, 10.0, 200.0, 30.0));
        assert_eq!(hits, [Key::new("a"), Key::new("b")]);
        assert!(marquee_hits(&bars, rect(320.0, 0.0, 380.0, 100.0)).is_empty());
        // What was selected before is kept, without duplicates.
        assert_eq!(
            merged(&[Key::new("c"), Key::new("a")], hits),
            [Key::new("c"), Key::new("a"), Key::new("b")]
        );
        assert_eq!(merged(&[], vec![Key::new("b")]), [Key::new("b")]);
    }
}
