//! Moving, resizing and rotating layers: from a pointer to new properties.
//!
//! Everything here is geometry on plain numbers. A drag starts by noting
//! each layer as an [`Item`]: its resting box and the transform of its
//! parent. Each step of the drag then turns the pointer's travel into new
//! `x`, `y`, `width`, `height` and `rotation` values, always computed from
//! the start, so that a drag never drifts.
//!
//! Positions are in world (canvas) coordinates, angles in degrees clockwise.

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use moshionly_core::build::instances_follow_mains;
use moshionly_core::model::{AutoSize, NodeData};
use moshionly_core::{Affine, Document, Edit, Family, Node, NodeId, NodeType, Point};
use serde_json::{Map, Value};

use super::inspect::{text_of, tidy};
use crate::viewport::Anchor;

/// The smallest width or height a resize leaves, in pixels.
pub const MIN_SIZE: f64 = 1.0;

/// A layer (or artboard) as a drag found it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The node.
    pub id: NodeId,
    /// Whether it is an artboard, which moves on the canvas and never turns.
    pub artboard: bool,
    /// The node's resting `x`, `y`, `width`, `height`, `rotation`, `scale`.
    pub x: f64,
    /// See `x`.
    pub y: f64,
    /// See `x`.
    pub width: f64,
    /// See `x`.
    pub height: f64,
    /// See `x`.
    pub rotation: f64,
    /// See `x`.
    pub scale: f64,
    /// Where the artboard's top-left corner is on the canvas.
    pub origin: Point,
    /// From the parent's coordinates to the artboard's.
    pub parent: Affine,
    /// The text box rule, for text layers.
    pub auto_size: Option<AutoSize>,
    /// Whether resizing it scales what is inside it (see
    /// [`scales_contents`]).
    pub scales_contents: bool,
}

/// Whether resizing `node` scales what is inside it. A group that does not
/// clip behaves like a Figma group: its contents stretch with it. A group
/// that clips behaves like a Figma frame: its box changes and its contents
/// stay as they are. A mask group stretches like a group, and a main
/// component like the group it was made from. An instance holds no layers:
/// its box changes, and the engine scales what it shows to fill it.
pub fn scales_contents(node: &Node) -> bool {
    match &node.data {
        NodeData::Group(group) => !group.clip,
        NodeData::Component(component) => !component.clip,
        NodeData::MaskGroup(_) => true,
        _ => false,
    }
}

/// The edits that stretch everything inside `parent` by `sx` across and
/// `sy` down its own axes, as its box is stretched: positions and sizes,
/// all the way down, while font sizes and stroke widths stay. A layer that
/// is turned is stretched along its own axes as nearly as a box can be.
/// Text whose box hugs its words keeps its size, which the words decide,
/// and moves with its centre; settle it first so that centre is the one
/// drawn.
pub fn scale_contents(document: &Document, parent: &NodeId, sx: f64, sy: f64) -> Vec<Edit> {
    let mut edits = Vec::new();
    for child in document.children(parent, Family::Layer) {
        let Some(node) = document.get(child) else {
            continue;
        };
        let Some(layer) = node.layer() else {
            continue;
        };
        let center = layer.center();
        let center = Point::new(center.x * sx, center.y * sy);
        let (sin, cos) = layer.rotation.to_radians().sin_cos();
        let along = (sx * cos).hypot(sy * sin);
        let across = (sx * sin).hypot(sy * cos);
        let hugs = text_of(node).is_some_and(|text| text.auto_size == AutoSize::WidthAndHeight);
        let (width, height) = if hugs {
            (layer.width, layer.height)
        } else {
            (
                (layer.width * along).max(MIN_SIZE),
                (layer.height * across).max(MIN_SIZE),
            )
        };
        edits.push(Edit::update(
            child.clone(),
            [
                ("x", Value::from(tidy(center.x - width / 2.0))),
                ("y", Value::from(tidy(center.y - height / 2.0))),
                ("width", Value::from(tidy(width))),
                ("height", Value::from(tidy(height))),
            ],
        ));
        if document.children(child, Family::Layer).is_empty() {
            continue;
        }
        edits.extend(scale_contents(document, child, along, across));
    }
    edits
}

impl Item {
    /// Notes the node `id` of `document` as it is now.
    pub fn of(document: &Document, id: &NodeId) -> Option<Self> {
        let node = document.get(id)?;
        let layer = node.layer()?;
        let artboard = node.node_type() == NodeType::Artboard;
        let (origin, parent) = if artboard {
            (Point::ZERO, Affine::IDENTITY)
        } else {
            let board = document.artboard_of(id)?.layer()?;
            let parent = node.parent.as_ref()?;
            (
                Point::new(board.x, board.y),
                document.world_transform(parent)?,
            )
        };
        Some(Self {
            id: id.clone(),
            artboard,
            x: layer.x,
            y: layer.y,
            width: layer.width,
            height: layer.height,
            rotation: layer.rotation,
            scale: layer.scale,
            origin,
            parent,
            auto_size: text_of(node).map(|text| text.auto_size),
            scales_contents: scales_contents(node),
        })
    }

