//! Path data for pen-drawn shapes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::defaults;
use crate::geometry::{Point, Rect};

/// The outline of a `path` layer: one or more subpaths made of anchors.
///
/// On a layer, coordinates are normalised to the layer's box: `[0, 0]` is the
/// top-left corner of the box and `[1, 1]` the bottom-right, so resizing the
/// layer stretches the path. In a scene the same type holds pixels in the
/// layer's own coordinate space.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathData {
    /// The pieces of the outline. Default: none.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub subpaths: Vec<Subpath>,
    /// Which enclosed areas count as inside when subpaths overlap. Default `non_zero`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub fill_rule: FillRule,
}

/// One continuous run of anchors.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Subpath {
    /// The points the outline passes through, in drawing order.
    pub anchors: Vec<Anchor>,
    /// Whether the last anchor connects back to the first. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub closed: bool,
}

/// A point on a path, with optional curve handles.
///
/// The segment between two anchors is a cubic Bezier whose control points are
/// the first anchor's `out` handle and the second anchor's `in` handle. A
/// missing handle sits on its anchor, so two anchors without handles are
/// joined by a straight line.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// Position of the anchor as `[x, y]`.
    pub point: [f64; 2],
    /// Position (not offset) of the handle that shapes the curve arriving at
    /// this anchor, as `[x, y]`. Default: none, the curve arrives straight.
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub handle_in: Option<[f64; 2]>,
    /// Position (not offset) of the handle that shapes the curve leaving this
    /// anchor, as `[x, y]`. Default: none, the curve leaves straight.
    #[serde(rename = "out", default, skip_serializing_if = "Option::is_none")]
    pub handle_out: Option<[f64; 2]>,
}

impl Anchor {
    /// A corner anchor without handles.
    pub const fn corner(x: f64, y: f64) -> Self {
        Self {
            point: [x, y],
            handle_in: None,
            handle_out: None,
        }
    }

    /// An anchor with both handles.
    pub const fn smooth(point: [f64; 2], handle_in: [f64; 2], handle_out: [f64; 2]) -> Self {
        Self {
            point,
            handle_in: Some(handle_in),
            handle_out: Some(handle_out),
        }
    }
}

/// How overlapping subpaths decide what is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    /// A point is inside when the outline winds around it a non-zero number of times.
    #[default]
    NonZero,
    /// A point is inside when a ray from it crosses the outline an odd number
    /// of times, so a shape drawn inside another cuts a hole.
    EvenOdd,
}

impl PathData {
    /// A closed polygon through the given points.
    pub fn polygon(points: &[[f64; 2]]) -> Self {
        Self {
            subpaths: vec![Subpath {
                anchors: points.iter().map(|p| Anchor::corner(p[0], p[1])).collect(),
                closed: true,
            }],
            fill_rule: FillRule::default(),
        }
    }

    /// The same path with every coordinate multiplied by `(sx, sy)`.
    ///
    /// Scaling normalised path data by a layer's width and height gives the
    /// path in pixels.
    pub fn scaled(&self, sx: f64, sy: f64) -> PathData {
        let scale = |p: [f64; 2]| [p[0] * sx, p[1] * sy];
        PathData {
            subpaths: self
                .subpaths
                .iter()
                .map(|subpath| Subpath {
                    anchors: subpath
                        .anchors
                        .iter()
                        .map(|anchor| Anchor {
                            point: scale(anchor.point),
                            handle_in: anchor.handle_in.map(scale),
                            handle_out: anchor.handle_out.map(scale),
                        })
                        .collect(),
                    closed: subpath.closed,
                })
                .collect(),
            fill_rule: self.fill_rule,
        }
    }

    /// A box that contains every anchor and handle, or `None` for an empty path.
    ///
    /// Curves stay inside the hull of their control points, so this box always
    /// contains the outline; it can be slightly larger than the tight bounds.
    pub fn control_bounds(&self) -> Option<Rect> {
        Rect::bounding(self.subpaths.iter().flat_map(|subpath| {
            subpath.anchors.iter().flat_map(|anchor| {
                [Some(anchor.point), anchor.handle_in, anchor.handle_out]
                    .into_iter()
                    .flatten()
                    .map(|p| Point::new(p[0], p[1]))
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn handles_are_called_in_and_out_in_files() {
        let path: PathData = serde_json::from_value(json!({
            "subpaths": [{
                "closed": true,
                "anchors": [
                    { "point": [0, 0], "out": [0.5, 0] },
                    { "point": [1, 1], "in": [1, 0.5] },
                    { "point": [0, 1] }
                ]
            }],
            "fill_rule": "even_odd"
        }))
        .unwrap();
        let anchors = &path.subpaths[0].anchors;
        assert_eq!(anchors[0].handle_out, Some([0.5, 0.0]));
        assert_eq!(anchors[1].handle_in, Some([1.0, 0.5]));
        assert_eq!(anchors[2], Anchor::corner(0.0, 1.0));
        assert_eq!(path.fill_rule, FillRule::EvenOdd);

        let written = serde_json::to_value(&path).unwrap();
        assert_eq!(
            written["subpaths"][0]["anchors"][0]["out"],
            json!([0.5, 0.0])
        );
        assert!(written["subpaths"][0]["anchors"][2].get("in").is_none());
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_level() {
        assert!(serde_json::from_value::<PathData>(json!({ "d": "M0 0" })).is_err());
        assert!(
            serde_json::from_value::<PathData>(
                json!({ "subpaths": [{ "anchors": [], "open": true }] })
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<PathData>(
                json!({ "subpaths": [{ "anchors": [{ "point": [0, 0], "handle": [1, 1] }] }] })
            )
            .is_err()
        );
    }

    #[test]
    fn scaling_turns_unit_coordinates_into_pixels() {
        let path = PathData {
            subpaths: vec![Subpath {
                anchors: vec![
                    Anchor::smooth([0.5, 0.0], [0.25, 0.0], [0.75, 0.0]),
                    Anchor::corner(1.0, 1.0),
                ],
                closed: false,
            }],
            fill_rule: FillRule::NonZero,
        };
        let pixels = path.scaled(200.0, 100.0);
        let first = pixels.subpaths[0].anchors[0];
        assert_eq!(first.point, [100.0, 0.0]);
        assert_eq!(first.handle_in, Some([50.0, 0.0]));
        assert_eq!(first.handle_out, Some([150.0, 0.0]));
        assert_eq!(
            pixels.control_bounds(),
            Some(Rect::new(50.0, 0.0, 150.0, 100.0))
        );
    }

    #[test]
    fn empty_path_has_no_bounds() {
        assert_eq!(PathData::default().control_bounds(), None);
        assert_eq!(
            serde_json::to_value(PathData::default()).unwrap(),
            json!({})
        );
    }
}
