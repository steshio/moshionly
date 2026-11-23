//! Shape geometry as Lottie paths: rectangles with different corners,
//! arcs and rings, stars and pen paths, built to match what the renderer
//! draws, point for point.
//!
//! A Lottie path is one contour: vertices, and for each the handles of the
//! curves arriving and leaving, relative to the vertex. A shape of several
//! contours is several paths in one group, filled together.

use std::f64::consts::{FRAC_PI_2, PI};

use moshionly_core::model::PathData;
use serde_json::Value;

use super::fit::{Animated, Keyframe};
use super::json::object;
use super::props::array;

/// How far along a quarter circle's tangents its Bezier handles reach, as
/// a part of the radius.
const KAPPA: f64 = 0.552_284_749_830_793_6;

/// Sweeps this close to a full turn or to nothing count as exactly that,
/// as the renderer counts them.
const ANGLE_EPSILON: f64 = 1e-6;

/// The most points a star is drawn with, as the renderer draws it.
const MAX_STAR_POINTS: u32 = 1024;

/// One contour: points joined by cubic curves.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Contour {
    /// Whether the last point joins the first.
    pub closed: bool,
    /// The points.
    pub vertices: Vec<[f64; 2]>,
    /// Each point's incoming handle, relative to the point.
    pub ins: Vec<[f64; 2]>,
    /// Each point's outgoing handle, relative to the point.
    pub outs: Vec<[f64; 2]>,
}

impl Contour {
    /// Starts a contour at `point`.
    pub fn starting_at(point: [f64; 2]) -> Self {
        Self {
            closed: false,
            vertices: vec![point],
            ins: vec![[0.0, 0.0]],
            outs: vec![[0.0, 0.0]],
        }
    }

    /// The last point.
    fn last(&self) -> [f64; 2] {
        self.vertices.last().copied().unwrap_or_default()
    }

    /// Adds a straight line to `point`.
    pub fn line_to(&mut self, point: [f64; 2]) {
        self.vertices.push(point);
        self.ins.push([0.0, 0.0]);
        self.outs.push([0.0, 0.0]);
    }

    /// Adds a cubic curve to `point` with absolute control points `c1`, `c2`.
    pub fn cubic_to(&mut self, c1: [f64; 2], c2: [f64; 2], point: [f64; 2]) {
        let last = self.last();
        if let Some(out) = self.outs.last_mut() {
            *out = [c1[0] - last[0], c1[1] - last[1]];
        }
        self.vertices.push(point);
        self.ins.push([c2[0] - point[0], c2[1] - point[1]]);
        self.outs.push([0.0, 0.0]);
    }

    /// Closes the contour. A last point on top of the first is merged into
    /// it, keeping its incoming handle.
    pub fn close(&mut self) {
        self.closed = true;
        if self.vertices.len() > 1 {
            let (first, last) = (self.vertices[0], self.last());
            if (first[0] - last[0]).abs() < 1e-9 && (first[1] - last[1]).abs() < 1e-9 {
                self.vertices.pop();
                self.outs.pop();
                if let Some(incoming) = self.ins.pop() {
                    self.ins[0] = incoming;
                }
            }
        }
    }

    /// The contour as the value of a Lottie path.
    pub fn to_json(&self, places: i32) -> Value {
        let points = |list: &[[f64; 2]]| {
            Value::Array(list.iter().map(|point| array(point, places)).collect())
        };
        object([
            ("c", Value::Bool(self.closed)),
            ("v", points(&self.vertices)),
            ("i", points(&self.ins)),
            ("o", points(&self.outs)),
        ])
    }

    /// Every number of the contour in one list, for fitting keyframes to a
    /// changing shape.
    pub fn flatten(&self, into: &mut Vec<f64>) {
        for list in [&self.vertices, &self.ins, &self.outs] {
            for point in list {
                into.extend_from_slice(point);
            }
        }
    }

    /// The contour with the numbers `flatten` gave, read back from
    /// `numbers`, which it advances.
    fn unflatten(&self, numbers: &mut &[f64]) -> Contour {
        let mut take = |count: usize| {
            let (head, tail) = numbers.split_at(count.min(numbers.len()));
            *numbers = tail;
            head.as_chunks::<2>().0.to_vec()
        };
        let count = self.vertices.len() * 2;
        Contour {
            closed: self.closed,
            vertices: take(count),
            ins: take(count),
            outs: take(count),
        }
    }
}

