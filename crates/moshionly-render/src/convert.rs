//! Conversions from the scene's plain values to Skia's.
//!
//! The scene works in `f64`, Skia in `f32`. These helpers are the only
//! place the two meet.

use moshionly_core::{Affine, Color, Point, Rect};
use skia_safe as sk;

/// A Skia matrix for a scene transform.
pub(crate) fn matrix(affine: &Affine) -> sk::Matrix {
    let [a, b, c, d, e, f] = affine.0;
    sk::Matrix::new_all(
        a as f32, c as f32, e as f32, b as f32, d as f32, f as f32, 0.0, 0.0, 1.0,
    )
}

/// Whether every number of a transform is finite. Skia must not be handed
/// anything else.
pub(crate) fn is_finite(affine: &Affine) -> bool {
    affine.0.iter().all(|value| value.is_finite())
}

/// A Skia point.
pub(crate) fn point(point: Point) -> sk::Point {
    sk::Point::new(point.x as f32, point.y as f32)
}

/// A Skia rectangle.
pub(crate) fn rect(rect: &Rect) -> sk::Rect {
    sk::Rect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    )
}

/// A Skia rounded rectangle from a box and its radii in the scene's order:
/// top-left, top-right, bottom-right, bottom-left. Negative radii count as 0.
pub(crate) fn rounded_rect(bounds: &Rect, radii: &[f64; 4]) -> sk::RRect {
    let corner = |radius: f64| {
        let radius = if radius.is_finite() {
            radius.max(0.0) as f32
        } else {
            0.0
        };
        sk::Vector::new(radius, radius)
    };
    sk::RRect::new_rect_radii(rect(bounds), &radii.map(corner))
}

/// A Skia colour, unpremultiplied, each channel from 0 to 1.
pub(crate) fn color(color: Color) -> sk::Color4f {
    sk::Color4f::new(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        f32::from(color.a) / 255.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_map_points_like_the_scene_does() {
        let affine = Affine::layer(30.0, 40.0, 100.0, 50.0, 33.0, 1.5);
        let expected = affine.apply(Point::new(7.0, 9.0));
        let mapped = matrix(&affine).map_point(sk::Point::new(7.0, 9.0));
        assert!(
            (f64::from(mapped.x) - expected.x).abs() < 1e-3,
            "{mapped:?}"
        );
        assert!(
            (f64::from(mapped.y) - expected.y).abs() < 1e-3,
            "{mapped:?}"
        );
        assert!(is_finite(&affine));
        assert!(!is_finite(&Affine([1.0, 0.0, 0.0, 1.0, f64::NAN, 0.0])));
    }

    #[test]
    fn rounded_rects_keep_the_corner_order() {
        let rounded = rounded_rect(
            &Rect::new(0.0, 0.0, 100.0, 60.0),
            &[1.0, 2.0, 3.0, f64::NAN],
        );
        assert_eq!(rounded.radii(sk::rrect::Corner::UpperLeft).x, 1.0);
        assert_eq!(rounded.radii(sk::rrect::Corner::UpperRight).x, 2.0);
        assert_eq!(rounded.radii(sk::rrect::Corner::LowerRight).x, 3.0);
        assert_eq!(rounded.radii(sk::rrect::Corner::LowerLeft).x, 0.0);
    }

    #[test]
    fn colours_keep_their_alpha_apart() {
        let converted = color(Color::rgba(255, 0, 51, 128));
        assert_eq!((converted.r, converted.g), (1.0, 0.0));
        assert!((converted.b - 0.2).abs() < 1e-6);
        assert!((converted.a - 128.0 / 255.0).abs() < 1e-6);
    }
}
