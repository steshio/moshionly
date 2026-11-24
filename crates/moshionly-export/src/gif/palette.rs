//! Choosing a GIF's colours, and finding the nearest one quickly.
//!
//! A GIF shows at most 256 colours. One palette is chosen for the whole
//! animation, from the colours of all its frames, so a colour never shifts
//! from one frame to the next and pixels that do not change keep the same
//! palette entry.
//!
//! The palette is chosen by splitting the colour space: start with one box
//! holding every colour seen, and keep splitting the box whose colours are
//! furthest from their average, at the median of its widest channel, until
//! there are as many boxes as palette entries. A few rounds of refinement
//! then move each entry to the average of the colours nearest to it. Exact
//! counts are kept for every colour, so a colour that covers a large area,
//! such as a brand colour or a background, ends up in the palette exactly.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// An RGB colour.
pub(crate) type Rgb = [u8; 3];

/// Colours seen in an animation's frames, and how often.
#[derive(Debug, Default, Clone)]
pub(crate) struct ColorCounts {
    counts: HashMap<u32, u64, BuildHasherDefault<ColorHasher>>,
}

impl ColorCounts {
    /// Counts the opaque pixels of `pixels`, premultiplied RGBA. Pixels
    /// that are mostly transparent are left out: a GIF does not show them.
    pub fn add_premultiplied(&mut self, pixels: &[u8]) {
        let (pixels, _) = pixels.as_chunks::<4>();
        for pixel in pixels {
            if pixel[3] >= ALPHA_CUTOFF {
                *self.counts.entry(pack(straight(*pixel))).or_insert(0) += 1;
            }
        }
    }

    /// Adds the counts of `other`.
    pub fn merge(&mut self, other: ColorCounts) {
        if self.counts.is_empty() {
            self.counts = other.counts;
            return;
        }
        for (color, count) in other.counts {
            *self.counts.entry(color).or_insert(0) += count;
        }
    }

    /// The number of different colours.
    pub fn len(&self) -> usize {
        self.counts.len()
    }
}

/// Pixels with less alpha than this are transparent in a GIF; the rest are
/// solid.
pub(crate) const ALPHA_CUTOFF: u8 = 128;

/// More different colours than this are first merged into bins of
/// neighbouring colours, which keeps choosing a palette quick for photos.
const MAX_EXACT_COLORS: usize = 1 << 17;

/// Rounds of refinement after the boxes are split.
const REFINE_ROUNDS: usize = 4;

/// A pixel's colour without its alpha multiplied in.
pub(crate) fn straight([r, g, b, a]: [u8; 4]) -> Rgb {
    match a {
        255 => [r, g, b],
        0 => [0, 0, 0],
        _ => {
            let alpha = u32::from(a);
            let channel = |c: u8| ((u32::from(c) * 255 + alpha / 2) / alpha).min(255) as u8;
            [channel(r), channel(g), channel(b)]
        }
    }
}

fn pack([r, g, b]: Rgb) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

fn unpack(color: u32) -> Rgb {
    [(color >> 16) as u8, (color >> 8) as u8, color as u8]
}

/// Squared distance between two colours.
fn distance(a: [i32; 3], b: Rgb) -> u32 {
    let d = |i: usize| (a[i] - i32::from(b[i])).unsigned_abs();
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}

/// A colour and how often it was seen.
#[derive(Debug, Clone, Copy)]
struct Entry {
    color: Rgb,
    count: u64,
}

/// The colours of a GIF, and a fast nearest-colour lookup.
#[derive(Debug, Clone)]
pub(crate) struct Palette {
    colors: Vec<Rgb>,
    grid: NearestGrid,
}

impl Palette {
    /// The best `max_colors` colours (at most 255, leaving one entry for
    /// transparency) for the colours counted. Deterministic: the same
    /// counts always give the same palette.
    pub fn from_counts(counts: &ColorCounts, max_colors: usize) -> Palette {
        let max_colors = max_colors.clamp(1, 255);
        let mut entries: Vec<Entry> = if counts.len() > MAX_EXACT_COLORS {
            binned(counts)
        } else {
            counts
                .counts
                .iter()
                .map(|(&color, &count)| Entry {
                    color: unpack(color),
                    count,
                })
                .collect()
        };
        // Hash maps have no order; sorting makes the result deterministic.
        entries.sort_unstable_by(|a, b| b.count.cmp(&a.count).then(a.color.cmp(&b.color)));
        if entries.is_empty() {
            return Palette::new(vec![[0, 0, 0]]);
        }
        if entries.len() <= max_colors {
            return Palette::new(entries.iter().map(|entry| entry.color).collect());
        }
        let colors = median_cut(&mut entries, max_colors);
        Palette::new(refine(&entries, colors))
    }

