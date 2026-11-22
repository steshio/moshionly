//! Default values of optional properties.
//!
//! Every optional property has one default, defined here or as the type's
//! `Default`. Serialising leaves properties at their default out, so files
//! stay small.

/// `skip_serializing_if` helper for properties whose default is `Default::default()`.
pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Defines a named default: a function that returns it (for `serde(default = ..)`)
/// and a function that says whether a value equals it (for `skip_serializing_if`).
macro_rules! named_default {
    ($(#[$doc:meta])* $get:ident, $skip:ident, $ty:ty, $value:expr) => {
        $(#[$doc])*
        pub(crate) fn $get() -> $ty {
            $value
        }

        pub(crate) fn $skip(value: &$ty) -> bool {
            *value == $value
        }
    };
}

named_default!(
    /// Stroke width in pixels.
    stroke_width,
    is_stroke_width,
    f64,
    1.0
);
named_default!(
    /// Vertical offset of a new shadow, in pixels.
    shadow_y,
    is_shadow_y,
    f64,
    4.0
);
named_default!(
    /// Blur of a new shadow, in pixels.
    shadow_blur,
    is_shadow_blur,
    f64,
    12.0
);
named_default!(
    /// Colour of a new shadow: black at a quarter opacity.
    shadow_color,
    is_shadow_color,
    crate::Color,
    crate::Color::rgba(0, 0, 0, 0x40)
);
named_default!(
    /// Paint of a new stroke: opaque black.
    stroke_paint,
    is_stroke_paint,
    crate::model::Paint,
    crate::model::Paint::Solid(crate::Color::BLACK)
);
named_default!(
    /// Start of a linear gradient: the middle of the top edge.
    gradient_start,
    is_gradient_start,
    [f64; 2],
    [0.5, 0.0]
);
named_default!(
    /// End of a linear gradient: the middle of the bottom edge.
    gradient_end,
    is_gradient_end,
    [f64; 2],
    [0.5, 1.0]
);
named_default!(
    /// Centre of a radial gradient, and its radius: both half the box.
    half_half,
    is_half_half,
    [f64; 2],
    [0.5, 0.5]
);
named_default!(
    /// Easing intensity that every curve is tuned around.
    intensity,
    is_intensity,
    f64,
    50.0
);
