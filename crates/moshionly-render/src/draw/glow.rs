//! The outer glow: a node's shape, grown by the spread, softened by the
//! radius and painted in the glow's colour, drawn behind the node.
//!
//! Growing a shape exactly (Skia's dilate filter) costs Skia's CPU code
//! time in proportion to the spread: about 55 ms for each pixel of spread
//! on a 1920 by 1080 layer. So the glow is made from one blur instead, and
//! an alpha curve does the growing: for a straight edge, the blurred shape's
//! alpha at a distance from the edge says how far away the edge is, so it
//! can be mapped straight to the alpha the grown and softened edge has
//! there. That costs one blur whatever the spread. Corners come out round
//! and very thin parts grow a little less than wide ones, which a glow,
//! soft by nature, hides.

use moshionly_core::model::Glow;
use skia_safe as sk;

use super::effects::sigma;
use crate::convert;

/// The softness of a glow with no radius, as a standard deviation in the
/// node's units: just enough for a smooth edge.
const EDGE: f64 = 0.4;

/// A filter that draws only the glow of what the node draws, or `None` for
/// a glow that cannot be seen.
pub(crate) fn filter(glow: &Glow) -> Option<sk::ImageFilter> {
    if glow.color.a == 0 {
        return None;
    }
    let shape = Shape::of(glow);
    let blurred = if shape.blur > 0.0 {
        let blur = shape.blur as f32;
        sk::image_filters::blur((blur, blur), None, None, None)
    } else {
        None
    };
    let paint = sk::color_filters::blend_with_color_space(
        convert::color(glow.color),
        None,
        sk::BlendMode::SrcIn,
    )?;
    let paint = match shape.curve() {
        Some(curve) => {
            let grow = sk::color_filters::table_argb(&curve, None, None, None)?;
            sk::color_filters::compose(paint, grow)?
        }
        None => paint,
    };
    sk::image_filters::color_filter(paint, blurred, None)
}

/// How far a glow reaches past the shape on every side, in the node's
/// units: as far as the blur it is made from.
pub(crate) fn reach(glow: &Glow) -> f64 {
    if glow.color.a == 0 {
        return 0.0;
    }
    3.0 * Shape::of(glow).blur
}

/// The numbers a glow is made from, in the node's units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Shape {
    /// How far the shape grows.
    spread: f64,
    /// The standard deviation the glow should have, from its radius.
    soft: f64,
    /// The standard deviation of the blur actually done: wider than `soft`
    /// when the shape grows, so that the blurred alpha still says where
    /// the grown edge is, out to where the glow fades.
    blur: f64,
}

impl Shape {
    fn of(glow: &Glow) -> Self {
        let usable = |value: f64| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        let spread = usable(glow.spread);
        let soft = sigma(usable(glow.radius)).map_or(0.0, f64::from);
        let blur = if spread > 0.0 {
            // The blurred alpha runs out (below half a level in 8 bits)
            // about 2.9 deviations out; the grown, softened edge has faded
            // 2.5 deviations past the spread.
            soft.max(spread / 2.0).max((spread + 2.5 * soft) / 2.9)
        } else {
            soft
        };
        Self { spread, soft, blur }
    }

    /// The alpha table that turns the blurred shape into the grown and
    /// softened one, or `None` when there is no growing to do.
    ///
    /// Behind a straight edge, a shape blurred with deviation `blur` has
    /// alpha `Φ(-d / blur)` at distance `d` outside the edge, so `d` is
    /// `-blur · Φ⁻¹(alpha)`; the grown, softened edge has alpha
    /// `Φ((spread - d) / soft)` there.
    fn curve(&self) -> Option<[u8; 256]> {
        if self.spread <= 0.0 {
            return None;
        }
        let soft = self.soft.max(EDGE);
        let mut table = [0u8; 256];
        for (level, out) in table.iter_mut().enumerate() {
            let alpha = match level {
                0 => 0.0,
                255 => 1.0,
                _ => {
                    let distance = -self.blur * quantile(level as f64 / 255.0);
                    normal((self.spread - distance) / soft)
                }
            };
            *out = (alpha * 255.0).round() as u8;
        }
        Some(table)
    }
}

/// The standard normal distribution: the share of it below `x`.
fn normal(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// The inverse of [`normal`], for a share strictly between 0 and 1.
fn quantile(share: f64) -> f64 {
    let (mut low, mut high) = (-10.0, 10.0);
    for _ in 0..64 {
        let middle = (low + high) / 2.0;
        if normal(middle) < share {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

/// The complementary error function, to about seven digits (the Chebyshev
/// fit from *Numerical Recipes*).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let value = t * poly.exp();
    if x >= 0.0 { value } else { 2.0 - value }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::Color;

    fn glow(radius: f64, spread: f64) -> Glow {
        Glow {
            color: Color::WHITE,
            radius,
            spread,
        }
    }

    #[test]
    fn the_normal_distribution_and_its_inverse() {
        assert!((normal(0.0) - 0.5).abs() < 1e-7);
        assert!((normal(1.0) - 0.841_344_7).abs() < 1e-6);
        assert!((normal(-2.0) - 0.022_750_1).abs() < 1e-6);
        for x in [-3.0, -1.2, 0.0, 0.7, 2.5] {
            assert!((quantile(normal(x)) - x).abs() < 1e-6, "{x}");
        }
    }

    #[test]
    fn a_glow_without_spread_is_a_plain_blur() {
        let shape = Shape::of(&glow(8.0, 0.0));
        assert_eq!((shape.soft, shape.blur), (4.0, 4.0));
        assert_eq!(shape.curve(), None);
        assert_eq!(reach(&glow(8.0, 0.0)), 12.0);
        assert_eq!(reach(&glow(0.0, 0.0)), 0.0);
    }

    #[test]
    fn the_curve_grows_the_edge_by_the_spread() {
        // No radius: the grown edge is sharp, so alpha jumps from nothing
        // to full where the blurred alpha says the distance is the spread.
        let shape = Shape::of(&glow(0.0, 6.0));
        assert_eq!(shape.blur, 3.0);
        let curve = shape.curve().unwrap();
        let level_at = |distance: f64| (normal(-distance / shape.blur) * 255.0).round() as usize;
        assert_eq!(curve[level_at(4.0)], 255, "inside the grown edge");
        assert_eq!(curve[level_at(8.0)], 0, "outside it");
        assert!(curve.windows(2).all(|pair| pair[0] <= pair[1]), "rises");
        assert_eq!((curve[0], curve[255]), (0, 255));

        // With a radius the grown edge is soft: half alpha at the spread.
        let soft = Shape::of(&glow(8.0, 6.0));
        let curve = soft.curve().unwrap();
        let level = (normal(-6.0 / soft.blur) * 255.0).round() as usize;
        assert!(curve[level].abs_diff(128) <= 8, "{}", curve[level]);
        assert!(reach(&glow(8.0, 6.0)) >= 6.0 + 2.5 * 4.0);
    }

    #[test]
    fn invisible_glows_have_no_filter() {
        let clear = Glow {
            color: Color::TRANSPARENT,
            ..Glow::default()
        };
        assert!(filter(&clear).is_none());
        assert_eq!(reach(&clear), 0.0);
        assert!(filter(&glow(0.0, 0.0)).is_some());
        assert!(filter(&glow(10.0, 4.0)).is_some());
    }
}
