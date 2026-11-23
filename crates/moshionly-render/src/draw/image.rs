//! Drawing image layers: the picture fitted into the box, cut to its
//! rounded corners, and the box's outline on top.

use moshionly_core::Rect;
use moshionly_core::model::ImageFit;
use moshionly_core::scene::SceneStroke;
use skia_safe as sk;

use super::geometry::{self, Outline};
use super::stroke;
use crate::convert;

/// Colours of the placeholder drawn where an image cannot be loaded.
const PLACEHOLDER_FILL: sk::Color = sk::Color::from_argb(255, 229, 231, 235);
const PLACEHOLDER_LINES: sk::Color = sk::Color::from_argb(255, 156, 163, 175);

/// What an image layer draws.
pub(crate) struct ImageBox<'a> {
    /// The box's width and height.
    pub size: (f64, f64),
    /// Corner radii, as the scene gives them.
    pub radii: &'a [f64; 4],
    /// How the picture fills the box.
    pub fit: ImageFit,
    /// The outline of the box.
    pub stroke: Option<&'a SceneStroke>,
}

/// Draws the picture, or a placeholder when there is none, faded by
/// `opacity`.
pub(crate) fn draw(
    canvas: &sk::Canvas,
    frame: &ImageBox<'_>,
    picture: Option<&sk::Image>,
    opacity: f32,
) {
    let (width, height) = frame.size;
    let outline = Outline::new(geometry::rect(width, height, frame.radii), true);
    canvas.save();
    canvas.clip_path(&outline.path, sk::ClipOp::Intersect, true);
    match picture {
        Some(picture) => {
            let size = (f64::from(picture.width()), f64::from(picture.height()));
            let target = fitted(frame.fit, (width, height), size);
            let mut paint = sk::Paint::default();
            paint.set_anti_alias(true);
            paint.set_alpha_f(opacity);
            canvas.draw_image_rect_with_sampling_options(
                picture,
                None,
                convert::rect(&target),
                sk::SamplingOptions::new(sk::FilterMode::Linear, sk::MipmapMode::Linear),
                &paint,
            );
        }
        None => placeholder(canvas, width, height, opacity),
    }
    canvas.restore();
    if let Some(outline_stroke) = frame.stroke {
        stroke::draw(canvas, &outline, outline_stroke, opacity);
    }
}

/// Where a picture of `image` size goes in a box of `size`, by `fit`.
pub(crate) fn fitted(fit: ImageFit, size: (f64, f64), image: (f64, f64)) -> Rect {
    let (width, height) = size;
    let (image_width, image_height) = image;
    if fit == ImageFit::Stretch || image_width <= 0.0 || image_height <= 0.0 {
        return Rect::new(0.0, 0.0, width, height);
    }
    let (across, down) = (width / image_width, height / image_height);
    let scale = match fit {
        ImageFit::Cover => across.max(down),
        _ => across.min(down),
    };
    let (fitted_width, fitted_height) = (image_width * scale, image_height * scale);
    Rect::new(
        (width - fitted_width) / 2.0,
        (height - fitted_height) / 2.0,
        fitted_width,
        fitted_height,
    )
}

/// A grey box with a cross, so a missing picture is noticed.
fn placeholder(canvas: &sk::Canvas, width: f64, height: f64, opacity: f32) {
    let (width, height) = (width as f32, height as f32);
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(PLACEHOLDER_FILL);
    paint.set_alpha_f(opacity);
    canvas.draw_rect(sk::Rect::from_wh(width, height), &paint);
    paint.set_color(PLACEHOLDER_LINES);
    paint.set_alpha_f(opacity);
    paint.set_style(sk::PaintStyle::Stroke);
    paint.set_stroke_width((width.min(height) / 60.0).clamp(1.0, 4.0));
    canvas.draw_line((0.0, 0.0), (width, height), &paint);
    canvas.draw_line((width, 0.0), (0.0, height), &paint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_modes_place_the_picture() {
        // A 200 by 100 picture in a 100 by 100 box.
        let cover = fitted(ImageFit::Cover, (100.0, 100.0), (200.0, 100.0));
        assert_eq!(cover, Rect::new(-50.0, 0.0, 200.0, 100.0));
        let contain = fitted(ImageFit::Contain, (100.0, 100.0), (200.0, 100.0));
        assert_eq!(contain, Rect::new(0.0, 25.0, 100.0, 50.0));
        let stretch = fitted(ImageFit::Stretch, (100.0, 100.0), (200.0, 100.0));
        assert_eq!(stretch, Rect::new(0.0, 0.0, 100.0, 100.0));
        // A picture without size fills the box rather than dividing by 0.
        let empty = fitted(ImageFit::Cover, (100.0, 50.0), (0.0, 10.0));
        assert_eq!(empty, Rect::new(0.0, 0.0, 100.0, 50.0));
    }
}