    /// The parent's uniform scale and its rotation in degrees.
    fn parent_scale_rotation(&self) -> (f64, f64) {
        let [a, b, ..] = self.parent.0;
        (a.hypot(b), b.atan2(a).to_degrees())
    }

    /// How much larger the layer looks than its `width` and `height`.
    pub fn world_scale(&self) -> f64 {
        self.parent_scale_rotation().0 * self.scale
    }

    /// The layer's rotation on the canvas: its own and its parents'.
    pub fn world_rotation(&self) -> f64 {
        self.parent_scale_rotation().1 + self.rotation
    }

    /// The centre of the box on the canvas.
    pub fn world_center(&self) -> Point {
        let local = Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0);
        let at = self.parent.apply(local);
        Point::new(at.x + self.origin.x, at.y + self.origin.y)
    }

    /// The box on the canvas: centre, size as it looks, and rotation.
    pub fn world_box(&self) -> WorldBox {
        let scale = self.world_scale();
        WorldBox {
            center: self.world_center(),
            size: (self.width * scale, self.height * scale),
            rotation: self.world_rotation(),
        }
    }

    /// The `x` and `y` that put the centre of a `width` by `height` box at
    /// `center` on the canvas.
    fn position_for(&self, center: Point, width: f64, height: f64) -> (f64, f64) {
        let in_artboard = Point::new(center.x - self.origin.x, center.y - self.origin.y);
        let local = self
            .parent
            .invert()
            .map_or(in_artboard, |inverse| inverse.apply(in_artboard));
        (local.x - width / 2.0, local.y - height / 2.0)
    }

    /// The edit that moves the layer by `delta` on the canvas.
    pub fn moved(&self, delta: Vec2) -> Edit {
        let (dx, dy) = (f64::from(delta.x), f64::from(delta.y));
        let local = if self.artboard {
            Point::new(dx, dy)
        } else {
            self.parent.invert().map_or(Point::new(dx, dy), |inverse| {
                inverse.apply_vector(Point::new(dx, dy))
            })
        };
        Edit::update(
            self.id.clone(),
            [
                ("x", Value::from(tidy(self.x + local.x))),
                ("y", Value::from(tidy(self.y + local.y))),
            ],
        )
    }

    /// The `width` and `height` that make the layer look `to` big.
    fn local_size(&self, to: &WorldBox) -> (f64, f64) {
        let scale = self.world_scale().max(1e-9);
        (
            (to.size.0 / scale).max(MIN_SIZE),
            (to.size.1 / scale).max(MIN_SIZE),
        )
    }

    /// The edits that give the layer the box `to` on the canvas, keeping
    /// its rotation: its own, and for a group that stretches its contents,
    /// theirs (see [`scales_contents`]).
    fn resized_to(
        &self,
        document: &Document,
        to: &WorldBox,
        changed_width: bool,
        changed_height: bool,
    ) -> Vec<Edit> {
        let mut edits = vec![self.placed(to, changed_width, changed_height)];
        if self.scales_contents {
            let (width, height) = self.local_size(to);
            let factor = |new: f64, old: f64| if old > 0.0 { new / old } else { 1.0 };
            edits.extend(scale_contents(
                document,
                &self.id,
                factor(width, self.width),
                factor(height, self.height),
            ));
        }
        edits
    }

    /// The edit that gives the layer the box `to` on the canvas, keeping its
    /// rotation. A text box that hugged its text stops doing so on the
    /// sides that were dragged.
    fn placed(&self, to: &WorldBox, changed_width: bool, changed_height: bool) -> Edit {
        let (width, height) = self.local_size(to);
        let (x, y) = self.position_for(to.center, width, height);
        let mut patch = Map::new();
        patch.insert("x".into(), tidy(x).into());
        patch.insert("y".into(), tidy(y).into());
        patch.insert("width".into(), tidy(width).into());
        patch.insert("height".into(), tidy(height).into());
        if let Some(auto) = self.auto_size {
            let next = match auto {
                AutoSize::WidthAndHeight if changed_height => Some("fixed"),
                AutoSize::WidthAndHeight if changed_width => Some("height"),
                AutoSize::Height if changed_height => Some("fixed"),
                _ => None,
            };
            if let Some(next) = next {
                patch.insert("auto_size".into(), next.into());
            }
        }
        Edit::update(self.id.clone(), patch)
    }
}

