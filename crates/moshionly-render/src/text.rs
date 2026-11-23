//! Text layout: where the lines, letters and glyphs of a text go.
//!
//! Text is shaped and broken into lines by Skia's paragraph module. What it
//! works out is copied into a [`TextBlock`], plain data that can be shared
//! between threads and kept: the lines, the grapheme clusters (what a
//! reader sees as one letter) and the positioned glyphs. Measuring for
//! core and drawing both read the same block, so text is split exactly
//! where it is drawn.
//!
//! The rules, which core's `TextMeasurer` documents:
//!
//! - `line_height` is the distance between baselines; every line is exactly
//!   that tall, with the font's own height centred in it.
//! - `letter_spacing` is added after every letter. A line's width counts
//!   it after every letter but the last, so alignment centres the ink.
//! - Lines break at line breaks and, with a wrap width, where a line would
//!   pass it. They are aligned inside the block, which is the wrap width
//!   wide, or as wide as the widest line without one.
//! - Coordinates are relative to the top-left corner of the block.

use std::collections::HashMap;
use std::sync::Arc;

use moshionly_core::Rect;
use moshionly_core::model::TextAlign;
use moshionly_core::scene::TextStyle;
use moshionly_core::text::{TextCluster, TextLayout, TextLine, TextMeasurer, TextRequest};
use skia_safe as sk;
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, RectHeightStyle, RectWidthStyle,
    StrutStyle,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::fonts::{DEFAULT_FAMILY, FontLibrary, FontState};

/// The smallest and largest font sizes laid out. Text outside this range
/// is drawn at the nearest one.
const FONT_SIZE_RANGE: (f64, f64) = (0.01, 10_000.0);

/// How many laid-out texts are kept.
const CACHE_ENTRIES: usize = 512;

/// A laid-out text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextBlock {
    /// Width of the widest line, not counting trailing spaces.
    pub width: f64,
    /// Height of all lines together.
    pub height: f64,
    /// The lines, top to bottom. There is always at least one.
    pub lines: Vec<TextLine>,
    /// Every grapheme cluster but line breaks, in reading order.
    pub clusters: Vec<TextCluster>,
    /// The glyphs to draw, run by run.
    pub runs: Vec<GlyphRun>,
}

/// Glyphs of one font on one line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlyphRun {
    /// The font, set up for drawing at any scale.
    pub font: sk::Font,
    /// The glyphs.
    pub glyphs: Vec<sk::GlyphId>,
    /// Where each glyph's origin goes, on its baseline.
    pub positions: Vec<sk::Point>,
    /// Byte offset in the text where each glyph's cluster starts.
    pub starts: Vec<usize>,
    /// The ink of each glyph, around its origin.
    pub bounds: Vec<sk::Rect>,
}

impl TextBlock {
    /// The box around the ink of the glyphs whose letters start in
    /// `range`, or of all glyphs. `None` when nothing is drawn.
    pub(crate) fn ink(&self, range: Option<&std::ops::Range<usize>>) -> Option<Rect> {
        let mut ink: Option<sk::Rect> = None;
        for run in &self.runs {
            for ((start, bounds), position) in
                run.starts.iter().zip(&run.bounds).zip(&run.positions)
            {
                if range.is_some_and(|range| !range.contains(start)) || bounds.is_empty() {
                    continue;
                }
                let glyph = bounds.with_offset(*position);
                ink = Some(ink.map_or(glyph, |ink| sk::Rect::join2(ink, glyph)));
            }
        }
        ink.map(|ink| {
            Rect::new(
                f64::from(ink.left),
                f64::from(ink.top),
                f64::from(ink.width()),
                f64::from(ink.height()),
            )
        })
    }

    /// The answer core's measurer gives.
    pub(crate) fn to_layout(&self) -> TextLayout {
        TextLayout {
            width: self.width,
            height: self.height,
            lines: self.lines.clone(),
            clusters: self.clusters.clone(),
        }
    }
}

