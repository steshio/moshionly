//! The document model: nodes and the values their properties hold.

pub mod paint;

pub use paint::{
    BlendMode, CornerRadius, Gradient, GradientStop, LinearGradient, Paint, RadialGradient, Shadow,
    Stroke, StrokeCap, StrokeJoin, StrokePosition,
};