/// A box on the canvas: its centre, its size as it looks, and its rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBox {
    /// The centre.
    pub center: Point,
    /// Width and height.
    pub size: (f64, f64),
    /// Rotation in degrees, clockwise.
    pub rotation: f64,
}

impl WorldBox {
    /// An upright box covering `rect`.
    pub fn from_rect(rect: Rect) -> Self {
        Self {
            center: Point::new(f64::from(rect.center().x), f64::from(rect.center().y)),
            size: (f64::from(rect.width()), f64::from(rect.height())),
            rotation: 0.0,
        }
    }

    /// The upright rectangle around the box.
    pub fn bounds(&self) -> Rect {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let (hw, hh) = (self.size.0 / 2.0, self.size.1 / 2.0);
        let ex = (hw * cos).abs() + (hh * sin).abs();
        let ey = (hw * sin).abs() + (hh * cos).abs();
        Rect::from_min_max(
            pos2((self.center.x - ex) as f32, (self.center.y - ey) as f32),
            pos2((self.center.x + ex) as f32, (self.center.y + ey) as f32),
        )
    }

    /// The point at `unit` (each part -1, 0 or 1) on the box.
    pub fn point(&self, unit: Vec2) -> Point {
        let local = (
            f64::from(unit.x) * self.size.0 / 2.0,
            f64::from(unit.y) * self.size.1 / 2.0,
        );
        let turned = rotate(local, self.rotation);
        Point::new(self.center.x + turned.0, self.center.y + turned.1)
    }
}

/// Turns a vector by `degrees` clockwise (y grows downward).
fn rotate((x, y): (f64, f64), degrees: f64) -> (f64, f64) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    (x * cos - y * sin, x * sin + y * cos)
}

/// How a resize treats the box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeMode {
    /// Keep the width-to-height ratio (Shift).
    pub keep_proportions: bool,
    /// Resize about the centre instead of the opposite side (Alt).
    pub from_center: bool,
}

/// The box after dragging the `anchor` handle of `start` by `travel` (the
/// pointer's movement on the canvas since the drag began). Also says
/// whether the width and the height changed.
pub fn resized(
    start: &WorldBox,
    anchor: Anchor,
    travel: Vec2,
    mode: ResizeMode,
) -> (WorldBox, bool, bool) {
    let unit = anchor.unit();
    let (ux, uy) = (f64::from(unit.x), f64::from(unit.y));
    let (w0, h0) = start.size;
    let (hw, hh) = (w0 / 2.0, h0 / 2.0);
    // The pointer's travel along the box's own axes.
    let (dx, dy) = rotate((f64::from(travel.x), f64::from(travel.y)), -start.rotation);
    // The handle's new place, relative to the centre, in the box's axes.
    let handle = (ux * hw + dx, uy * hh + dy);
    // One side of each axis that moves: [low, high] relative to the centre.
    let axis = |u: f64, half: f64, to: f64| -> (f64, f64) {
        if u == 0.0 {
            (-half, half)
        } else if mode.from_center {
            let reach = to.abs().max(MIN_SIZE / 2.0);
            (-reach, reach)
        } else {
            let fixed = -u * half;
            let to = if (to - fixed).abs() < MIN_SIZE {
                fixed + u * MIN_SIZE
            } else {
                to
            };
            (fixed.min(to), fixed.max(to))
        }
    };
    let (mut x0, mut x1) = axis(ux, hw, handle.0);
    let (mut y0, mut y1) = axis(uy, hh, handle.1);

    if mode.keep_proportions && w0 > 0.0 && h0 > 0.0 {
        let kx = (x1 - x0) / w0;
        let ky = (y1 - y0) / h0;
        let k = match (ux != 0.0, uy != 0.0) {
            (true, true) => kx.max(ky),
            (true, false) => kx,
            (false, true) => ky,
            (false, false) => 1.0,
        };
        let (w, h) = (w0 * k, h0 * k);
        // Corners grow away from the fixed corner; edges grow the other
        // side evenly about the centre line.
        // One end of a moving axis is the fixed side; the box keeps growing
        // away from it, on whichever side the pointer is.
        let place = |u: f64, low: f64, length: f64, half: f64| -> (f64, f64) {
            if u == 0.0 || mode.from_center {
                (-length / 2.0, length / 2.0)
            } else {
                let fixed = -u * half;
                if (low - fixed).abs() < 1e-9 {
                    (fixed, fixed + length)
                } else {
                    (fixed - length, fixed)
                }
            }
        };
        (x0, x1) = place(ux, x0, w, hw);
        (y0, y1) = place(uy, y0, h, hh);
    }

    let middle = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let offset = rotate(middle, start.rotation);
    let size = (x1 - x0, y1 - y0);
    let changed = |a: f64, b: f64| (a - b).abs() > 1e-6;
    (
        WorldBox {
            center: Point::new(start.center.x + offset.0, start.center.y + offset.1),
            size,
            rotation: start.rotation,
        },
        changed(size.0, w0),
        changed(size.1, h0),
    )
}