impl TextMeasurer for FontLibrary {
    fn measure(&self, request: &TextRequest<'_>) -> TextLayout {
        self.layout(request.text, request.style, request.wrap_width)
            .to_layout()
    }
}

impl FontLibrary {
    /// Lays out `text`, or finds it laid out already.
    pub(crate) fn layout(
        &self,
        text: &str,
        style: &TextStyle,
        wrap_width: Option<f64>,
    ) -> Arc<TextBlock> {
        let mut state = self.state();
        let key = LayoutKey::new(text, style, wrap_width);
        let generation = state.generation;
        if let Some(block) = state.layouts.get(&key, generation) {
            return block;
        }
        let block = Arc::new(lay_out(&mut state, text, style, wrap_width));
        state.layouts.insert(key, block.clone(), generation);
        block
    }
}

/// What a laid-out text depends on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LayoutKey {
    text: String,
    family: String,
    weight: u16,
    italic: bool,
    /// Sizes as bits, so that the key can be hashed.
    numbers: [u64; 3],
    /// The alignment's factor, as bits.
    align: u64,
    wrap_width: Option<u64>,
}

impl LayoutKey {
    fn new(text: &str, style: &TextStyle, wrap_width: Option<f64>) -> Self {
        Self {
            text: text.to_owned(),
            family: style.font.family.clone(),
            weight: style.font.weight,
            italic: style.font.italic,
            numbers: [style.font_size, style.line_height, style.letter_spacing].map(f64::to_bits),
            align: style.align.factor().to_bits(),
            wrap_width: wrap_width.map(f64::to_bits),
        }
    }
}

/// Laid-out texts, the most recently used kept.
#[derive(Default)]
pub(crate) struct LayoutCache {
    entries: HashMap<LayoutKey, (Arc<TextBlock>, u64)>,
    /// The font generation the entries were laid out with.
    generation: u64,
    /// Counts uses, to know which entries are old.
    clock: u64,
}

impl LayoutCache {
    fn get(&mut self, key: &LayoutKey, generation: u64) -> Option<Arc<TextBlock>> {
        if generation != self.generation {
            self.entries.clear();
            self.generation = generation;
            return None;
        }
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|(block, used)| {
            *used = clock;
            block.clone()
        })
    }

    fn insert(&mut self, key: LayoutKey, block: Arc<TextBlock>, generation: u64) {
        if generation != self.generation {
            self.entries.clear();
            self.generation = generation;
        }
        if self.entries.len() >= CACHE_ENTRIES {
            // Forget the least recently used quarter in one go, so that
            // this happens rarely.
            let mut uses: Vec<u64> = self.entries.values().map(|(_, used)| *used).collect();
            uses.sort_unstable();
            let cut = uses[uses.len() / 4];
            self.entries.retain(|_, (_, used)| *used > cut);
        }
        self.clock += 1;
        self.entries.insert(key, (block, self.clock));
    }

    /// Number of texts kept.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// A number that can be laid out with: finite, else `fallback`.
fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

