//! The outlines of the scene's shapes as Skia paths.
//!
//! Painting, hit testing and bounds all build their paths here, so a shape
//! is hit and measured exactly where it is drawn.

use moshionly_core::Rect;
use moshionly_core::model::{FillRule, PathData};
use moshionly_core::scene::{Content, SceneNode};
use skia_safe as sk;

use crate::convert;

/// The most points a star is drawn with. The document allows 64; a scene
/// built by hand must not be able to ask for billions.
const MAX_STAR_POINTS: u32 = 1024;

/// Sweeps this close to a full turn or to nothing count as exactly that.
const ANGLE_EPSILON: f64 = 1e-6;

/// The outline of a shape, in the coordinates of its node.
#[derive(Debug, Clone)]
pub(crate) struct Outline {
    /// The outline.
    pub path: sk::Path,
    /// Whether every piece of the outline is closed. Only a closed outline
    /// has an inside and an outside for a stroke to sit on.
    pub closed: bool,
    /// For an outline with closed and open pieces, the closed ones and the
    /// open ones apart: a stroke keeps to its side of the first and is
    /// centred on the second. `None` when the pieces are all of one kind.
    pub split: Option<Box<(Outline, Outline)>>,
}

impl Outline {
    /// An outline whose pieces are all closed, or all open.
    pub fn new(path: sk::Path, closed: bool) -> Self {
        Self {
            path,
            closed,
            split: None,
        }
    }

    /// The parts a stroke follows, each all closed or all open: the outline
    /// itself, or its closed and its open pieces.
    pub fn parts(&self) -> impl Iterator<Item = &Outline> {
        let (first, second) = match &self.split {
            Some(split) => (&split.0, Some(&split.1)),
            None => (self, None),
        };
        std::iter::once(first).chain(second)
    }
}

/// The outline of the shape a node draws, or `None` for a node that is not
/// a shape (a group or a text). An image or a video counts as its rounded
/// box.
pub(crate) fn outline(node: &SceneNode) -> Option<Outline> {
    let closed = |path: sk::Path| Outline::new(path, true);
    Some(match &node.content {
        Content::Rect { radii, .. }
        | Content::Image { radii, .. }
        | Content::Video { radii, .. } => closed(rect(node.width, node.height, radii)),
        Content::Ellipse {
            start_angle,
            sweep,
            inner_radius,
            ..
        } => closed(ellipse(
            node.width,
            node.height,
            *start_angle,
            *sweep,
            *inner_radius,
        )),
        Content::Star {
            points,
            inner_radius,
            ..
        } => closed(star(node.width, node.height, *points, *inner_radius)),
        Content::Path { path, .. } => pen(path),
        Content::Group { .. } | Content::Text(_) => return None,
    })
}

/// A `width` by `height` rectangle with its top-left corner at the origin.
/// `radii` are top-left, top-right, bottom-right, bottom-left.
pub(crate) fn rect(width: f64, height: f64, radii: &[f64; 4]) -> sk::Path {
    let bounds = Rect::new(0.0, 0.0, width.max(0.0), height.max(0.0));
    if !radii.iter().any(|radius| *radius > 0.0) {
        sk::Path::rect(convert::rect(&bounds), None)
    } else {
        sk::Path::rrect(convert::rounded_rect(&bounds, radii), None)
    }
}

