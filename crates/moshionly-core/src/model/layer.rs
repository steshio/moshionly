//! Layer types: the things you see on an artboard.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::effect::{DirectionalBlur, Glow, Noise};
use super::paint::{BlendMode, CornerRadius, Paint, Shadow, Stroke};
use super::path::PathData;
use super::text::{AutoSize, Font, TextAlign, TextCase, VerticalAlign};
use crate::defaults;
use crate::geometry::{Affine, Point, Rect};
use crate::id::NodeId;

/// The properties every layer has. They describe the layer at rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayerProps {
    /// Name shown in the layer list. Default: empty, the editor shows the type.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub name: String,
    /// Left edge of the layer's box in its parent's coordinates, in pixels.
    /// For an artboard: its position on the canvas. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub x: f64,
    /// Top edge of the layer's box in its parent's coordinates, in pixels,
    /// growing downward. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub y: f64,
    /// Width of the box in pixels, 0 or more. Default 100.
    #[serde(
        default = "defaults::layer_size",
        skip_serializing_if = "defaults::is_layer_size"
    )]
    #[schemars(range(min = 0.0))]
    pub width: f64,
    /// Height of the box in pixels, 0 or more. Default 100.
    #[serde(
        default = "defaults::layer_size",
        skip_serializing_if = "defaults::is_layer_size"
    )]
    #[schemars(range(min = 0.0))]
    pub height: f64,
    /// Rotation about the centre of the box in degrees, clockwise. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub rotation: f64,
    /// Uniform scale about the centre of the box, 0 or more; 1 is actual size.
    /// Default 1.
    #[serde(default = "defaults::one", skip_serializing_if = "defaults::is_one")]
    #[schemars(range(min = 0.0))]
    pub scale: f64,
    /// Opacity from 0 (invisible) to 100 (opaque). Default 100.
    #[serde(
        default = "defaults::hundred",
        skip_serializing_if = "defaults::is_hundred"
    )]
    #[schemars(range(min = 0.0, max = 100.0))]
    pub opacity: f64,
    /// Whether the layer starts hidden. `show` and `hide` animations change
    /// this over time. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub hidden: bool,
    /// Whether the editor refuses to select or move the layer. Has no effect
    /// on rendering. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub locked: bool,
    /// How the layer combines with what is behind it. Default `normal`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub blend_mode: BlendMode,
    /// Blur of the whole layer in pixels, 0 or more: twice the standard
    /// deviation of the Gaussian. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    #[schemars(range(min = 0.0))]
    pub blur: f64,
    /// Drop shadow. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// Outer glow, behind the layer. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
    /// Film grain over the layer's pixels. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<Noise>,
    /// Blur along one direction, on top of `blur`. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directional_blur: Option<DirectionalBlur>,
    /// Where an imported layer came from, for example
    /// `figma:<file key>:<node id>`. Re-importing a design matches layers by
    /// this value to keep their animations. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

impl Default for LayerProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            x: 0.0,
            y: 0.0,
            width: defaults::layer_size(),
            height: defaults::layer_size(),
            rotation: 0.0,
            scale: defaults::one(),
            opacity: defaults::hundred(),
            hidden: false,
            locked: false,
            blend_mode: BlendMode::default(),
            blur: 0.0,
            shadow: None,
            glow: None,
            noise: None,
            directional_blur: None,
            source_id: None,
        }
    }
}

impl LayerProps {
    /// Layer properties with a name and a box, everything else at its default.
    pub fn new(name: impl Into<String>, x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            name: name.into(),
            x,
            y,
            width,
            height,
            ..Self::default()
        }
    }

    /// The layer's box in its parent's coordinates, before rotation and scale.
    pub fn frame(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    /// The centre of the box in its parent's coordinates. Rotation and scale
    /// pivot here, so it does not move when they change.
    pub fn center(&self) -> Point {
        self.frame().center()
    }

    /// The transform from the layer's own coordinates (origin at the top-left
    /// corner of its box) to its parent's coordinates, at rest.
    pub fn transform(&self) -> Affine {
        Affine::layer(
            self.x,
            self.y,
            self.width,
            self.height,
            self.rotation,
            self.scale,
        )
    }
}

