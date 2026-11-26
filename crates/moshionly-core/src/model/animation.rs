//! Animation types: the things that change layers over time.
//!
//! An animation says "between `start` and `end`, take this property of the
//! `target` layer from `from` to `to`, paced by `easing`". `from` and `to` are
//! optional; what a missing one means is defined by the combining rules in
//! `docs/file-format.md` and implemented in [`crate::eval`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::paint::{CornerRadius, Paint};
use crate::color::Color;
use crate::defaults;
use crate::easing::Easing;
use crate::id::NodeId;

/// When an animation runs, on what, and how it is paced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Timing {
    /// Label shown on the timeline row. Default: empty, the editor shows the type.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub name: String,
    /// Id of the layer this animation changes. It must be in the same
    /// artboard as the animation.
    pub target: NodeId,
    /// When the animation starts, in milliseconds from the start of the
    /// artboard, 0 or more.
    #[schemars(range(min = 0.0))]
    pub start: f64,
    /// When the animation ends, in milliseconds from the start of the
    /// artboard. Not before `start`; equal to `start` makes it instant.
    #[schemars(range(min = 0.0))]
    pub end: f64,
    /// How progress is paced between `start` and `end`.
    /// Default `{ "type": "smooth", "intensity": 50 }`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub easing: Easing,
}

impl Timing {
    /// Timing on `target` from `start` to `end` with the default easing.
    pub fn new(target: impl Into<NodeId>, start: f64, end: f64) -> Self {
        Self {
            name: String::new(),
            target: target.into(),
            start,
            end,
            easing: Easing::default(),
        }
    }

    /// The same timing with another easing.
    pub fn with_easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Length of the animation in milliseconds.
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

/// `move`: shifts the layer from its resting position. Additive.
///
/// `from` and `to` are offsets from where the layer rests, not positions.
/// Several moves on one layer add up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Offset at the start, in pixels. Default: no offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Offset>,
    /// Offset at the end, in pixels. Default: no offset, back at rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Offset>,
}

/// A distance on both axes, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Offset {
    /// Distance to the right, in pixels; negative moves left. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub x: f64,
    /// Distance downward, in pixels; negative moves up. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub y: f64,
}

impl Offset {
    /// An offset of `(x, y)` pixels.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// `rotate`: turns the layer about the centre of its box. Additive.
///
/// `from` and `to` are angles added to the layer's resting rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Extra rotation at the start, in degrees clockwise. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<f64>,
    /// Extra rotation at the end, in degrees clockwise. Default 0, back at rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<f64>,
}

/// `scale`: scales the layer about the centre of its box. Replacing.
///
/// Values are multipliers of the layer's resting `scale`: 1 is the resting
/// size, 2 is twice as large.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScaleAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Multiplier at the start, 0 or more. Default: the value at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub from: Option<f64>,
    /// Multiplier at the end, 0 or more. Default 1, the resting size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub to: Option<f64>,
}

/// `opacity`: fades the layer. Replacing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpacityAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Opacity at the start, 0 to 100. Default: the value at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 100.0))]
    pub from: Option<f64>,
    /// Opacity at the end, 0 to 100. Default: the layer's resting opacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 100.0))]
    pub to: Option<f64>,
}

/// `resize`: changes the size of the layer's box. Replacing.
///
/// The point of the box named by `anchor` stays where it is while the box
/// grows or shrinks around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResizeAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Size at the start. Default: the size at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<SizeChange>,
    /// Size at the end. Default: the layer's resting size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<SizeChange>,
    /// Which point of the box stays fixed. Default `center`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub anchor: ResizeAnchor,
}

/// A size, of which either side may be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SizeChange {
    /// Width in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub width: Option<f64>,
    /// Height in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub height: Option<f64>,
}

/// The point of a box that a `resize` keeps fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResizeAnchor {
    /// The top-left corner.
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top-right corner.
    TopRight,
    /// The middle of the left edge.
    Left,
    /// The centre of the box.
    #[default]
    Center,
    /// The middle of the right edge.
    Right,
    /// The bottom-left corner.
    BottomLeft,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom-right corner.
    BottomRight,
}

