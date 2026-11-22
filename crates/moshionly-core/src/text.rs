//! Text helpers for evaluation: measuring, splitting into units, and the
//! number inside a text that a counter animates.
//!
//! Text needs fonts to be measured, and fonts belong to the renderer. The
//! engine therefore asks a [`TextMeasurer`] where lines and letters are. The
//! renderer supplies a real one; [`MonospaceMeasurer`] is an estimate that
//! needs no fonts, for tests and headless use.

use unicode_segmentation::UnicodeSegmentation;

use crate::geometry::Rect;
use crate::model::TextSplit;
use crate::scene::TextStyle;

/// Lays text out and reports where its lines and letters are.
pub trait TextMeasurer {
    /// Measures `request.text`.
    ///
    /// Lines break at `\n` and, when `request.wrap_width` is set, wherever a
    /// line would otherwise pass that width. The block is `wrap_width` wide
    /// when that is set and as wide as its widest line otherwise; each line
    /// is aligned inside the block by `request.style.align`. All rectangles
    /// in the answer are relative to the top-left corner of the block.
    fn measure(&self, request: &TextRequest<'_>) -> TextLayout;
}

/// What to measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextRequest<'a> {
    /// The text, with letter case already applied.
    pub text: &'a str,
    /// Font, size, spacing and alignment, in pixels.
    pub style: &'a TextStyle,
    /// Width at which lines wrap. `None`: only at line breaks in the text.
    pub wrap_width: Option<f64>,
}

/// Where the lines and letters of a text are.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLayout {
    /// Width of the widest line in pixels, not counting trailing spaces.
    pub width: f64,
    /// Height of all lines together in pixels.
    pub height: f64,
    /// The lines, top to bottom. Text always has at least one line.
    pub lines: Vec<TextLine>,
    /// The grapheme clusters (what a reader sees as one letter), in reading
    /// order, whitespace included.
    pub clusters: Vec<TextCluster>,
}

/// One line of laid-out text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLine {
    /// Byte offset in the text where the line starts.
    pub start: usize,
    /// Byte offset just past the line, before its line break.
    pub end: usize,
    /// The box of the line: its ink from side to side, a full line height tall.
    pub rect: Rect,
}

/// One grapheme cluster of laid-out text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextCluster {
    /// Byte offset in the text where the cluster starts.
    pub start: usize,
    /// Byte offset just past the cluster.
    pub end: usize,
    /// The box of the cluster: its advance wide, a full line height tall.
    pub rect: Rect,
    /// Index of the line the cluster is on.
    pub line: usize,
}

/// A measurer that needs no fonts: every letter is `advance` times the font
/// size wide, whatever the font.
///
/// It is good enough to lay out a scene headlessly and to test text
/// animation. Real text is narrower or wider letter by letter, so anything
/// drawn to pixels should use the renderer's measurer instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonospaceMeasurer {
    /// Width of a letter as a fraction of the font size.
    pub advance: f64,
}

impl Default for MonospaceMeasurer {
    /// Letters 0.6 of the font size wide, about the average of a text face.
    fn default() -> Self {
        Self { advance: 0.6 }
    }
}

