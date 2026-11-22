//! The scene: one frame of an artboard with every value resolved.
//!
//! [`evaluate`](crate::eval::evaluate) turns a document and a time into a
//! [`Scene`]. A scene is plain data. It does not know about animations,
//! fonts or a graphics library, and it can be serialised. The renderer, the
//! Lottie exporter and hit testing all read it.
//!
//! # How to draw a node
//!
//! Every [`SceneNode`] has its own coordinate space with the origin at the
//! top-left corner of its `width` by `height` box. `transform` maps that
//! space into the space of the node's parent (for top-level nodes: the
//! artboard, whose origin is its top-left corner).
//!
//! Inside the node's space, from the inside out:
//!
//! 1. draw the `content`;
//! 2. add the `shadow` of what was drawn, behind it;
//! 3. `blur` the result;
//! 4. keep only what is inside `clip`, and multiply by the alpha of `mask`
//!    (the mask is a node in the same coordinate space and is not drawn);
//! 5. composite onto what is behind with `blend_mode` and `opacity`.
//!
//! Shadow offsets, blur amounts and clips are in the node's own space, so
//! they turn and scale with it. Nodes that cannot be seen (hidden, fully
//! transparent, scaled to nothing) are not in the scene at all.

use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::geometry::{Affine, Point, Rect};
use crate::id::NodeId;
use crate::model::{
    BlendMode, Font, GradientStop, ImageFit, PathData, Shadow, StrokeCap, StrokeJoin,
    StrokePosition, TextAlign,
};

/// One frame of one artboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// The artboard this is a frame of.
    pub artboard: NodeId,
    /// The time the frame was evaluated at, in milliseconds.
    pub time_ms: f64,
    /// Width of the artboard in pixels.
    pub width: f64,
    /// Height of the artboard in pixels.
    pub height: f64,
    /// Paint behind everything, covering the whole artboard. `None` leaves
    /// the artboard transparent.
    pub background: Option<ScenePaint>,
    /// The top-level nodes, back to front. Everything is clipped to the
    /// artboard.
    pub nodes: Vec<SceneNode>,
}

/// One thing to draw. See the [module docs](self) for the drawing order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    /// The layer in the document this node comes from. Several scene nodes
    /// can share an id: a group with a fill, for example, becomes a group
    /// node with a rectangle node inside it.
    pub id: NodeId,
    /// Maps the node's coordinates into its parent's.
    pub transform: Affine,
    /// Width of the node's box in pixels.
    pub width: f64,
    /// Height of the node's box in pixels.
    pub height: f64,
    /// Opacity from 0 to 1. Never 0: such nodes are left out.
    pub opacity: f64,
    /// How the node combines with what is behind it.
    pub blend_mode: BlendMode,
    /// Blur in pixels (twice the standard deviation of the Gaussian). 0 for none.
    pub blur: f64,
    /// Drop shadow, if any.
    pub shadow: Option<Shadow>,
    /// Region outside which nothing of the node is drawn, if any.
    pub clip: Option<Clip>,
    /// A node whose alpha masks this one, if any.
    pub mask: Option<Box<SceneNode>>,
    /// What to draw.
    pub content: Content,
}

/// A rounded rectangle to clip to, in the node's own coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// The rectangle.
    pub rect: Rect,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f64; 4],
}

/// What a scene node draws. Shapes fill the node's `width` by `height` box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    /// Other nodes, back to front, in this node's coordinate space.
    Group {
        /// The nodes inside.
        children: Vec<SceneNode>,
    },
    /// A rectangle covering the box.
    Rect {
        /// Corner radii in pixels: top-left, top-right, bottom-right,
        /// bottom-left. Already limited to what fits in the box.
        radii: [f64; 4],
        /// Paint of the inside.
        fill: Option<ScenePaint>,
        /// Outline.
        stroke: Option<SceneStroke>,
    },
    /// An ellipse inscribed in the box, possibly cut to an arc or a ring.
    Ellipse {
        /// Where the arc starts, in degrees clockwise from 3 o'clock.
        start_angle: f64,
        /// How far the arc runs, in degrees clockwise; 360 or -360 is whole.
        sweep: f64,
        /// Radius of the hole as a fraction of the outer radius, 0 to 1.
        inner_radius: f64,
        /// Paint of the inside.
        fill: Option<ScenePaint>,
        /// Outline.
        stroke: Option<SceneStroke>,
    },
    /// A star inscribed in the box, first point at the top.
    Star {
        /// Number of points.
        points: u32,
        /// Radius of the inner corners as a fraction of the outer radius.
        inner_radius: f64,
        /// Paint of the inside.
        fill: Option<ScenePaint>,
        /// Outline.
        stroke: Option<SceneStroke>,
    },
    /// A pen-drawn outline.
    Path {
        /// The outline in pixels in the node's coordinates.
        path: PathData,
        /// Paint of the inside.
        fill: Option<ScenePaint>,
        /// Outline.
        stroke: Option<SceneStroke>,
    },
    /// Text.
    Text(TextContent),
    /// A bitmap image filling the box.
    Image {
        /// Where the image comes from, as written in the document. Resolve
        /// it with [`resolve_asset`](crate::io::resolve_asset).
        source: String,
        /// How the image fills the box when their proportions differ.
        fit: ImageFit,
        /// Corner radii in pixels: top-left, top-right, bottom-right,
        /// bottom-left. Already limited to what fits in the box.
        radii: [f64; 4],
        /// Outline of the box.
        stroke: Option<SceneStroke>,
    },
}

