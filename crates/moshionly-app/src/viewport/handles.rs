//! Boxes on the canvas and the handles around a selection.
//!
//! Everything here is plain geometry, so it can be tested without a window:
//! [`OrientedBox`] is a rectangle that may be rotated, [`Handle`] names the
//! places a selection can be grabbed, and [`hit_handle`] finds the handle
//! under the pointer.

use egui::{CursorIcon, Pos2, Rect, Vec2};

use super::camera::Camera;

/// A rectangle that may be rotated about its centre: the box of a layer or
/// of a whole selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrientedBox {
    /// The centre of the box.
    pub center: Pos2,
    /// Width and height before rotation.
    pub size: Vec2,
    /// Rotation about the centre in degrees, clockwise positive, as in a
    /// document.
    pub rotation: f32,
}

impl OrientedBox {
    /// An unrotated box.
    pub fn from_rect(rect: Rect) -> Self {
        Self {
            center: rect.center(),
            size: rect.size(),
            rotation: 0.0,
        }
    }

    /// A box from its top-left corner and size before rotation, turned about
    /// its centre: the way a layer is stored.
    pub fn new(rect: Rect, rotation: f32) -> Self {
        Self {
            rotation,
            ..Self::from_rect(rect)
        }
    }

    /// Turns an offset from the centre in the box's own axes into an offset
    /// in the space the box lives in.
    fn rotate(&self, local: Vec2) -> Vec2 {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos)
    }

    /// The offset of a point from the centre, measured along the box's own
    /// axes: the inverse of the box's rotation.
    pub fn to_local(&self, point: Pos2) -> Vec2 {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let d = point - self.center;
        Vec2::new(d.x * cos + d.y * sin, -d.x * sin + d.y * cos)
    }

    /// The point of the box at `unit`, where each component runs from -1
    /// (left or top) to 1 (right or bottom) in the box's own axes.
    pub fn point(&self, unit: Vec2) -> Pos2 {
        self.center + self.rotate(unit * self.size / 2.0)
    }

    /// The corners, clockwise from the top-left one (before rotation).
    pub fn corners(&self) -> [Pos2; 4] {
        [
            self.point(Vec2::new(-1.0, -1.0)),
            self.point(Vec2::new(1.0, -1.0)),
            self.point(Vec2::new(1.0, 1.0)),
            self.point(Vec2::new(-1.0, 1.0)),
        ]
    }

    /// The upright rectangle that encloses the box.
    pub fn bounds(&self) -> Rect {
        Rect::from_points(&self.corners())
    }

    /// Whether `point` is inside the box.
    pub fn contains(&self, point: Pos2) -> bool {
        let local = self.to_local(point);
        local.x.abs() <= self.size.x / 2.0 && local.y.abs() <= self.size.y / 2.0
    }

    /// The same box in screen coordinates.
    pub fn to_screen(&self, camera: &Camera, viewport: Rect) -> Self {
        Self {
            center: camera.world_to_screen(viewport, self.center),
            size: self.size * camera.zoom,
            rotation: self.rotation,
        }
    }
}

/// A place on a box: its corners and the middles of its edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// The top-left corner.
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top-right corner.
    TopRight,
    /// The middle of the right edge.
    Right,
    /// The bottom-right corner.
    BottomRight,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom-left corner.
    BottomLeft,
    /// The middle of the left edge.
    Left,
}

