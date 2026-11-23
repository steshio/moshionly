//! Snapping: pulling what is dragged onto the edges and centres of what is
//! around it.
//!
//! The targets are the artboard and the other layers in it. Every target
//! offers three vertical lines (left edge, centre, right edge) and three
//! horizontal ones. A dragged box snaps when one of its own six lines comes
//! within the threshold of a target line, and a guide is drawn along the
//! line it snapped to, spanning both boxes.

use egui::{Pos2, Rect, Vec2, pos2};

/// One line a box can snap to: where it is and the stretch it covers
/// across, for drawing its guide.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Line {
    /// The x of a vertical line or the y of a horizontal one.
    at: f32,
    /// Where the target begins across the line.
    from: f32,
    /// Where it ends.
    to: f32,
}

/// The lines a drag can snap to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapTargets {
    vertical: Vec<Line>,
    horizontal: Vec<Line>,
}

impl SnapTargets {
    /// The edges and centres of `rects`.
    pub fn from_rects(rects: impl IntoIterator<Item = Rect>) -> Self {
        let mut targets = Self::default();
        for rect in rects {
            if !rect.is_finite() {
                continue;
            }
            for x in [rect.left(), rect.center().x, rect.right()] {
                targets.vertical.push(Line {
                    at: x,
                    from: rect.top(),
                    to: rect.bottom(),
                });
            }
            for y in [rect.top(), rect.center().y, rect.bottom()] {
                targets.horizontal.push(Line {
                    at: y,
                    from: rect.left(),
                    to: rect.right(),
                });
            }
        }
        targets
    }

    /// Whether there is nothing to snap to.
    pub fn is_empty(&self) -> bool {
        self.vertical.is_empty() && self.horizontal.is_empty()
    }
}

/// The outcome of snapping: how far to move, and the guides to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapped {
    /// The move that puts the box on the lines it snapped to.
    pub offset: Vec2,
    /// The lines to draw, each from one end to the other.
    pub guides: Vec<[Pos2; 2]>,
}

/// The nearest line within `threshold` of any of `values`: how far to move
/// to reach it.
fn nearest(lines: &[Line], values: &[f32], threshold: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for line in lines {
        for value in values {
            let gap = line.at - value;
            if gap.abs() <= threshold && best.is_none_or(|b| gap.abs() < b.abs()) {
                best = Some(gap);
            }
        }
    }
    best
}

/// Snaps a box being moved. Its left, centre and right snap to vertical
/// lines, its top, middle and bottom to horizontal ones, each axis on its
/// own.
pub fn snap_rect(moving: Rect, targets: &SnapTargets, threshold: f32) -> Snapped {
    let xs = [moving.left(), moving.center().x, moving.right()];
    let ys = [moving.top(), moving.center().y, moving.bottom()];
    let dx = nearest(&targets.vertical, &xs, threshold).unwrap_or(0.0);
    let dy = nearest(&targets.horizontal, &ys, threshold).unwrap_or(0.0);
    let offset = Vec2::new(dx, dy);
    let placed = moving.translate(offset);
    Snapped {
        offset,
        guides: guides(
            placed,
            targets,
            dx != 0.0 || hits(&targets.vertical, &xs),
            dy != 0.0 || hits(&targets.horizontal, &ys),
        ),
    }
}

/// Snaps one point, such as the corner of a shape being drawn or the handle
/// of a box being resized. `x` and `y` say which coordinates may move.
pub fn snap_point(point: Pos2, targets: &SnapTargets, threshold: f32, x: bool, y: bool) -> Snapped {
    let dx = if x {
        nearest(&targets.vertical, &[point.x], threshold).unwrap_or(0.0)
    } else {
        0.0
    };
    let dy = if y {
        nearest(&targets.horizontal, &[point.y], threshold).unwrap_or(0.0)
    } else {
        0.0
    };
    let offset = Vec2::new(dx, dy);
    let placed = Rect::from_min_max(point + offset, point + offset);
    Snapped {
        offset,
        guides: guides(
            placed,
            targets,
            x && (dx != 0.0 || hits(&targets.vertical, &[point.x])),
            y && (dy != 0.0 || hits(&targets.horizontal, &[point.y])),
        ),
    }
}

/// Whether any value lies exactly on a line already.
fn hits(lines: &[Line], values: &[f32]) -> bool {
    lines
        .iter()
        .any(|line| values.iter().any(|value| (line.at - value).abs() < 0.01))
}