    /// A palette of exactly these colours.
    pub fn new(colors: Vec<Rgb>) -> Palette {
        let grid = NearestGrid::new(&colors);
        Palette { colors, grid }
    }

    /// The number of colours.
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    /// The colours, in palette order.
    pub fn colors(&self) -> &[Rgb] {
        &self.colors
    }

    /// The index of the colour nearest `color`, and its squared distance.
    /// The channels may lie outside 0 to 255.
    pub fn nearest(&self, color: [i32; 3]) -> (u8, u32) {
        self.grid.nearest(&self.colors, color)
    }
}

/// Merges colours into bins of 4 by 4 by 4 neighbouring values, each
/// counted at the average colour of what fell into it.
fn binned(counts: &ColorCounts) -> Vec<Entry> {
    let mut bins: HashMap<u32, [u64; 4], BuildHasherDefault<ColorHasher>> = HashMap::default();
    for (&color, &count) in &counts.counts {
        let [r, g, b] = unpack(color);
        let bin = bins.entry(pack([r >> 2, g >> 2, b >> 2])).or_default();
        bin[0] += u64::from(r) * count;
        bin[1] += u64::from(g) * count;
        bin[2] += u64::from(b) * count;
        bin[3] += count;
    }
    bins.into_values()
        .map(|[r, g, b, count]| Entry {
            color: [r, g, b].map(|sum| ((sum + count / 2) / count) as u8),
            count,
        })
        .collect()
}

/// A run of `entries` that becomes one palette colour.
#[derive(Debug, Clone, Copy)]
struct Cut {
    start: usize,
    end: usize,
    /// How far the colours are from their average, weighted by count. The
    /// box with the most is split next.
    error: f64,
}

impl Cut {
    fn new(entries: &[Entry], start: usize, end: usize) -> Cut {
        let error = if end - start < 2 {
            0.0
        } else {
            let (mean, _) = average(&entries[start..end]);
            entries[start..end]
                .iter()
                .map(|entry| {
                    let d: f64 = (0..3)
                        .map(|i| (f64::from(entry.color[i]) - mean[i]).powi(2))
                        .sum();
                    d * entry.count as f64
                })
                .sum()
        };
        Cut { start, end, error }
    }
}

/// The count-weighted average colour of `entries`, and their total count.
fn average(entries: &[Entry]) -> ([f64; 3], u64) {
    let mut sums = [0.0; 3];
    let mut total = 0;
    for entry in entries {
        for (sum, channel) in sums.iter_mut().zip(entry.color) {
            *sum += f64::from(channel) * entry.count as f64;
        }
        total += entry.count;
    }
    let total_f = total.max(1) as f64;
    (sums.map(|sum| sum / total_f), total)
}

/// Splits the colours into `boxes` runs and returns the average of each.
fn median_cut(entries: &mut [Entry], boxes: usize) -> Vec<Rgb> {
    let mut cuts = vec![Cut::new(entries, 0, entries.len())];
    while cuts.len() < boxes {
        let Some((at, cut)) = cuts
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, cut)| cut.end - cut.start >= 2)
            .max_by(|(_, a), (_, b)| a.error.total_cmp(&b.error))
        else {
            break;
        };
        let part = &mut entries[cut.start..cut.end];
        let channel = widest_channel(part);
        part.sort_unstable_by_key(|entry| (entry.color[channel], entry.color));
        // Split where half the pixels are on each side, keeping at least
        // one colour in each half.
        let total: u64 = part.iter().map(|entry| entry.count).sum();
        let mut seen = 0;
        let mut split = part.len() - 1;
        for (i, entry) in part.iter().enumerate() {
            seen += entry.count;
            if seen * 2 >= total {
                split = i + 1;
                break;
            }
        }
        let split = cut.start + split.clamp(1, part.len() - 1);
        cuts[at] = Cut::new(entries, cut.start, split);
        cuts.push(Cut::new(entries, split, cut.end));
    }
    cuts.iter()
        .map(|cut| {
            let (mean, _) = average(&entries[cut.start..cut.end]);
            mean.map(|channel| channel.round() as u8)
        })
        .collect()
}

/// The channel along which `entries` spread the most, weighted by count.
fn widest_channel(entries: &[Entry]) -> usize {
    let (mean, _) = average(entries);
    let mut spread = [0.0; 3];
    for entry in entries {
        for (i, s) in spread.iter_mut().enumerate() {
            *s += (f64::from(entry.color[i]) - mean[i]).powi(2) * entry.count as f64;
        }
    }
    (0..3)
        .max_by(|&a, &b| spread[a].total_cmp(&spread[b]).then(b.cmp(&a)))
        .unwrap_or(0)
}

