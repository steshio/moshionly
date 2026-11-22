//! Small 2D geometry types shared by the model, tree queries and the scene.
//!
//! The coordinate system is the one of the document: x grows to the right,
//! y grows downward, and positive angles turn clockwise on screen.

use std::ops::Mul;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A point or a vector.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate, growing downward.
    pub y: f64,
}

impl Point {
    /// The origin.
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    /// Creates a point.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width, never negative.
    pub width: f64,
    /// Height, never negative.
    pub height: f64,
}

impl Rect {
    /// Creates a rectangle from its top-left corner and size.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The right edge.
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    /// The bottom edge.
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// The centre point.
    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// The four corners: top-left, top-right, bottom-right, bottom-left.
    pub fn corners(&self) -> [Point; 4] {
        [
            Point::new(self.x, self.y),
            Point::new(self.right(), self.y),
            Point::new(self.right(), self.bottom()),
            Point::new(self.x, self.bottom()),
        ]
    }

    /// Whether the point is inside or on the edge.
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.right()
            && point.y >= self.y
            && point.y <= self.bottom()
    }

    /// The smallest rectangle containing both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }

    /// The smallest rectangle containing all points, or `None` for no points.
    pub fn bounding(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
        let mut points = points.into_iter();
        let first = points.next()?;
        let (mut min, mut max) = (first, first);
        for p in points {
            min = Point::new(min.x.min(p.x), min.y.min(p.y));
            max = Point::new(max.x.max(p.x), max.y.max(p.y));
        }
        Some(Rect::new(min.x, min.y, max.x - min.x, max.y - min.y))
    }
}

/// A 2D affine transform.
///
/// The six numbers `[a, b, c, d, e, f]` map a point like this, which is the
/// same order as CSS `matrix(a, b, c, d, e, f)` and the HTML canvas:
///
/// ```text
/// x' = a·x + c·y + e
/// y' = b·x + d·y + f
/// ```
///
/// It serialises as that array of six numbers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Affine(pub [f64; 6]);

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    /// The transform that changes nothing.
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// Moves by `(x, y)`.
    pub const fn translate(x: f64, y: f64) -> Self {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// Scales by `sx` horizontally and `sy` vertically about the origin.
    pub const fn scale(sx: f64, sy: f64) -> Self {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// Rotates about the origin. Positive angles turn clockwise on screen.
    pub fn rotate(degrees: f64) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Affine([cos, sin, -sin, cos, 0.0, 0.0])
    }

    /// The transform of a layer box: rotation and uniform scale about the
    /// centre of a `width` by `height` box whose top-left corner is at `(x, y)`.
    ///
    /// The result maps the box's own coordinates (origin at its top-left
    /// corner) into its parent's coordinates.
    pub fn layer(x: f64, y: f64, width: f64, height: f64, rotation: f64, scale: f64) -> Self {
        let (cx, cy) = (width / 2.0, height / 2.0);
        Affine::translate(x + cx, y + cy)
            * Affine::rotate(rotation)
            * Affine::scale(scale, scale)
            * Affine::translate(-cx, -cy)
    }

    /// Applies the transform to a point.
    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// Applies only the linear part (rotation and scale) to a vector.
    pub fn apply_vector(&self, v: Point) -> Point {
        let [a, b, c, d, _, _] = self.0;
        Point::new(a * v.x + c * v.y, b * v.x + d * v.y)
    }

    /// The inverse transform, or `None` when the transform flattens space
    /// (for example a scale of zero).
    pub fn invert(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Affine([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ]))
    }

    /// The axis-aligned box around a transformed rectangle.
    pub fn bounds_of(&self, rect: &Rect) -> Rect {
        Rect::bounding(rect.corners().map(|corner| self.apply(corner)))
            .expect("a rectangle has four corners")
    }

    /// Whether this is the identity, within rounding noise.
    pub fn is_identity(&self) -> bool {
        self.0
            .iter()
            .zip(Self::IDENTITY.0)
            .all(|(value, identity)| (value - identity).abs() < 1e-12)
    }
}

impl Mul for Affine {
    type Output = Affine;

    /// `outer * inner` applies `inner` first, then `outer`.
    fn mul(self, inner: Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = inner.0;
        Affine([
            a1 * a2 + c1 * b2,
            b1 * a2 + d1 * b2,
            a1 * c2 + c1 * d2,
            b1 * c2 + d1 * d2,
            a1 * e2 + c1 * f2 + e1,
            b1 * e2 + d1 * f2 + f1,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn rotation_is_clockwise_on_a_y_down_screen() {
        // A quarter turn takes "right" to "down".
        let turned = Affine::rotate(90.0).apply(Point::new(1.0, 0.0));
        assert!(close(turned, Point::new(0.0, 1.0)), "{turned:?}");
    }

    #[test]
    fn multiplication_applies_the_right_hand_side_first() {
        let m = Affine::translate(10.0, 0.0) * Affine::scale(2.0, 2.0);
        assert!(close(m.apply(Point::new(1.0, 1.0)), Point::new(12.0, 2.0)));
    }

    #[test]
    fn layer_transform_pivots_on_the_box_centre() {
        let m = Affine::layer(100.0, 50.0, 40.0, 20.0, 90.0, 2.0);
        // The centre stays where the unrotated box has it.
        assert!(close(
            m.apply(Point::new(20.0, 10.0)),
            Point::new(120.0, 60.0)
        ));
        // The top-left corner swings to the top-right of the scaled box.
        assert!(close(m.apply(Point::ZERO), Point::new(140.0, 20.0)));
    }

    #[test]
    fn invert_round_trips() {
        let m = Affine::layer(3.0, 4.0, 50.0, 80.0, 33.0, 1.7);
        let back = m.invert().unwrap() * m;
        assert!(back.is_identity(), "{back:?}");
        assert!(Affine::scale(0.0, 0.0).invert().is_none());
    }

    #[test]
    fn bounds_of_a_rotated_box() {
        let m = Affine::layer(0.0, 0.0, 100.0, 50.0, 90.0, 1.0);
        let bounds = m.bounds_of(&Rect::new(0.0, 0.0, 100.0, 50.0));
        assert!((bounds.x - 25.0).abs() < 1e-9 && (bounds.y + 25.0).abs() < 1e-9);
        assert!((bounds.width - 50.0).abs() < 1e-9 && (bounds.height - 100.0).abs() < 1e-9);
    }

    #[test]
    fn rect_helpers() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, -5.0, 10.0, 10.0);
        assert_eq!(a.union(&b), Rect::new(0.0, -5.0, 15.0, 15.0));
        assert!(a.contains(Point::new(10.0, 10.0)));
        assert!(!a.contains(Point::new(10.1, 10.0)));
        assert_eq!(Rect::bounding([]), None);
    }
}
