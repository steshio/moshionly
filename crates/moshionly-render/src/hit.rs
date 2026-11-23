//! Questions about where things are in a frame: what is under a point, and
//! where a layer's box and ink are.
//!
//! The answers come from the same outlines the painter fills and strokes,
//! so a layer is found and measured exactly where it is drawn: a star
//! between its points is not hit, a ring not through its hole, an outline
//! only near its edge.

use moshionly_core::model::StrokeJoin;
use moshionly_core::scene::{Clip, Content, Scene, SceneNode, SceneStroke};
use moshionly_core::{Affine, NodeId, Point, Rect};
use skia_safe as sk;

use crate::convert;
use crate::draw::{effects, geometry, stroke};
use crate::fonts::FontLibrary;

/// How close, in artboard pixels, a point must come to a thin outline to
/// hit it, unless asked otherwise.
pub const HIT_TOLERANCE: f64 = 3.0;

/// Where a layer is in a frame, in artboard coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerBounds {
    /// The corners of the layer's box: its top-left, top-right,
    /// bottom-right and bottom-left before it was turned. This is where
    /// selection handles go.
    pub corners: [Point; 4],
    /// The box around everything the layer draws: strokes, shadow and blur
    /// included, before anything around it clips it. `None` when the layer
    /// draws nothing.
    pub visual: Option<Rect>,
}

/// Hit testing and bounds over one font library.
pub(crate) struct Geometry<'a> {
    pub fonts: &'a FontLibrary,
}

impl Geometry<'_> {
    /// The layers under `point`, frontmost first, each once. See
    /// [`Renderer::hit_test`](crate::Renderer::hit_test).
    pub(crate) fn hit_test(&self, scene: &Scene, point: Point, tolerance: f64) -> Vec<NodeId> {
        let artboard = Rect::new(0.0, 0.0, scene.width, scene.height);
        if !artboard.contains(point) {
            return Vec::new();
        }
        let tolerance = if tolerance.is_finite() {
            tolerance.max(0.0)
        } else {
            0.0
        };
        let mut hits = Vec::new();
        for node in scene.nodes.iter().rev() {
            self.collect(node, point, 1.0, tolerance, &mut hits);
        }
        let mut seen = std::collections::HashSet::new();
        hits.retain(|id| seen.insert(id.clone()));
        hits
    }

    /// Adds the layers of `node` under `point` (in its parent's
    /// coordinates) to `hits`, front to back. `scale` is how much the
    /// parent's coordinates are magnified on the artboard.
    fn collect(
        &self,
        node: &SceneNode,
        point: Point,
        scale: f64,
        tolerance: f64,
        hits: &mut Vec<NodeId>,
    ) {
        let Some(inverse) = usable(&node.transform).and_then(|transform| transform.invert()) else {
            return;
        };
        let local = inverse.apply(point);
        let [a, b, c, d, _, _] = node.transform.0;
        let scale = scale * (a * d - b * c).abs().sqrt();
        if node
            .clip
            .as_ref()
            .is_some_and(|clip| !clip_contains(clip, local))
        {
            return;
        }
        if let Some(mask) = &node.mask {
            let mut under = Vec::new();
            self.collect(mask, local, scale, 0.0, &mut under);
            if under.is_empty() {
                return;
            }
        }
        match &node.content {
            Content::Group { children } => {
                for child in children.iter().rev() {
                    self.collect(child, local, scale, tolerance, hits);
                }
            }
            _ => {
                if touches(node, local, tolerance / scale.max(1e-9)) {
                    hits.push(node.id.clone());
                }
            }
        }
    }

    /// Where the layer `id` is. See
    /// [`Renderer::bounds_of`](crate::Renderer::bounds_of).
    pub(crate) fn bounds_of(&self, scene: &Scene, id: &NodeId) -> Option<LayerBounds> {
        let node = scene.find(id)?;
        let to_artboard = usable(&scene.transform_of(id)?)?;
        Some(LayerBounds {
            corners: node
                .frame()
                .corners()
                .map(|corner| to_artboard.apply(corner)),
            visual: self
                .visual(node, Precision::Exact)
                .map(|local| to_artboard.bounds_of(&local)),
        })
    }

    /// The box around everything `node` draws, in its own coordinates, or
    /// `None` when it draws nothing.
    pub(crate) fn visual(&self, node: &SceneNode, precision: Precision) -> Option<Rect> {
        let content = self.content(node, precision)?;
        let [left, top, right, bottom] = effects::reach(node);
        let mut visual = Rect::new(
            content.x - left,
            content.y - top,
            content.width + left + right,
            content.height + top + bottom,
        );
        if let Some(clip) = &node.clip {
            visual = intersection(&visual, &clip.rect)?;
        }
        if let Some(mask) = &node.mask {
            let transform = usable(&mask.transform)?;
            visual = intersection(
                &visual,
                &transform.bounds_of(&self.visual(mask, precision)?),
            )?;
        }
        Some(visual)
    }

    /// The box around what `node`'s content draws, before effects.
    pub(crate) fn content(&self, node: &SceneNode, precision: Precision) -> Option<Rect> {
        match &node.content {
            Content::Group { children } => children
                .iter()
                .filter_map(|child| {
                    let transform = usable(&child.transform)?;
                    Some(transform.bounds_of(&self.visual(child, precision)?))
                })
                .reduce(|all, one| all.union(&one)),
            Content::Rect { fill, stroke, .. }
            | Content::Ellipse { fill, stroke, .. }
            | Content::Star { fill, stroke, .. }
            | Content::Path { fill, stroke, .. } => {
                let outline = geometry::outline(node)?;
                let shape = || match precision {
                    Precision::Exact => rect(&outline.path.compute_tight_bounds()),
                    Precision::Quick => rect(outline.path.bounds()),
                };
                let filled = fill.as_ref().map(|_| shape());
                let stroked = stroke.as_ref().and_then(|stroke| match precision {
                    Precision::Exact => stroke::area(&outline, stroke)
                        .map(|area| rect(&area.compute_tight_bounds())),
                    Precision::Quick => {
                        Some(outset(&shape(), stroke_reach(stroke, outline.closed)))
                    }
                });
                union(filled, stroked)
            }
            Content::Image { stroke, .. } => {
                let reach = stroke
                    .as_ref()
                    .map_or(0.0, |stroke| stroke_reach(stroke, true));
                Some(outset(&node.frame(), reach))
            }
            Content::Text(text) => {
                if text.fill.is_none() && text.stroke.is_none() {
                    return None;
                }
                let block = self.fonts.layout(&text.text, &text.style, text.wrap_width);
                let outset_by = text
                    .stroke
                    .as_ref()
                    .map_or(0.0, |stroke| stroke_reach(stroke, true));
                // The same rule as drawing: one unit over everything is
                // the whole text.
                let whole = matches!(
                    text.units.as_slice(),
                    [only] if only.start == 0 && only.end >= text.text.len()
                );
                text.units
                    .iter()
                    .filter_map(|unit| {
                        let range = unit.start..unit.end;
                        let ink = block.ink((!whole).then_some(&range))?;
                        let spread = effects::sigma(unit.blur).map_or(0.0, |s| 3.0 * f64::from(s));
                        let ink = Rect::new(ink.x, ink.y + text.offset_y, ink.width, ink.height);
                        Some(usable(&unit.transform)?.bounds_of(&outset(&ink, outset_by + spread)))
                    })
                    .reduce(|all, one| all.union(&one))
            }
        }
    }
}

