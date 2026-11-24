//! Moshionly's mark: three bars of action on a rounded tile.
//!
//! The same geometry is painted in the top bar ([`paint_mark`]),
//! rasterised for the window icon ([`icon_rgba`]) and for the macOS app
//! icon ([`app_icon_rgba`], written out by the `app_icon` example when the
//! app is packaged). It is a placeholder until the project has a designed
//! icon; replace this file to change it.

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

/// Margin around the tile of the window icon, as a fraction of the image's
/// side.
const WINDOW_ICON_MARGIN: f32 = 0.09;

/// Margin around the tile of the macOS app icon, as a fraction of the
/// image's side: macOS's icon grid centres an 824-pixel tile in a
/// 1024-pixel image, so that the icon lines up with the system's own.
pub const APP_ICON_MARGIN: f32 = 100.0 / 1024.0;

/// The images a macOS `.iconset` folder holds, as `(file name, side in
/// pixels)`: every size from 16 to 512 points, at 1x and 2x. `iconutil`
/// packs the folder into the app's `.icns` file.
pub const APP_ICON_SET: [(&str, u32); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

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
    mark_rgba(size, WINDOW_ICON_MARGIN)
}

/// The mark as the macOS app icon, a square image of `size` pixels: the
/// same as [`icon_rgba`], with the tile sized and centred on macOS's icon
/// grid.
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    mark_rgba(size, APP_ICON_MARGIN)
}

/// The mark as a square image of `size` pixels, with `margin` (a fraction
/// of the side) of transparent space around the tile.
fn mark_rgba(size: u32, margin: f32) -> Vec<u8> {
    let theme = Theme::light();
    let side = size as f32;
    let margin = side * margin;
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
    fn the_app_icon_sits_on_the_macos_grid() {
        let size = 1024;
        let rgba = app_icon_rgba(size);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        // The 824-pixel tile runs from 100 to 924 on both axes: clear
        // outside it, opaque just inside, along the middle of each edge.
        for (x, y) in [(98, 512), (925, 512), (512, 98), (512, 925)] {
            assert_eq!(pixel(&rgba, size, x, y)[3], 0, "outside at {x}, {y}");
        }
        for (x, y) in [(101, 512), (922, 512), (512, 101), (512, 922)] {
            assert_eq!(pixel(&rgba, size, x, y)[3], 255, "inside at {x}, {y}");
        }
        // The corners of the image and of the tile's box are transparent.
        for (x, y) in [
            (0, 0),
            (1023, 0),
            (0, 1023),
            (1023, 1023),
            (104, 104),
            (919, 919),
        ] {
            assert_eq!(pixel(&rgba, size, x, y)[3], 0, "corner at {x}, {y}");
        }
        // The same mark as the window icon: white on the middle bar.
        assert_eq!(pixel(&rgba, size, 560, 512), [255, 255, 255, 255]);
    }

    #[test]
    fn the_icon_set_has_every_size_macos_asks_for() {
        let mut sizes: Vec<u32> = APP_ICON_SET.iter().map(|(_, side)| *side).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [16, 32, 32, 64, 128, 256, 256, 512, 512, 1024]);
        for (name, side) in APP_ICON_SET {
            let points: u32 = name
                .trim_start_matches("icon_")
                .split('x')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap();
            let scale = if name.ends_with("@2x.png") { 2 } else { 1 };
            assert_eq!(points * scale, side, "{name}");
            assert_eq!(
                name,
                format!(
                    "icon_{points}x{points}{}.png",
                    if scale == 2 { "@2x" } else { "" }
                )
            );
            // Every size is drawn cleanly, with clear corners.
            let rgba = app_icon_rgba(side);
            assert_eq!(rgba.len(), (side * side * 4) as usize, "{name}");
            assert_eq!(pixel(&rgba, side, 0, 0)[3], 0, "{name}");
            assert_eq!(pixel(&rgba, side, side - 1, side - 1)[3], 0, "{name}");
        }
    }

    #[test]
    fn the_bars_stay_inside_the_tile() {
        for [x, y, w, h] in BARS {
            assert!(x > 0.1 && x + w < 0.9);
            assert!(y > 0.1 && y + h < 0.9);
        }
    }
}