/// Skia's text style for a scene text style.
fn skia_style(style: &TextStyle) -> (sk::textlayout::TextStyle, StrutStyle) {
    let size =
        finite_or(style.font_size, FONT_SIZE_RANGE.0).clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1);
    let line_height = finite_or(style.line_height, size * 1.2).max(0.0);
    let families = families(&style.font.family);
    let font_style = sk::FontStyle::new(
        sk::font_style::Weight::from(i32::from(style.font.weight.clamp(1, 1000))),
        sk::font_style::Width::NORMAL,
        if style.font.italic {
            sk::font_style::Slant::Italic
        } else {
            sk::font_style::Slant::Upright
        },
    );

    let mut text = sk::textlayout::TextStyle::new();
    text.set_font_families(&families);
    text.set_font_style(font_style);
    text.set_font_size(size as f32);
    text.set_height((line_height / size) as f32);
    text.set_height_override(true);
    text.set_half_leading(true);
    text.set_letter_spacing(finite_or(style.letter_spacing, 0.0) as f32);
    // Unhinted, at fractional positions: text keeps its shape at every
    // scale and moves smoothly when animated.
    text.set_font_hinting(sk::FontHinting::None);
    text.set_subpixel(true);
    text.set_font_edging(sk::font::Edging::AntiAlias);
    text.set_color(sk::Color::BLACK);

    // The strut makes every line exactly `line_height` tall, even where a
    // fallback font (emoji, say) is taller than the main one.
    let mut strut = StrutStyle::new();
    strut.set_font_families(&families);
    strut.set_font_style(font_style);
    strut.set_font_size(size as f32);
    strut.set_height((line_height / size) as f32);
    strut.set_height_override(true);
    strut.set_half_leading(true);
    strut.set_strut_enabled(true);
    strut.set_force_strut_height(true);
    (text, strut)
}

/// The families to look for, in order: the one asked for, then Inter.
fn families(family: &str) -> Vec<&str> {
    let mut families = Vec::with_capacity(2);
    if !family.trim().is_empty() && !family.eq_ignore_ascii_case(DEFAULT_FAMILY) {
        families.push(family);
    }
    families.push(DEFAULT_FAMILY);
    families
}

/// Builds a font collection over a library's fonts.
pub(crate) fn collection(state: &FontState) -> FontCollection {
    let mut collection = FontCollection::new();
    collection.set_asset_font_manager(Some(state.provider.clone().into()));
    match &state.system {
        Some(system) => collection.set_default_font_manager(system.clone(), None),
        None => {
            collection.set_default_font_manager(None, None);
            collection.disable_font_fallback();
        }
    }
    collection
}

/// Lays text out with the library's fonts.
fn lay_out(
    state: &mut FontState,
    text: &str,
    style: &TextStyle,
    wrap_width: Option<f64>,
) -> TextBlock {
    let (text_style, strut) = skia_style(style);
    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_text_style(&text_style);
    paragraph_style.set_strut_style(strut);
    paragraph_style.set_text_align(sk::textlayout::TextAlign::Left);
    paragraph_style.set_apply_rounding_hack(false);
    paragraph_style.turn_hinting_off();

    let mut builder = ParagraphBuilder::new(&paragraph_style, state.collection.clone());
    builder.push_style(&text_style);
    builder.add_text(text);
    let mut paragraph = builder.build();
    let wrap_width = wrap_width
        .filter(|width| width.is_finite())
        .map(|width| width.max(0.0));
    paragraph.layout(wrap_width.map_or(f32::INFINITY, |width| width as f32));

    let line_height = f64::from(text_style.height() * text_style.font_size());
    Builder::new(text, &mut paragraph, line_height).finish(wrap_width, style.align)
}

/// Copies what a laid-out paragraph knows into a [`TextBlock`].
struct Builder<'a> {
    text: &'a str,
    paragraph: &'a mut Paragraph,
    line_height: f64,
    offsets: Offsets,
    /// Per line: byte range (start, end before trailing whitespace, end
    /// with the line break) and baseline.
    lines: Vec<(usize, usize, usize, f64)>,
}

impl<'a> Builder<'a> {
    fn new(text: &'a str, paragraph: &'a mut Paragraph, line_height: f64) -> Self {
        let offsets = Offsets::new(text);
        let mut metrics = paragraph.get_line_metrics();
        metrics.sort_by_key(|line| line.line_number);
        // A line starts where the one before it ended. (For an empty last
        // line Skia reports a start on the line break before it.)
        let mut ended = 0;
        let mut lines: Vec<(usize, usize, usize, f64)> = metrics
            .iter()
            .map(|line| {
                let start = offsets.utf8(line.start_index).max(ended);
                let end = offsets.utf8(line.end_excluding_whitespaces).max(start);
                ended = offsets.utf8(line.end_including_newline).max(end);
                (start, end, ended, line.baseline)
            })
            .collect();
        if lines.is_empty() {
            // Text always has a line, even when it is empty.
            lines.push((0, 0, text.len(), line_height / 2.0));
        }
        Self {
            text,
            paragraph,
            line_height,
            offsets,
            lines,
        }
    }

