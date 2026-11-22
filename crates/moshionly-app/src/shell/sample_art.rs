//! Artwork for the sample scene's artboards.
//!
//! The real editor shows artboards rendered by the render crate. Until the
//! shell is bound to it, these two pictures stand in, drawn with the app's
//! own small rasteriser. Like the rest of the sample content they go away
//! when the document engine is connected.

use egui::{Color32, Rect, Vec2, pos2};

use crate::raster::Canvas;

/// The side of an artboard picture in pixels. The sample artboards are 1080
/// world units square, so the pictures are drawn at half size.
pub const SIDE: usize = 540;

/// World units per pixel of the pictures.
const SCALE: f32 = 1080.0 / SIDE as f32;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// A rectangle given in the artboard's own 1080-unit coordinates.
fn at(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_size(pos2(x, y) / SCALE, Vec2::new(width, height) / SCALE)
}

fn radius(units: f32) -> f32 {
    units / SCALE
}

/// The first scene: a headline, a card with a photo, and a badge.
pub fn scene_one() -> Canvas {
    let mut canvas = Canvas::new(SIDE, SIDE);
    let white = Color32::WHITE;

    canvas.fill_gradient(
        at(0.0, 0.0, 1080.0, 1080.0),
        0.0,
        rgb(0xFBFAFF),
        rgb(0xEFEEFA),
    );
    // Backdrop.
    canvas.fill(
        at(-140.0, 690.0, 560.0, 560.0),
        radius(280.0),
        rgb(0xE6E5F8),
    );
    // Headline, as two bars of text.
    canvas.fill(at(120.0, 112.0, 560.0, 62.0), radius(16.0), rgb(0x1C1C20));
    canvas.fill(at(120.0, 196.0, 372.0, 62.0), radius(16.0), rgb(0x1C1C20));
    // Card.
    canvas.fill_gradient(
        at(240.0, 340.0, 600.0, 400.0),
        radius(32.0),
        rgb(0x6F73FF),
        rgb(0x4B4FDC),
    );
    // Photo on the card.
    canvas.fill_gradient(
        at(280.0, 380.0, 236.0, 320.0),
        radius(20.0),
        rgb(0xFFD9E8),
        rgb(0xC9C6FF),
    );
    canvas.fill(at(326.0, 424.0, 76.0, 76.0), radius(38.0), white);
    canvas.fill(at(280.0, 596.0, 236.0, 104.0), radius(20.0), rgb(0xA9A4F5));
    // Text on the card.
    canvas.fill(at(560.0, 404.0, 224.0, 26.0), radius(13.0), white);
    canvas.fill(at(560.0, 452.0, 168.0, 22.0), radius(11.0), rgb(0xC3C5FF));
    canvas.fill(at(560.0, 490.0, 196.0, 22.0), radius(11.0), rgb(0xC3C5FF));
    // Button on the card.
    canvas.fill(at(560.0, 622.0, 172.0, 60.0), radius(30.0), white);
    canvas.fill(at(596.0, 645.0, 100.0, 14.0), radius(7.0), rgb(0x4B4FDC));
    // Badge, overlapping the card's corner.
    canvas.fill(at(752.0, 246.0, 184.0, 184.0), radius(92.0), rgb(0xF5B83D));
    canvas.fill(at(790.0, 284.0, 108.0, 108.0), radius(54.0), rgb(0xFFDD86));
    // Confetti.
    canvas.fill(at(168.0, 820.0, 38.0, 38.0), radius(19.0), rgb(0x2BC4A8));
    canvas.fill(at(884.0, 852.0, 30.0, 30.0), radius(15.0), rgb(0xF06292));
    canvas.fill(at(930.0, 150.0, 24.0, 24.0), radius(12.0), rgb(0x6F73FF));
    canvas.fill(at(104.0, 540.0, 22.0, 22.0), radius(11.0), rgb(0xF5B83D));
    canvas
}

/// The second scene: a dark outro.
pub fn scene_two() -> Canvas {
    let mut canvas = Canvas::new(SIDE, SIDE);
    canvas.fill_gradient(
        at(0.0, 0.0, 1080.0, 1080.0),
        0.0,
        rgb(0x1E1E26),
        rgb(0x121217),
    );
    canvas.fill(at(492.0, 330.0, 96.0, 96.0), radius(48.0), rgb(0x6F73FF));
    canvas.fill(at(280.0, 480.0, 520.0, 62.0), radius(16.0), rgb(0xF4F4F6));
    canvas.fill(at(390.0, 572.0, 300.0, 34.0), radius(17.0), rgb(0x8286FF));
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_are_opaque_squares() {
        for canvas in [scene_one(), scene_two()] {
            assert_eq!((canvas.width(), canvas.height()), (SIDE, SIDE));
            for (x, y) in [(0, 0), (SIDE - 1, SIDE - 1), (SIDE / 2, SIDE / 2)] {
                assert_eq!(canvas.pixel(x, y).unwrap()[3], 255);
            }
        }
    }

    #[test]
    fn the_card_is_where_the_sample_says() {
        // The sample's Card layer is at 240, 340 and 600 by 400; its middle
        // right, clear of the photo and the text, is card-coloured.
        let canvas = scene_one();
        let [r, g, b, _] = canvas.pixel(410, 300).unwrap();
        assert!(b > 200 && b > r + 100 && b > g + 100, "{r} {g} {b}");
    }
}
