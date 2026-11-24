//! The pen: drawing a path one anchor at a time.
//!
//! A click adds a corner, a drag adds a smooth anchor whose handles follow
//! the pointer (the handle behind mirrors the one ahead). Clicking the
//! first anchor closes the path; Enter or Escape finishes it open. A path
//! needs two anchors to be kept.
//!
//! Anchors are held in canvas coordinates while drawing. When the path is
//! finished it becomes a `path` layer whose box surrounds every anchor and
//! handle, with the anchors stored relative to that box (`[0, 0]` its
//! top-left corner, `[1, 1]` its bottom-right).

use egui::{Pos2, Rect, Vec2, pos2};
use moshionly_core::model::{FillRule, LayerProps, PathData, PathLayer, Stroke, Subpath};
use moshionly_core::{Color, NodeId, build};

use super::Editor;
use super::create::CreateError;
use super::describe::next_name;
use super::inspect::tidy;
use crate::viewport::PenOverlay;

/// How near, in screen points, a click must come to the first anchor to
/// close the path.
pub const CLOSE_DISTANCE: f32 = 8.0;

/// Width of the outline a new path gets.
const STROKE_WIDTH: f64 = 2.0;

/// One anchor while drawing, in canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenAnchor {
    /// The point the path passes through.
    pub point: Pos2,
    /// The handle shaping the curve arriving here.
    pub handle_in: Option<Pos2>,
    /// The handle shaping the curve leaving here.
    pub handle_out: Option<Pos2>,
}

impl PenAnchor {
    fn corner(point: Pos2) -> Self {
        Self {
            point,
            handle_in: None,
            handle_out: None,
        }
    }
}

/// What a press with the pen did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenPress {
    /// An anchor was added.
    Added,
    /// The path was closed on its first anchor: time to finish it.
    Closed,
}

/// A path being drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    /// The artboard the path goes into.
    pub artboard: NodeId,
    /// The anchors so far.
    pub anchors: Vec<PenAnchor>,
}

impl Pen {
    /// A new path in `artboard`.
    pub fn new(artboard: NodeId) -> Self {
        Self {
            artboard,
            anchors: Vec::new(),
        }
    }

    /// Whether a press at `world` would close the path.
    pub fn closes_at(&self, world: Pos2, zoom: f32) -> bool {
        self.anchors.len() >= 2
            && self
                .anchors
                .first()
                .is_some_and(|first| first.point.distance(world) * zoom <= CLOSE_DISTANCE)
    }

    /// A press at `world`: adds a corner anchor, or closes the path when it
    /// is on the first anchor.
    pub fn press(&mut self, world: Pos2, zoom: f32) -> PenPress {
        if self.closes_at(world, zoom) {
            return PenPress::Closed;
        }
        self.anchors.push(PenAnchor::corner(world));
        PenPress::Added
    }

    /// Drags out the handles of the last anchor: the outgoing handle to
    /// `world`, the incoming one mirrored about the anchor.
    pub fn pull_handles(&mut self, world: Pos2) {
        if let Some(last) = self.anchors.last_mut() {
            if last.point.distance(world) < 0.5 {
                last.handle_in = None;
                last.handle_out = None;
            } else {
                last.handle_out = Some(world);
                last.handle_in = Some(last.point - (world - last.point));
            }
        }
    }

    /// Whether the path has enough anchors to keep.
    pub fn is_usable(&self) -> bool {
        self.anchors.len() >= 2
    }

    /// The box around every anchor and handle, in canvas coordinates.
    pub fn bounds(&self) -> Option<Rect> {
        self.anchors
            .iter()
            .flat_map(|a| [Some(a.point), a.handle_in, a.handle_out])
            .flatten()
            .map(|p| Rect::from_min_max(p, p))
            .reduce(|a, b| a.union(b))
    }

