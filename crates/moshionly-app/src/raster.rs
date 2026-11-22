//! A tiny software rasteriser for rounded rectangles.
//!
//! The editor draws documents with the render crate. This module only exists
//! for the two images the app shell makes on its own: the window icon and the
//! artwork of the sample scene. It fills anti-aliased rounded rectangles
//! (and, with a radius of half the size, circles) into an RGBA buffer.

use egui::{Color32, Pos2, Rect, Vec2};

/// An image being drawn: opaque or translucent pixels, alpha not
/// premultiplied.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    width: usize,
    height: usize,
    /// Rows of `[r, g, b, a]`.
    pixels: Vec<[u8; 4]>,
}

impl Canvas {
    /// A fully transparent canvas.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0; 4]; width * height],
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The pixel at `x`, `y` as `[r, g, b, a]`, or `None` outside the canvas.
    pub fn pixel(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        (x < self.width && y < self.height).then(|| self.pixels[y * self.width + x])
    }

    /// Fills a rounded rectangle with one colour.
    pub fn fill(&mut self, rect: Rect, radius: f32, color: Color32) {
        self.fill_with(rect, radius, |_| color);
    }

    /// Fills a rounded rectangle with a colour that changes from `top` to
    /// `bottom`.
    pub fn fill_gradient(&mut self, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
        let height = rect.height().max(f32::EPSILON);
        self.fill_with(rect, radius, |p| {
            mix(top, bottom, ((p.y - rect.top()) / height).clamp(0.0, 1.0))
        });
    }

    /// Fills a rounded rectangle, asking `color_at` for the colour of each
    /// pixel centre. Opaque colours only; the shape's edge is anti-aliased.
    pub fn fill_with(&mut self, rect: Rect, radius: f32, color_at: impl Fn(Pos2) -> Color32) {
        if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let x0 = rect.left().floor().max(0.0) as usize;
        let y0 = rect.top().floor().max(0.0) as usize;
        let x1 = (rect.right().ceil().max(0.0) as usize).min(self.width);
        let y1 = (rect.bottom().ceil().max(0.0) as usize).min(self.height);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                let cover = coverage(p, rect, radius);
                if cover <= 0.0 {
                    continue;
                }
                let color = color_at(p);
                let pixel = &mut self.pixels[y * self.width + x];
                *pixel = over(*pixel, color, cover);
            }
        }
    }

    /// The image as bytes: `r, g, b, a` per pixel, row by row.
    pub fn into_rgba(self) -> Vec<u8> {
        self.pixels.into_iter().flatten().collect()
    }

    /// The image as an egui image, ready to become a texture.
    pub fn to_color_image(&self) -> egui::ColorImage {
        let bytes: Vec<u8> = self.pixels.iter().flatten().copied().collect();
        egui::ColorImage::from_rgba_unmultiplied([self.width, self.height], &bytes)
    }
}

/// How much of the pixel centred on `p` a rounded rectangle covers, from 0 to
/// 1, judged by the distance from the pixel centre to the shape's edge.
pub fn coverage(p: Pos2, rect: Rect, radius: f32) -> f32 {
    let half = rect.size() / 2.0;
    let radius = radius.clamp(0.0, half.x.min(half.y));
    let d = (p - rect.center()).abs() - (half - Vec2::splat(radius));
    let outside = Vec2::new(d.x.max(0.0), d.y.max(0.0)).length();
    let distance = outside + d.x.max(d.y).min(0.0) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Blends two opaque colours: `t` of 0 is `a`, 1 is `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

/// Puts an opaque `color` with the given coverage over a pixel.
fn over(below: [u8; 4], color: Color32, cover: f32) -> [u8; 4] {
    let below_alpha = f32::from(below[3]) / 255.0;
    let alpha = cover + below_alpha * (1.0 - cover);
    if alpha <= 0.0 {
        return [0; 4];
    }
    let channel = |top: u8, bottom: u8| {
        ((f32::from(top) * cover + f32::from(bottom) * below_alpha * (1.0 - cover)) / alpha).round()
            as u8
    };
    [
        channel(color.r(), below[0]),
        channel(color.g(), below[1]),
        channel(color.b(), below[2]),
        (alpha * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    #[test]
    fn coverage_is_one_inside_and_zero_outside() {
        let rect = Rect::from_min_max(pos2(10.0, 10.0), pos2(30.0, 20.0));
        assert_eq!(coverage(pos2(20.0, 15.0), rect, 3.0), 1.0);
        assert_eq!(coverage(pos2(40.0, 15.0), rect, 3.0), 0.0);
        // The sharp corner of the box is outside the rounded shape.
        assert_eq!(coverage(pos2(10.2, 10.2), rect, 5.0), 0.0);
        // On the straight edge, half the pixel is covered.
        assert_eq!(coverage(pos2(20.0, 10.0), rect, 3.0), 0.5);
        // A radius larger than the shape is cut down to a capsule.
        assert_eq!(coverage(pos2(20.0, 15.0), rect, 999.0), 1.0);
    }

    #[test]
    fn filling_paints_only_the_shape() {
        let mut canvas = Canvas::new(16, 16);
        let red = Color32::from_rgb(255, 0, 0);
        canvas.fill(
            Rect::from_min_max(pos2(4.0, 4.0), pos2(12.0, 12.0)),
            0.0,
            red,
        );
        assert_eq!(canvas.pixel(8, 8), Some([255, 0, 0, 255]));
        assert_eq!(canvas.pixel(1, 1), Some([0, 0, 0, 0]));
        assert_eq!(canvas.pixel(16, 0), None);
        assert_eq!(canvas.clone().into_rgba().len(), 16 * 16 * 4);
        assert_eq!(canvas.to_color_image().size, [16, 16]);
    }

    #[test]
    fn a_later_shape_covers_an_earlier_one() {
        let mut canvas = Canvas::new(8, 8);
        let all = Rect::from_min_max(pos2(0.0, 0.0), pos2(8.0, 8.0));
        canvas.fill(all, 0.0, Color32::from_rgb(0, 0, 255));
        canvas.fill(
            Rect::from_min_max(pos2(2.0, 2.0), pos2(6.0, 6.0)),
            0.0,
            Color32::WHITE,
        );
        assert_eq!(canvas.pixel(4, 4), Some([255, 255, 255, 255]));
        assert_eq!(canvas.pixel(0, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn shapes_outside_the_canvas_are_ignored() {
        let mut canvas = Canvas::new(4, 4);
        let before = canvas.clone();
        canvas.fill(
            Rect::from_min_max(pos2(-50.0, -50.0), pos2(-10.0, -10.0)),
            2.0,
            Color32::WHITE,
        );
        canvas.fill(
            Rect::from_min_max(pos2(2.0, 2.0), pos2(2.0, 9.0)),
            0.0,
            Color32::WHITE,
        );
        canvas.fill(Rect::NAN, 0.0, Color32::WHITE);
        assert_eq!(canvas, before);
    }

    #[test]
    fn gradients_run_top_to_bottom() {
        let mut canvas = Canvas::new(2, 10);
        canvas.fill_gradient(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(2.0, 10.0)),
            0.0,
            Color32::BLACK,
            Color32::WHITE,
        );
        let top = canvas.pixel(0, 0).unwrap()[0];
        let bottom = canvas.pixel(0, 9).unwrap()[0];
        assert!(top < 30 && bottom > 225);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
    }
}
