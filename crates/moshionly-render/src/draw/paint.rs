//! Scene paints as Skia paints: one colour, or a linear or radial gradient.

use moshionly_core::model::GradientStop;
use moshionly_core::scene::ScenePaint;
use skia_safe as sk;
use skia_safe::gradient::{Colors, Gradient, Interpolation, interpolation, shaders};

use crate::convert;

/// A Skia paint that fills with `paint`, or `None` when there is nothing to
/// see.
///
/// `opacity` (0 to 1) fades the whole paint. Gradient geometry is in the
/// coordinates the shape is drawn in.
pub(crate) fn fill(paint: &ScenePaint, opacity: f32) -> Option<sk::Paint> {
    if opacity.is_nan() || opacity <= 0.0 {
        return None;
    }
    let opacity = opacity.min(1.0);
    let mut result = sk::Paint::default();
    result.set_anti_alias(true);
    match paint {
        ScenePaint::Solid { color } => {
            let mut color = convert::color(*color);
            color.a *= opacity;
            result.set_color4f(color, None);
        }
        ScenePaint::LinearGradient { start, end, stops } => {
            set_gradient(&mut result, stops, opacity, |gradient| {
                shaders::linear_gradient(
                    (convert::point(*start), convert::point(*end)),
                    gradient,
                    None,
                )
            })?;
        }
        ScenePaint::RadialGradient {
            center,
            radius_x,
            radius_y,
            rotation,
            stops,
        } => {
            // A unit circle, stretched to the ellipse, turned and moved.
            let mut placement = sk::Matrix::translate(convert::point(*center));
            placement.pre_rotate(*rotation as f32, None);
            placement.pre_scale((*radius_x as f32, *radius_y as f32), None);
            set_gradient(&mut result, stops, opacity, |gradient| {
                shaders::radial_gradient((sk::Point::new(0.0, 0.0), 1.0), gradient, &placement)
            })?;
        }
    }
    (result.alpha_f() > 0.0).then_some(result)
}

