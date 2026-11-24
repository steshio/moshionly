//! Converting frames to the colour form video codecs store: BT.709 Y'CbCr,
//! limited range, with colour at half resolution (4:2:0).
//!
//! BT.709 is the colour standard of HD video and what players assume for
//! anything HD-sized; the file says so too (see `mux.rs`), so colours come
//! out the same in every player. Limited range puts black at 16 and white
//! at 235, which is what video players expect.

use openh264::formats::YUVSource;

/// Red's share of brightness in BT.709.
const KR: f64 = 0.2126;
/// Blue's share of brightness in BT.709.
const KB: f64 = 0.0722;
/// Green's share of brightness in BT.709.
const KG: f64 = 1.0 - KR - KB;

/// Fixed-point fractional bits used in the conversion.
const SHIFT: u32 = 16;
const ONE: f64 = (1 << SHIFT) as f64;

/// The conversion from 8-bit RGB to limited-range Y'CbCr, in fixed point.
struct Matrix {
    y: [i32; 3],
    cb: [i32; 3],
    cr: [i32; 3],
}

const fn fixed(value: f64) -> i32 {
    let scaled = value * ONE;
    (if scaled < 0.0 {
        scaled - 0.5
    } else {
        scaled + 0.5
    }) as i32
}

/// Brightness spans 219 of 255 steps and colour difference 224.
const MATRIX: Matrix = {
    let luma = 219.0 / 255.0;
    let chroma = 224.0 / 255.0;
    let cb = chroma / (2.0 * (1.0 - KB));
    let cr = chroma / (2.0 * (1.0 - KR));
    Matrix {
        y: [fixed(KR * luma), fixed(KG * luma), fixed(KB * luma)],
        cb: [fixed(-KR * cb), fixed(-KG * cb), fixed((1.0 - KB) * cb)],
        cr: [fixed((1.0 - KR) * cr), fixed(-KG * cr), fixed(-KB * cr)],
    }
};

fn apply(row: [i32; 3], rgb: [i32; 3], offset: i32, scale: i32) -> u8 {
    let sum = row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2];
    let divisor = scale << SHIFT;
    // Rounded to nearest; `div_euclid` rounds down for negative sums too.
    (offset + (sum + divisor / 2).div_euclid(divisor)).clamp(0, 255) as u8
}

/// A picture in planar 4:2:0 Y'CbCr: a full-size brightness plane, then two
/// colour planes at half the width and half the height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Yuv420 {
    width: usize,
    height: usize,
    /// Brightness, then blue difference, then red difference.
    data: Vec<u8>,
}

impl Yuv420 {
    /// Converts opaque RGBA pixels, `width` by `height`, both even.
    ///
    /// The pixels must be opaque, which export guarantees for video by
    /// putting a colour behind every frame; alpha is ignored.
    pub fn from_rgba(pixels: &[u8], width: usize, height: usize) -> Self {
        debug_assert!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "even sizes only"
        );
        debug_assert_eq!(pixels.len(), width * height * 4);
        let (cw, ch) = (width / 2, height / 2);
        let mut data = vec![0; width * height + 2 * cw * ch];
        let (luma, chroma) = data.split_at_mut(width * height);
        let (blue, red) = chroma.split_at_mut(cw * ch);
        let rgb = |x: usize, y: usize| {
            let at = (y * width + x) * 4;
            [0, 1, 2].map(|i| i32::from(pixels[at + i]))
        };
        for y in 0..height {
            for x in 0..width {
                luma[y * width + x] = apply(MATRIX.y, rgb(x, y), 16, 1);
            }
        }
        for y in 0..ch {
            for x in 0..cw {
                // The four pixels a colour sample covers, added together.
                let mut sum = [0; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let pixel = rgb(2 * x + dx, 2 * y + dy);
                    for i in 0..3 {
                        sum[i] += pixel[i];
                    }
                }
                blue[y * cw + x] = apply(MATRIX.cb, sum, 128, 4);
                red[y * cw + x] = apply(MATRIX.cr, sum, 128, 4);
            }
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Converts back to RGB, for tests that compare decoded video with
    /// rendered frames.
    #[cfg(test)]
    pub fn planes_to_rgb(
        width: usize,
        height: usize,
        luma: impl Fn(usize, usize) -> u8,
        blue: impl Fn(usize, usize) -> u8,
        red: impl Fn(usize, usize) -> u8,
    ) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let l = (f64::from(luma(x, y)) - 16.0) / 219.0;
                let pb = (f64::from(blue(x / 2, y / 2)) - 128.0) / 224.0;
                let pr = (f64::from(red(x / 2, y / 2)) - 128.0) / 224.0;
                let r = l + 2.0 * (1.0 - KR) * pr;
                let b = l + 2.0 * (1.0 - KB) * pb;
                let g = (l - KR * r - KB * b) / KG;
                rgb.extend([r, g, b].map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8));
            }
        }
        rgb
    }
}

impl YUVSource for Yuv420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }

    fn y(&self) -> &[u8] {
        &self.data[..self.width * self.height]
    }

    fn u(&self) -> &[u8] {
        let start = self.width * self.height;
        &self.data[start..start + self.width * self.height / 4]
    }

    fn v(&self) -> &[u8] {
        &self.data[self.width * self.height + self.width * self.height / 4..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(rgb: [u8; 3]) -> Yuv420 {
        let pixels: Vec<u8> = (0..16)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect();
        Yuv420::from_rgba(&pixels, 4, 4)
    }

    #[test]
    fn black_white_and_primaries_land_on_the_standard_values() {
        let check = |rgb, expected: [u8; 3]| {
            let yuv = solid(rgb);
            assert_eq!([yuv.y()[0], yuv.u()[0], yuv.v()[0]], expected, "{rgb:?}");
        };
        check([0, 0, 0], [16, 128, 128]);
        check([255, 255, 255], [235, 128, 128]);
        // The BT.709 values for full red, green and blue.
        check([255, 0, 0], [63, 102, 240]);
        check([0, 255, 0], [173, 42, 26]);
        check([0, 0, 255], [32, 240, 118]);
    }

    #[test]
    fn planes_have_the_right_sizes() {
        let yuv = solid([10, 20, 30]);
        assert_eq!(yuv.dimensions(), (4, 4));
        assert_eq!(yuv.strides(), (4, 2, 2));
        assert_eq!((yuv.y().len(), yuv.u().len(), yuv.v().len()), (16, 4, 4));
    }

    #[test]
    fn colour_survives_the_round_trip() {
        let pixels: Vec<u8> = (0..64u32)
            .flat_map(|i| [(i * 4) as u8, (255 - i * 3) as u8, 128, 255])
            .collect();
        // Pairs of equal columns and rows, so halving the colour loses nothing.
        let mut doubled = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let at = ((y / 2) * 8 + x / 2) * 4;
                doubled.extend_from_slice(&pixels[at..at + 4]);
            }
        }
        let yuv = Yuv420::from_rgba(&doubled, 8, 8);
        let rgb = Yuv420::planes_to_rgb(
            8,
            8,
            |x, y| yuv.y()[y * 8 + x],
            |x, y| yuv.u()[y * 4 + x],
            |x, y| yuv.v()[y * 4 + x],
        );
        for (got, want) in rgb
            .iter()
            .zip(doubled.as_chunks::<4>().0.iter().flat_map(|p| &p[..3]))
        {
            assert!(got.abs_diff(*want) <= 2, "{got} vs {want}");
        }
    }
}