impl TextMeasurer for MonospaceMeasurer {
    fn measure(&self, request: &TextRequest<'_>) -> TextLayout {
        let style = request.style;
        let advance = (self.advance * style.font_size + style.letter_spacing).max(0.0);
        let limit = request.wrap_width.filter(|width| width.is_finite());

        // Break the text into lines of clusters: (start, end, is_space).
        let mut lines: Vec<Vec<(usize, usize, bool)>> = Vec::new();
        let mut line_starts: Vec<usize> = Vec::new();
        let mut offset = 0;
        for paragraph in request.text.split('\n') {
            let mut line = Vec::new();
            let mut line_start = offset;
            // Width of the line up to its last non-space cluster.
            let mut ink = 0usize;
            for (word, is_space) in runs(paragraph) {
                let clusters: Vec<(usize, usize, bool)> = word
                    .grapheme_indices(true)
                    .map(|(at, cluster)| {
                        let start = offset + at;
                        (start, start + cluster.len(), is_space)
                    })
                    .collect();
                let too_wide = limit.is_some_and(|limit| {
                    (line.len() + clusters.len()) as f64 * advance > limit + 1e-9
                });
                if !is_space && too_wide && ink > 0 {
                    lines.push(std::mem::take(&mut line));
                    line_starts.push(line_start);
                    line_start = offset;
                    ink = 0;
                }
                line.extend(clusters);
                if !is_space {
                    ink = line.len();
                }
                offset += word.len();
            }
            lines.push(line);
            line_starts.push(line_start);
            offset += 1; // the line break
        }

        let ink_width = |line: &[(usize, usize, bool)]| {
            let ink = line
                .iter()
                .rposition(|cluster| !cluster.2)
                .map_or(0, |i| i + 1);
            // Letter spacing follows every letter but the last one.
            (ink as f64 * advance - if ink > 0 { style.letter_spacing } else { 0.0 }).max(0.0)
        };
        let width = lines.iter().map(|line| ink_width(line)).fold(0.0, f64::max);
        let block = limit.unwrap_or(width);
        let mut layout = TextLayout {
            width,
            height: lines.len() as f64 * style.line_height,
            ..TextLayout::default()
        };
        for (number, (line, start)) in lines.iter().zip(line_starts).enumerate() {
            let line_width = ink_width(line);
            let left = (block - line_width) * style.align.factor();
            let top = number as f64 * style.line_height;
            let ink_end = line
                .iter()
                .rfind(|cluster| !cluster.2)
                .map_or(start, |cluster| cluster.1);
            layout.lines.push(TextLine {
                start,
                end: ink_end.max(start),
                rect: Rect::new(left, top, line_width, style.line_height),
            });
            for (position, (start, end, _)) in line.iter().enumerate() {
                layout.clusters.push(TextCluster {
                    start: *start,
                    end: *end,
                    rect: Rect::new(
                        left + position as f64 * advance,
                        top,
                        advance,
                        style.line_height,
                    ),
                    line: number,
                });
            }
        }
        layout
    }
}

/// Cuts a paragraph into runs of whitespace and runs of everything else,
/// each with whether it is whitespace. Lines may break before a run that is
/// not whitespace.
fn runs(paragraph: &str) -> Vec<(&str, bool)> {
    let mut runs: Vec<(&str, bool)> = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (at, cluster) in paragraph.grapheme_indices(true) {
        let is_space = cluster.chars().all(char::is_whitespace);
        if current.is_some_and(|current| current != is_space) {
            runs.push((&paragraph[start..at], !is_space));
            start = at;
        }
        current = Some(is_space);
    }
    if let Some(is_space) = current {
        runs.push((&paragraph[start..], is_space));
    }
    runs
}

/// A piece of text that a text animation moves as one: a letter, a word or
/// a line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPiece {
    /// Byte offset in the text where the piece starts.
    pub start: usize,
    /// Byte offset just past the piece.
    pub end: usize,
    /// The box around the piece, relative to the top-left of the text block.
    pub rect: Rect,
}