/// Points `paint` at a gradient built by `shader`. `None` when the gradient
/// has no stops.
fn set_gradient(
    paint: &mut sk::Paint,
    stops: &[GradientStop],
    opacity: f32,
    shader: impl FnOnce(&Gradient<'_>) -> Option<sk::Shader>,
) -> Option<()> {
    let (colors, offsets) = clean_stops(stops);
    let last = *colors.last()?;
    paint.set_alpha_f(opacity);
    // Colours are blended with their alpha multiplied in, as the engine
    // blends colours over time: a fade to a transparent colour stays clean
    // instead of passing through the transparent colour's hue.
    let interpolation = Interpolation {
        in_premul: interpolation::InPremul::Yes,
        color_space: interpolation::ColorSpace::Destination,
        hue_method: interpolation::HueMethod::Shorter,
    };
    let gradient = Gradient::new(
        Colors::new(&colors, Some(&offsets), sk::TileMode::Clamp, None),
        interpolation,
    );
    let shader = if colors.len() > 1 {
        shader(&gradient)
    } else {
        None
    };
    match shader {
        Some(shader) => {
            paint.set_shader(shader);
            // 8 bits per channel band visibly on long, soft gradients.
            paint.set_dither(true);
        }
        // One stop, or geometry without extent (a zero radius, say): what
        // is left of such a gradient is its last colour.
        None => {
            let mut color = last;
            color.a *= opacity;
            paint.set_color4f(color, None);
        }
    }
    Some(())
}

/// Stops as Skia wants them: offsets from 0 to 1 that never decrease.
fn clean_stops(stops: &[GradientStop]) -> (Vec<sk::Color4f>, Vec<f32>) {
    let mut reached = 0.0f32;
    stops
        .iter()
        .map(|stop| {
            let offset = if stop.offset.is_finite() {
                stop.offset.clamp(0.0, 1.0) as f32
            } else {
                reached
            };
            reached = reached.max(offset);
            (convert::color(stop.color), reached)
        })
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::{Color, Point};

    /// Fills a 100 by 20 surface with `paint` and reads back premultiplied
    /// RGBA at `x` along the middle row.
    fn sample(paint: &ScenePaint, opacity: f32, xs: &[i32]) -> Vec<[u8; 4]> {
        let info = sk::ImageInfo::new(
            (100, 20),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Premul,
            None,
        );
        let mut surface = sk::surfaces::raster(&info, None, None).unwrap();
        if let Some(paint) = fill(paint, opacity) {
            surface.canvas().draw_paint(&paint);
        }
        let mut pixels = vec![0u8; 100 * 20 * 4];
        assert!(surface.read_pixels(&info, &mut pixels, 400, (0, 0)));
        xs.iter()
            .map(|x| {
                let at = (10 * 100 + *x as usize) * 4;
                pixels[at..at + 4].try_into().unwrap()
            })
            .collect()
    }

    fn close(actual: [u8; 4], expected: [u8; 4]) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 2)
    }

    fn stops(colors: &[(f64, Color)]) -> Vec<GradientStop> {
        colors
            .iter()
            .map(|(offset, color)| GradientStop::new(*offset, *color))
            .collect()
    }

    #[test]
    fn solid_paints_carry_colour_alpha_and_opacity() {
        let red = ScenePaint::solid(Color::rgb(255, 0, 0));
        assert_eq!(sample(&red, 1.0, &[50]), [[255, 0, 0, 255]]);
        // Premultiplied: half opacity halves the colour too.
        assert!(close(sample(&red, 0.5, &[50])[0], [128, 0, 0, 128]));
        let faint = ScenePaint::solid(Color::rgba(0, 0, 255, 128));
        assert!(close(sample(&faint, 0.5, &[50])[0], [0, 0, 64, 64]));
        assert!(fill(&red, 0.0).is_none());
        assert!(fill(&red, f32::NAN).is_none());
        assert!(fill(&ScenePaint::solid(Color::TRANSPARENT), 1.0).is_none());
    }

    #[test]
    fn linear_gradients_run_from_start_to_end() {
        let gradient = ScenePaint::LinearGradient {
            start: Point::new(20.0, 0.0),
            end: Point::new(80.0, 0.0),
            stops: stops(&[(0.0, Color::BLACK), (1.0, Color::WHITE)]),
        };
        let pixels = sample(&gradient, 1.0, &[5, 20, 50, 80, 95]);
        // Before the start and past the end the end colours carry on.
        assert!(close(pixels[0], [0, 0, 0, 255]), "{pixels:?}");
        assert!(close(pixels[1], [2, 2, 2, 255]), "{pixels:?}");
        assert!(close(pixels[2], [130, 130, 130, 255]), "{pixels:?}");
        assert!(close(pixels[3], [255, 255, 255, 255]), "{pixels:?}");
        assert!(close(pixels[4], [255, 255, 255, 255]), "{pixels:?}");
    }

    #[test]
    fn fading_to_transparent_keeps_the_hue() {
        // Opaque red to transparent blue: no purple on the way.
        let gradient = ScenePaint::LinearGradient {
            start: Point::new(0.0, 0.0),
            end: Point::new(100.0, 0.0),
            stops: stops(&[
                (0.0, Color::rgb(255, 0, 0)),
                (1.0, Color::rgba(0, 0, 255, 0)),
            ]),
        };
        let middle = sample(&gradient, 1.0, &[50])[0];
        assert!(close(middle, [126, 0, 0, 126]), "{middle:?}");
    }

    #[test]
    fn radial_gradients_spread_from_the_centre_of_a_turned_ellipse() {
        // Centred on a pixel centre of the sampled row, so that offsets
        // from the centre are whole pixels.
        let radial = |rotation: f64| ScenePaint::RadialGradient {
            center: Point::new(50.5, 10.5),
            radius_x: 40.0,
            radius_y: 5.0,
            rotation,
            stops: stops(&[(0.0, Color::WHITE), (1.0, Color::BLACK)]),
        };
        let wide = sample(&radial(0.0), 1.0, &[50, 70, 90, 99]);
        assert!(close(wide[0], [255, 255, 255, 255]), "{wide:?}");
        assert!(close(wide[1], [128, 128, 128, 255]), "{wide:?}");
        assert!(close(wide[2], [0, 0, 0, 255]), "{wide:?}");
        assert!(close(wide[3], [0, 0, 0, 255]), "{wide:?}");
        // Turned a quarter, the short radius lies along the row: three
        // pixels out is 60% of the way.
        let tall = sample(&radial(90.0), 1.0, &[50, 53, 56]);
        assert!(close(tall[1], [102, 102, 102, 255]), "{tall:?}");
        assert!(close(tall[2], [0, 0, 0, 255]), "{tall:?}");
    }

    #[test]
    fn broken_gradients_still_paint_something_sensible() {
        // No stops: nothing. One stop: that colour.
        let empty = ScenePaint::LinearGradient {
            start: Point::ZERO,
            end: Point::new(100.0, 0.0),
            stops: Vec::new(),
        };
        assert!(fill(&empty, 1.0).is_none());
        let one = ScenePaint::LinearGradient {
            start: Point::ZERO,
            end: Point::new(100.0, 0.0),
            stops: stops(&[(0.3, Color::rgb(10, 20, 30))]),
        };
        assert_eq!(sample(&one, 1.0, &[0, 99]), [[10, 20, 30, 255]; 2]);
        // A radius of nothing leaves the outermost colour everywhere.
        let flat = ScenePaint::RadialGradient {
            center: Point::new(50.0, 10.0),
            radius_x: 0.0,
            radius_y: 10.0,
            rotation: 0.0,
            stops: stops(&[(0.0, Color::WHITE), (1.0, Color::rgb(0, 128, 0))]),
        };
        assert_eq!(sample(&flat, 1.0, &[10, 50]), [[0, 128, 0, 255]; 2]);
        // Offsets out of order or out of range are pulled into line.
        let (_, offsets) = clean_stops(&stops(&[
            (0.5, Color::BLACK),
            (0.2, Color::WHITE),
            (f64::NAN, Color::BLACK),
            (7.0, Color::WHITE),
        ]));
        assert_eq!(offsets, [0.5, 0.5, 0.5, 1.0]);
    }
}