impl ResizeAnchor {
    /// All nine anchors, row by row from the top-left.
    pub const ALL: [ResizeAnchor; 9] = [
        ResizeAnchor::TopLeft,
        ResizeAnchor::Top,
        ResizeAnchor::TopRight,
        ResizeAnchor::Left,
        ResizeAnchor::Center,
        ResizeAnchor::Right,
        ResizeAnchor::BottomLeft,
        ResizeAnchor::Bottom,
        ResizeAnchor::BottomRight,
    ];

    /// Where the anchor sits in the box as fractions `(x, y)`: 0 is the left
    /// or top edge, 0.5 the middle, 1 the right or bottom edge.
    pub fn factors(self) -> (f64, f64) {
        match self {
            ResizeAnchor::TopLeft => (0.0, 0.0),
            ResizeAnchor::Top => (0.5, 0.0),
            ResizeAnchor::TopRight => (1.0, 0.0),
            ResizeAnchor::Left => (0.0, 0.5),
            ResizeAnchor::Center => (0.5, 0.5),
            ResizeAnchor::Right => (1.0, 0.5),
            ResizeAnchor::BottomLeft => (0.0, 1.0),
            ResizeAnchor::Bottom => (0.5, 1.0),
            ResizeAnchor::BottomRight => (1.0, 1.0),
        }
    }
}

/// `fill`: changes the fill of a layer, or the background of an artboard.
/// Replacing.
///
/// Colours blend channel by channel. Two gradients of the same kind blend
/// their geometry and stops; a colour blends with a gradient as if it were
/// that gradient in one colour. A linear and a radial gradient cannot blend
/// and switch halfway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Fill at the start. Default: the fill at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Paint>,
    /// Fill at the end. Default: the layer's resting fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Paint>,
}

/// `stroke`: changes the paint and width of a layer's stroke. Replacing.
///
/// The stroke's position, cap and join are taken from the layer and do not
/// animate. A layer without a stroke at rest is treated as having a
/// transparent stroke of width 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrokeAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Stroke at the start. Default: the stroke at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<StrokeChange>,
    /// Stroke at the end. Default: the layer's resting stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<StrokeChange>,
}

/// The animatable parts of a stroke, of which either may be left alone.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrokeChange {
    /// Colour or gradient of the stroke. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<Paint>,
    /// Width in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub width: Option<f64>,
}

/// `corner_radius`: changes the corner rounding of a rect, group or image.
/// Replacing. Each corner animates on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CornerRadiusAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Radius at the start: a number or four numbers. Default: the radius at
    /// that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<CornerRadius>,
    /// Radius at the end: a number or four numbers. Default: the layer's
    /// resting radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<CornerRadius>,
}

/// `blur`: changes the blur of the layer. Replacing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlurAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Blur at the start in pixels, 0 or more. Default: the blur at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub from: Option<f64>,
    /// Blur at the end in pixels, 0 or more. Default: the layer's resting blur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub to: Option<f64>,
}

/// `shadow`: changes the layer's drop shadow. Replacing.
///
/// A layer without a shadow at rest is treated as having a fully transparent
/// one, so a shadow can fade in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShadowAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Shadow at the start. Default: the shadow at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ShadowChange>,
    /// Shadow at the end. Default: the layer's resting shadow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<ShadowChange>,
}

/// The parts of a shadow, of which any may be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShadowChange {
    /// Shadow colour; its alpha is the strength. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Horizontal offset in pixels. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    /// Vertical offset in pixels. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Blur in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub blur: Option<f64>,
}

/// `glow`: changes the layer's outer glow. Replacing.
///
/// A layer without a glow at rest is treated as having the default glow,
/// fully transparent, so a glow can fade in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GlowAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Glow at the start. Default: the glow at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<GlowChange>,
    /// Glow at the end. Default: the layer's resting glow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<GlowChange>,
}