/// Moves each colour to the average of the colours nearest to it, a few
/// times over. A colour that most of its group's pixels share exactly is
/// used as it is, so large flat areas keep their exact colour.
fn refine(entries: &[Entry], mut colors: Vec<Rgb>) -> Vec<Rgb> {
    for _ in 0..REFINE_ROUNDS {
        let palette = Palette::new(colors.clone());
        let mut sums = vec![[0u64; 4]; colors.len()];
        let mut top: Vec<Option<Entry>> = vec![None; colors.len()];
        for entry in entries {
            let (index, _) = palette.nearest(entry.color.map(i32::from));
            let index = usize::from(index);
            let sum = &mut sums[index];
            for (s, channel) in sum.iter_mut().zip(entry.color) {
                *s += u64::from(channel) * entry.count;
            }
            sum[3] += entry.count;
            if top[index].is_none_or(|best| entry.count > best.count) {
                top[index] = Some(*entry);
            }
        }
        let mut changed = false;
        for ((color, sum), top) in colors.iter_mut().zip(&sums).zip(&top) {
            let count = sum[3];
            if count == 0 {
                continue;
            }
            let next = match top {
                Some(top) if top.count * 2 > count => top.color,
                _ => [0, 1, 2].map(|i| ((sum[i] + count / 2) / count) as u8),
            };
            changed |= next != *color;
            *color = next;
        }
        if !changed {
            break;
        }
    }
    colors
}

/// Cells along each side of the colour cube.
const CELLS: usize = 16;
/// Colour values per cell along each side.
const CELL_SIZE: i32 = 256 / CELLS as i32;

/// For each cell of the colour cube, the palette colours that can be the
/// nearest to some colour in it. Finding the nearest colour then checks a
/// handful of candidates instead of the whole palette.
#[derive(Debug, Clone)]
struct NearestGrid {
    /// Where each cell's candidates start in `candidates`; one more entry
    /// than there are cells.
    starts: Vec<u32>,
    candidates: Vec<u8>,
}

impl NearestGrid {
    fn new(colors: &[Rgb]) -> Self {
        let mut starts = Vec::with_capacity(CELLS * CELLS * CELLS + 1);
        let mut candidates = Vec::new();
        let mut near = Vec::with_capacity(colors.len());
        for cell in 0..CELLS * CELLS * CELLS {
            let low = [cell / (CELLS * CELLS), cell / CELLS % CELLS, cell % CELLS]
                .map(|i| i as i32 * CELL_SIZE);
            let high = low.map(|value| value + CELL_SIZE - 1);
            // The nearest colour to any point of the cell is no further than
            // the closest "furthest corner" of any palette colour.
            near.clear();
            let mut bound = u32::MAX;
            for color in colors {
                let (closest, furthest) = box_distances(*color, low, high);
                bound = bound.min(furthest);
                near.push(closest);
            }
            starts.push(candidates.len() as u32);
            for (index, closest) in near.iter().enumerate() {
                if *closest <= bound {
                    candidates.push(index as u8);
                }
            }
        }
        starts.push(candidates.len() as u32);
        Self { starts, candidates }
    }

    fn nearest(&self, colors: &[Rgb], color: [i32; 3]) -> (u8, u32) {
        let mut best = (0, u32::MAX);
        let mut consider = |index: u8| {
            let d = distance(color, colors[usize::from(index)]);
            if d < best.1 {
                best = (index, d);
            }
        };
        if color.iter().all(|channel| (0..=255).contains(channel)) {
            let cell = color.map(|channel| (channel / CELL_SIZE) as usize);
            let cell = (cell[0] * CELLS + cell[1]) * CELLS + cell[2];
            let range = self.starts[cell] as usize..self.starts[cell + 1] as usize;
            self.candidates[range]
                .iter()
                .copied()
                .for_each(&mut consider);
        } else {
            // The cells only cover the colour cube; a colour outside it,
            // which dithering can ask about, is compared with every entry.
            (0..colors.len()).for_each(|index| consider(index as u8));
        }
        best
    }
}

/// The squared distances from `color` to the nearest and the furthest
/// point of the box from `low` to `high`.
fn box_distances(color: Rgb, low: [i32; 3], high: [i32; 3]) -> (u32, u32) {
    let (mut closest, mut furthest) = (0, 0);
    for i in 0..3 {
        let c = i32::from(color[i]);
        let near = if c < low[i] {
            low[i] - c
        } else if c > high[i] {
            c - high[i]
        } else {
            0
        };
        let far = (c - low[i]).abs().max((c - high[i]).abs());
        closest += (near * near) as u32;
        furthest += (far * far) as u32;
    }
    (closest, furthest)
}

/// A quick hash for packed colours; the standard hasher is built to resist
/// attacks, which colour counting does not need.
#[derive(Default)]
pub(crate) struct ColorHasher(u64);