/// Contours whose points may move over time: a template for their
/// structure and the fitted numbers.
pub(crate) struct Outline {
    /// The contours at one moment, for their structure.
    pub template: Vec<Contour>,
    /// Every number of every contour over time.
    pub numbers: Animated,
}

impl Outline {
    /// One Lottie path item per contour.
    pub fn items(&self, places: i32) -> Vec<Value> {
        (0..self.template.len())
            .map(|index| {
                object([
                    ("ty", Value::from("sh")),
                    ("ks", self.contour_property(index, places)),
                ])
            })
            .collect()
    }

    /// The path property of contour `index`.
    fn contour_property(&self, index: usize, places: i32) -> Value {
        let contour = |numbers: &[f64]| {
            let mut rest = numbers;
            let mut found = Contour::default();
            for (position, template) in self.template.iter().enumerate() {
                let read = template.unflatten(&mut rest);
                if position == index {
                    found = read;
                }
            }
            found
        };
        match &self.numbers {
            Animated::Static(numbers) => object([("k", contour(numbers).to_json(places))]),
            Animated::Keyframes(keys) => {
                let frames: Vec<Keyframe> = keys.to_vec();
                let mut value = super::props::property(&self.numbers, places);
                // Each keyframe's value is a list holding one path.
                if let Some(Value::Array(written)) = value.get_mut("k") {
                    for (frame, key) in written.iter_mut().zip(&frames) {
                        if let Some(start) = frame.get_mut("s") {
                            *start = Value::Array(vec![contour(&key.value).to_json(places)]);
                        }
                    }
                }
                value
            }
        }
    }
}

/// All numbers of `contours`, in order.
pub(crate) fn flatten(contours: &[Contour]) -> Vec<f64> {
    let mut numbers = Vec::new();
    for contour in contours {
        contour.flatten(&mut numbers);
    }
    numbers
}

/// A `width` by `height` rectangle at the origin with corner radii
/// `[top-left, top-right, bottom-right, bottom-left]`, each limited to half
/// the shorter side as the renderer limits them. Clockwise from the top
/// left corner.
pub(crate) fn rounded_rect(width: f64, height: f64, radii: [f64; 4]) -> Contour {
    let limit = (width.min(height) / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|radius| radius.clamp(0.0, limit));
    let (w, h) = (width.max(0.0), height.max(0.0));
    let mut contour = Contour::starting_at([tl, 0.0]);
    let corner =
        |contour: &mut Contour, radius: f64, from: [f64; 2], to: [f64; 2], center: [f64; 2]| {
            if radius <= 0.0 {
                contour.line_to(to);
                return;
            }
            if contour.last() != from {
                contour.line_to(from);
            }
            // Handles point along the tangents, toward the corner.
            let toward = |point: [f64; 2]| {
                let corner = [from[0] + to[0] - center[0], from[1] + to[1] - center[1]];
                [
                    point[0] + (corner[0] - point[0]) * KAPPA,
                    point[1] + (corner[1] - point[1]) * KAPPA,
                ]
            };
            contour.cubic_to(toward(from), toward(to), to);
        };
    corner(&mut contour, tr, [w - tr, 0.0], [w, tr], [w - tr, tr]);
    corner(&mut contour, br, [w, h - br], [w - br, h], [w - br, h - br]);
    corner(&mut contour, bl, [bl, h], [0.0, h - bl], [bl, h - bl]);
    corner(&mut contour, tl, [0.0, tl], [tl, 0.0], [tl, tl]);
    contour.close();
    contour
}

/// Points on the ellipse inscribed in a `width` by `height` box, as the
/// arc from `start` to `start + sweep` degrees (clockwise from 3 o'clock),
/// in curves of at most a quarter turn. The first point is not added.
fn arc(contour: &mut Contour, width: f64, height: f64, inset: f64, start: f64, sweep: f64) {
    let (rx, ry) = (width / 2.0 * inset, height / 2.0 * inset);
    let (cx, cy) = (width / 2.0, height / 2.0);
    let pieces = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
    let step = sweep.to_radians() / pieces as f64;
    // Handle length for an arc of `step` radians on a unit circle.
    let reach = 4.0 / 3.0 * (step / 4.0).tan();
    let mut angle = start.to_radians();
    for _ in 0..pieces {
        let next = angle + step;
        let point = |a: f64| [cx + rx * a.cos(), cy + ry * a.sin()];
        let tangent = |a: f64| [-rx * a.sin(), ry * a.cos()];
        let (p0, p1) = (point(angle), point(next));
        let (t0, t1) = (tangent(angle), tangent(next));
        contour.cubic_to(
            [p0[0] + t0[0] * reach, p0[1] + t0[1] * reach],
            [p1[0] - t1[0] * reach, p1[1] - t1[1] * reach],
            p1,
        );
        angle = next;
    }
}