/// A paint with its geometry in pixels in the node's coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScenePaint {
    /// One colour.
    Solid {
        /// The colour.
        color: Color,
    },
    /// Colours change along the line from `start` to `end`.
    LinearGradient {
        /// Where offset 0 sits.
        start: Point,
        /// Where offset 1 sits.
        end: Point,
        /// The colour stops, by ascending offset.
        stops: Vec<GradientStop>,
    },
    /// Colours change outward from `center` to an ellipse.
    RadialGradient {
        /// Where offset 0 sits.
        center: Point,
        /// Horizontal radius of the ellipse where offset 1 sits, before rotation.
        radius_x: f64,
        /// Vertical radius of that ellipse, before rotation.
        radius_y: f64,
        /// Rotation of the ellipse about its centre, in degrees clockwise.
        rotation: f64,
        /// The colour stops, by ascending offset.
        stops: Vec<GradientStop>,
    },
}

impl ScenePaint {
    /// A solid paint.
    pub const fn solid(color: Color) -> Self {
        ScenePaint::Solid { color }
    }

    /// The colour, when the paint is solid.
    pub fn as_solid(&self) -> Option<Color> {
        match self {
            ScenePaint::Solid { color } => Some(*color),
            _ => None,
        }
    }
}

/// A resolved stroke. Never has a width of 0: such strokes are left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneStroke {
    /// Paint of the outline.
    pub paint: ScenePaint,
    /// Thickness in pixels.
    pub width: f64,
    /// Whether the outline sits inside, on, or outside the edge.
    pub position: StrokePosition,
    /// Shape of the ends of open paths.
    pub cap: StrokeCap,
    /// Shape of corners.
    pub join: StrokeJoin,
}

/// Text, laid out by the renderer and drawn unit by unit.
///
/// To draw it: lay `text` out with `style` in a block `width` wide, breaking
/// lines at line breaks and, when `wrap_width` is set, also where a line
/// would pass that width. Align each line inside `width` by `style.align`.
/// Put the top of the block `offset_y` below the top of the node's box. Then
/// draw each unit: the glyphs of its byte range, with its transform applied
/// on top of where layout put them, at its opacity and blur.
///
/// Text without a text animation in flight is one unit covering everything.
/// Parts of the text that no unit covers are not drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    /// The text to draw, with letter case applied and counters filled in.
    pub text: String,
    /// Font and spacing.
    pub style: TextStyle,
    /// Width at which lines wrap. `None`: only at line breaks in the text.
    pub wrap_width: Option<f64>,
    /// Distance from the top of the box to the top of the first line, from
    /// vertical alignment. Negative when the text is taller than a fixed box.
    pub offset_y: f64,
    /// Paint of the glyphs. Gradients span the node's box.
    pub fill: Option<ScenePaint>,
    /// Outline of the glyphs.
    pub stroke: Option<SceneStroke>,
    /// The pieces to draw, in reading order.
    pub units: Vec<TextUnit>,
}

/// Everything about text that layout needs, in pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// The font.
    pub font: Font,
    /// Font size in pixels.
    pub font_size: f64,
    /// Distance between baselines in pixels.
    pub line_height: f64,
    /// Extra space after every letter in pixels; may be negative.
    pub letter_spacing: f64,
    /// Horizontal alignment of lines.
    pub align: TextAlign,
}

/// A run of text drawn with its own transform, opacity and blur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextUnit {
    /// Byte offset in the text where the unit starts.
    pub start: usize,
    /// Byte offset in the text just past the unit.
    pub end: usize,
    /// Applied in the node's coordinates on top of the laid-out position.
    pub transform: Affine,
    /// Opacity from 0 to 1, multiplied with the node's. Never 0.
    pub opacity: f64,
    /// Blur in pixels, on top of the node's.
    pub blur: f64,
}