/// An artboard: a root of the document and the frame of one animation.
///
/// Its `x` and `y` place it on the editor canvas and do not affect rendering.
/// An artboard always clips its content to its box. It cannot be rotated,
/// scaled, blurred, faded or blended; those properties must stay at their
/// defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Artboard {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Length of the artboard's timeline in milliseconds, at least 1.
    /// Default 4000.
    #[serde(
        default = "defaults::artboard_duration",
        skip_serializing_if = "defaults::is_artboard_duration"
    )]
    #[schemars(range(min = 1.0))]
    pub duration: f64,
    /// Background. Default: none, a transparent artboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
}

/// A group: a box that holds other layers and moves them together.
///
/// Children are positioned relative to the group's top-left corner. The box
/// is explicit, not derived from the children, and can have its own fill,
/// stroke and rounded corners, like a frame.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Background drawn behind the children. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline of the box, drawn over the children. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Rounding of the box corners in pixels. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub corner_radius: CornerRadius,
    /// Whether children are cut off at the (rounded) box. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub clip: bool,
}

/// A mask group: its first child (lowest index) is an alpha mask for the rest.
///
/// The other children are only visible where the mask child is opaque. The
/// mask child itself is not drawn. Animating the mask child animates the mask.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskGroup {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
}

/// A rectangle, optionally with rounded corners.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RectLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Paint of the inside. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Rounding of the corners in pixels. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub corner_radius: CornerRadius,
}

/// An ellipse inscribed in the box. It can be cut down to an arc or a ring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EllipseLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Paint of the inside. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Where the arc starts, in degrees clockwise from the 3 o'clock
    /// position. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub start_angle: f64,
    /// How far the arc runs from its start, in degrees clockwise, from -360
    /// to 360. 360 is the whole ellipse. Default 360.
    #[serde(
        default = "defaults::full_turn",
        skip_serializing_if = "defaults::is_full_turn"
    )]
    #[schemars(range(min = -360.0, max = 360.0))]
    pub sweep: f64,
    /// Radius of the hole as a fraction of the outer radius, 0 to 1.
    /// 0 is a filled ellipse (a pie slice for an arc), larger values make a
    /// ring. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub inner_radius: f64,
}

impl Default for EllipseLayer {
    fn default() -> Self {
        Self {
            layer: LayerProps::default(),
            fill: None,
            stroke: None,
            start_angle: 0.0,
            sweep: defaults::full_turn(),
            inner_radius: 0.0,
        }
    }
}

/// A star inscribed in the box, with its first point at the top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Paint of the inside. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Number of points, 3 to 64. Default 5.
    #[serde(
        default = "defaults::star_points",
        skip_serializing_if = "defaults::is_star_points"
    )]
    #[schemars(range(min = 3, max = 64))]
    pub points: u32,
    /// Radius of the inner corners as a fraction of the outer radius, 0 to 1.
    /// 1 makes a regular polygon with twice as many corners. Default 0.4.
    #[serde(
        default = "defaults::star_inner_radius",
        skip_serializing_if = "defaults::is_star_inner_radius"
    )]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub inner_radius: f64,
}

impl Default for StarLayer {
    fn default() -> Self {
        Self {
            layer: LayerProps::default(),
            fill: None,
            stroke: None,
            points: defaults::star_points(),
            inner_radius: defaults::star_inner_radius(),
        }
    }
}

/// A shape drawn with the pen: straight and curved segments.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Paint of the inside. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// The outline, with coordinates normalised to the box: `[0, 0]` is its
    /// top-left corner and `[1, 1]` its bottom-right. Default: empty.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub path: PathData,
}