    fn finish(mut self, wrap_width: Option<f64>, align: TextAlign) -> TextBlock {
        let mut runs = self.glyph_runs();

        // The ink of each line from side to side, as Skia placed it.
        let mut ink: Vec<Option<(f64, f64)>> = vec![None; self.lines.len()];
        for (line, run) in &runs {
            let mut widths = vec![0.0; run.glyphs.len()];
            run.font.get_widths(&run.glyphs, &mut widths);
            for (position, width) in run.positions.iter().zip(widths) {
                let (left, right) = (f64::from(position.x), f64::from(position.x + width));
                let extent = ink[*line].get_or_insert((left, right));
                *extent = (extent.0.min(left), extent.1.max(right));
            }
        }
        let width_of =
            |extent: &Option<(f64, f64)>| extent.map_or(0.0, |(left, right)| right - left);
        let width = ink.iter().map(width_of).fold(0.0, f64::max);
        let block = wrap_width.unwrap_or(width);
        // How far to move each line to align it.
        let shifts: Vec<f64> = ink
            .iter()
            .map(|extent| {
                let left = (block - width_of(extent)) * align.factor();
                left - extent.map_or(0.0, |(start, _)| start)
            })
            .collect();

        for (line, run) in &mut runs {
            let shift = shifts[*line] as f32;
            for position in &mut run.positions {
                position.x += shift;
            }
        }
        let lines = self
            .lines
            .iter()
            .zip(&ink)
            .enumerate()
            .map(|(number, (&(start, end, _, _), extent))| {
                let line_width = width_of(extent);
                TextLine {
                    start,
                    end: end.max(start),
                    rect: Rect::new(
                        (block - line_width) * align.factor(),
                        number as f64 * self.line_height,
                        line_width,
                        self.line_height,
                    ),
                }
            })
            .collect();
        let lefts: Vec<f64> = ink
            .iter()
            .map(|extent| (block - width_of(extent)) * align.factor())
            .collect();
        let clusters = self.clusters(&shifts, &lefts);
        TextBlock {
            width,
            height: self.lines.len() as f64 * self.line_height,
            lines,
            clusters,
            runs: runs.into_iter().map(|(_, run)| run).collect(),
        }
    }

    /// The glyphs Skia placed, with the line each is on.
    fn glyph_runs(&mut self) -> Vec<(usize, GlyphRun)> {
        let mut runs = Vec::new();
        let lines = &self.lines;
        self.paragraph.visit(|line, info| {
            let Some(info) = info else {
                return;
            };
            let Some(&(_, _, _, baseline)) = lines.get(line) else {
                return;
            };
            let origin = info.origin();
            let mut font = info.font().clone();
            font.set_hinting(sk::FontHinting::None);
            font.set_subpixel(true);
            font.set_baseline_snap(false);
            font.set_edging(sk::font::Edging::AntiAlias);
            let count = info.count();
            let mut bounds = vec![sk::Rect::default(); count];
            font.get_bounds(info.glyphs(), &mut bounds, None);
            runs.push((
                line,
                GlyphRun {
                    font,
                    glyphs: info.glyphs().to_vec(),
                    // Skia rounds the baseline it reports here; the line's
                    // own baseline is exact.
                    positions: info
                        .positions()
                        .iter()
                        .map(|at| sk::Point::new(origin.x + at.x, baseline as f32 + at.y))
                        .collect(),
                    starts: info.utf8_starts()[..count]
                        .iter()
                        .map(|start| *start as usize)
                        .collect(),
                    bounds,
                },
            ));
        });
        runs
    }