/// How carefully to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Precision {
    /// The tight box, for callers. Strokes are measured as drawn.
    Exact,
    /// A box that may be a little loose but is cheap, for sizing the
    /// painter's offscreen layers every frame.
    Quick,
}

/// Whether `point`, in `node`'s coordinates, is on what the node draws.
/// Thin outlines are widened to `reach` either side.
fn touches(node: &SceneNode, point: Point, reach: f64) -> bool {
    let at = convert::point(point);
    match &node.content {
        Content::Rect { fill, stroke, .. }
        | Content::Ellipse { fill, stroke, .. }
        | Content::Star { fill, stroke, .. }
        | Content::Path { fill, stroke, .. } => {
            let Some(outline) = geometry::outline(node) else {
                return false;
            };
            let inside = outline.path.contains(at);
            match (fill, stroke) {
                (Some(_), _) if inside => true,
                (_, Some(stroke)) => {
                    let wide = widened(stroke, reach, outline.closed);
                    stroke::area(&outline, &wide).is_some_and(|area| area.contains(at))
                }
                // A shape that draws nothing is still found by its outline,
                // so it can be selected.
                (None, None) => inside,
                (Some(_), None) => false,
            }
        }
        Content::Image { radii, .. } => geometry::rect(node.width, node.height, radii).contains(at),
        // Text is found by its box: between letters counts.
        Content::Text(_) => node.frame().contains(point),
        Content::Group { .. } => false,
    }
}