impl SceneNode {
    /// The nodes inside a group node; empty for anything else.
    pub fn children(&self) -> &[SceneNode] {
        match &self.content {
            Content::Group { children } => children,
            _ => &[],
        }
    }

    /// The node's box in its own coordinates.
    pub fn frame(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width, self.height)
    }

    /// Depth-first search for the first node with this id, itself included.
    /// Masks are searched too.
    pub fn find(&self, id: &NodeId) -> Option<&SceneNode> {
        if &self.id == id {
            return Some(self);
        }
        self.mask
            .as_deref()
            .and_then(|mask| mask.find(id))
            .or_else(|| self.children().iter().find_map(|child| child.find(id)))
    }

    /// The transform from the coordinates of the node with this id up to
    /// the coordinates of this node's parent.
    fn transform_to(&self, id: &NodeId) -> Option<Affine> {
        if &self.id == id {
            return Some(self.transform);
        }
        self.mask
            .as_deref()
            .and_then(|mask| mask.transform_to(id))
            .or_else(|| {
                self.children()
                    .iter()
                    .find_map(|child| child.transform_to(id))
            })
            .map(|inner| self.transform * inner)
    }

    /// The chain of ids from this node down to the frontmost node under
    /// `point`, which is in the coordinates of this node's parent.
    fn hit(&self, point: Point, chain: &mut Vec<NodeId>) -> bool {
        let Some(local) = self.transform.invert().map(|inverse| inverse.apply(point)) else {
            return false;
        };
        if self.clip.is_some_and(|clip| !clip.rect.contains(local)) {
            return false;
        }
        if let Some(mask) = &self.mask
            && !mask.hit(local, &mut Vec::new())
        {
            return false;
        }
        chain.push(self.id.clone());
        let hit = match &self.content {
            Content::Group { children } => {
                children.iter().rev().any(|child| child.hit(local, chain))
            }
            Content::Path { path, .. } => path
                .control_bounds()
                .is_some_and(|bounds| bounds.contains(local)),
            Content::Ellipse { fill, stroke, .. } => {
                let inside = |inset: f64| {
                    let (rx, ry) = (self.width / 2.0 - inset, self.height / 2.0 - inset);
                    let (dx, dy) = (local.x - self.width / 2.0, local.y - self.height / 2.0);
                    rx > 0.0 && ry > 0.0 && (dx / rx).powi(2) + (dy / ry).powi(2) <= 1.0
                };
                inside(0.0) && !outline_only(fill, stroke).is_some_and(inside)
            }
            Content::Rect { fill, stroke, .. } => {
                let inside = |inset: f64| {
                    Rect::new(
                        inset,
                        inset,
                        self.width - 2.0 * inset,
                        self.height - 2.0 * inset,
                    )
                    .contains(local)
                };
                inside(0.0) && !outline_only(fill, stroke).is_some_and(inside)
            }
            _ => self.frame().contains(local),
        };
        if !hit {
            chain.pop();
        }
        hit
    }
}

/// For a shape that is drawn as an outline only, how far in from its edge a
/// point still counts as on the outline. `None` for a shape with a fill, or
/// with nothing at all (which is then found by its box).
fn outline_only(fill: &Option<ScenePaint>, stroke: &Option<SceneStroke>) -> Option<f64> {
    /// Thin outlines get a margin that can be clicked.
    const REACH: f64 = 6.0;
    match (fill, stroke) {
        (None, Some(stroke)) => Some(stroke.width.max(REACH)),
        _ => None,
    }
}

impl Scene {
    /// Depth-first search for the first node that comes from the layer `id`.
    pub fn find(&self, id: &NodeId) -> Option<&SceneNode> {
        self.nodes.iter().find_map(|node| node.find(id))
    }

    /// The transform from the coordinates of the layer `id` to artboard
    /// coordinates in this frame. Together with the node's `width` and
    /// `height` this gives the outline to draw around a selected layer.
    ///
    /// `None` when the layer is not in the scene, for example because it is
    /// hidden at this time.
    pub fn transform_of(&self, id: &NodeId) -> Option<Affine> {
        self.nodes.iter().find_map(|node| node.transform_to(id))
    }