impl Anchor {
    /// All eight, clockwise from the top-left corner.
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];

    /// The four corners, clockwise from the top-left one.
    pub const CORNERS: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomRight,
        Self::BottomLeft,
    ];

    /// The four edges, clockwise from the top one.
    pub const EDGES: [Self; 4] = [Self::Top, Self::Right, Self::Bottom, Self::Left];

    /// The anchor's place on a box, each component -1, 0 or 1 (see
    /// [`OrientedBox::point`]).
    pub fn unit(self) -> Vec2 {
        match self {
            Self::TopLeft => Vec2::new(-1.0, -1.0),
            Self::Top => Vec2::new(0.0, -1.0),
            Self::TopRight => Vec2::new(1.0, -1.0),
            Self::Right => Vec2::new(1.0, 0.0),
            Self::BottomRight => Vec2::new(1.0, 1.0),
            Self::Bottom => Vec2::new(0.0, 1.0),
            Self::BottomLeft => Vec2::new(-1.0, 1.0),
            Self::Left => Vec2::new(-1.0, 0.0),
        }
    }

    /// Whether this is one of the four corners.
    pub fn is_corner(self) -> bool {
        let unit = self.unit();
        unit.x != 0.0 && unit.y != 0.0
    }

    /// The anchor on the other side of the box: the point that stays put
    /// while this one is dragged to resize.
    pub fn opposite(self) -> Self {
        match self {
            Self::TopLeft => Self::BottomRight,
            Self::Top => Self::Bottom,
            Self::TopRight => Self::BottomLeft,
            Self::Right => Self::Left,
            Self::BottomRight => Self::TopLeft,
            Self::Bottom => Self::Top,
            Self::BottomLeft => Self::TopRight,
            Self::Left => Self::Right,
        }
    }
}

/// A way to grab the selection box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handle {
    /// Resize by dragging a corner or an edge.
    Resize(Anchor),
    /// Rotate by dragging from just outside a corner.
    Rotate(Anchor),
}

/// The sizes that decide what counts as grabbing a handle, in screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandleMetrics {
    /// Side of a handle square.
    pub size: f32,
    /// How far outside a corner the rotation zone reaches.
    pub rotate_zone: f32,
}

impl HandleMetrics {
    /// How far from a handle's centre, or from an edge, the pointer still
    /// grabs it. A little more generous than what is drawn.
    pub fn grab(&self) -> f32 {
        self.size / 2.0 + 2.0
    }
}

/// The handle of `screen_box` under `pointer`, both in screen coordinates.
///
/// Corners win over edges, and both win over rotation. The whole length of
/// an edge resizes, not only its middle. The rotation zone is the ring just
/// outside each corner.
pub fn hit_handle(
    screen_box: &OrientedBox,
    pointer: Pos2,
    metrics: HandleMetrics,
) -> Option<Handle> {
    let local = screen_box.to_local(pointer);
    let half = screen_box.size / 2.0;
    let grab = metrics.grab();

    // Corners: the nearest one within reach.
    let corner = Anchor::CORNERS
        .into_iter()
        .map(|anchor| {
            let d = local - anchor.unit() * half;
            (anchor, d.x.abs().max(d.y.abs()))
        })
        .filter(|(_, distance)| *distance <= grab)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((anchor, _)) = corner {
        return Some(Handle::Resize(anchor));
    }

    // Edges: anywhere along them.
    let within_x = local.x.abs() <= half.x;
    let within_y = local.y.abs() <= half.y;
    for anchor in Anchor::EDGES {
        let unit = anchor.unit();
        let hit = if unit.x == 0.0 {
            within_x && (local.y - unit.y * half.y).abs() <= grab
        } else {
            within_y && (local.x - unit.x * half.x).abs() <= grab
        };
        if hit {
            return Some(Handle::Resize(anchor));
        }
    }

    // Rotation: outside the box, near a corner.
    if within_x && within_y {
        return None;
    }
    Anchor::CORNERS
        .into_iter()
        .map(|anchor| (anchor, (local - anchor.unit() * half).length()))
        .filter(|(_, distance)| *distance <= grab + metrics.rotate_zone)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(anchor, _)| Handle::Rotate(anchor))
}