/// A block of text in one style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Paint of the glyphs. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline of the glyphs. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// The text. Line breaks are `\n`. Default: empty.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub content: String,
    /// The font. Default: Inter, weight 400, upright.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub font: Font,
    /// Font size in pixels, more than 0. Default 48.
    #[serde(
        default = "defaults::font_size",
        skip_serializing_if = "defaults::is_font_size"
    )]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub font_size: f64,
    /// Distance between baselines as a percentage of the font size, more
    /// than 0. Default 120.
    #[serde(
        default = "defaults::line_height",
        skip_serializing_if = "defaults::is_line_height"
    )]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub line_height: f64,
    /// Extra space after every letter as a percentage of the font size; may
    /// be negative. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub letter_spacing: f64,
    /// Horizontal alignment of lines in the box. Default `left`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub align: TextAlign,
    /// Vertical alignment of the text in the box. Default `top`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub vertical_align: VerticalAlign,
    /// How the box follows the content. Default `width_and_height`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub auto_size: AutoSize,
    /// Letter case applied when drawing. Default `original`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub case: TextCase,
}

impl Default for TextLayer {
    fn default() -> Self {
        Self {
            layer: LayerProps::default(),
            fill: None,
            stroke: None,
            content: String::new(),
            font: Font::default(),
            font_size: defaults::font_size(),
            line_height: defaults::line_height(),
            letter_spacing: 0.0,
            align: TextAlign::default(),
            vertical_align: VerticalAlign::default(),
            auto_size: AutoSize::default(),
            case: TextCase::default(),
        }
    }
}

/// A bitmap image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Where the image comes from: a path relative to the project file, an
    /// absolute path, or a `data:` URI.
    pub source: String,
    /// How the image fills the box when their proportions differ.
    /// Default `cover`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub fit: ImageFit,
    /// Outline of the box. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Rounding of the corners in pixels. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub corner_radius: CornerRadius,
}

/// A main component: a box of layers that instances show copies of.
///
/// It is drawn where it is, exactly like a [`Group`], and has the same
/// fields. What it draws itself (fill, stroke, corner radius, clip) and its
/// children, with the animations that target layers inside it, are what
/// every [`Instance`] of it shows. Its own placement and effects (position,
/// size, rotation, scale, opacity, blend mode, blur, shadow) and the
/// animations that target the component itself belong to where it is
/// placed, not to the component.
///
/// A component cannot contain another component, at any depth; instances
/// of other components inside it are fine.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Background drawn behind the children. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// Outline of the box, drawn over the children. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Rounding of the box corners in pixels. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub corner_radius: CornerRadius,
    /// Whether children are cut off at the (rounded) box. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub clip: bool,
}

impl From<Group> for Component {
    fn from(group: Group) -> Self {
        let Group {
            layer,
            fill,
            stroke,
            corner_radius,
            clip,
        } = group;
        Self {
            layer,
            fill,
            stroke,
            corner_radius,
            clip,
        }
    }
}

impl From<Component> for Group {
    fn from(component: Component) -> Self {
        let Component {
            layer,
            fill,
            stroke,
            corner_radius,
            clip,
        } = component;
        Self {
            layer,
            fill,
            stroke,
            corner_radius,
            clip,
        }
    }
}

/// An instance: a layer that shows a copy of a main [`Component`].
///
/// At time `t` it shows the component's look and children as the component
/// looks at `t - time_offset`, with every animation that targets a layer
/// inside the component. When its box differs from the component's, that
/// content is scaled to fill it; a resize animation on the instance changes
/// its box the way it changes a group's, not the content's scale. Its own
/// placement and effects, and the animations that target the instance, are
/// its own. It holds no layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Id of the main component this instance shows: a `component` layer
    /// anywhere in the document, in any artboard.
    pub component: NodeId,
    /// How much later than the component this instance plays the
    /// component's animations, in milliseconds; negative plays them
    /// earlier. Give each copy a little more to stagger them. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub time_offset: f64,
}

/// How an image fills its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// Scale to cover the whole box, cropping what does not fit.
    #[default]
    Cover,
    /// Scale to fit inside the box, leaving empty space.
    Contain,
    /// Stretch to the box, ignoring the image's proportions.
    Stretch,
}