    /// The grapheme clusters, each as wide as its advance and as tall as a
    /// line, moved with their line by `shifts`. `lefts` is where each line
    /// starts once aligned.
    fn clusters(&self, shifts: &[f64], lefts: &[f64]) -> Vec<TextCluster> {
        let mut clusters = Vec::new();
        let mut line = 0;
        for (start, cluster) in self.text.grapheme_indices(true) {
            if cluster.contains('\n') {
                continue;
            }
            let end = start + cluster.len();
            while line + 1 < self.lines.len() && self.lines[line].2 <= start {
                line += 1;
            }
            let boxes = self.paragraph.get_rects_for_range(
                self.offsets.utf16(start)..self.offsets.utf16(end),
                RectHeightStyle::Tight,
                RectWidthStyle::Tight,
            );
            let span = boxes
                .iter()
                .fold(None, |span: Option<(f32, f32)>, text_box| {
                    let (left, right) = (text_box.rect.left, text_box.rect.right);
                    Some(span.map_or((left, right), |(a, b)| (a.min(left), b.max(right))))
                });
            let top = line as f64 * self.line_height;
            let rect = match span {
                Some((left, right)) => Rect::new(
                    f64::from(left) + shifts[line],
                    top,
                    f64::from(right - left),
                    self.line_height,
                ),
                // Nothing drawn (a zero-width mark, say): a sliver where
                // the previous letter on the line ends.
                None => {
                    let x = clusters
                        .last()
                        .filter(|previous: &&TextCluster| previous.line == line)
                        .map_or(lefts[line], |previous| previous.rect.right());
                    Rect::new(x, top, 0.0, self.line_height)
                }
            };
            clusters.push(TextCluster {
                start,
                end,
                rect,
                line,
            });
        }
        clusters
    }
}

/// Converts between byte offsets (UTF-8) and the UTF-16 offsets some of
/// Skia's answers use.
struct Offsets {
    /// Byte offset of every UTF-16 offset, one past the end included.
    utf8_of: Vec<usize>,
    /// UTF-16 offset of every byte offset, one past the end included.
    utf16_of: Vec<usize>,
}

impl Offsets {
    fn new(text: &str) -> Self {
        let mut utf8_of = Vec::with_capacity(text.len() + 1);
        let mut utf16_of = vec![0; text.len() + 1];
        for (byte, character) in text.char_indices() {
            let unit = utf8_of.len();
            for _ in 0..character.len_utf16() {
                utf8_of.push(byte);
            }
            utf16_of[byte..byte + character.len_utf8()].fill(unit);
        }
        utf16_of[text.len()] = utf8_of.len();
        utf8_of.push(text.len());
        Self { utf8_of, utf16_of }
    }

    fn utf8(&self, utf16: usize) -> usize {
        self.utf8_of[utf16.min(self.utf8_of.len() - 1)]
    }