/// The pointer shape for a handle of a box rotated by `rotation` degrees.
///
/// Resize arrows follow the box as it turns. There is no system pointer for
/// rotation, so that case returns `None` and the viewport draws its own.
pub fn handle_cursor(handle: Handle, rotation: f32) -> Option<CursorIcon> {
    let Handle::Resize(anchor) = handle else {
        return None;
    };
    let unit = anchor.unit();
    let angle = unit.y.atan2(unit.x).to_degrees() + rotation;
    // Arrows are the same turned half a circle; pick the nearest of four.
    let step = (angle / 45.0).round().rem_euclid(4.0) as i32;
    Some(match step {
        0 => CursorIcon::ResizeHorizontal,
        1 => CursorIcon::ResizeNwSe,
        2 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNeSw,
    })
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};

    use super::*;

    const METRICS: HandleMetrics = HandleMetrics {
        size: 7.0,
        rotate_zone: 14.0,
    };

    fn close(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    fn upright() -> OrientedBox {
        OrientedBox::from_rect(Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 100.0)))
    }

    #[test]
    fn an_upright_box_has_the_corners_of_its_rect() {
        let b = upright();
        let [tl, tr, br, bl] = b.corners();
        assert!(close(tl, pos2(100.0, 100.0)));
        assert!(close(tr, pos2(300.0, 100.0)));
        assert!(close(br, pos2(300.0, 200.0)));
        assert!(close(bl, pos2(100.0, 200.0)));
        assert_eq!(b.bounds().size(), vec2(200.0, 100.0));
    }

    #[test]
    fn rotation_is_clockwise_on_screen() {
        // A quarter turn clockwise sends the top-left corner to the
        // top-right of the (now upright, 100 by 200) shape.
        let b = OrientedBox {
            rotation: 90.0,
            ..upright()
        };
        let [tl, tr, ..] = b.corners();
        assert!(close(tl, pos2(250.0, 50.0)), "{tl:?}");
        assert!(close(tr, pos2(250.0, 250.0)), "{tr:?}");
        let bounds = b.bounds();
        assert!((bounds.width() - 100.0).abs() < 1e-3);
        assert!((bounds.height() - 200.0).abs() < 1e-3);
    }

    #[test]
    fn local_coordinates_undo_the_rotation() {
        let b = OrientedBox {
            rotation: 37.0,
            ..upright()
        };
        for anchor in Anchor::ALL {
            let local = b.to_local(b.point(anchor.unit()));
            let expected = anchor.unit() * b.size / 2.0;
            assert!((local - expected).length() < 1e-3, "{anchor:?}");
        }
        assert!(b.contains(b.center));
        assert!(b.contains(b.point(vec2(0.9, -0.9))));
        assert!(!b.contains(b.point(vec2(1.1, 0.0))));
        // The bounding rect's corner is outside a rotated box.
        assert!(!b.contains(b.bounds().min));
    }

    #[test]
    fn boxes_follow_the_camera() {
        let camera = Camera {
            center: pos2(200.0, 150.0),
            zoom: 2.0,
        };
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let screen = upright().to_screen(&camera, viewport);
        assert!(close(screen.center, pos2(400.0, 300.0)));
        assert_eq!(screen.size, vec2(400.0, 200.0));
    }

    #[test]
    fn corners_and_edges_resize() {
        let b = upright();
        for anchor in Anchor::ALL {
            assert_eq!(
                hit_handle(&b, b.point(anchor.unit()), METRICS),
                Some(Handle::Resize(anchor)),
                "{anchor:?}"
            );
        }
        // Anywhere along an edge, a few points either side of it.
        assert_eq!(
            hit_handle(&b, pos2(140.0, 102.0), METRICS),
            Some(Handle::Resize(Anchor::Top))
        );
        assert_eq!(
            hit_handle(&b, pos2(302.0, 180.0), METRICS),
            Some(Handle::Resize(Anchor::Right))
        );
        // Near a corner, the corner wins over the edge.
        assert_eq!(
            hit_handle(&b, pos2(103.0, 100.0), METRICS),
            Some(Handle::Resize(Anchor::TopLeft))
        );
    }

    #[test]
    fn the_inside_and_the_far_outside_are_not_handles() {
        let b = upright();
        assert_eq!(hit_handle(&b, b.center, METRICS), None);
        assert_eq!(hit_handle(&b, pos2(120.0, 120.0), METRICS), None);
        assert_eq!(hit_handle(&b, pos2(60.0, 60.0), METRICS), None);
        assert_eq!(hit_handle(&b, pos2(200.0, 80.0), METRICS), None);
    }

    #[test]
    fn just_outside_a_corner_rotates() {
        let b = upright();
        assert_eq!(
            hit_handle(&b, pos2(90.0, 90.0), METRICS),
            Some(Handle::Rotate(Anchor::TopLeft))
        );
        assert_eq!(
            hit_handle(&b, pos2(310.0, 212.0), METRICS),
            Some(Handle::Rotate(Anchor::BottomRight))
        );
        // Beside the corner but level with the edge is still outside.
        assert_eq!(
            hit_handle(&b, pos2(312.0, 104.0), METRICS),
            Some(Handle::Rotate(Anchor::TopRight))
        );
    }

    #[test]
    fn hit_testing_follows_a_rotated_box() {
        let b = OrientedBox {
            rotation: 30.0,
            ..upright()
        };
        for anchor in Anchor::ALL {
            assert_eq!(
                hit_handle(&b, b.point(anchor.unit()), METRICS),
                Some(Handle::Resize(anchor))
            );
        }
        // 12 points diagonally outside the bottom-right corner, along the
        // box's own diagonal.
        let outside = b.point(vec2(1.0, 1.0)) + b.rotate(vec2(8.5, 8.5));
        assert_eq!(
            hit_handle(&b, outside, METRICS),
            Some(Handle::Rotate(Anchor::BottomRight))
        );
    }

    #[test]
    fn a_tiny_box_still_offers_its_corners() {
        let b = OrientedBox::from_rect(Rect::from_min_size(pos2(50.0, 50.0), vec2(4.0, 4.0)));
        assert_eq!(
            hit_handle(&b, pos2(49.0, 49.0), METRICS),
            Some(Handle::Resize(Anchor::TopLeft))
        );
        assert_eq!(
            hit_handle(&b, pos2(55.0, 55.0), METRICS),
            Some(Handle::Resize(Anchor::BottomRight))
        );
    }

    #[test]
    fn resize_cursors_turn_with_the_box() {
        use CursorIcon::{ResizeHorizontal, ResizeNeSw, ResizeNwSe, ResizeVertical};
        let cursor = |anchor, rotation| handle_cursor(Handle::Resize(anchor), rotation);
        assert_eq!(cursor(Anchor::Right, 0.0), Some(ResizeHorizontal));
        assert_eq!(cursor(Anchor::Left, 0.0), Some(ResizeHorizontal));
        assert_eq!(cursor(Anchor::Top, 0.0), Some(ResizeVertical));
        assert_eq!(cursor(Anchor::BottomRight, 0.0), Some(ResizeNwSe));
        assert_eq!(cursor(Anchor::TopLeft, 0.0), Some(ResizeNwSe));
        assert_eq!(cursor(Anchor::TopRight, 0.0), Some(ResizeNeSw));
        // A quarter turn swaps horizontal and vertical, and the diagonals.
        assert_eq!(cursor(Anchor::Right, 90.0), Some(ResizeVertical));
        assert_eq!(cursor(Anchor::BottomRight, 90.0), Some(ResizeNeSw));
        // An eighth of a turn makes an edge handle diagonal.
        assert_eq!(cursor(Anchor::Right, 45.0), Some(ResizeNwSe));
        assert_eq!(cursor(Anchor::Right, -45.0), Some(ResizeNeSw));
        assert_eq!(cursor(Anchor::Right, 720.0 + 10.0), Some(ResizeHorizontal));
        assert_eq!(handle_cursor(Handle::Rotate(Anchor::TopLeft), 0.0), None);
    }

    #[test]
    fn anchors_have_opposites() {
        for anchor in Anchor::ALL {
            assert_eq!(anchor.opposite().unit(), -anchor.unit());
            assert_eq!(anchor.opposite().opposite(), anchor);
            assert_eq!(anchor.is_corner(), Anchor::CORNERS.contains(&anchor));
        }
    }
}