    /// The path as a layer's data: its box on the canvas and the anchors
    /// relative to it. `None` with fewer than two anchors.
    pub fn path_data(&self, closed: bool) -> Option<(Rect, PathData)> {
        if !self.is_usable() {
            return None;
        }
        let bounds = self.bounds()?;
        // A straight horizontal or vertical line has no height or width;
        // give it one pixel so the relative coordinates stay finite.
        let size = bounds.size().max(Vec2::splat(1.0));
        let relative = |p: Pos2| {
            let r = (p - bounds.min) / size;
            [tidy_fraction(r.x), tidy_fraction(r.y)]
        };
        let anchors = self
            .anchors
            .iter()
            .map(|a| moshionly_core::model::Anchor {
                point: relative(a.point),
                handle_in: a.handle_in.map(relative),
                handle_out: a.handle_out.map(relative),
            })
            .collect();
        Some((
            Rect::from_min_size(bounds.min, size),
            PathData {
                subpaths: vec![Subpath { anchors, closed }],
                fill_rule: FillRule::default(),
            },
        ))
    }

    /// What to draw while the path is being drawn, with the pointer at
    /// `pointer`.
    pub fn overlay(&self, pointer: Option<Pos2>, zoom: f32) -> PenOverlay {
        let mut path = Vec::new();
        for pair in self.anchors.windows(2) {
            flatten(&pair[0], &pair[1], &mut path);
        }
        if let (Some(last), Some(pointer)) = (self.anchors.last(), pointer) {
            let to = PenAnchor::corner(pointer);
            if path.is_empty() {
                path.push(last.point);
            }
            flatten(last, &to, &mut path);
        }
        let handles = self
            .anchors
            .iter()
            .flat_map(|a| {
                [a.handle_in, a.handle_out]
                    .into_iter()
                    .flatten()
                    .map(move |h| [a.point, h])
            })
            .collect();
        PenOverlay {
            path,
            anchors: self.anchors.iter().map(|a| a.point).collect(),
            handles,
            closing: pointer.is_some_and(|p| self.closes_at(p, zoom)),
        }
    }
}

/// Keeps relative coordinates short in files.
fn tidy_fraction(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

/// Adds the points of the curve from `a` to `b` to `out`, without `a`
/// when `out` already ends there.
fn flatten(a: &PenAnchor, b: &PenAnchor, out: &mut Vec<Pos2>) {
    const STEPS: usize = 16;
    let p0 = a.point;
    let p1 = a.handle_out.unwrap_or(a.point);
    let p2 = b.handle_in.unwrap_or(b.point);
    let p3 = b.point;
    if out.last() != Some(&p0) {
        out.push(p0);
    }
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let u = 1.0 - t;
        let point = p0.to_vec2() * (u * u * u)
            + p1.to_vec2() * (3.0 * u * u * t)
            + p2.to_vec2() * (3.0 * u * t * t)
            + p3.to_vec2() * (t * t * t);
        out.push(pos2(point.x, point.y));
    }
}

