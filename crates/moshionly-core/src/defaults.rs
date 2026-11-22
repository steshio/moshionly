//! Default values, and the switch that decides whether they are written out.
//!
//! Every optional property has one default, defined here or as the type's
//! `Default`. Serialising normally leaves properties at their default out, so
//! files stay small. Two callers need the opposite: the JSON Schema (which
//! shows defaults) and "full" node dumps for inspectors and AI agents. They
//! wrap their work in [`with_defaults`], which makes every `is_*` helper in
//! this module answer "do not skip" on the current thread.

use std::cell::Cell;

thread_local! {
    static WRITE_DEFAULTS: Cell<bool> = const { Cell::new(false) };
}

/// Whether defaults are currently being written out on this thread.
fn writing_defaults() -> bool {
    WRITE_DEFAULTS.with(Cell::get)
}

/// Runs `work` with default values written out instead of skipped.
pub(crate) fn with_defaults<R>(work: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WRITE_DEFAULTS.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(WRITE_DEFAULTS.with(|flag| flag.replace(true)));
    work()
}

/// `skip_serializing_if` helper for properties whose default is `Default::default()`.
pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    !writing_defaults() && *value == T::default()
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
            !writing_defaults() && *value == $value
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
named_default!(
    /// A multiplier that changes nothing.
    one,
    is_one,
    f64,
    1.0
);
named_default!(
    /// Fully opaque, as a percentage.
    hundred,
    is_hundred,
    f64,
    100.0
);
named_default!(
    /// Width and height of a new layer, in pixels.
    layer_size,
    is_layer_size,
    f64,
    100.0
);
named_default!(
    /// Length of a new artboard's timeline, in milliseconds.
    artboard_duration,
    is_artboard_duration,
    f64,
    4000.0
);
named_default!(
    /// A full turn, in degrees.
    full_turn,
    is_full_turn,
    f64,
    360.0
);
named_default!(
    /// Points on a new star.
    star_points,
    is_star_points,
    u32,
    5
);
named_default!(
    /// Inner radius of a new star as a fraction of its outer radius.
    star_inner_radius,
    is_star_inner_radius,
    f64,
    0.4
);
named_default!(
    /// Family of the default font.
    font_family,
    is_font_family,
    String,
    "Inter".to_owned()
);
named_default!(
    /// Font size of new text, in pixels.
    font_size,
    is_font_size,
    f64,
    48.0
);
named_default!(
    /// Line height of new text, as a percentage of the font size.
    line_height,
    is_line_height,
    f64,
    120.0
);
named_default!(
    /// Regular font weight.
    font_weight,
    is_font_weight,
    u16,
    400
);
named_default!(
    /// Travel distance of slide presets, in pixels.
    slide_distance,
    is_slide_distance,
    f64,
    100.0
);
named_default!(
    /// Starting scale of `grow_in` and end scale of `shrink_out`.
    small_scale,
    is_small_scale,
    f64,
    0.5
);
named_default!(
    /// Starting scale of `shrink_in` and end scale of `grow_out`.
    large_scale,
    is_large_scale,
    f64,
    1.5
);
named_default!(
    /// Rotation of spin presets, in degrees.
    spin_angle,
    is_spin_angle,
    f64,
    180.0
);
named_default!(
    /// Blur of blur presets and the text blur effect, in pixels.
    preset_blur,
    is_preset_blur,
    f64,
    20.0
);
named_default!(
    /// Direction of the reveal presets' wipe edge.
    reveal_direction,
    is_reveal_direction,
    crate::model::Direction,
    crate::model::Direction::Right
);
named_default!(
    /// Delay between two text units starting, in milliseconds.
    text_stagger,
    is_text_stagger,
    f64,
    40.0
);
named_default!(
    /// Travel of the text slide effects, as a percentage of the line height.
    text_distance,
    is_text_distance,
    f64,
    100.0
);
named_default!(
    /// A flag that is on unless turned off.
    yes,
    is_yes,
    bool,
    true
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_skipped_unless_forced() {
        assert!(is_default(&0.0_f64));
        assert!(is_hundred(&100.0));
        assert!(!is_hundred(&99.0));
        with_defaults(|| {
            assert!(!is_default(&0.0_f64));
            assert!(!is_hundred(&100.0));
        });
        assert!(is_hundred(&100.0), "the switch resets afterwards");
    }

    #[test]
    fn nested_scopes_restore_the_outer_state() {
        with_defaults(|| {
            with_defaults(|| assert!(!is_one(&1.0)));
            assert!(!is_one(&1.0), "still forced inside the outer scope");
        });
        assert!(is_one(&1.0));
    }
}