/// Cuts laid-out text into the pieces a text animation works on.
///
/// Whitespace is never a piece or part of the edge of one: letters skip it,
/// words are the runs between it, and a line runs from its first to its last
/// visible letter. Lines without visible letters give no piece.
pub fn split(text: &str, layout: &TextLayout, by: TextSplit) -> Vec<TextPiece> {
    let visible = |cluster: &&TextCluster| {
        text.get(cluster.start..cluster.end)
            .is_some_and(|letters| !letters.chars().all(char::is_whitespace))
    };
    let mut pieces: Vec<TextPiece> = Vec::new();
    // The cluster that the piece being built ends with, to know whether the
    // next visible cluster continues it.
    let mut previous: Option<(usize, usize)> = None;
    for (position, cluster) in layout.clusters.iter().enumerate() {
        if !visible(&cluster) {
            continue;
        }
        let continues = previous.is_some_and(|(at, line)| {
            line == cluster.line
                && match by {
                    TextSplit::Letters => false,
                    TextSplit::Words => at + 1 == position,
                    TextSplit::Lines => true,
                }
        });
        match pieces.last_mut() {
            Some(piece) if continues => {
                piece.end = cluster.end;
                piece.rect = piece.rect.union(&cluster.rect);
            }
            _ => pieces.push(TextPiece {
                start: cluster.start,
                end: cluster.end,
                rect: cluster.rect,
            }),
        }
        previous = Some((position, cluster.line));
    }
    pieces
}

/// A number found inside a text, and how it was written.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberInText {
    /// Byte offset where the number starts, its sign included.
    pub start: usize,
    /// Byte offset just past the number.
    pub end: usize,
    /// The number.
    pub value: f64,
    /// How to write another number the same way.
    pub format: NumberFormat,
}

/// The way a number is written: decimals, separators, padding and sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberFormat {
    /// Digits after the decimal separator.
    pub decimals: usize,
    /// Character between groups of three digits, if any.
    pub thousands: Option<char>,
    /// The decimal separator: `.` or `,`.
    pub decimal_point: char,
    /// Digits before the decimal separator, padded with zeros: `007` has 3.
    pub min_integer_digits: usize,
    /// Whether positive numbers are written with a `+`.
    pub plus_sign: bool,
    /// The character used for a minus sign.
    pub minus: char,
}

/// Thousands and decimal separators that are tried, most likely first.
const NUMBER_STYLES: [(Option<char>, char); 11] = [
    (Some(','), '.'),
    (None, '.'),
    (Some(' '), '.'),
    (Some('\u{a0}'), '.'),
    (Some('\u{202f}'), '.'),
    (Some('\''), '.'),
    (Some('.'), ','),
    (None, ','),
    (Some(' '), ','),
    (Some('\u{a0}'), ','),
    (Some('\u{202f}'), ','),
];

/// Number of ASCII digits at the start of `text`.
fn leading_digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

/// Reads a number at the start of `text` in one style. Returns how many
/// bytes it takes, its digits as a plain decimal string, and its format.
fn read_number(
    text: &str,
    thousands: Option<char>,
    decimal_point: char,
) -> Option<(usize, String, NumberFormat)> {
    let first = leading_digits(text);
    if first == 0 {
        return None;
    }
    let mut end = first;
    let mut digits = text[..first].to_owned();
    let mut grouped = None;
    if let Some(separator) = thousands
        && first <= 3
    {
        while let Some(rest) = text[end..].strip_prefix(separator)
            && leading_digits(rest) == 3
        {
            digits.push_str(&rest[..3]);
            end += separator.len_utf8() + 3;
            grouped = Some(separator);
        }
    }
    let mut decimals = 0;
    if let Some(rest) = text[end..].strip_prefix(decimal_point) {
        decimals = leading_digits(rest);
        if decimals > 0 {
            digits.push('.');
            digits.push_str(&rest[..decimals]);
            end += decimal_point.len_utf8() + decimals;
        }
    }
    let padded = grouped.is_none() && first > 1 && text.starts_with('0');
    Some((
        end,
        digits,
        NumberFormat {
            decimals,
            thousands: grouped,
            decimal_point,
            min_integer_digits: if padded { first } else { 1 },
            plus_sign: false,
            minus: '-',
        },
    ))
}