impl Editor {
    /// Turns a finished pen path into a `path` layer with a dark outline
    /// and no fill, selects it, and returns it. One undo step.
    pub fn finish_pen(&mut self, pen: &Pen, closed: bool) -> Result<Option<NodeId>, CreateError> {
        let Some((bounds, path)) = pen.path_data(closed) else {
            return Ok(None);
        };
        let origin = self
            .artboard_rect(&pen.artboard)
            .ok_or(CreateError::NoArtboard)?
            .min;
        let names: Vec<&str> = self
            .document
            .nodes_unordered()
            .filter(|node| node.node_type() == moshionly_core::NodeType::Path)
            .map(|node| node.name())
            .collect();
        let local = bounds.translate(-origin.to_vec2());
        let layer = PathLayer {
            layer: LayerProps::new(
                next_name("Path", names),
                tidy(f64::from(local.min.x)),
                tidy(f64::from(local.min.y)),
                tidy(f64::from(local.width())),
                tidy(f64::from(local.height())),
            ),
            fill: None,
            stroke: Some(Stroke::solid(Color::rgb(0x1E, 0x1E, 0x1E), STROKE_WIDTH)),
            path,
        };
        let (id, edits) = build::add_layer(&self.document, &pen.artboard, layer)?;
        self.apply("Add path", &edits)?;
        self.selection.select(id.clone());
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use moshionly_core::NodeType;
    use moshionly_core::model::NodeData;

    use super::super::view::tests::editor;
    use super::*;

    #[test]
    fn clicks_make_corners_and_the_first_anchor_closes() {
        let mut pen = Pen::new(NodeId::from("ab"));
        assert_eq!(pen.press(pos2(100.0, 100.0), 1.0), PenPress::Added);
        // One anchor cannot close yet.
        assert_eq!(pen.press(pos2(102.0, 100.0), 1.0), PenPress::Added);
        assert_eq!(pen.press(pos2(200.0, 200.0), 1.0), PenPress::Added);
        assert_eq!(pen.press(pos2(103.0, 103.0), 1.0), PenPress::Closed);
        assert_eq!(pen.anchors.len(), 3);
        // Zoomed out, the same distance on the canvas is closer on screen.
        assert!(pen.closes_at(pos2(115.0, 100.0), 0.5));
        assert!(!pen.closes_at(pos2(115.0, 100.0), 1.0));
    }

    #[test]
    fn a_drag_pulls_mirrored_handles() {
        let mut pen = Pen::new(NodeId::from("ab"));
        pen.press(pos2(100.0, 100.0), 1.0);
        pen.pull_handles(pos2(130.0, 90.0));
        let anchor = pen.anchors[0];
        assert_eq!(anchor.handle_out, Some(pos2(130.0, 90.0)));
        assert_eq!(anchor.handle_in, Some(pos2(70.0, 110.0)));
        // Back on the anchor, the handles go.
        pen.pull_handles(pos2(100.0, 100.0));
        assert_eq!(pen.anchors[0].handle_out, None);
    }

    #[test]
    fn the_path_is_stored_relative_to_its_box() {
        let mut pen = Pen::new(NodeId::from("ab"));
        assert!(pen.path_data(false).is_none());
        pen.press(pos2(100.0, 100.0), 1.0);
        pen.press(pos2(300.0, 100.0), 1.0);
        pen.press(pos2(300.0, 200.0), 1.0);
        let (bounds, path) = pen.path_data(true).unwrap();
        assert_eq!(
            bounds,
            Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0))
        );
        let points: Vec<[f64; 2]> = path.subpaths[0].anchors.iter().map(|a| a.point).collect();
        assert_eq!(points, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
        assert!(path.subpaths[0].closed);
        // A flat line still gets a box one pixel high.
        let mut flat = Pen::new(NodeId::from("ab"));
        flat.press(pos2(0.0, 50.0), 1.0);
        flat.press(pos2(80.0, 50.0), 1.0);
        let (bounds, _) = flat.path_data(false).unwrap();
        assert_eq!(bounds.size(), vec2(80.0, 1.0));
    }

    #[test]
    fn the_overlay_follows_the_pointer() {
        let mut pen = Pen::new(NodeId::from("ab"));
        pen.press(pos2(0.0, 0.0), 1.0);
        pen.press(pos2(100.0, 0.0), 1.0);
        pen.pull_handles(pos2(120.0, 20.0));
        let overlay = pen.overlay(Some(pos2(100.0, 100.0)), 1.0);
        assert_eq!(overlay.anchors.len(), 2);
        assert_eq!(overlay.handles.len(), 2);
        assert_eq!(overlay.path.first(), Some(&pos2(0.0, 0.0)));
        assert_eq!(overlay.path.last(), Some(&pos2(100.0, 100.0)));
        assert!(!overlay.closing);
        assert!(pen.overlay(Some(pos2(2.0, 2.0)), 1.0).closing);
    }

    #[test]
    fn a_finished_path_is_a_layer_with_an_outline() {
        let mut editor = editor();
        let mut pen = Pen::new(NodeId::from("ab"));
        pen.press(pos2(100.0, 500.0), 1.0);
        pen.press(pos2(300.0, 600.0), 1.0);
        let id = editor.finish_pen(&pen, false).unwrap().unwrap();
        let node = editor.document().get(&id).unwrap();
        assert_eq!(node.node_type(), NodeType::Path);
        assert_eq!(node.name(), "Path 1");
        let NodeData::Path(path) = &node.data else {
            panic!("a path");
        };
        assert_eq!(
            (
                path.layer.x,
                path.layer.y,
                path.layer.width,
                path.layer.height
            ),
            (100.0, 500.0, 200.0, 100.0)
        );
        assert!(path.fill.is_none() && path.stroke.is_some());
        assert_eq!(editor.selection.ids(), [id]);
        // Too short a path is dropped without a word.
        let short = Pen::new(NodeId::from("ab"));
        assert_eq!(editor.finish_pen(&short, false), Ok(None));
    }
}