    /// The layers under a point given in artboard coordinates: the ids from
    /// the outermost group down to the frontmost node that the point is on.
    /// Empty when the point is on nothing.
    ///
    /// Rectangles, text, images and stars count as their box, ellipses as
    /// the full ellipse, paths as the box around their points. A rectangle
    /// or ellipse that is only an outline is hit near its edge, not through
    /// its empty middle. Clips and masks hide what they cut off. Repeated
    /// ids (a group and its own background) are listed once.
    pub fn hit_test(&self, point: Point) -> Vec<NodeId> {
        if !Rect::new(0.0, 0.0, self.width, self.height).contains(point) {
            return Vec::new();
        }
        let mut chain = Vec::new();
        for node in self.nodes.iter().rev() {
            if node.hit(point, &mut chain) {
                break;
            }
        }
        chain.dedup();
        chain
    }

    /// Number of nodes in the scene, masks included.
    pub fn node_count(&self) -> usize {
        fn count(node: &SceneNode) -> usize {
            1 + node.mask.as_deref().map_or(0, count)
                + node.children().iter().map(count).sum::<usize>()
        }
        self.nodes.iter().map(count).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    fn rect(id: &str, x: f64, y: f64, width: f64, height: f64) -> SceneNode {
        let content = Content::Rect {
            radii: [0.0; 4],
            fill: Some(ScenePaint::solid(Color::BLACK)),
            stroke: None,
        };
        node(id, Affine::translate(x, y), width, height, content)
    }

    fn ellipse(id: &str, x: f64, y: f64, width: f64, height: f64) -> SceneNode {
        let content = Content::Ellipse {
            start_angle: 0.0,
            sweep: 360.0,
            inner_radius: 0.0,
            fill: None,
            stroke: None,
        };
        node(id, Affine::translate(x, y), width, height, content)
    }

    /// A 400 x 300 artboard: a rect at the back, and in front a group at
    /// (100, 100), scaled 2, holding its own background, a rect and an
    /// ellipse.
    fn sample() -> Scene {
        let group = node(
            "group",
            Affine::translate(100.0, 100.0) * Affine::scale(2.0, 2.0),
            100.0,
            50.0,
            Content::Group {
                children: vec![
                    rect("group", 0.0, 0.0, 100.0, 50.0),
                    rect("inner", 10.0, 10.0, 20.0, 20.0),
                    ellipse("dot", 60.0, 10.0, 30.0, 30.0),
                ],
            },
        );
        Scene {
            artboard: NodeId::from("ab"),
            time_ms: 250.0,
            width: 400.0,
            height: 300.0,
            background: Some(ScenePaint::solid(Color::WHITE)),
            nodes: vec![rect("back", 0.0, 0.0, 50.0, 50.0), group],
        }
    }

    fn hit(scene: &Scene, x: f64, y: f64) -> Vec<String> {
        scene
            .hit_test(Point::new(x, y))
            .iter()
            .map(NodeId::to_string)
            .collect()
    }

    #[test]
    fn nodes_are_found_by_id() {
        let scene = sample();
        assert_eq!(scene.find(&NodeId::from("inner")).unwrap().width, 20.0);
        assert!(matches!(
            scene.find(&NodeId::from("group")).unwrap().content,
            Content::Group { .. }
        ));
        assert!(scene.find(&NodeId::from("nope")).is_none());
        assert_eq!(scene.node_count(), 5);
        assert_eq!(scene.nodes[1].children().len(), 3);
        assert!(scene.nodes[0].children().is_empty());
    }

    #[test]
    fn transforms_reach_up_to_the_artboard() {
        let scene = sample();
        let inner = scene.transform_of(&NodeId::from("inner")).unwrap();
        assert_eq!(inner.apply(Point::ZERO), Point::new(120.0, 120.0));
        assert_eq!(
            inner.apply(Point::new(20.0, 20.0)),
            Point::new(160.0, 160.0)
        );
        assert_eq!(
            scene.transform_of(&NodeId::from("back")),
            Some(Affine::IDENTITY)
        );
        assert!(scene.transform_of(&NodeId::from("nope")).is_none());
    }

    #[test]
    fn hit_testing_finds_the_frontmost_node() {
        let scene = sample();
        assert_eq!(hit(&scene, 10.0, 10.0), ["back"]);
        assert_eq!(hit(&scene, 130.0, 130.0), ["group", "inner"]);
        // On the group's own background only: the repeated id is listed once.
        assert_eq!(hit(&scene, 290.0, 190.0), ["group"]);
        assert!(hit(&scene, 60.0, 60.0).is_empty());
        assert!(hit(&scene, 500.0, 10.0).is_empty(), "outside the artboard");
        // The centre of the ellipse hits it; the corner of its box does not.
        assert_eq!(hit(&scene, 250.0, 150.0), ["group", "dot"]);
        assert_eq!(hit(&scene, 221.0, 121.0), ["group"]);
    }

    #[test]
    fn clips_and_masks_hide_what_they_cut_off() {
        let mut scene = sample();
        scene.nodes[1].clip = Some(Clip {
            rect: Rect::new(0.0, 0.0, 50.0, 50.0),
            radii: [0.0; 4],
        });
        assert_eq!(hit(&scene, 130.0, 130.0), ["group", "inner"]);
        assert!(
            hit(&scene, 250.0, 150.0).is_empty(),
            "the ellipse is clipped away"
        );

        let mut masked = sample();
        masked.nodes[1].mask = Some(Box::new(rect("mask", 50.0, 0.0, 50.0, 50.0)));
        assert!(hit(&masked, 130.0, 130.0).is_empty());
        assert_eq!(hit(&masked, 250.0, 150.0), ["group", "dot"]);
        assert_eq!(masked.node_count(), 6);
        assert!(masked.find(&NodeId::from("mask")).is_some());
    }

    #[test]
    fn outlines_are_hit_near_their_edge_only() {
        let mut scene = sample();
        let outline = |width: f64| {
            Some(SceneStroke {
                paint: ScenePaint::solid(Color::BLACK),
                width,
                position: StrokePosition::Center,
                cap: StrokeCap::Butt,
                join: StrokeJoin::Miter,
            })
        };
        scene.nodes[0].content = Content::Rect {
            radii: [0.0; 4],
            fill: None,
            stroke: outline(1.0),
        };
        assert_eq!(
            hit(&scene, 2.0, 25.0),
            ["back"],
            "thin outlines have some reach"
        );
        assert!(hit(&scene, 25.0, 25.0).is_empty(), "the middle is empty");
        scene.nodes[0].content = Content::Rect {
            radii: [0.0; 4],
            fill: None,
            stroke: outline(20.0),
        };
        assert_eq!(hit(&scene, 15.0, 25.0), ["back"]);
        // A shape with nothing to draw is still found by its box.
        scene.nodes[0].content = Content::Ellipse {
            start_angle: 0.0,
            sweep: 360.0,
            inner_radius: 0.0,
            fill: None,
            stroke: None,
        };
        assert_eq!(hit(&scene, 25.0, 25.0), ["back"]);
        scene.nodes[0].content = Content::Ellipse {
            start_angle: 0.0,
            sweep: 360.0,
            inner_radius: 0.0,
            fill: None,
            stroke: outline(2.0),
        };
        assert!(hit(&scene, 25.0, 25.0).is_empty());
        assert_eq!(hit(&scene, 25.0, 3.0), ["back"]);
    }

    #[test]
    fn a_scene_is_plain_json() {
        let scene = sample();
        let value = serde_json::to_value(&scene).unwrap();
        assert_eq!(value["artboard"], "ab");
        assert_eq!(
            value["background"],
            json!({ "type": "solid", "color": "#FFFFFF" })
        );
        assert_eq!(value["nodes"][0]["content"]["type"], "rect");
        assert_eq!(
            value["nodes"][0]["transform"],
            json!([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        );
        assert_eq!(value["nodes"][1]["content"]["type"], "group");
        assert_eq!(
            value["nodes"][1]["content"]["children"][2]["content"]["type"],
            "ellipse"
        );
        let back: Scene = serde_json::from_value(value).unwrap();
        assert_eq!(back, scene);
    }

    #[test]
    fn text_content_round_trips() {
        let text = Content::Text(TextContent {
            text: "Hi there".into(),
            style: TextStyle {
                font: Font::default(),
                font_size: 48.0,
                line_height: 57.6,
                letter_spacing: 0.0,
                align: TextAlign::Center,
            },
            wrap_width: Some(300.0),
            offset_y: 12.0,
            fill: Some(ScenePaint::LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(300.0, 0.0),
                stops: vec![
                    GradientStop::new(0.0, Color::BLACK),
                    GradientStop::new(1.0, Color::WHITE),
                ],
            }),
            stroke: None,
            units: vec![TextUnit {
                start: 0,
                end: 2,
                transform: Affine::translate(0.0, 10.0),
                opacity: 0.5,
                blur: 4.0,
            }],
        });
        let value = serde_json::to_value(&text).unwrap();
        assert_eq!(value["type"], "text");
        assert_eq!(value["units"][0]["end"], 2);
        assert_eq!(value["fill"]["type"], "linear_gradient");
        assert_eq!(serde_json::from_value::<Content>(value).unwrap(), text);
    }
}