/// `stroke`, at least `reach` wide on each side it covers.
fn widened(stroke: &SceneStroke, reach: f64, closed: bool) -> SceneStroke {
    let least = match stroke::side(stroke, closed) {
        None => 2.0 * reach,
        Some(_) => reach,
    };
    SceneStroke {
        width: stroke.width.max(least),
        ..stroke.clone()
    }
}

/// How far a stroke can reach past the outline it follows, corners and
/// ends included.
pub(crate) fn stroke_reach(stroke: &SceneStroke, closed: bool) -> f64 {
    let width = if stroke.width.is_finite() {
        stroke.width.max(0.0)
    } else {
        0.0
    };
    let side = match stroke::side(stroke, closed) {
        None => width / 2.0,
        Some(sk::ClipOp::Intersect) => 0.0,
        Some(_) => width,
    };
    // Mitred corners reach furthest; square caps and the diagonal of a
    // round join reach half a width times the square root of 2.
    let corner = match stroke.join {
        StrokeJoin::Miter => f64::from(stroke::MITER_LIMIT),
        StrokeJoin::Round | StrokeJoin::Bevel => std::f64::consts::SQRT_2,
    };
    side * corner
}

/// The transform, if every number in it is finite and it does not flatten
/// space.
fn usable(transform: &Affine) -> Option<Affine> {
    (convert::is_finite(transform) && transform.invert().is_some()).then_some(*transform)
}

fn clip_contains(clip: &Clip, point: Point) -> bool {
    if !clip.rect.contains(point) {
        return false;
    }
    !clip.radii.iter().any(|radius| *radius > 0.0)
        || sk::Path::rrect(convert::rounded_rect(&clip.rect, &clip.radii), None)
            .contains(convert::point(point))
}

fn rect(bounds: &sk::Rect) -> Rect {
    Rect::new(
        f64::from(bounds.left),
        f64::from(bounds.top),
        f64::from(bounds.width()),
        f64::from(bounds.height()),
    )
}

/// `rect` grown by `by` on every side.
fn outset(rect: &Rect, by: f64) -> Rect {
    Rect::new(
        rect.x - by,
        rect.y - by,
        rect.width + 2.0 * by,
        rect.height + 2.0 * by,
    )
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(&b)),
        (a, b) => a.or(b),
    }
}

