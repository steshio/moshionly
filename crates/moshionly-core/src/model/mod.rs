//! The document model: nodes and the values their properties hold.
//!
//! Node types are defined once, here. The file format, the validator, the
//! JSON Schema and the reference documentation are all derived from these
//! definitions, so a new property is added in exactly one place.

pub mod animation;
pub mod effect;
pub mod layer;
pub mod media;
pub mod node;
pub mod paint;
pub mod path;
pub mod text;

pub use animation::{
    AnimationGroup, AnimationKind, BlurAnimation, BlurPreset, CornerRadiusAnimation,
    CounterAnimation, Direction, DirectionalBlurAnimation, DirectionalBlurChange, FadePreset,
    FillAnimation, GlowAnimation, GlowChange, GrowIn, GrowOut, InstantAnimation, MoveAnimation,
    NoiseAnimation, NoiseChange, Offset, OpacityAnimation, ResizeAnchor, ResizeAnimation,
    RevealPreset, RotateAnimation, ScaleAnimation, ShadowAnimation, ShadowChange, ShrinkIn,
    ShrinkOut, SizeChange, SlidePreset, SpinPreset, StrokeAnimation, StrokeChange, TextAnimation,
    TextEffect, TextOrder, TextSplit, Timing,
};
pub use effect::{DirectionalBlur, Glow, Noise};
pub use layer::{
    Artboard, Component, EllipseLayer, Group, ImageFit, ImageLayer, Instance, LayerProps,
    MaskGroup, PathLayer, RectLayer, StarLayer, TextLayer,
};
pub use media::{Audio, ClipTiming, VideoLayer};
pub use node::{Family, Node, NodeData, NodeType};
pub use paint::{
    BlendMode, CornerRadius, Gradient, GradientStop, LinearGradient, Paint, RadialGradient, Shadow,
    Stroke, StrokeCap, StrokeJoin, StrokePosition,
};
pub use path::{Anchor, FillRule, PathData, Subpath};
pub use text::{AutoSize, Font, TextAlign, TextCase, VerticalAlign};