/// An ellipse inscribed in a `width` by `height` box, cut to an arc or a
/// ring as the renderer cuts it. `None` for a whole ellipse without a hole,
/// which Lottie draws as an ellipse shape.
pub(crate) fn ellipse(
    width: f64,
    height: f64,
    start_angle: f64,
    sweep: f64,
    inner_radius: f64,
) -> Option<Vec<Contour>> {
    let inner = if inner_radius.is_finite() {
        inner_radius.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let sweep = if sweep.is_finite() {
        sweep.clamp(-360.0, 360.0)
    } else {
        360.0
    };
    let start = if start_angle.is_finite() {
        start_angle.rem_euclid(360.0)
    } else {
        0.0
    };
    let (w, h) = (width.max(0.0), height.max(0.0));
    let point = |scale: f64, degrees: f64| {
        let a = degrees.to_radians();
        [
            w / 2.0 + w / 2.0 * scale * a.cos(),
            h / 2.0 + h / 2.0 * scale * a.sin(),
        ]
    };
    if sweep.abs() >= 360.0 - ANGLE_EPSILON {
        if inner <= 0.0 {
            return None;
        }
        // The outside clockwise and the hole the other way, so the hole is
        // cut under either fill rule.
        let mut outside = Contour::starting_at(point(1.0, 0.0));
        arc(&mut outside, w, h, 1.0, 0.0, 360.0);
        outside.close();
        let mut hole = Contour::starting_at(point(inner, 0.0));
        arc(&mut hole, w, h, inner, 0.0, -360.0);
        hole.close();
        return Some(vec![outside, hole]);
    }
    if sweep.abs() <= ANGLE_EPSILON {
        return Some(Vec::new());
    }
    let mut contour;
    if inner > 0.0 {
        contour = Contour::starting_at(point(1.0, start));
        arc(&mut contour, w, h, 1.0, start, sweep);
        contour.line_to(point(inner, start + sweep));
        arc(&mut contour, w, h, inner, start + sweep, -sweep);
    } else {
        contour = Contour::starting_at([w / 2.0, h / 2.0]);
        contour.line_to(point(1.0, start));
        arc(&mut contour, w, h, 1.0, start, sweep);
    }
    contour.close();
    Some(vec![contour])
}

/// A star inscribed in a `width` by `height` box, first point at the top,
/// as the renderer draws it.
pub(crate) fn star(width: f64, height: f64, points: u32, inner_radius: f64) -> Vec<Contour> {
    let points = points.min(MAX_STAR_POINTS);
    if points == 0 {
        return Vec::new();
    }
    let inner = if inner_radius.is_finite() {
        inner_radius.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let (rx, ry) = (width.max(0.0) / 2.0, height.max(0.0) / 2.0);
    let step = PI / f64::from(points);
    let corner = |index: u32| {
        let angle = -FRAC_PI_2 + f64::from(index) * step;
        let reach = if index.is_multiple_of(2) { 1.0 } else { inner };
        [rx + rx * reach * angle.cos(), ry + ry * reach * angle.sin()]
    };
    let mut contour = Contour::starting_at(corner(0));
    for index in 1..points * 2 {
        contour.line_to(corner(index));
    }
    contour.close();
    vec![contour]
}

/// A pen path, already in the node's pixels.
///
/// Two neighbouring anchors are joined by a straight line when neither has
/// a handle on that side, and by a cubic curve otherwise, a missing handle
/// sitting on its anchor: exactly Lottie's own rule, so this is a direct
/// translation.
pub(crate) fn pen(data: &PathData) -> Vec<Contour> {
    data.subpaths
        .iter()
        .filter(|subpath| !subpath.anchors.is_empty())
        .map(|subpath| {
            let relative = |handle: Option<[f64; 2]>, point: [f64; 2]| {
                handle.map_or([0.0, 0.0], |handle| {
                    [handle[0] - point[0], handle[1] - point[1]]
                })
            };
            Contour {
                closed: subpath.closed,
                vertices: subpath.anchors.iter().map(|anchor| anchor.point).collect(),
                ins: subpath
                    .anchors
                    .iter()
                    .map(|anchor| relative(anchor.handle_in, anchor.point))
                    .collect(),
                outs: subpath
                    .anchors
                    .iter()
                    .map(|anchor| relative(anchor.handle_out, anchor.point))
                    .collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point at `t` on the cubic from `p0` with handles `c1`, `c2` to `p3`.
    fn on_curve(p0: [f64; 2], c1: [f64; 2], c2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
        let u = 1.0 - t;
        let mix = |i: usize| {
            u * u * u * p0[i]
                + 3.0 * u * u * t * c1[i]
                + 3.0 * u * t * t * c2[i]
                + t * t * t * p3[i]
        };
        [mix(0), mix(1)]
    }

    /// Every curve of a contour, as absolute points.
    fn curves(contour: &Contour) -> Vec<[[f64; 2]; 4]> {
        let n = contour.vertices.len();
        let count = if contour.closed { n } else { n - 1 };
        (0..count)
            .map(|i| {
                let j = (i + 1) % n;
                let (a, b) = (contour.vertices[i], contour.vertices[j]);
                let out = contour.outs[i];
                let into = contour.ins[j];
                [
                    a,
                    [a[0] + out[0], a[1] + out[1]],
                    [b[0] + into[0], b[1] + into[1]],
                    b,
                ]
            })
            .collect()
    }

    #[test]
    fn round_corners_stay_on_their_circles() {
        let contour = rounded_rect(200.0, 100.0, [10.0, 20.0, 30.0, 80.0]);
        assert!(contour.closed);
        // The bottom-left radius is limited to half the height.
        let centers = [[10.0, 10.0], [180.0, 20.0], [170.0, 70.0], [50.0, 50.0]];
        let radii = [10.0, 20.0, 30.0, 50.0];
        let mut arcs = 0;
        for [p0, c1, c2, p3] in curves(&contour) {
            if c1 == p0 && c2 == p3 {
                continue;
            }
            arcs += 1;
            let (center, radius) = centers
                .iter()
                .zip(radii)
                .find(|(center, radius)| {
                    let d = ((p0[0] - center[0]).powi(2) + (p0[1] - center[1]).powi(2)).sqrt();
                    (d - radius).abs() < 1e-9
                })
                .expect("each arc is on a corner circle");
            for step in 0..=10 {
                let point = on_curve(p0, c1, c2, p3, f64::from(step) / 10.0);
                let d = ((point[0] - center[0]).powi(2) + (point[1] - center[1]).powi(2)).sqrt();
                assert!((d - radius).abs() < radius * 0.001, "{point:?}");
            }
        }
        assert_eq!(arcs, 4);
    }

    #[test]
    fn rings_and_slices_are_cut_like_the_renderer_cuts_them() {
        assert!(
            ellipse(100.0, 50.0, 0.0, 360.0, 0.0).is_none(),
            "a whole ellipse"
        );
        let ring = ellipse(100.0, 100.0, 0.0, 360.0, 0.5).unwrap();
        assert_eq!(ring.len(), 2);
        for [p0, c1, c2, p3] in curves(&ring[1]) {
            for step in 0..=8 {
                let point = on_curve(p0, c1, c2, p3, f64::from(step) / 8.0);
                let d = ((point[0] - 50.0).powi(2) + (point[1] - 50.0).powi(2)).sqrt();
                assert!((d - 25.0).abs() < 0.01, "the hole has half the radius");
            }
        }
        let slice = ellipse(100.0, 100.0, 0.0, 90.0, 0.0).unwrap();
        assert_eq!(
            slice[0].vertices[0],
            [50.0, 50.0],
            "a slice starts at the centre"
        );
        let last = slice[0].vertices.last().unwrap();
        assert!((last[0] - 50.0).abs() < 1e-9 && (last[1] - 100.0).abs() < 1e-9);
        assert!(ellipse(100.0, 100.0, 0.0, 0.0, 0.0).unwrap().is_empty());
    }

    #[test]
    fn stars_start_at_the_top() {
        let five = star(100.0, 100.0, 5, 0.5);
        assert_eq!(five[0].vertices.len(), 10);
        assert_eq!(five[0].vertices[0], [50.0, 0.0]);
        assert!(star(100.0, 100.0, 0, 0.5).is_empty());
    }

    #[test]
    fn shapes_flatten_and_come_back() {
        let mut shapes = vec![rounded_rect(10.0, 10.0, [2.0; 4])];
        shapes.extend(star(10.0, 20.0, 3, 0.4));
        let numbers = flatten(&shapes);
        let mut rest = numbers.as_slice();
        let back: Vec<Contour> = shapes
            .iter()
            .map(|shape| shape.unflatten(&mut rest))
            .collect();
        assert_eq!(back, shapes);
        assert!(rest.is_empty());
    }
}