/// The parts of a glow, of which any may be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GlowChange {
    /// Glow colour; its alpha is the strength. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Softness in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub radius: Option<f64>,
    /// Growth of the shape in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub spread: Option<f64>,
}

/// `noise`: changes the strength and grain size of the layer's noise.
/// Replacing.
///
/// A layer without noise at rest is treated as having the default noise
/// with an amount of 0, so grain can fade in. Whether the grain moves is
/// the layer's own `noise.animated` and does not animate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoiseAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Noise at the start. Default: the noise at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<NoiseChange>,
    /// Noise at the end. Default: the layer's resting noise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<NoiseChange>,
}

/// The animatable parts of noise, of which either may be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoiseChange {
    /// Strength from 0 to 1. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub amount: Option<f64>,
    /// Size of one grain in pixels, more than 0. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub size: Option<f64>,
}

/// `directional_blur`: changes the layer's directional blur. Replacing.
///
/// A layer without a directional blur at rest is treated as having one of
/// amount 0 at angle 0, so the blur can grow in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectionalBlurAnimation {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Directional blur at the start. Default: the directional blur at that
    /// moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<DirectionalBlurChange>,
    /// Directional blur at the end. Default: the layer's resting
    /// directional blur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<DirectionalBlurChange>,
}

/// The parts of a directional blur, of which either may be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectionalBlurChange {
    /// Blur along the direction in pixels, 0 or more. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub amount: Option<f64>,
    /// Direction in degrees, clockwise from horizontal. Default: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
}

/// `show` and `hide`: switch the layer's visibility at one moment.
///
/// A layer with a `show` is hidden until the first one happens; a layer with
/// a `hide` first is visible until then. After that the most recent of them
/// decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstantAnimation {
    /// Label shown on the timeline row. Default: empty, the editor shows the type.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub name: String,
    /// Id of the layer to show or hide. It must be in the same artboard.
    pub target: NodeId,
    /// The moment it happens, in milliseconds from the start of the
    /// artboard, 0 or more.
    #[schemars(range(min = 0.0))]
    pub start: f64,
    /// Accepted so that every animation can be written with a `start` and an
    /// `end`, and ignored: the switch happens at `start`. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

impl InstantAnimation {
    /// An instant on `target` at `start`.
    pub fn new(target: impl Into<NodeId>, start: f64) -> Self {
        Self {
            name: String::new(),
            target: target.into(),
            start,
            end: None,
        }
    }
}

/// A direction of travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Toward the left.
    Left,
    /// Toward the right.
    Right,
    /// Toward the top.
    #[default]
    Up,
    /// Toward the bottom.
    Down,
}

impl Direction {
    /// The direction as a unit vector `(x, y)`, with y growing downward.
    pub fn vector(self) -> (f64, f64) {
        match self {
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
        }
    }
}

/// `fade_in` and `fade_out`: the layer fades from or to invisible.
///
/// A layer with a `fade_in` is invisible until it starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FadePreset {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
}

/// `slide_in` and `slide_out`: the layer travels into or out of its resting
/// position, fading as it goes.
///
/// `slide_in` starts `distance` away and arrives at rest; `slide_out` leaves
/// rest and ends `distance` away. Both move in `direction`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlidePreset {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// The direction the layer travels in. Default `up`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub direction: Direction,
    /// How far it travels, in pixels, 0 or more. Default 100.
    #[serde(
        default = "defaults::slide_distance",
        skip_serializing_if = "defaults::is_slide_distance"
    )]
    #[schemars(range(min = 0.0))]
    pub distance: f64,
    /// Whether the layer also fades. Without it the layer is visible the
    /// whole time. Default true.
    #[serde(default = "defaults::yes", skip_serializing_if = "defaults::is_yes")]
    pub fade: bool,
}