impl Hasher for ColorHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 =
                (self.0.rotate_left(5) ^ u64::from(*byte)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = u64::from(value)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .rotate_right(17);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(colors: &[(Rgb, u32)]) -> ColorCounts {
        let mut counts = ColorCounts::default();
        for (color, n) in colors {
            let pixels: Vec<u8> = (0..*n)
                .flat_map(|_| [color[0], color[1], color[2], 255])
                .collect();
            counts.add_premultiplied(&pixels);
        }
        counts
    }

    fn brute_nearest(colors: &[Rgb], color: [i32; 3]) -> u32 {
        colors.iter().map(|c| distance(color, *c)).min().unwrap()
    }

    #[test]
    fn few_colours_are_kept_exactly() {
        let blue = [13, 153, 255];
        let counts = counts(&[(blue, 50), ([255, 255, 255], 30), ([17, 17, 17], 5)]);
        assert_eq!(counts.len(), 3);
        let palette = Palette::from_counts(&counts, 255);
        assert_eq!(palette.colors(), [blue, [255, 255, 255], [17, 17, 17]]);
        assert_eq!(palette.nearest([13, 153, 255]), (0, 0));
        assert_eq!(palette.nearest([20, 20, 20]).0, 2);
    }

    #[test]
    fn transparent_pixels_are_not_counted() {
        let mut counts = ColorCounts::default();
        // Premultiplied half-transparent red counts as red; nearly
        // invisible pixels do not count.
        counts.add_premultiplied(&[128, 0, 0, 128, 9, 9, 9, 10, 0, 0, 0, 0]);
        let palette = Palette::from_counts(&counts, 255);
        assert_eq!(palette.colors(), [[255, 0, 0]]);
        let empty = Palette::from_counts(&ColorCounts::default(), 255);
        assert_eq!(empty.len(), 1);
    }

    #[test]
    fn a_gradient_is_spread_over_the_palette_and_large_areas_stay_exact() {
        // A smooth grey ramp, and a large flat brand colour.
        let mut colors: Vec<(Rgb, u32)> = (0..=255).map(|v| ([v, v, v], 4)).collect();
        let brand = [123, 97, 255];
        colors.push((brand, 5000));
        colors.push(([124, 97, 254], 20));
        let palette = Palette::from_counts(&counts(&colors), 16);
        assert_eq!(palette.len(), 16);
        assert!(palette.colors().contains(&brand), "{:?}", palette.colors());
        // Every grey is within reach of a palette entry.
        for v in (0..=255).step_by(5) {
            let (_, d) = palette.nearest([v, v, v]);
            assert!(d <= 3 * 20 * 20, "grey {v} is {d} away");
        }
    }

    #[test]
    fn choosing_is_deterministic() {
        let colors: Vec<(Rgb, u32)> = (0..2000u32)
            .map(|i| {
                (
                    [
                        (i * 7 % 256) as u8,
                        (i * 13 % 256) as u8,
                        (i * 31 % 256) as u8,
                    ],
                    1 + i % 5,
                )
            })
            .collect();
        let first = Palette::from_counts(&counts(&colors), 64);
        let second = Palette::from_counts(&counts(&colors), 64);
        assert_eq!(first.colors(), second.colors());
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn the_grid_finds_the_true_nearest_colour() {
        let colors: Vec<Rgb> = (0..200u32)
            .map(|i| {
                [
                    (i * 37 % 256) as u8,
                    (i * 91 % 256) as u8,
                    (i * 53 % 256) as u8,
                ]
            })
            .collect();
        let palette = Palette::new(colors.clone());
        for r in (-20..280).step_by(23) {
            for g in (0..256).step_by(17) {
                for b in (0..256).step_by(29) {
                    let (index, d) = palette.nearest([r, g, b]);
                    assert_eq!(d, brute_nearest(&colors, [r, g, b]));
                    assert_eq!(d, distance([r, g, b], colors[usize::from(index)]));
                }
            }
        }
    }

    #[test]
    fn many_colours_are_binned_first() {
        let mut counts = ColorCounts::default();
        let pixels: Vec<u8> = (0..(MAX_EXACT_COLORS as u32 + 1000) * 3)
            .flat_map(|i| [(i >> 16) as u8, (i >> 8) as u8, i as u8, 255])
            .collect();
        counts.add_premultiplied(&pixels);
        assert!(counts.len() > MAX_EXACT_COLORS);
        let palette = Palette::from_counts(&counts, 255);
        assert_eq!(palette.len(), 255);
    }

    #[test]
    fn straight_alpha_is_recovered() {
        assert_eq!(straight([128, 64, 0, 128]), [255, 128, 0]);
        assert_eq!(straight([1, 2, 3, 255]), [1, 2, 3]);
        assert_eq!(straight([0, 0, 0, 0]), [0, 0, 0]);
    }
}
