//! Moshionly's mark: three bars of action on a rounded tile.
//!
//! The same geometry is painted in the top bar ([`paint_mark`]) and
//! rasterised for the window icon ([`icon_rgba`]). It is a placeholder until
//! the project has a designed icon; replace this file to change it.

use egui::{Color32, Painter, Rect, Vec2, pos2};

use super::{Theme, corner};
use crate::raster::{Canvas, mix};

/// The three bars as `[x, y, width, height]` in a unit square.
const BARS: [[f32; 4]; 3] = [
    [0.20, 0.250, 0.42, 0.135],
    [0.36, 0.4325, 0.44, 0.135],
    [0.27, 0.615, 0.30, 0.135],
];

/// Corner radius of the tile, as a fraction of its side.
const TILE_RADIUS: f32 = 0.27;

/// Paints the mark into `rect`, which should be square: an accent tile with
/// three light bars.
pub fn paint_mark(painter: &Painter, rect: Rect, theme: &Theme) {
    let side = rect.width().min(rect.height());
    painter.rect_filled(rect, corner(side * TILE_RADIUS), theme.color.accent);
    for [x, y, w, h] in BARS {
        let bar = Rect::from_min_size(
            rect.min + Vec2::new(x * side, y * side),
            Vec2::new(w * side, h * side),
        );
        painter.rect_filled(bar, bar.height() / 2.0, theme.color.text_on_accent);
    }
}

/// The mark as a square image of `size` pixels: RGBA, alpha not
/// premultiplied, on a transparent background. Used as the window icon.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let theme = Theme::light();
    let side = size as f32;
    // Desktop icons leave a margin around the tile.
    let margin = side * 0.09;
    let tile = Rect::from_min_max(pos2(margin, margin), pos2(side - margin, side - margin));
    let tile_side = tile.width();

    let mut canvas = Canvas::new(size as usize, size as usize);
    canvas.fill_gradient(
        tile,
        tile_side * TILE_RADIUS,
        mix(theme.color.accent, Color32::WHITE, 0.16),
        theme.color.accent_pressed,
    );
    for [x, y, w, h] in BARS {
        let bar = Rect::from_min_size(
            tile.min + Vec2::new(x * tile_side, y * tile_side),
            Vec2::new(w * tile_side, h * tile_side),
        );
        canvas.fill(bar, bar.height() / 2.0, Color32::WHITE);
    }
    canvas.into_rgba()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn the_icon_is_a_rounded_tile_with_light_bars() {
        let size = 128;
        let rgba = icon_rgba(size);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        // Transparent outside the tile and at its rounded corner.
        assert_eq!(pixel(&rgba, size, 0, 0)[3], 0);
        assert_eq!(pixel(&rgba, size, 13, 13)[3], 0);
        // Opaque accent in the tile, near the bottom where there is no bar.
        let body = pixel(&rgba, size, 64, 108);
        assert_eq!(body[3], 255);
        assert!(
            body[2] > body[0] && body[2] > body[1],
            "accent is blue-violet"
        );
        // White on the middle bar.
        let bar = pixel(&rgba, size, 72, 64);
        assert_eq!(bar, [255, 255, 255, 255]);
    }

    #[test]
    fn the_bars_stay_inside_the_tile() {
        for [x, y, w, h] in BARS {
            assert!(x > 0.1 && x + w < 0.9);
            assert!(y > 0.1 && y + h < 0.9);
        }
    }
}
