//! Turning a frame into palette indices, with an optional fine pattern
//! that shows colours between the palette's.
//!
//! The pattern is ordered, not diffused: whether a pixel takes the lighter
//! or the darker of two palette colours depends only on its colour and its
//! position, never on its neighbours. Error diffusion looks smoother in a
//! still image but makes the noise crawl from frame to frame, and every
//! pixel that changes costs bytes in a GIF; an ordered pattern stays put
//! wherever the picture does.
//!
//! Each pixel picks between the palette colour nearest to it and the one
//! nearest to its mirror image on the other side of the colour, in the
//! proportion that averages out to the true colour over a small patch.
//! Colours the palette holds exactly are never dithered, so flat areas and
//! text stay clean.

use super::palette::{ALPHA_CUTOFF, Palette, straight};

/// A colour closer than this (squared distance) to a palette colour is
/// drawn with it as it is: the difference cannot be seen.
const CLOSE_ENOUGH: u32 = 6;

/// Side of the threshold pattern, in pixels.
const PATTERN: usize = 8;

/// Turns frames into indices into one palette.
#[derive(Debug, Clone)]
pub(crate) struct Indexer<'a> {
    /// The colours.
    pub palette: &'a Palette,
    /// Whether to dither.
    pub dither: bool,
    /// The index that means "transparent", for pixels with little alpha.
    pub transparent: u8,
}

impl Indexer<'_> {
    /// The palette index of every pixel of `pixels`, premultiplied RGBA
    /// rows of `width` pixels.
    pub fn index(&self, pixels: &[u8], width: usize) -> Vec<u8> {
        let thresholds = thresholds();
        let (pixels, _) = pixels.as_chunks::<4>();
        pixels
            .iter()
            .enumerate()
            .map(|(at, pixel)| {
                if pixel[3] < ALPHA_CUTOFF {
                    return self.transparent;
                }
                let color = straight(*pixel).map(i32::from);
                let (x, y) = (at % width, at / width);
                let threshold = thresholds[(y % PATTERN) * PATTERN + x % PATTERN];
                self.pick(color, threshold)
            })
            .collect()
    }

    /// The index for `color` at a place whose pattern threshold is
    /// `threshold`, from 0 to 1.
    fn pick(&self, color: [i32; 3], threshold: f32) -> u8 {
        let (near, distance) = self.palette.nearest(color);
        if !self.dither || distance <= CLOSE_ENOUGH {
            return near;
        }
        let first = self.palette.colors()[usize::from(near)].map(i32::from);
        // The colour as far past `color` as `first` is short of it, kept
        // inside the colour cube.
        let mirror = [0, 1, 2].map(|i| (2 * color[i] - first[i]).clamp(0, 255));
        let (other, _) = self.palette.nearest(mirror);
        if other == near {
            return near;
        }
        let second = self.palette.colors()[usize::from(other)].map(i32::from);
        // How far along the line from `first` to `second` the colour lies.
        let step = [0, 1, 2].map(|i| second[i] - first[i]);
        let length: i32 = step.iter().map(|s| s * s).sum();
        let along: i32 = (0..3).map(|i| (color[i] - first[i]) * step[i]).sum();
        let mix = along as f32 / length as f32;
        if mix > threshold { other } else { near }
    }
}

/// The ordered dithering thresholds for an 8 by 8 patch, from 0 to 1, row
/// by row. Neighbouring thresholds are as different as possible, so any
/// share of pixels is spread evenly over the patch.
fn thresholds() -> [f32; PATTERN * PATTERN] {
    let mut table = [0.0; PATTERN * PATTERN];
    for y in 0..PATTERN {
        for x in 0..PATTERN {
            table[y * PATTERN + x] = (rank(x, y) as f32 + 0.5) / (PATTERN * PATTERN) as f32;
        }
    }
    table
}

/// The order in which pixel (`x`, `y`) of the patch switches as a share
/// grows: built by interleaving the bits of `x ^ y` and `y`, most
/// significant first.
fn rank(x: usize, y: usize) -> usize {
    let bits = PATTERN.trailing_zeros();
    let mut rank = 0;
    for bit in 0..bits {
        let shift = 2 * (bits - 1 - bit);
        rank |= (((x ^ y) >> bit) & 1) << (shift + 1);
        rank |= ((y >> bit) & 1) << shift;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rank_is_used_once() {
        let mut seen = [false; PATTERN * PATTERN];
        for y in 0..PATTERN {
            for x in 0..PATTERN {
                assert!(!seen[rank(x, y)]);
                seen[rank(x, y)] = true;
            }
        }
        // The top-left 2 by 2 corner alternates on the diagonal.
        assert_eq!(
            [rank(0, 0), rank(1, 0), rank(0, 1), rank(1, 1)],
            [0, 32, 48, 16]
        );
    }

    fn indexer(palette: &Palette, dither: bool) -> Indexer<'_> {
        Indexer {
            palette,
            dither,
            transparent: palette.len() as u8,
        }
    }

    #[test]
    fn exact_colours_and_transparency_are_kept() {
        let palette = Palette::new(vec![[0, 0, 0], [255, 255, 255], [13, 153, 255]]);
        let pixels = [
            13, 153, 255, 255, 255, 255, 255, 255, 1, 1, 1, 255, 0, 0, 0, 0,
        ];
        for dither in [false, true] {
            let indices = indexer(&palette, dither).index(&pixels, 2);
            assert_eq!(indices, [2, 1, 0, 3]);
        }
    }

    #[test]
    fn a_mid_grey_becomes_an_even_pattern() {
        let palette = Palette::new(vec![[0, 0, 0], [255, 255, 255]]);
        let grey: Vec<u8> = (0..64).flat_map(|_| [64, 64, 64, 255]).collect();
        let plain = indexer(&palette, false).index(&grey, 8);
        assert!(
            plain.iter().all(|index| *index == 0),
            "without dithering, the nearest"
        );
        let dithered = indexer(&palette, true).index(&grey, 8);
        let white = dithered.iter().filter(|index| **index == 1).count();
        // A quarter of the way to white is a quarter of the pixels.
        assert_eq!(white, 16);
        // The same picture is always the same pattern.
        assert_eq!(dithered, indexer(&palette, true).index(&grey, 8));
    }

    #[test]
    fn dithering_mixes_neighbouring_palette_colours() {
        // A ramp from black to white with a mid grey in the palette: a
        // light grey mixes the mid grey and white, never black.
        let palette = Palette::new(vec![[0, 0, 0], [128, 128, 128], [255, 255, 255]]);
        let light: Vec<u8> = (0..64).flat_map(|_| [192, 192, 192, 255]).collect();
        let indices = indexer(&palette, true).index(&light, 8);
        assert!(indices.iter().all(|index| *index != 0));
        let white = indices.iter().filter(|index| **index == 2).count();
        assert!((28..=36).contains(&white), "{white} of 64 white");
    }
}