/// `grow_in`: the layer appears while growing from a smaller size to its
/// resting size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrowIn {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Size it starts at, as a multiplier of the resting size, 0 or more.
    /// Default 0.5.
    #[serde(
        default = "defaults::small_scale",
        skip_serializing_if = "defaults::is_small_scale"
    )]
    #[schemars(range(min = 0.0))]
    pub scale: f64,
    /// Whether the layer also fades in. Default true.
    #[serde(default = "defaults::yes", skip_serializing_if = "defaults::is_yes")]
    pub fade: bool,
}

/// `grow_out`: the layer disappears while growing from its resting size to a
/// larger one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrowOut {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Size it ends at, as a multiplier of the resting size, 0 or more.
    /// Default 1.5.
    #[serde(
        default = "defaults::large_scale",
        skip_serializing_if = "defaults::is_large_scale"
    )]
    #[schemars(range(min = 0.0))]
    pub scale: f64,
    /// Whether the layer also fades out. Default true.
    #[serde(default = "defaults::yes", skip_serializing_if = "defaults::is_yes")]
    pub fade: bool,
}

/// `shrink_in`: the layer appears while shrinking from a larger size to its
/// resting size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShrinkIn {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Size it starts at, as a multiplier of the resting size, 0 or more.
    /// Default 1.5.
    #[serde(
        default = "defaults::large_scale",
        skip_serializing_if = "defaults::is_large_scale"
    )]
    #[schemars(range(min = 0.0))]
    pub scale: f64,
    /// Whether the layer also fades in. Default true.
    #[serde(default = "defaults::yes", skip_serializing_if = "defaults::is_yes")]
    pub fade: bool,
}

/// `shrink_out`: the layer disappears while shrinking from its resting size
/// to a smaller one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShrinkOut {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Size it ends at, as a multiplier of the resting size, 0 or more.
    /// Default 0.5.
    #[serde(
        default = "defaults::small_scale",
        skip_serializing_if = "defaults::is_small_scale"
    )]
    #[schemars(range(min = 0.0))]
    pub scale: f64,
    /// Whether the layer also fades out. Default true.
    #[serde(default = "defaults::yes", skip_serializing_if = "defaults::is_yes")]
    pub fade: bool,
}

/// `spin_in` and `spin_out`: the layer turns and scales from or to nothing.
///
/// `spin_in` grows from a scale of 0 while turning through `angle` into its
/// resting rotation; `spin_out` shrinks to 0 while turning `angle` away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpinPreset {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// How far it turns, in degrees clockwise; negative turns the other way.
    /// Default 180.
    #[serde(
        default = "defaults::spin_angle",
        skip_serializing_if = "defaults::is_spin_angle"
    )]
    pub angle: f64,
    /// Whether the layer also fades. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub fade: bool,
}

/// `blur_in` and `blur_out`: the layer fades while coming into or going out
/// of focus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlurPreset {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Blur when out of focus, in pixels, 0 or more; added to the layer's
    /// own blur. Default 20.
    #[serde(
        default = "defaults::preset_blur",
        skip_serializing_if = "defaults::is_preset_blur"
    )]
    #[schemars(range(min = 0.0))]
    pub blur: f64,
}

/// `reveal_in` and `reveal_out`: a straight edge wipes across the layer,
/// uncovering or covering it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevealPreset {
    /// Target, time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// The direction the wipe edge travels in. With `right`, `reveal_in`
    /// uncovers the layer from its left side to its right. Default `right`.
    #[serde(
        default = "defaults::reveal_direction",
        skip_serializing_if = "defaults::is_reveal_direction"
    )]
    pub direction: Direction,
}