/// The guides for a box that now sits where it snapped: one along every
/// target line that one of its lines lies on.
fn guides(placed: Rect, targets: &SnapTargets, vertical: bool, horizontal: bool) -> Vec<[Pos2; 2]> {
    let mut out = Vec::new();
    if vertical {
        for x in [placed.left(), placed.center().x, placed.right()] {
            let along: Vec<&Line> = targets
                .vertical
                .iter()
                .filter(|line| (line.at - x).abs() < 0.01)
                .collect();
            if along.is_empty() {
                continue;
            }
            let from = along.iter().map(|l| l.from).fold(placed.top(), f32::min);
            let to = along.iter().map(|l| l.to).fold(placed.bottom(), f32::max);
            out.push([pos2(x, from), pos2(x, to)]);
        }
    }
    if horizontal {
        for y in [placed.top(), placed.center().y, placed.bottom()] {
            let along: Vec<&Line> = targets
                .horizontal
                .iter()
                .filter(|line| (line.at - y).abs() < 0.01)
                .collect();
            if along.is_empty() {
                continue;
            }
            let from = along.iter().map(|l| l.from).fold(placed.left(), f32::min);
            let to = along.iter().map(|l| l.to).fold(placed.right(), f32::max);
            out.push([pos2(from, y), pos2(to, y)]);
        }
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use egui::vec2;

    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn artboard() -> SnapTargets {
        SnapTargets::from_rects([rect(0.0, 0.0, 1000.0, 1000.0)])
    }

    #[test]
    fn a_box_near_an_edge_snaps_onto_it() {
        let snapped = snap_rect(rect(4.0, 300.0, 100.0, 100.0), &artboard(), 6.0);
        assert_eq!(snapped.offset, vec2(-4.0, 0.0));
        assert_eq!(snapped.guides, [[pos2(0.0, 0.0), pos2(0.0, 1000.0)]]);
    }

    #[test]
    fn centres_snap_to_centres() {
        // The box's centre is at 498, 503; the artboard's at 500, 500.
        let snapped = snap_rect(rect(448.0, 453.0, 100.0, 100.0), &artboard(), 6.0);
        assert_eq!(snapped.offset, vec2(2.0, -3.0));
        assert!(
            snapped
                .guides
                .contains(&[pos2(500.0, 0.0), pos2(500.0, 1000.0)])
        );
        assert!(
            snapped
                .guides
                .contains(&[pos2(0.0, 500.0), pos2(1000.0, 500.0)])
        );
    }

    #[test]
    fn the_nearest_line_wins_and_far_lines_are_ignored() {
        let targets = SnapTargets::from_rects([
            rect(0.0, 0.0, 1000.0, 1000.0),
            rect(205.0, 600.0, 50.0, 50.0),
        ]);
        // Left edge at 203: 2 from the small box's left edge (205), 203
        // from the artboard's.
        let snapped = snap_rect(rect(203.0, 300.0, 40.0, 40.0), &targets, 6.0);
        assert_eq!(snapped.offset.x, 2.0);
        // The guide spans from the dragged box down to the target.
        assert_eq!(snapped.guides[0], [pos2(205.0, 300.0), pos2(205.0, 650.0)]);
        let far = snap_rect(rect(120.0, 300.0, 40.0, 40.0), &targets, 6.0);
        assert_eq!(far, Snapped::default());
    }

    #[test]
    fn points_snap_on_the_axes_asked_for() {
        let targets = artboard();
        let both = snap_point(pos2(997.0, 3.0), &targets, 6.0, true, true);
        assert_eq!(both.offset, vec2(3.0, -3.0));
        assert_eq!(both.guides.len(), 2);
        let only_x = snap_point(pos2(997.0, 3.0), &targets, 6.0, true, false);
        assert_eq!(only_x.offset, vec2(3.0, 0.0));
        assert_eq!(only_x.guides.len(), 1);
        assert!(SnapTargets::default().is_empty());
        assert!(!targets.is_empty());
    }

    #[test]
    fn a_box_already_on_a_line_shows_its_guide() {
        let snapped = snap_rect(rect(0.0, 300.0, 100.0, 100.0), &artboard(), 6.0);
        assert_eq!(snapped.offset, Vec2::ZERO);
        assert_eq!(snapped.guides.len(), 1);
    }
}