/// The edits that resize `items` of `document` from their box `start` to
/// `to`. A single layer takes the box as it is; several are stretched with
/// it, each about its own axes. A group that does not clip stretches its
/// contents with it (see [`scales_contents`]), and the instances of a main
/// it resizes follow it (see [`instances_follow_mains`]).
pub fn resize_edits(
    document: &Document,
    items: &[Item],
    start: &WorldBox,
    to: &WorldBox,
    changed: (bool, bool),
) -> Vec<Edit> {
    let mut edits = boxes_resized(document, items, start, to, changed);
    let follow = instances_follow_mains(document, &edits);
    edits.extend(follow);
    edits
}

/// The edits that give `items` their part of the box `to`, and stretch
/// the contents of those that stretch them.
fn boxes_resized(
    document: &Document,
    items: &[Item],
    start: &WorldBox,
    to: &WorldBox,
    changed: (bool, bool),
) -> Vec<Edit> {
    let [only] = items else {
        let (sx, sy) = (
            to.size.0 / start.size.0.max(1e-9),
            to.size.1 / start.size.1.max(1e-9),
        );
        let (from, into) = (start.bounds(), to.bounds());
        return items
            .iter()
            .flat_map(|item| {
                let own = item.world_box();
                let c = own.center;
                let center = Point::new(
                    f64::from(into.min.x) + (c.x - f64::from(from.min.x)) * sx,
                    f64::from(into.min.y) + (c.y - f64::from(from.min.y)) * sy,
                );
                let (sin, cos) = own.rotation.to_radians().sin_cos();
                let along = (sx * cos).hypot(sy * sin);
                let across = (sx * sin).hypot(sy * cos);
                let placed = WorldBox {
                    center,
                    size: (own.size.0 * along, own.size.1 * across),
                    rotation: own.rotation,
                };
                item.resized_to(document, &placed, changed.0, changed.1)
            })
            .collect();
    };
    only.resized_to(document, to, changed.0, changed.1)
}

/// The angle in degrees from `pivot` to `point`, clockwise from the x axis.
pub fn angle(pivot: Pos2, point: Pos2) -> f64 {
    let d = point - pivot;
    f64::from(d.y).atan2(f64::from(d.x)).to_degrees()
}

/// An angle brought into the range -180 to 180.
pub fn normal_angle(degrees: f64) -> f64 {
    let turned = degrees.rem_euclid(360.0);
    if turned > 180.0 {
        turned - 360.0
    } else {
        turned
    }
}

/// The edits that turn `items` by `delta` degrees about `pivot`. A single
/// layer turns about its own centre. Artboards do not turn.
pub fn rotate_edits(items: &[Item], pivot: Point, delta: f64) -> Vec<Edit> {
    let single = items.len() == 1;
    items
        .iter()
        .filter(|item| !item.artboard)
        .map(|item| {
            let rotation = tidy(normal_angle(item.rotation + delta));
            if single {
                return Edit::set(item.id.clone(), "rotation", rotation);
            }
            let c = item.world_center();
            let turned = rotate((c.x - pivot.x, c.y - pivot.y), delta);
            let center = Point::new(pivot.x + turned.0, pivot.y + turned.1);
            let (x, y) = item.position_for(center, item.width, item.height);
            Edit::update(
                item.id.clone(),
                [
                    ("x", Value::from(tidy(x))),
                    ("y", Value::from(tidy(y))),
                    ("rotation", Value::from(rotation)),
                ],
            )
        })
        .collect()
}

/// The turn after dragging the rotation handle: `delta` degrees, snapped
/// with Shift so that the first item's rotation on the canvas lands on a
/// multiple of 15°.
pub fn rotation_delta(raw: f64, start_rotation: f64, snap: bool) -> f64 {
    if !snap {
        return raw;
    }
    let target = ((start_rotation + raw) / 15.0).round() * 15.0;
    target - start_rotation
}

/// The rectangle around the boxes of `items`.
pub fn bounds_of(items: &[Item]) -> Option<Rect> {
    items
        .iter()
        .map(|item| item.world_box().bounds())
        .reduce(|a, b| a.union(b))
}

/// The box a resize or rotation of `items` works on: a single item's own
/// box, or the upright rectangle around several.
pub fn box_of(items: &[Item]) -> Option<WorldBox> {
    match items {
        [] => None,
        [only] => Some(only.world_box()),
        several => bounds_of(several).map(WorldBox::from_rect),
    }
}