/// `text_in` and `text_out`: a text layer appears or disappears piece by piece.
///
/// The text is split into units. Each unit plays the same effect, one
/// starting `stagger` milliseconds after the other, and together they fill
/// the time from `start` to `end`. The `easing` paces each unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextAnimation {
    /// Target (a text layer), time span, and the easing of each unit.
    #[serde(flatten)]
    pub timing: Timing,
    /// What a unit is. Default `letters`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub split: TextSplit,
    /// What each unit does. Default `fade`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub effect: TextEffect,
    /// Delay between one unit starting and the next, in milliseconds, 0 or
    /// more. When the delays do not fit between `start` and `end` they are
    /// shortened so the units play one after another. Default 40.
    #[serde(
        default = "defaults::text_stagger",
        skip_serializing_if = "defaults::is_text_stagger"
    )]
    #[schemars(range(min = 0.0))]
    pub stagger: f64,
    /// Which unit goes first. Default `forward`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub order: TextOrder,
    /// Travel of the `slide_up` and `slide_down` effects as a percentage of
    /// the line height, 0 or more. Default 100.
    #[serde(
        default = "defaults::text_distance",
        skip_serializing_if = "defaults::is_text_distance"
    )]
    #[schemars(range(min = 0.0))]
    pub distance: f64,
    /// Size a unit starts at (`text_in`) or ends at (`text_out`) with the
    /// `scale` effect, as a multiplier, 0 or more. Default 0.5.
    #[serde(
        default = "defaults::small_scale",
        skip_serializing_if = "defaults::is_small_scale"
    )]
    #[schemars(range(min = 0.0))]
    pub scale: f64,
    /// Blur of an out-of-focus unit with the `blur` effect, in pixels, 0 or
    /// more. Default 20.
    #[serde(
        default = "defaults::preset_blur",
        skip_serializing_if = "defaults::is_preset_blur"
    )]
    #[schemars(range(min = 0.0))]
    pub blur: f64,
}

impl TextAnimation {
    /// A text animation with every setting at its default.
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            split: TextSplit::default(),
            effect: TextEffect::default(),
            stagger: defaults::text_stagger(),
            order: TextOrder::default(),
            distance: defaults::text_distance(),
            scale: defaults::small_scale(),
            blur: defaults::preset_blur(),
        }
    }
}

/// How a text animation cuts the text into units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextSplit {
    /// Every letter on its own (spaces are skipped).
    #[default]
    Letters,
    /// Every word on its own.
    Words,
    /// Every line on its own, after wrapping.
    Lines,
}

/// What each unit of a text animation does.
///
/// Every effect includes a fade, so units are invisible before they come in
/// and after they go out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextEffect {
    /// Fades only.
    #[default]
    Fade,
    /// Travels upward: `text_in` rises into place, `text_out` rises away.
    SlideUp,
    /// Travels downward: `text_in` drops into place, `text_out` drops away.
    SlideDown,
    /// Scales about the unit's centre.
    Scale,
    /// Comes into or goes out of focus.
    Blur,
}

/// Which unit of a text animation goes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextOrder {
    /// From the first unit to the last.
    #[default]
    Forward,
    /// From the last unit to the first.
    Backward,
}

/// `counter`: counts the first number in a text layer's content up or down.
/// Replacing.
///
/// The text around the number, its decimals and its thousands separators are
/// kept: `"$1,250.50 raised"` counted from 0 shows `"$0.00 raised"`,
/// `"$625.25 raised"` and so on. A text layer without a number is left alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CounterAnimation {
    /// Target (a text layer), time span and easing.
    #[serde(flatten)]
    pub timing: Timing,
    /// Number shown at the start. Default: the number at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<f64>,
    /// Number shown at the end. Default: the number written in the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<f64>,
}

/// `animation_group`: a folder on the timeline. It has no effect on playback.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnimationGroup {
    /// Label shown on the timeline row. Default: empty.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub name: String,
}

/// The broad kinds of animation, as the timeline groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnimationKind {
    /// Changes one property between two values.
    Property,
    /// Happens at one moment: `show` and `hide`.
    Instant,
    /// A ready-made entrance or exit.
    Preset,
    /// Works on the content of a text layer.
    Text,
    /// Plays sound on the timeline and changes no layer: `audio`.
    Media,
    /// Organises other animations: `animation_group`.
    Structure,
}