/// Where two rectangles overlap, or `None` when they do not.
fn intersection(a: &Rect, b: &Rect) -> Option<Rect> {
    let (left, top) = (a.x.max(b.x), a.y.max(b.y));
    let (right, bottom) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetLoader, Renderer};
    use moshionly_core::model::{BlendMode, Font, Shadow, StrokeCap, StrokePosition, TextAlign};
    use moshionly_core::scene::{ScenePaint, TextContent, TextStyle, TextUnit};
    use moshionly_core::{Color, NodeId};

    fn renderer() -> Renderer {
        Renderer::new(FontLibrary::bundled(), AssetLoader::default())
    }

    fn node(id: &str, transform: Affine, width: f64, height: f64, content: Content) -> SceneNode {
        SceneNode {
            id: NodeId::from(id),
            transform,
            width,
            height,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            blur: 0.0,
            shadow: None,
            clip: None,
            mask: None,
            content,
        }
    }

    fn filled() -> Option<ScenePaint> {
        Some(ScenePaint::solid(Color::BLACK))
    }

    fn rect(id: &str, transform: Affine, width: f64, height: f64) -> SceneNode {
        let content = Content::Rect {
            radii: [0.0; 4],
            fill: filled(),
            stroke: None,
        };
        node(id, transform, width, height, content)
    }

    fn outline(width: f64, position: StrokePosition) -> Option<SceneStroke> {
        Some(SceneStroke {
            paint: ScenePaint::solid(Color::BLACK),
            width,
            position,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
        })
    }

    fn scene(nodes: Vec<SceneNode>) -> Scene {
        Scene {
            artboard: NodeId::from("ab"),
            time_ms: 0.0,
            width: 400.0,
            height: 300.0,
            background: None,
            nodes,
        }
    }

    fn hit(scene: &Scene, x: f64, y: f64) -> Vec<String> {
        renderer()
            .hit_test(scene, Point::new(x, y))
            .iter()
            .map(NodeId::to_string)
            .collect()
    }

    fn near(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    #[test]
    fn hits_come_front_to_back_through_transforms() {
        // A 100 by 20 bar at (50, 90), turned a quarter about its centre:
        // it now stands from (90, 50) to (110, 150).
        let turned = rect(
            "bar",
            Affine::layer(50.0, 90.0, 100.0, 20.0, 90.0, 1.0),
            100.0,
            20.0,
        );
        let back = rect("back", Affine::translate(0.0, 0.0), 200.0, 200.0);
        let scene = scene(vec![back, turned]);
        assert_eq!(hit(&scene, 100.0, 60.0), ["bar", "back"]);
        assert_eq!(hit(&scene, 60.0, 100.0), ["back"], "where the bar was");
        assert!(hit(&scene, 250.0, 100.0).is_empty());
        assert!(hit(&scene, 500.0, 100.0).is_empty(), "off the artboard");

        // A group scaled twice: its 10 by 10 square covers 20 by 20.
        let group = node(
            "group",
            Affine::translate(200.0, 0.0) * Affine::scale(2.0, 2.0),
            50.0,
            50.0,
            Content::Group {
                children: vec![rect("small", Affine::translate(5.0, 5.0), 10.0, 10.0)],
            },
        );
        let scene = self::scene(vec![group]);
        assert_eq!(hit(&scene, 229.0, 29.0), ["small"]);
        assert!(hit(&scene, 231.0, 29.0).is_empty());
        assert!(
            hit(&scene, 205.0, 5.0).is_empty(),
            "groups are found through layers"
        );
    }

    #[test]
    fn shapes_are_hit_by_their_outline() {
        let star = node(
            "star",
            Affine::IDENTITY,
            100.0,
            100.0,
            Content::Star {
                points: 5,
                inner_radius: 0.4,
                fill: filled(),
                stroke: None,
            },
        );
        let ring = node(
            "ring",
            Affine::translate(200.0, 0.0),
            100.0,
            100.0,
            Content::Ellipse {
                start_angle: 0.0,
                sweep: 360.0,
                inner_radius: 0.5,
                fill: filled(),
                stroke: None,
            },
        );
        let scene = scene(vec![star, ring]);
        assert_eq!(hit(&scene, 50.0, 50.0), ["star"]);
        assert!(hit(&scene, 50.0, 95.0).is_empty(), "between two points");
        assert!(hit(&scene, 250.0, 50.0).is_empty(), "the hole");
        assert_eq!(hit(&scene, 290.0, 50.0), ["ring"]);
        assert!(hit(&scene, 205.0, 5.0).is_empty(), "the corner of its box");
    }

    #[test]
    fn thin_outlines_are_hit_near_their_edge() {
        let frame = node(
            "frame",
            Affine::translate(100.0, 100.0),
            100.0,
            100.0,
            Content::Rect {
                radii: [0.0; 4],
                fill: None,
                stroke: outline(1.0, StrokePosition::Center),
            },
        );
        let scene = scene(vec![frame.clone()]);
        assert_eq!(hit(&scene, 102.0, 150.0), ["frame"], "within the tolerance");
        assert!(hit(&scene, 105.0, 150.0).is_empty());
        assert!(hit(&scene, 150.0, 150.0).is_empty(), "the empty middle");
        let precise = renderer().hit_test_within(&scene, Point::new(102.0, 150.0), 0.0);
        assert!(precise.is_empty());

        // The tolerance is in artboard pixels: inside a layer scaled 4x
        // the stroke covers 98 to 102, and the reach is still 3 pixels
        // from the edge on the artboard.
        let mut zoomed = frame;
        zoomed.transform = Affine::translate(100.0, 100.0) * Affine::scale(4.0, 4.0);
        let scene = self::scene(vec![zoomed]);
        assert_eq!(hit(&scene, 97.5, 150.0), ["frame"]);
        assert_eq!(hit(&scene, 102.5, 150.0), ["frame"]);
        assert!(hit(&scene, 104.0, 150.0).is_empty());
    }

    #[test]
    fn clips_and_masks_hide_what_they_cut_off() {
        let mut clipped = node(
            "group",
            Affine::IDENTITY,
            100.0,
            100.0,
            Content::Group {
                children: vec![rect("wide", Affine::IDENTITY, 200.0, 50.0)],
            },
        );
        clipped.clip = Some(Clip {
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            radii: [20.0; 4],
        });
        let scene = scene(vec![clipped.clone()]);
        assert_eq!(hit(&scene, 50.0, 25.0), ["wide"]);
        assert!(hit(&scene, 150.0, 25.0).is_empty(), "clipped away");
        assert!(hit(&scene, 2.0, 2.0).is_empty(), "the rounded corner");

        let mut masked = clipped;
        masked.clip = None;
        masked.mask = Some(Box::new(node(
            "mask",
            Affine::translate(100.0, 0.0),
            100.0,
            100.0,
            Content::Ellipse {
                start_angle: 0.0,
                sweep: 360.0,
                inner_radius: 0.0,
                fill: filled(),
                stroke: None,
            },
        )));
        let scene = self::scene(vec![masked]);
        assert_eq!(hit(&scene, 150.0, 25.0), ["wide"]);
        assert!(hit(&scene, 50.0, 25.0).is_empty(), "outside the mask");
        assert!(
            hit(&scene, 105.0, 5.0).is_empty(),
            "outside the mask's ellipse"
        );
    }

    #[test]
    fn bounds_follow_the_box_and_the_ink() {
        // Turned a quarter and scaled 2 about its centre (100, 100).
        let mut bar = rect(
            "bar",
            Affine::layer(50.0, 90.0, 100.0, 20.0, 90.0, 2.0),
            100.0,
            20.0,
        );
        let renderer = renderer();
        let bounds = renderer
            .bounds_of(&scene(vec![bar.clone()]), &NodeId::from("bar"))
            .unwrap();
        let expected = [(120.0, 0.0), (120.0, 200.0), (80.0, 200.0), (80.0, 0.0)];
        for (corner, (x, y)) in bounds.corners.iter().zip(expected) {
            assert!(near(*corner, Point::new(x, y)), "{:?}", bounds.corners);
        }
        let visual = bounds.visual.unwrap();
        assert!((visual.x - 80.0).abs() < 1e-3 && (visual.height - 200.0).abs() < 1e-3);

        // An outside stroke and a shadow reach past the box.
        bar.transform = Affine::translate(10.0, 10.0);
        bar.content = Content::Rect {
            radii: [0.0; 4],
            fill: filled(),
            stroke: outline(4.0, StrokePosition::Outside),
        };
        bar.shadow = Some(Shadow {
            color: Color::BLACK,
            x: 0.0,
            y: 10.0,
            blur: 0.0,
        });
        let bounds = renderer
            .bounds_of(&scene(vec![bar]), &NodeId::from("bar"))
            .unwrap();
        assert_eq!(bounds.corners[0], Point::new(10.0, 10.0));
        assert_eq!(bounds.visual, Some(Rect::new(6.0, 6.0, 108.0, 38.0)));

        assert!(
            renderer
                .bounds_of(&scene(Vec::new()), &NodeId::from("bar"))
                .is_none()
        );
    }

    #[test]
    fn bounds_of_nested_clipped_and_text_layers() {
        let mut group = node(
            "group",
            Affine::translate(100.0, 50.0),
            50.0,
            50.0,
            Content::Group {
                children: vec![rect("inner", Affine::translate(10.0, 10.0), 100.0, 10.0)],
            },
        );
        group.clip = Some(Clip {
            rect: Rect::new(0.0, 0.0, 50.0, 50.0),
            radii: [0.0; 4],
        });
        let text = node(
            "text",
            Affine::translate(20.0, 200.0),
            150.0,
            40.0,
            Content::Text(TextContent {
                text: "Hi".into(),
                style: TextStyle {
                    font: Font::default(),
                    font_size: 30.0,
                    line_height: 40.0,
                    letter_spacing: 0.0,
                    align: TextAlign::Left,
                },
                wrap_width: None,
                offset_y: 0.0,
                fill: filled(),
                stroke: None,
                units: vec![TextUnit {
                    start: 0,
                    end: 2,
                    transform: Affine::IDENTITY,
                    opacity: 1.0,
                    blur: 0.0,
                }],
            }),
        );
        let scene = scene(vec![group, text]);
        let renderer = renderer();
        let inner = renderer.bounds_of(&scene, &NodeId::from("inner")).unwrap();
        assert_eq!(inner.corners[0], Point::new(110.0, 60.0));
        assert_eq!(
            inner.corners[2],
            Point::new(210.0, 70.0),
            "the box, unclipped"
        );
        let group = renderer.bounds_of(&scene, &NodeId::from("group")).unwrap();
        assert_eq!(
            group.visual,
            Some(Rect::new(110.0, 60.0, 40.0, 10.0)),
            "clipped"
        );

        let text = renderer.bounds_of(&scene, &NodeId::from("text")).unwrap();
        assert_eq!(text.corners[2], Point::new(170.0, 240.0));
        let ink = text.visual.unwrap();
        // The ink of "Hi" sits inside the box, starting near its left.
        assert!(ink.x >= 20.0 && ink.x < 26.0, "{ink:?}");
        assert!(ink.y > 200.0 && ink.bottom() < 240.0, "{ink:?}");
        assert!(ink.width > 20.0 && ink.width < 60.0, "{ink:?}");
    }
}