    fn utf16(&self, utf8: usize) -> usize {
        self.utf16_of[utf8.min(self.utf16_of.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::model::{Font, TextSplit};
    use moshionly_core::text::split;

    fn style(size: f64, line_height: f64, letter_spacing: f64, align: TextAlign) -> TextStyle {
        TextStyle {
            font: Font::default(),
            font_size: size,
            line_height,
            letter_spacing,
            align,
        }
    }

    fn measure(text: &str, style: &TextStyle, wrap_width: Option<f64>) -> TextLayout {
        FontLibrary::bundled().measure(&TextRequest {
            text,
            style,
            wrap_width,
        })
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn lines_are_exactly_the_line_height_apart() {
        let layout = measure(
            "Hello\nWorld",
            &style(20.0, 30.0, 0.0, TextAlign::Left),
            None,
        );
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.height, 60.0);
        let line = |n: usize| layout.lines[n];
        assert_eq!((line(0).start, line(0).end), (0, 5));
        assert_eq!((line(1).start, line(1).end), (6, 11));
        assert_eq!((line(0).rect.y, line(0).rect.height), (0.0, 30.0));
        assert_eq!((line(1).rect.y, line(1).rect.height), (30.0, 30.0));
        assert!(line(0).rect.width > 40.0 && line(0).rect.width < 70.0);
        assert!(close(
            layout.width,
            line(0).rect.width.max(line(1).rect.width)
        ));
        // Tighter and looser lines: only the spacing changes.
        let tight = measure("A\nB\nC", &style(20.0, 10.0, 0.0, TextAlign::Left), None);
        assert_eq!(tight.height, 30.0);
        assert_eq!(tight.lines[2].rect.y, 20.0);
    }

    #[test]
    fn clusters_are_the_letters_a_reader_sees() {
        // "e" with a combining accent is one letter of three bytes.
        let text = "He\u{301}llo\nwörld";
        let layout = measure(text, &style(20.0, 24.0, 0.0, TextAlign::Left), None);
        let letters: Vec<&str> = layout
            .clusters
            .iter()
            .map(|cluster| &text[cluster.start..cluster.end])
            .collect();
        assert_eq!(
            letters,
            ["H", "e\u{301}", "l", "l", "o", "w", "ö", "r", "l", "d"]
        );
        let lines: Vec<usize> = layout.clusters.iter().map(|cluster| cluster.line).collect();
        assert_eq!(lines, [0, 0, 0, 0, 0, 1, 1, 1, 1, 1]);
        for pair in layout.clusters.windows(2) {
            if pair[0].line == pair[1].line {
                assert!(close(pair[0].rect.right(), pair[1].rect.x), "{pair:?}");
            }
        }
        for cluster in &layout.clusters {
            assert!(cluster.rect.width > 0.0, "{cluster:?}");
            assert_eq!(cluster.rect.height, 24.0);
            assert_eq!(cluster.rect.y, cluster.line as f64 * 24.0);
        }
        assert_eq!(layout.clusters[0].rect.x, 0.0, "the ink starts the line");
        let last = layout.clusters[4];
        assert!(close(last.rect.right(), layout.lines[0].rect.right()));
    }

    #[test]
    fn letter_spacing_follows_every_letter_and_counts_between_them() {
        let plain = measure("Wave", &style(30.0, 36.0, 0.0, TextAlign::Left), None);
        let spaced = measure("Wave", &style(30.0, 36.0, 6.0, TextAlign::Left), None);
        // Three gaps between four letters.
        assert!(
            close(spaced.width, plain.width + 18.0),
            "{} {}",
            spaced.width,
            plain.width
        );
        for (wide, narrow) in spaced.clusters.iter().zip(&plain.clusters) {
            assert!(close(wide.rect.width, narrow.rect.width + 6.0));
        }
        assert_eq!(spaced.clusters[0].rect.x, 0.0);
        // Pulling letters together works the same way.
        let tight = measure("Wave", &style(30.0, 36.0, -2.0, TextAlign::Left), None);
        assert!(close(tight.width, plain.width - 6.0));
    }

    #[test]
    fn lines_align_inside_the_block() {
        let text = "Hi\nHello";
        let left = measure(text, &style(20.0, 24.0, 0.0, TextAlign::Left), None);
        let short = left.lines[0].rect.width;
        let long = left.lines[1].rect.width;
        assert!(close(left.width, long));
        assert_eq!(left.lines[0].rect.x, 0.0);

        let centre = measure(text, &style(20.0, 24.0, 0.0, TextAlign::Center), None);
        assert!(close(centre.lines[0].rect.x, (long - short) / 2.0));
        assert!(close(centre.lines[1].rect.x, 0.0));
        assert!(close(centre.clusters[0].rect.x, centre.lines[0].rect.x));

        let right = measure(text, &style(20.0, 24.0, 0.0, TextAlign::Right), Some(200.0));
        assert!(close(right.lines[0].rect.right(), 200.0));
        assert!(close(right.lines[1].rect.right(), 200.0));
        assert!(close(right.clusters[1].rect.right(), 200.0));
        assert!(
            close(right.width, long),
            "the width is the ink, not the block"
        );
    }

    #[test]
    fn wrapping_breaks_lines_between_words() {
        let style = style(20.0, 24.0, 0.0, TextAlign::Left);
        let one_line = measure("one two", &style, None);
        let layout = measure("one two three", &style, Some(one_line.width + 2.0));
        assert_eq!(layout.lines.len(), 2);
        assert_eq!((layout.lines[0].start, layout.lines[0].end), (0, 7));
        assert_eq!(layout.lines[1].start, 8);
        assert!(close(layout.lines[0].rect.width, one_line.width));
        let space = layout.clusters[7];
        assert_eq!((space.start, space.line), (7, 0), "the space stays behind");
        assert_eq!(layout.clusters[8].line, 1);
        assert_eq!(layout.clusters[8].rect.x, 0.0);
        // Room for the longest word only: every word gets a line.
        let longest = measure("three", &style, None).width;
        assert_eq!(
            measure("one two three", &style, Some(longest + 1.0))
                .lines
                .len(),
            3
        );
        // A word longer than the line is broken between letters rather
        // than left to stick out.
        let broken = measure("three", &style, Some(longest / 2.0));
        assert!(broken.lines.len() >= 2, "{:?}", broken.lines);
        assert!(
            broken
                .lines
                .iter()
                .all(|line| line.rect.width <= longest / 2.0 + 0.01)
        );
    }

    #[test]
    fn empty_text_and_trailing_breaks_still_have_lines() {
        let style = style(20.0, 24.0, 0.0, TextAlign::Center);
        let empty = measure("", &style, None);
        assert_eq!(empty.lines.len(), 1);
        assert_eq!((empty.width, empty.height), (0.0, 24.0));
        assert!(empty.clusters.is_empty());
        let trailing = measure("abc\n", &style, None);
        assert_eq!(trailing.lines.len(), 2);
        assert_eq!(trailing.height, 48.0);
        assert_eq!((trailing.lines[1].start, trailing.lines[1].end), (4, 4));
        // Nonsense sizes lay out as something small rather than failing.
        let broken = measure(
            "abc",
            &self::style(f64::NAN, f64::NAN, f64::NAN, TextAlign::Left),
            None,
        );
        assert_eq!(broken.lines.len(), 1);
        assert!(broken.width.is_finite() && broken.height.is_finite());
    }

    #[test]
    fn offsets_convert_between_utf8_and_utf16() {
        // "a" is 1 byte and 1 unit, the emoji 4 bytes and 2 units, "é" 2
        // bytes and 1 unit.
        let offsets = Offsets::new("a😀é");
        assert_eq!(
            (0..=4).map(|unit| offsets.utf8(unit)).collect::<Vec<_>>(),
            [0, 1, 1, 5, 7]
        );
        assert_eq!([0, 1, 5, 7].map(|byte| offsets.utf16(byte)), [0, 1, 3, 4]);
        assert_eq!(offsets.utf8(99), 7);
    }

    #[test]
    fn glyphs_carry_the_offsets_of_their_letters() {
        let library = FontLibrary::bundled();
        let text = "ab\ncd";
        let block = library.layout(text, &style(20.0, 24.0, 0.0, TextAlign::Center), None);
        let starts: Vec<usize> = block
            .runs
            .iter()
            .flat_map(|run| run.starts.clone())
            .collect();
        assert_eq!(starts, [0, 1, 3, 4]);
        let first = block.runs[0].positions[0];
        assert!(close(f64::from(first.x), block.clusters[0].rect.x));
        // On the baseline: below the middle of the first line, above its
        // bottom.
        assert!(first.y > 12.0 && first.y < 24.0, "{first:?}");
    }

    #[test]
    fn weights_pick_the_nearest_face_and_missing_families_fall_back() {
        let library = FontLibrary::bundled();
        let face = |family: &str, weight: u16| {
            let mut style = style(20.0, 24.0, 0.0, TextAlign::Left);
            style.font = Font {
                family: family.into(),
                weight,
                italic: false,
            };
            let block = library.layout("x", &style, None);
            let typeface = block.runs[0].font.typeface();
            (typeface.family_name(), *typeface.font_style().weight())
        };
        assert_eq!(face("Inter", 400), ("Inter".into(), 400));
        assert_eq!(face("Inter", 500), ("Inter".into(), 500));
        assert_eq!(
            face("Inter", 700),
            ("Inter".into(), 600),
            "the heaviest there is"
        );
        assert_eq!(
            face("Inter", 100),
            ("Inter".into(), 400),
            "the lightest there is"
        );
        assert_eq!(face("No Such Font", 500), ("Inter".into(), 500));
    }

    #[test]
    fn letters_split_for_animation_are_the_clusters() {
        let text = "Go on";
        let layout = measure(text, &style(20.0, 24.0, 0.0, TextAlign::Left), None);
        let pieces = split(text, &layout, TextSplit::Letters);
        assert_eq!(pieces.len(), 4, "the space is not a piece");
        let words = split(text, &layout, TextSplit::Words);
        assert_eq!((words[1].start, words[1].end), (3, 5));
        assert!(close(words[1].rect.x, layout.clusters[3].rect.x));
    }

    #[test]
    fn core_sizes_auto_sized_text_by_this_measurer() {
        let document = moshionly_core::io::parse(
            r#"{"format":"moshionly","version":1,"name":"t","nodes":[
                {"id":"ab","parent":null,"index":"a0","type":"artboard","name":"A",
                 "width":400,"height":200},
                {"id":"hug","parent":"ab","index":"a0","type":"text","name":"Hug",
                 "x":10,"y":10,"content":"Hello there","font_size":32},
                {"id":"wrap","parent":"ab","index":"a1","type":"text","name":"Wrap",
                 "x":10,"y":80,"width":120,"height":10,"auto_size":"height",
                 "content":"Hello there","font_size":32,"line_height":125}
            ]}"#,
        )
        .unwrap();
        let library = FontLibrary::bundled();
        let scene = moshionly_core::evaluate_at_rest(
            &document,
            &moshionly_core::NodeId::from("ab"),
            &library,
        )
        .unwrap();
        let hug = scene.find(&moshionly_core::NodeId::from("hug")).unwrap();
        let measured = measure(
            "Hello there",
            &style(32.0, 32.0 * 1.2, 0.0, TextAlign::Left),
            None,
        );
        assert!(
            close(hug.width, measured.width),
            "{} {}",
            hug.width,
            measured.width
        );
        // The default line height is 120% of the size.
        assert!(close(hug.height, 38.4), "{}", hug.height);
        let wrap = scene.find(&moshionly_core::NodeId::from("wrap")).unwrap();
        assert_eq!(wrap.width, 120.0);
        assert_eq!(wrap.height, 80.0, "two lines of 40");
    }

    #[test]
    fn layouts_are_kept_until_the_fonts_change() {
        let library = FontLibrary::bundled();
        let style = style(20.0, 24.0, 0.0, TextAlign::Left);
        let first = library.layout("cached", &style, None);
        assert!(Arc::ptr_eq(&first, &library.layout("cached", &style, None)));
        assert!(!Arc::ptr_eq(
            &first,
            &library.layout("cached", &style, Some(30.0))
        ));
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/Inter-Medium.ttf");
        library.add_font_file(path).unwrap();
        let after = library.layout("cached", &style, None);
        assert!(!Arc::ptr_eq(&first, &after));
        assert_eq!(*first, *after, "same fonts, same layout");

        // The cache stays bounded.
        for n in 0..CACHE_ENTRIES + 10 {
            library.layout(&n.to_string(), &style, None);
        }
        assert!(library.state().layouts.len() <= CACHE_ENTRIES);
    }
}