/// Finds the first number in a text.
///
/// It understands thousands separators (`1,250`, `1 250`, `1'250`,
/// `1.250,50`), decimals with a point or a comma, leading zeros (`007`) and
/// a sign in front. Where a number can be read in more than one way, the
/// reading that takes in the most characters wins, and among equals the more
/// common notation: `1,250` is one thousand two hundred and fifty.
pub fn find_number(text: &str) -> Option<NumberInText> {
    let at = text.find(|c: char| c.is_ascii_digit())?;
    let (length, digits, mut format) = NUMBER_STYLES
        .iter()
        .filter_map(|(thousands, decimal_point)| {
            read_number(&text[at..], *thousands, *decimal_point)
        })
        // `max_by_key` keeps the last of equals, so look from the back.
        .rev()
        .max_by_key(|(length, ..)| *length)?;
    let mut value: f64 = digits.parse().ok()?;
    // A sign counts when it is attached to the number and not to a word
    // before it, as in "COVID-19".
    let before = &text[..at];
    let sign = before
        .chars()
        .next_back()
        .filter(|c| matches!(c, '-' | '\u{2212}' | '+'));
    let mut start = at;
    if let Some(sign) = sign {
        let earlier = before[..before.len() - sign.len_utf8()].chars().next_back();
        if earlier.is_none_or(|c| c.is_whitespace() || c == '(') {
            start -= sign.len_utf8();
            if sign == '+' {
                format.plus_sign = true;
            } else {
                format.minus = sign;
                value = -value;
            }
        }
    }
    Some(NumberInText {
        start,
        end: at + length,
        value,
        format,
    })
}

impl NumberFormat {
    /// Writes `value` in this format, rounded to its decimals.
    pub fn format(&self, value: f64) -> String {
        let value = if value.is_finite() { value } else { 0.0 };
        let plain = format!("{:.*}", self.decimals, value.abs());
        let (integer, fraction) = plain.split_once('.').unwrap_or((&plain, ""));
        let integer = format!("{integer:0>width$}", width = self.min_integer_digits);
        let mut text = String::new();
        let is_zero = plain.bytes().all(|b| b == b'0' || b == b'.');
        if value < 0.0 && !is_zero {
            text.push(self.minus);
        } else if self.plus_sign && !is_zero {
            text.push('+');
        }
        for (position, digit) in integer.chars().enumerate() {
            if let Some(separator) = self.thousands
                && position > 0
                && (integer.len() - position).is_multiple_of(3)
            {
                text.push(separator);
            }
            text.push(digit);
        }
        if !fraction.is_empty() {
            text.push(self.decimal_point);
            text.push_str(fraction);
        }
        text
    }
}