/// A delta locked to the axis it mostly follows, for Shift while moving.
pub fn locked_to_axis(delta: Vec2) -> Vec2 {
    if delta.x.abs() >= delta.y.abs() {
        vec2(delta.x, 0.0)
    } else {
        vec2(0.0, delta.y)
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","x":1000,"y":500,"width":1000,"height":1000},
            {"id":"r","parent":"ab","index":"a0","type":"rect","x":100,"y":100,"width":200,"height":100},
            {"id":"g","parent":"ab","index":"a1","type":"group","x":400,"y":400,"width":200,"height":200,"rotation":90,"scale":2},
            {"id":"in","parent":"g","index":"a0","type":"rect","x":0,"y":0,"width":50,"height":20},
            {"id":"t","parent":"ab","index":"a2","type":"text","x":10,"y":10,"content":"Hi"}
            ]}"##,
        )
        .unwrap()
    }

    fn item(id: &str) -> Item {
        Item::of(&document(), &NodeId::from(id)).unwrap()
    }

    fn apply(edits: &[Edit]) -> Document {
        let mut doc = document();
        doc.apply(edits).unwrap();
        doc
    }

    fn props(doc: &Document, id: &str) -> (f64, f64, f64, f64, f64) {
        let l = doc.get(&NodeId::from(id)).unwrap().layer().unwrap();
        (l.x, l.y, l.width, l.height, l.rotation)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.02
    }

    #[test]
    fn items_know_their_place_on_the_canvas() {
        let r = item("r");
        assert_eq!(r.world_center(), Point::new(1200.0, 650.0));
        assert_eq!(r.world_box().size, (200.0, 100.0));
        let inner = item("in");
        // Inside a group turned a quarter and scaled twice.
        assert!(close(inner.world_scale(), 2.0));
        assert!(close(inner.world_rotation(), 90.0));
        let artboard = item("ab");
        assert!(artboard.artboard);
        assert_eq!(artboard.world_center(), Point::new(1500.0, 1000.0));
    }

    #[test]
    fn moving_follows_the_pointer_through_turned_parents() {
        let doc = apply(&[item("r").moved(vec2(30.0, -10.0))]);
        assert_eq!(props(&doc, "r").0, 130.0);
        assert_eq!(props(&doc, "r").1, 90.0);
        // In the turned, doubled group, 30 to the right on the canvas is
        // 15 up the group's own axes (its x axis points down the canvas).
        let doc = apply(&[item("in").moved(vec2(30.0, 0.0))]);
        let (x, y, ..) = props(&doc, "in");
        assert!(close(x, 0.0) && close(y, -15.0), "{x}, {y}");
        // Artboards move on the canvas itself.
        let doc = apply(&[item("ab").moved(vec2(5.0, 5.0))]);
        assert_eq!((props(&doc, "ab").0, props(&doc, "ab").1), (1005.0, 505.0));
    }

    #[test]
    fn dragging_a_corner_keeps_the_opposite_corner() {
        let r = item("r");
        let start = r.world_box();
        let (to, w, h) = resized(
            &start,
            Anchor::BottomRight,
            vec2(50.0, 20.0),
            ResizeMode::default(),
        );
        assert!(w && h);
        let doc = apply(&resize_edits(&document(), &[r], &start, &to, (w, h)));
        assert_eq!(props(&doc, "r"), (100.0, 100.0, 250.0, 120.0, 0.0));
    }

    #[test]
    fn an_edge_changes_one_side() {
        let r = item("r");
        let start = r.world_box();
        let (to, w, h) = resized(
            &start,
            Anchor::Left,
            vec2(-40.0, 999.0),
            ResizeMode::default(),
        );
        assert!(w && !h);
        let doc = apply(&resize_edits(&document(), &[r], &start, &to, (w, h)));
        assert_eq!(props(&doc, "r"), (60.0, 100.0, 240.0, 100.0, 0.0));
    }

    #[test]
    fn shift_keeps_proportions_and_alt_resizes_from_the_centre() {
        let r = item("r");
        let start = r.world_box();
        let keep = ResizeMode {
            keep_proportions: true,
            from_center: false,
        };
        let (to, ..) = resized(&start, Anchor::BottomRight, vec2(200.0, 0.0), keep);
        assert_eq!(to.size, (400.0, 200.0));
        let doc = apply(&resize_edits(
            &document(),
            std::slice::from_ref(&r),
            &start,
            &to,
            (true, true),
        ));
        assert_eq!(props(&doc, "r"), (100.0, 100.0, 400.0, 200.0, 0.0));

        let centre = ResizeMode {
            keep_proportions: false,
            from_center: true,
        };
        let (to, ..) = resized(&start, Anchor::Right, vec2(50.0, 0.0), centre);
        let doc = apply(&resize_edits(
            &document(),
            std::slice::from_ref(&r),
            &start,
            &to,
            (true, false),
        ));
        assert_eq!(props(&doc, "r"), (50.0, 100.0, 300.0, 100.0, 0.0));

        // An edge with Shift grows the other side evenly.
        let (to, ..) = resized(&start, Anchor::Right, vec2(200.0, 0.0), keep);
        let doc = apply(&resize_edits(&document(), &[r], &start, &to, (true, true)));
        assert_eq!(props(&doc, "r"), (100.0, 50.0, 400.0, 200.0, 0.0));
    }

    #[test]
    fn dragging_past_the_other_side_flips_the_box_over() {
        let r = item("r");
        let start = r.world_box();
        let (to, ..) = resized(
            &start,
            Anchor::Right,
            vec2(-260.0, 0.0),
            ResizeMode::default(),
        );
        let doc = apply(&resize_edits(&document(), &[r], &start, &to, (true, false)));
        assert_eq!(props(&doc, "r"), (40.0, 100.0, 60.0, 100.0, 0.0));
    }

    #[test]
    fn resizing_a_turned_layer_works_along_its_own_axes() {
        let inner = item("in");
        let start = inner.world_box();
        // The inner rect is turned a quarter: its width runs down the
        // canvas. Its "right" handle moves down when dragged down.
        let (to, w, h) = resized(
            &start,
            Anchor::Right,
            vec2(0.0, 40.0),
            ResizeMode::default(),
        );
        assert!(w && !h);
        let doc = apply(&resize_edits(&document(), &[inner], &start, &to, (w, h)));
        let (x, y, width, height, _) = props(&doc, "in");
        // 40 on the canvas is 20 in the doubled group.
        assert!(
            close(width, 70.0) && close(height, 20.0),
            "{width} {height}"
        );
        assert!(
            close(x, 0.0) && close(y, 0.0),
            "the left end stays: {x} {y}"
        );
    }

    #[test]
    fn several_layers_stretch_with_their_box() {
        let doc = document();
        let items: Vec<Item> = ["r", "t"]
            .iter()
            .map(|id| Item::of(&doc, &NodeId::from(*id)).unwrap())
            .collect();
        let start = box_of(&items).unwrap();
        let (to, w, h) = resized(
            &start,
            Anchor::BottomRight,
            vec2(290.0, 0.0),
            ResizeMode::default(),
        );
        let doc = apply(&resize_edits(&document(), &items, &start, &to, (w, h)));
        let (x, _, width, ..) = props(&doc, "r");
        assert!(close(width, 400.0), "{width}");
        assert!(close(x, 190.0), "{x}");
    }

    #[test]
    fn resizing_text_fixes_the_sides_that_were_dragged() {
        let t = item("t");
        let start = t.world_box();
        let (to, w, h) = resized(
            &start,
            Anchor::Right,
            vec2(50.0, 0.0),
            ResizeMode::default(),
        );
        let doc = apply(&resize_edits(
            &document(),
            std::slice::from_ref(&t),
            &start,
            &to,
            (w, h),
        ));
        let text = text_of(doc.get(&NodeId::from("t")).unwrap()).unwrap();
        assert_eq!(text.auto_size, AutoSize::Height);
        let (to, w, h) = resized(
            &start,
            Anchor::BottomRight,
            vec2(50.0, 50.0),
            ResizeMode::default(),
        );
        let doc = apply(&resize_edits(&document(), &[t], &start, &to, (w, h)));
        let text = text_of(doc.get(&NodeId::from("t")).unwrap()).unwrap();
        assert_eq!(text.auto_size, AutoSize::Fixed);
    }

    /// A group that does not clip, holding a rect, a turned rect and a
    /// nested group with a rect; and a group that clips, holding a rect.
    fn groups() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":1000,"height":1000},
            {"id":"g","parent":"ab","index":"a0","type":"group","x":100,"y":100,"width":200,"height":100},
            {"id":"a","parent":"g","index":"a0","type":"rect","x":0,"y":0,"width":100,"height":50,"stroke":{"paint":"#000000","width":3}},
            {"id":"turned","parent":"g","index":"a1","type":"rect","x":120,"y":40,"width":40,"height":20,"rotation":90},
            {"id":"inner","parent":"g","index":"a2","type":"group","x":100,"y":50,"width":100,"height":50},
            {"id":"deep","parent":"inner","index":"a0","type":"ellipse","x":50,"y":0,"width":50,"height":50},
            {"id":"frame","parent":"ab","index":"a1","type":"group","x":500,"y":100,"width":200,"height":100,"clip":true},
            {"id":"kept","parent":"frame","index":"a0","type":"rect","x":10,"y":10,"width":100,"height":50}
            ]}"##,
        )
        .unwrap()
    }

    fn resize_by(doc: &Document, ids: &[&str], anchor: Anchor, travel: Vec2) -> Document {
        let items: Vec<Item> = ids
            .iter()
            .map(|id| Item::of(doc, &NodeId::from(*id)).unwrap())
            .collect();
        let start = box_of(&items).unwrap();
        let (to, w, h) = resized(&start, anchor, travel, ResizeMode::default());
        let mut after = doc.clone();
        after
            .apply(&resize_edits(doc, &items, &start, &to, (w, h)))
            .unwrap();
        after
    }

    #[test]
    fn resizing_a_group_stretches_what_is_in_it() {
        let doc = groups();
        // Twice as wide and half again as high.
        let after = resize_by(&doc, &["g"], Anchor::BottomRight, vec2(200.0, 50.0));
        assert_eq!(props(&after, "g"), (100.0, 100.0, 400.0, 150.0, 0.0));
        assert_eq!(props(&after, "a"), (0.0, 0.0, 200.0, 75.0, 0.0));
        // The stroke keeps its width.
        let stroke = after
            .get(&NodeId::from("a"))
            .unwrap()
            .data
            .stroke()
            .cloned();
        assert_eq!(stroke.map(|s| s.width), Some(3.0));
        // A turned layer stretches along its own axes: its width runs down.
        let (x, y, width, height, rotation) = props(&after, "turned");
        assert_eq!((width, height, rotation), (60.0, 40.0, 90.0));
        assert_eq!((x + width / 2.0, y + height / 2.0), (280.0, 75.0));
        // All the way down.
        assert_eq!(props(&after, "inner"), (200.0, 75.0, 200.0, 75.0, 0.0));
        assert_eq!(props(&after, "deep"), (100.0, 0.0, 100.0, 75.0, 0.0));
    }

    #[test]
    fn resizing_a_group_that_clips_leaves_what_is_in_it() {
        let doc = groups();
        let after = resize_by(&doc, &["frame"], Anchor::Right, vec2(100.0, 0.0));
        assert_eq!(props(&after, "frame"), (500.0, 100.0, 300.0, 100.0, 0.0));
        assert_eq!(props(&after, "kept"), props(&doc, "kept"));
    }

    #[test]
    fn groups_stretch_their_contents_in_a_selection_too() {
        let doc = groups();
        // Both groups, 100 to 700 wide: twice as wide is 1300.
        let after = resize_by(&doc, &["g", "frame"], Anchor::Right, vec2(600.0, 0.0));
        assert_eq!(props(&after, "g").2, 400.0);
        assert_eq!(props(&after, "a").2, 200.0);
        assert_eq!(props(&after, "frame").2, 400.0);
        assert_eq!(props(&after, "kept"), props(&doc, "kept"));
    }

    #[test]
    fn a_main_stretches_like_a_group_and_an_instance_only_changes_its_box() {
        let doc = io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":1000,"height":1000},
            {"id":"main","parent":"ab","index":"a0","type":"component","x":100,"y":100,"width":200,"height":100},
            {"id":"a","parent":"main","index":"a0","type":"rect","x":0,"y":0,"width":100,"height":50},
            {"id":"copy","parent":"ab","index":"a1","type":"instance","component":"main","x":400,"y":100,"width":200,"height":100}
            ]}"##,
        )
        .unwrap();
        let after = resize_by(&doc, &["main"], Anchor::Right, vec2(200.0, 0.0));
        assert_eq!(props(&after, "main").2, 400.0);
        assert_eq!(props(&after, "a").2, 200.0);
        // The instance's own box, which the engine fills with the main.
        let after = resize_by(&doc, &["copy"], Anchor::BottomRight, vec2(200.0, 100.0));
        assert_eq!(props(&after, "copy"), (400.0, 100.0, 400.0, 200.0, 0.0));
        assert_eq!(props(&after, "a"), props(&doc, "a"), "the main stays");
    }

    #[test]
    fn instances_the_size_of_their_main_follow_it() {
        // "same" is as large as the main; "wide" was made wider by hand;
        // "turned" is upright no more, and "inside" is in a group.
        let doc = io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":2000,"height":2000},
            {"id":"main","parent":"ab","index":"a0","type":"component","x":100,"y":100,"width":200,"height":100,"clip":true},
            {"id":"a","parent":"main","index":"a0","type":"rect","x":0,"y":0,"width":100,"height":50},
            {"id":"same","parent":"ab","index":"a1","type":"instance","component":"main","x":400,"y":100,"width":200,"height":100},
            {"id":"wide","parent":"ab","index":"a2","type":"instance","component":"main","x":700,"y":100,"width":300,"height":100},
            {"id":"turned","parent":"ab","index":"a3","type":"instance","component":"main","x":400,"y":600,"width":200,"height":100,"rotation":90},
            {"id":"bag","parent":"ab","index":"a4","type":"group","x":100,"y":1000,"width":200,"height":100},
            {"id":"inside","parent":"bag","index":"a0","type":"instance","component":"main","x":0,"y":0,"width":200,"height":100}
            ]}"##,
        )
        .unwrap();
        let after = resize_by(&doc, &["main"], Anchor::BottomRight, vec2(200.0, 50.0));
        assert_eq!(props(&after, "main"), (100.0, 100.0, 400.0, 150.0, 0.0));
        // Each side on its own, and the instances stay where they are.
        assert_eq!(props(&after, "same"), (400.0, 100.0, 400.0, 150.0, 0.0));
        assert_eq!(props(&after, "wide"), (700.0, 100.0, 300.0, 150.0, 0.0));
        assert_eq!(props(&after, "inside"), (0.0, 0.0, 400.0, 150.0, 0.0));
        // So none of them squeezes what it shows.
        let scene = moshionly_core::evaluate_at_rest(
            &after,
            &NodeId::from("ab"),
            &moshionly_core::MonospaceMeasurer::default(),
        )
        .unwrap();
        for id in ["same", "inside"] {
            let drawn = scene.find(&NodeId::from(id)).unwrap();
            assert!(
                !has_scale(drawn),
                "{id} shows the main at its own size: {drawn:?}"
            );
        }

        // A main stretched as part of a group carries its instances along.
        let doc_in_group = {
            let mut doc = doc.clone();
            doc.apply(&[
                Edit::set(NodeId::from("main"), "parent", "bag"),
                Edit::set(NodeId::from("main"), "x", 0.0),
                Edit::set(NodeId::from("main"), "y", 0.0),
                Edit::set(NodeId::from("main"), "index", "a1"),
            ])
            .unwrap();
            doc
        };
        let after = resize_by(&doc_in_group, &["bag"], Anchor::Right, vec2(200.0, 0.0));
        assert_eq!(props(&after, "main").2, 400.0);
        assert_eq!(props(&after, "same").2, 400.0);
        // One stretched in the same gesture keeps what the gesture gave it.
        assert_eq!(props(&after, "inside").2, 400.0);

        // Resized together, an instance takes its own new box, not the
        // main's: turned a quarter, a stretch across makes it taller.
        let after = resize_by(&doc, &["main", "turned"], Anchor::Right, vec2(450.0, 0.0));
        assert_eq!(props(&after, "main").2, 400.0);
        let (_, _, width, height, _) = props(&after, "turned");
        assert_eq!((width, height), (200.0, 200.0));
    }

    /// Whether anything in `node` is drawn stretched or squeezed.
    fn has_scale(node: &moshionly_core::SceneNode) -> bool {
        let [a, b, c, d, ..] = node.transform.0;
        (a.hypot(b) - 1.0).abs() > 1e-9
            || (c.hypot(d) - 1.0).abs() > 1e-9
            || node.children().iter().any(has_scale)
    }

    #[test]
    fn rotation_turns_about_the_centre_and_snaps_with_shift() {
        let r = item("r");
        let doc = apply(&rotate_edits(
            std::slice::from_ref(&r),
            r.world_center(),
            30.0,
        ));
        assert_eq!(props(&doc, "r").4, 30.0);
        assert_eq!(rotation_delta(22.0, 0.0, true), 15.0);
        assert_eq!(rotation_delta(23.0, 0.0, true), 30.0);
        assert_eq!(rotation_delta(10.0, 3.0, true), 12.0);
        assert_eq!(rotation_delta(10.0, 3.0, false), 10.0);
        assert_eq!(normal_angle(270.0), -90.0);
        let pivot = pos2(0.0, 0.0);
        assert!(close(angle(pivot, pos2(0.0, 10.0)), 90.0));
    }

    #[test]
    fn several_layers_turn_about_their_common_centre() {
        let doc = document();
        let items: Vec<Item> = ["r", "t"]
            .iter()
            .map(|id| Item::of(&doc, &NodeId::from(*id)).unwrap())
            .collect();
        let pivot = items[0].world_center();
        let doc = apply(&rotate_edits(&items, pivot, 180.0));
        // The rect turns in place: its centre is the pivot.
        let (x, y, _, _, rotation) = props(&doc, "r");
        assert!(close(x, 100.0) && close(y, 100.0));
        assert_eq!(rotation, 180.0);
        // Artboards never turn.
        assert!(rotate_edits(&[item("ab")], Point::ZERO, 45.0).is_empty());
    }

    #[test]
    fn shift_locks_a_move_to_one_axis() {
        assert_eq!(locked_to_axis(vec2(10.0, 3.0)), vec2(10.0, 0.0));
        assert_eq!(locked_to_axis(vec2(-2.0, -9.0)), vec2(0.0, -9.0));
    }
}