/// An ellipse inscribed in a `width` by `height` box, cut to an arc or a
/// ring.
///
/// `start_angle` and `sweep` are in degrees, clockwise from 3 o'clock;
/// a sweep of 360 either way is the whole ellipse. `inner_radius` is the
/// radius of the hole as a fraction of the outer radius: 0 gives a filled
/// ellipse or a pie slice, more gives a ring or a piece of one.
pub(crate) fn ellipse(
    width: f64,
    height: f64,
    start_angle: f64,
    sweep: f64,
    inner_radius: f64,
) -> sk::Path {
    let (width, height) = (width.max(0.0) as f32, height.max(0.0) as f32);
    let bounds = sk::Rect::from_wh(width, height);
    let inner = if inner_radius.is_finite() {
        inner_radius.clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let hole = bounds.with_inset((width / 2.0 * (1.0 - inner), height / 2.0 * (1.0 - inner)));
    let sweep = if sweep.is_finite() {
        sweep.clamp(-360.0, 360.0)
    } else {
        360.0
    };
    let start = if start_angle.is_finite() {
        start_angle.rem_euclid(360.0) as f32
    } else {
        0.0
    };

    let mut builder = sk::PathBuilder::new();
    if sweep.abs() >= 360.0 - ANGLE_EPSILON {
        builder.add_oval(bounds, sk::PathDirection::CW, None);
        if inner > 0.0 {
            // Wound the other way, so it cuts a hole under either fill rule.
            builder.add_oval(hole, sk::PathDirection::CCW, None);
        }
    } else if sweep.abs() > ANGLE_EPSILON {
        let sweep = sweep as f32;
        if inner > 0.0 {
            // Along the outside, across to the hole, and back along it.
            builder.arc_to(bounds, start, sweep, true);
            builder.arc_to(hole, start + sweep, -sweep, false);
        } else {
            builder.move_to(bounds.center());
            builder.arc_to(bounds, start, sweep, false);
        }
        builder.close();
    }
    builder.detach()
}

/// A star inscribed in a `width` by `height` box, first point at the top.
///
/// `inner_radius` is the radius of the inner corners as a fraction of the
/// outer radius; 1 gives a regular polygon with twice as many corners.
pub(crate) fn star(width: f64, height: f64, points: u32, inner_radius: f64) -> sk::Path {
    let points = points.min(MAX_STAR_POINTS);
    if points == 0 {
        return sk::Path::default();
    }
    let inner = if inner_radius.is_finite() {
        inner_radius.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let (radius_x, radius_y) = (width.max(0.0) / 2.0, height.max(0.0) / 2.0);
    let step = std::f64::consts::PI / f64::from(points);
    let corners: Vec<sk::Point> = (0..points * 2)
        .map(|corner| {
            let angle = -std::f64::consts::FRAC_PI_2 + f64::from(corner) * step;
            let reach = if corner % 2 == 0 { 1.0 } else { inner };
            sk::Point::new(
                (radius_x + radius_x * reach * angle.cos()) as f32,
                (radius_y + radius_y * reach * angle.sin()) as f32,
            )
        })
        .collect();
    let mut builder = sk::PathBuilder::new();
    builder.add_polygon(&corners, true);
    builder.detach()
}

/// A pen-drawn outline. Coordinates are pixels in the node's space.
///
/// Two neighbouring anchors are joined by a straight line when neither has
/// a handle on that side, and by a cubic curve otherwise, a missing handle
/// sitting on its anchor.
pub(crate) fn pen(data: &PathData) -> Outline {
    let point = |pair: [f64; 2]| sk::Point::new(pair[0] as f32, pair[1] as f32);
    let builder = || {
        sk::PathBuilder::new_with_fill_type(match data.fill_rule {
            FillRule::NonZero => sk::PathFillType::Winding,
            FillRule::EvenOdd => sk::PathFillType::EvenOdd,
        })
    };
    // The whole outline, and its closed and its open pieces apart.
    let (mut all, mut shut, mut open) = (builder(), builder(), builder());
    let (mut any_shut, mut any_open) = (false, false);
    for subpath in &data.subpaths {
        let Some(first) = subpath.anchors.first() else {
            continue;
        };
        let kind = if subpath.closed {
            any_shut = true;
            &mut shut
        } else {
            any_open = true;
            &mut open
        };
        for builder in [&mut all, kind] {
            builder.move_to(point(first.point));
            let following = subpath.anchors.iter().skip(1);
            // A closed piece ends with the segment back to its first anchor.
            let closing = subpath.closed.then_some(first);
            for (from, to) in subpath.anchors.iter().zip(following.chain(closing)) {
                match (from.handle_out, to.handle_in) {
                    (None, None) => builder.line_to(point(to.point)),
                    (handle_out, handle_in) => builder.cubic_to(
                        point(handle_out.unwrap_or(from.point)),
                        point(handle_in.unwrap_or(to.point)),
                        point(to.point),
                    ),
                };
            }
            if subpath.closed {
                builder.close();
            }
        }
    }
    let closed = !data.subpaths.is_empty() && !any_open;
    Outline {
        path: all.detach(),
        closed,
        split: (any_shut && any_open).then(|| {
            Box::new((
                Outline::new(shut.detach(), true),
                Outline::new(open.detach(), false),
            ))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::model::{Anchor, Subpath};

    fn inside(path: &sk::Path, x: f32, y: f32) -> bool {
        path.contains((x, y))
    }

    fn bounds(path: &sk::Path) -> (f32, f32, f32, f32) {
        let bounds = path.compute_tight_bounds();
        (bounds.left, bounds.top, bounds.right, bounds.bottom)
    }

    #[test]
    fn rectangles_round_each_corner_by_its_own_radius() {
        let square = rect(100.0, 60.0, &[0.0; 4]);
        assert!(square.is_rect().is_some());
        assert!(inside(&square, 1.0, 1.0));

        let rounded = rect(100.0, 60.0, &[30.0, 0.0, 10.0, 0.0]);
        assert!(!inside(&rounded, 3.0, 3.0), "top-left is cut by 30");
        assert!(inside(&rounded, 97.0, 3.0), "top-right is square");
        assert!(!inside(&rounded, 99.0, 59.0), "bottom-right is cut by 10");
        assert!(inside(&rounded, 96.0, 56.0));
        assert!(inside(&rounded, 1.0, 59.0), "bottom-left is square");
        assert_eq!(bounds(&rounded), (0.0, 0.0, 100.0, 60.0));
        // Sizes below zero are no rectangle at all, not a flipped one.
        assert!(
            rect(-10.0, 20.0, &[0.0; 4])
                .compute_tight_bounds()
                .is_empty()
        );
    }

    #[test]
    fn whole_ellipses_and_rings() {
        let whole = ellipse(100.0, 60.0, 0.0, 360.0, 0.0);
        assert!(inside(&whole, 50.0, 30.0));
        assert!(inside(&whole, 97.0, 30.0));
        assert!(
            !inside(&whole, 5.0, 5.0),
            "the corner of the box is outside"
        );
        assert_eq!(bounds(&whole), (0.0, 0.0, 100.0, 60.0));
        // A full turn the other way is the same shape.
        assert!(inside(&ellipse(100.0, 60.0, 90.0, -360.0, 0.0), 50.0, 30.0));

        let ring = ellipse(100.0, 100.0, 0.0, 360.0, 0.5);
        assert!(!inside(&ring, 50.0, 50.0), "the hole");
        assert!(!inside(&ring, 70.0, 50.0), "still the hole, radius 25");
        assert!(inside(&ring, 80.0, 50.0));
        assert!(inside(&ring, 50.0, 10.0));
        // A hole as large as the ellipse leaves no area, but an outline.
        let rim = ellipse(100.0, 100.0, 0.0, 360.0, 1.0);
        assert!(!inside(&rim, 80.0, 50.0));
        assert!(!rim.is_empty());
    }

    #[test]
    fn arcs_start_at_three_o_clock_and_run_clockwise() {
        // A quarter from 3 o'clock, clockwise on screen: the bottom-right.
        let quarter = ellipse(100.0, 100.0, 0.0, 90.0, 0.0);
        assert!(inside(&quarter, 70.0, 70.0), "bottom-right");
        assert!(!inside(&quarter, 70.0, 30.0), "top-right");
        assert!(!inside(&quarter, 30.0, 70.0), "bottom-left");
        assert!(
            inside(&quarter, 52.0, 52.0),
            "a pie slice reaches the centre"
        );

        // A negative sweep runs anticlockwise: the top-right.
        let back = ellipse(100.0, 100.0, 0.0, -90.0, 0.0);
        assert!(inside(&back, 70.0, 30.0));
        assert!(!inside(&back, 70.0, 70.0));

        // Three quarters starting at 6 o'clock leave out the bottom-right.
        let most = ellipse(100.0, 100.0, 90.0, 270.0, 0.0);
        assert!(inside(&most, 30.0, 70.0) && inside(&most, 30.0, 30.0));
        assert!(inside(&most, 70.0, 30.0));
        assert!(!inside(&most, 70.0, 70.0));

        // A piece of a ring does not reach the centre.
        let piece = ellipse(100.0, 100.0, 0.0, 90.0, 0.5);
        assert!(!inside(&piece, 55.0, 55.0));
        assert!(inside(&piece, 80.0, 80.0));
        assert!(!inside(&piece, 80.0, 20.0));

        // No sweep, no shape. Broken numbers fall back to something whole.
        assert!(ellipse(100.0, 100.0, 45.0, 0.0, 0.0).is_empty());
        assert!(inside(
            &ellipse(100.0, 100.0, f64::NAN, f64::NAN, f64::NAN),
            50.0,
            50.0
        ));
    }

    #[test]
    fn stars_point_up_and_fill_their_box() {
        let five = star(100.0, 100.0, 5, 0.4);
        assert_eq!(five.count_points(), 10);
        let (left, top, right, bottom) = bounds(&five);
        assert!(top.abs() < 1e-4, "the first point touches the top: {top}");
        assert!(left > 2.0 && right < 98.0 && bottom < 91.0);
        assert!(inside(&five, 50.0, 5.0), "just under the top point");
        assert!(inside(&five, 50.0, 50.0));
        assert!(!inside(&five, 50.0, 95.0), "between the two bottom points");
        assert!(!inside(&five, 20.0, 10.0));

        // An inner radius of 1 is a regular polygon with twice the corners.
        let decagon = star(100.0, 100.0, 5, 1.0);
        assert!(inside(&decagon, 50.0, 95.0));
        // A wide box stretches the star.
        assert!(inside(&star(300.0, 100.0, 4, 0.5), 290.0, 50.0));
        assert!(star(100.0, 100.0, 0, 0.5).is_empty());
        assert_eq!(star(10.0, 10.0, u32::MAX, 0.5).count_points(), 2048);
    }

    fn triangle(closed: bool) -> Subpath {
        Subpath {
            anchors: vec![
                Anchor::corner(0.0, 0.0),
                Anchor::corner(100.0, 0.0),
                Anchor::corner(50.0, 80.0),
            ],
            closed,
        }
    }

    #[test]
    fn pen_paths_are_lines_and_curves() {
        let closed = pen(&PathData {
            subpaths: vec![triangle(true)],
            fill_rule: FillRule::NonZero,
        });
        assert!(closed.closed);
        assert!(inside(&closed.path, 50.0, 30.0));
        assert!(!inside(&closed.path, 5.0, 60.0));
        assert_eq!(closed.path.count_verbs(), 5, "move, three lines, close");

        let open = pen(&PathData {
            subpaths: vec![triangle(false)],
            fill_rule: FillRule::NonZero,
        });
        assert!(!open.closed);
        assert_eq!(open.path.count_verbs(), 3, "move, two lines");
        // Filling still treats it as closed.
        assert!(inside(&open.path, 50.0, 30.0));

        // A handle on either side makes the segment a curve that bulges
        // toward it; the missing handle sits on its anchor.
        let curved = pen(&PathData {
            subpaths: vec![Subpath {
                anchors: vec![
                    Anchor {
                        point: [0.0, 50.0],
                        handle_in: None,
                        handle_out: Some([0.0, 0.0]),
                    },
                    Anchor {
                        point: [100.0, 50.0],
                        handle_in: Some([100.0, 0.0]),
                        handle_out: None,
                    },
                ],
                closed: true,
            }],
            fill_rule: FillRule::NonZero,
        });
        assert!(inside(&curved.path, 50.0, 20.0), "under the arch");
        assert!(!inside(&curved.path, 50.0, 5.0), "above it");
        assert!(!inside(&curved.path, 50.0, 60.0), "below the flat side");

        let nothing = pen(&PathData::default());
        assert!(nothing.path.is_empty() && !nothing.closed);
        assert!(closed.split.is_none() && open.split.is_none());
    }

    #[test]
    fn closed_and_open_pieces_are_also_kept_apart() {
        let mut line = triangle(false);
        line.anchors.truncate(2);
        let mixed = pen(&PathData {
            subpaths: vec![triangle(true), line],
            fill_rule: FillRule::NonZero,
        });
        assert!(!mixed.closed);
        assert_eq!(
            mixed.path.count_verbs(),
            7,
            "the closed piece, then the line"
        );
        let parts: Vec<&Outline> = mixed.parts().collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].closed && !parts[1].closed);
        assert_eq!(parts[0].path.count_verbs(), 5);
        assert_eq!(parts[1].path.count_verbs(), 2, "move, line");
    }

    #[test]
    fn the_fill_rule_decides_what_overlaps_keep() {
        let square = |inset: f64| Subpath {
            anchors: vec![
                Anchor::corner(inset, inset),
                Anchor::corner(100.0 - inset, inset),
                Anchor::corner(100.0 - inset, 100.0 - inset),
                Anchor::corner(inset, 100.0 - inset),
            ],
            closed: true,
        };
        let nested = |fill_rule| {
            pen(&PathData {
                subpaths: vec![square(0.0), square(30.0)],
                fill_rule,
            })
            .path
        };
        assert!(inside(&nested(FillRule::NonZero), 50.0, 50.0));
        assert!(!inside(&nested(FillRule::EvenOdd), 50.0, 50.0));
        assert!(inside(&nested(FillRule::EvenOdd), 10.0, 10.0));
    }
}