/// The text with its first number replaced by `value`, written the way the
/// original number was. A text without a number comes back unchanged.
pub fn replace_number(text: &str, value: f64) -> String {
    match find_number(text) {
        Some(number) => format!(
            "{}{}{}",
            &text[..number.start],
            number.format.format(value),
            &text[number.end..]
        ),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Font, TextAlign};

    fn style(align: TextAlign) -> TextStyle {
        TextStyle {
            font: Font::default(),
            font_size: 10.0,
            line_height: 12.0,
            letter_spacing: 0.0,
            align,
        }
    }

    fn measure(text: &str, wrap_width: Option<f64>, align: TextAlign) -> TextLayout {
        MonospaceMeasurer::default().measure(&TextRequest {
            text,
            style: &style(align),
            wrap_width,
        })
    }

    fn lines<'a>(text: &'a str, layout: &TextLayout) -> Vec<&'a str> {
        layout
            .lines
            .iter()
            .map(|line| &text[line.start..line.end])
            .collect()
    }

    #[test]
    fn one_line_is_measured_letter_by_letter() {
        let layout = measure("Hi there", None, TextAlign::Left);
        assert_eq!((layout.width, layout.height), (48.0, 12.0));
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.lines[0].rect, Rect::new(0.0, 0.0, 48.0, 12.0));
        assert_eq!(layout.clusters.len(), 8);
        assert_eq!(layout.clusters[3].rect, Rect::new(18.0, 0.0, 6.0, 12.0));
        assert_eq!((layout.clusters[3].start, layout.clusters[3].end), (3, 4));
    }

    #[test]
    fn line_breaks_in_the_text_always_break() {
        let text = "one\n\nthree ";
        let layout = measure(text, None, TextAlign::Left);
        assert_eq!(lines(text, &layout), ["one", "", "three"]);
        assert_eq!(layout.height, 36.0);
        assert_eq!(layout.width, 30.0, "the trailing space does not count");
        assert_eq!(layout.lines[2].rect.y, 24.0);
        assert_eq!(layout.clusters.last().unwrap().line, 2);

        let empty = measure("", None, TextAlign::Left);
        assert_eq!(
            (empty.width, empty.height, empty.lines.len()),
            (0.0, 12.0, 1)
        );
    }

    #[test]
    fn long_lines_wrap_between_words() {
        let text = "aaa bbb cccc dd";
        // 6 pixels a letter: 8 letters fit in 50.
        let layout = measure(text, Some(50.0), TextAlign::Left);
        assert_eq!(lines(text, &layout), ["aaa bbb", "cccc dd"]);
        assert_eq!(layout.width, 42.0);
        // A word longer than the width stays whole on its own line.
        let long = "a bbbbbbbbbbbb c";
        assert_eq!(
            lines(long, &measure(long, Some(50.0), TextAlign::Left)),
            ["a", "bbbbbbbbbbbb", "c"]
        );
    }

    #[test]
    fn lines_are_aligned_inside_the_block() {
        let text = "aaaa\nbb";
        let centred = measure(text, None, TextAlign::Center);
        // Without wrapping the block is as wide as the widest line.
        assert_eq!(centred.lines[0].rect.x, 0.0);
        assert_eq!(centred.lines[1].rect.x, 6.0);
        assert_eq!(centred.clusters[4].rect.x, 6.0);

        let right = measure(text, Some(100.0), TextAlign::Right);
        assert_eq!(right.lines[0].rect.x, 76.0);
        assert_eq!(right.lines[1].rect.x, 88.0);
        assert_eq!(right.width, 24.0);
    }

    #[test]
    fn letter_spacing_widens_and_clusters_stay_whole() {
        let mut spaced = style(TextAlign::Left);
        spaced.letter_spacing = 2.0;
        let layout = MonospaceMeasurer::default().measure(&TextRequest {
            text: "abc",
            style: &spaced,
            wrap_width: None,
        });
        assert_eq!(layout.width, 22.0, "three letters, two gaps");
        assert_eq!(layout.clusters[1].rect.x, 8.0);

        // A letter with a combining accent, and a flag, are one cluster each.
        let text = "e\u{301}🇷🇼";
        let layout = measure(text, None, TextAlign::Left);
        assert_eq!(layout.clusters.len(), 2);
        assert_eq!(layout.clusters[1].end, text.len());
    }

    fn pieces(text: &str, by: TextSplit) -> Vec<&str> {
        let layout = measure(text, Some(60.0), TextAlign::Left);
        split(text, &layout, by)
            .iter()
            .map(|piece| &text[piece.start..piece.end])
            .collect()
    }

    #[test]
    fn text_splits_into_letters_words_and_lines() {
        let text = "Hi  you\nall of us";
        assert_eq!(pieces(text, TextSplit::Letters).join(""), "Hiyouallofus");
        assert_eq!(
            pieces(text, TextSplit::Words),
            ["Hi", "you", "all", "of", "us"]
        );
        // "all of us" is 9 letters, 54 pixels: it fits in 60.
        assert_eq!(pieces(text, TextSplit::Lines), ["Hi  you", "all of us"]);
        assert!(pieces("  \n ", TextSplit::Lines).is_empty());
        assert!(pieces("", TextSplit::Letters).is_empty());
    }

    #[test]
    fn pieces_have_boxes() {
        let text = "ab cd";
        let layout = measure(text, None, TextAlign::Left);
        let words = split(text, &layout, TextSplit::Words);
        assert_eq!(words[0].rect, Rect::new(0.0, 0.0, 12.0, 12.0));
        assert_eq!(words[1].rect, Rect::new(18.0, 0.0, 12.0, 12.0));
        let line = split(text, &layout, TextSplit::Lines);
        assert_eq!(line[0].rect, Rect::new(0.0, 0.0, 30.0, 12.0));
        // A wrapped word group splits per line.
        let wrapped = "aaaa bbbb cccc";
        let layout = measure(wrapped, Some(60.0), TextAlign::Left);
        let lines = split(wrapped, &layout, TextSplit::Lines);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].rect.y, 12.0);
    }

    fn number(text: &str) -> (String, f64) {
        let found = find_number(text).unwrap();
        (text[found.start..found.end].to_owned(), found.value)
    }

    #[test]
    fn numbers_are_found_in_many_notations() {
        let cases = [
            ("$1,250.50 raised", "1,250.50", 1250.5),
            ("1250", "1250", 1250.0),
            ("Top 10 songs", "10", 10.0),
            ("12.5%", "12.5", 12.5),
            ("10 000 users", "10 000", 10000.0),
            ("2 500 000", "2 500 000", 2_500_000.0),
            ("€ 1.234,56", "1.234,56", 1234.56),
            ("1.250.000", "1.250.000", 1_250_000.0),
            ("1,250", "1,250", 1250.0),
            ("1.250", "1.250", 1.25),
            ("2,75", "2,75", 2.75),
            ("CHF 1'000", "1'000", 1000.0),
            ("In 2024, 5 people", "2024", 2024.0),
            ("Done 5.", "5", 5.0),
            ("Version 1.2.3", "1.2", 1.2),
            ("-40 degrees", "-40", -40.0),
            ("\u{2212}7", "\u{2212}7", -7.0),
            ("+15%", "+15", 15.0),
            ("(-3)", "-3", -3.0),
            ("COVID-19", "19", 19.0),
            ("007", "007", 7.0),
            ("1,25,000", "1,25", 1.25),
            ("12345,678", "12345,678", 12345.678),
        ];
        for (text, expected, value) in cases {
            assert_eq!(number(text), (expected.to_owned(), value), "{text}");
        }
        assert!(find_number("no digits here").is_none());
        assert!(find_number("").is_none());
    }

    #[test]
    fn numbers_are_written_back_the_way_they_were() {
        let cases = [
            ("$1,250.50 raised", 0.0, "$0.00 raised"),
            ("$1,250.50 raised", 625.254, "$625.25 raised"),
            ("$1,250.50 raised", 1_000_000.0, "$1,000,000.00 raised"),
            ("10 000 users", 999.6, "1 000 users"),
            ("€ 1.234,56", 98765.4321, "€ 98.765,43"),
            ("12.5%", 100.0, "100.0%"),
            ("007", 12.0, "012"),
            ("007", 1234.0, "1234"),
            ("+15%", 3.0, "+3%"),
            ("+15%", -3.0, "-3%"),
            ("+15%", 0.0, "0%"),
            ("-40 degrees", 12.0, "12 degrees"),
            ("-40 degrees", -0.2, "0 degrees"),
            ("\u{2212}7", -3.0, "\u{2212}3"),
            ("Top 10 songs", 9.5, "Top 10 songs"),
            ("Top 10 songs", f64::NAN, "Top 0 songs"),
            ("no digits here", 5.0, "no digits here"),
        ];
        for (text, value, expected) in cases {
            assert_eq!(replace_number(text, value), expected, "{text} with {value}");
        }
    }

    #[test]
    fn formats_remember_how_the_number_looked() {
        let found = find_number("Total: 1 250,5 kg").unwrap();
        assert_eq!(
            found.format,
            NumberFormat {
                decimals: 1,
                thousands: Some(' '),
                decimal_point: ',',
                min_integer_digits: 1,
                plus_sign: false,
                minus: '-',
            }
        );
        assert_eq!(found.format.format(-1234567.89), "-1 234 567,9");
    }
}
