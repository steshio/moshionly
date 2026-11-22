//! Moshionly's mark: three bars of action on a rounded tile.
//!
//! The same geometry is painted in the top bar ([`paint_mark`]) and
//! rasterised for the window icon ([`icon_rgba`]). It is a placeholder until
//! the project has a designed icon; replace this file to change it.

use egui::{Color32, Painter, Rect, Vec2, pos2};

use super::{Theme, corner};

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
    let top = mix(theme.color.accent, Color32::WHITE, 0.16);
    let bottom = theme.color.accent_pressed;
    let side = size as f32;
    // Desktop icons leave a margin around the tile.
    let margin = side * 0.09;
    let tile = Rect::from_min_max(pos2(margin, margin), pos2(side - margin, side - margin));
    let tile_side = tile.width();

    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let p = pos2(px as f32 + 0.5, py as f32 + 0.5);
            let tile_cover = coverage(p, tile, tile_side * TILE_RADIUS);
            let t = ((p.y - tile.top()) / tile_side).clamp(0.0, 1.0);
            let mut color = mix(top, bottom, t);
            for [x, y, w, h] in BARS {
                let bar = Rect::from_min_size(
                    tile.min + Vec2::new(x * tile_side, y * tile_side),
                    Vec2::new(w * tile_side, h * tile_side),
                );
                color = mix(color, Color32::WHITE, coverage(p, bar, bar.height() / 2.0));
            }
            rgba.extend_from_slice(&[
                color.r(),
                color.g(),
                color.b(),
                (tile_cover * 255.0).round() as u8,
            ]);
        }
    }
    rgba
}

/// How much of the pixel centred on `p` a rounded rectangle covers, from 0
/// to 1, from the distance to its edge.
fn coverage(p: egui::Pos2, rect: Rect, radius: f32) -> f32 {
    let half = rect.size() / 2.0;
    let radius = radius.min(half.x).min(half.y);
    let d = (p - rect.center()).abs() - (half - Vec2::splat(radius));
    let outside = Vec2::new(d.x.max(0.0), d.y.max(0.0)).length();
    let distance = outside + d.x.max(d.y).min(0.0) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Blends two opaque colours: `t` of 0 is `a`, 1 is `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
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

    #[test]
    fn coverage_is_one_inside_and_zero_outside() {
        let rect = Rect::from_min_max(pos2(10.0, 10.0), pos2(30.0, 20.0));
        assert_eq!(coverage(pos2(20.0, 15.0), rect, 3.0), 1.0);
        assert_eq!(coverage(pos2(40.0, 15.0), rect, 3.0), 0.0);
        // The sharp corner of the box is outside the rounded shape.
        assert_eq!(coverage(pos2(10.2, 10.2), rect, 5.0), 0.0);
        // On the straight edge, half the pixel is covered.
        assert_eq!(coverage(pos2(20.0, 10.0), rect, 3.0), 0.5);
    }
}
