//! Drawing text: the glyphs of a laid-out text, unit by unit.
//!
//! A text without a text animation in flight is one unit covering
//! everything, and all its glyphs are drawn at once. With an animation,
//! each unit draws the glyphs whose letters fall in its byte range, with
//! its own transform, opacity and blur. Both ways draw the same glyphs at
//! the same positions with the same paint, so at rest they land on the
//! same pixels.

use std::ops::Range;

use moshionly_core::model::StrokePosition;
use moshionly_core::scene::{SceneStroke, TextContent, TextUnit};
use skia_safe as sk;

use super::geometry::Outline;
use super::{effects, paint, stroke};
use crate::convert;
use crate::text::TextBlock;

/// Draws `text`, laid out as `block`, in its node's coordinates, faded by
/// `opacity`.
pub(crate) fn draw(canvas: &sk::Canvas, text: &TextContent, block: &TextBlock, opacity: f32) {
    if text.fill.is_none() && text.stroke.is_none() {
        return;
    }
    // Glyph positions are relative to the top of the text block, which
    // sits `offset_y` below the top of the box. Paint geometry stays in
    // the box's coordinates, so a gradient spans the box.
    let origin = sk::Point::new(0.0, text.offset_y as f32);
    match text.units.as_slice() {
        [] => {}
        [only] if only.start == 0 && only.end >= text.text.len() => {
            draw_unit(canvas, text, block, origin, only, None, opacity);
        }
        units => {
            for unit in units {
                draw_unit(
                    canvas,
                    text,
                    block,
                    origin,
                    unit,
                    Some(unit.start..unit.end),
                    opacity,
                );
            }
        }
    }
}

/// Draws the glyphs of one unit: those whose letters start in `range`,
/// or all of them.
fn draw_unit(
    canvas: &sk::Canvas,
    text: &TextContent,
    block: &TextBlock,
    origin: sk::Point,
    unit: &TextUnit,
    range: Option<Range<usize>>,
    opacity: f32,
) {
    let opacity = opacity * unit.opacity as f32;
    if opacity.is_nan() || opacity <= 0.0 || !convert::is_finite(&unit.transform) {
        return;
    }
    let pieces = pieces(block, range);
    if pieces.is_empty() {
        return;
    }
    canvas.save();
    if !unit.transform.is_identity() {
        canvas.concat(&convert::matrix(&unit.transform));
    }
    let blur = effects::sigma(unit.blur)
        .and_then(|sigma| sk::image_filters::blur((sigma, sigma), None, None, None));
    // Fill and stroke overlap: fading them separately would show the fill
    // through the stroke, so a faded unit with both gets a layer.
    let overlapping = text.fill.is_some() && text.stroke.is_some() && opacity < 1.0;
    if blur.is_some() || overlapping {
        let mut layer = sk::Paint::default();
        layer.set_alpha_f(opacity.min(1.0));
        if let Some(blur) = blur {
            layer.set_image_filter(blur);
        }
        canvas.save_layer(&sk::canvas::SaveLayerRec::default().paint(&layer));
        paint_pieces(canvas, text, &pieces, origin, 1.0);
        canvas.restore();
    } else {
        paint_pieces(canvas, text, &pieces, origin, opacity);
    }
    canvas.restore();
}

/// Glyphs of one font to draw together.
struct Piece<'a> {
    font: &'a sk::Font,
    glyphs: Vec<sk::GlyphId>,
    positions: Vec<sk::Point>,
}

/// The glyphs whose letters start in `range`, or all glyphs.
fn pieces(block: &TextBlock, range: Option<Range<usize>>) -> Vec<Piece<'_>> {
    block
        .runs
        .iter()
        .filter_map(|run| {
            let (glyphs, positions) = match &range {
                None => (run.glyphs.clone(), run.positions.clone()),
                Some(range) => run
                    .starts
                    .iter()
                    .zip(run.glyphs.iter().zip(&run.positions))
                    .filter(|(start, _)| range.contains(start))
                    .map(|(_, (glyph, position))| (*glyph, *position))
                    .unzip(),
            };
            (!glyphs.is_empty()).then_some(Piece {
                font: &run.font,
                glyphs,
                positions,
            })
        })
        .collect()
}

/// Fills, then strokes, the glyphs.
fn paint_pieces(
    canvas: &sk::Canvas,
    text: &TextContent,
    pieces: &[Piece<'_>],
    origin: sk::Point,
    opacity: f32,
) {
    if let Some(fill) = text
        .fill
        .as_ref()
        .and_then(|fill| paint::fill(fill, opacity))
    {
        for piece in pieces {
            canvas.draw_glyphs_at(
                &piece.glyphs,
                piece.positions.as_slice(),
                origin,
                piece.font,
                &fill,
            );
        }
    }
    if let Some(outline) = &text.stroke {
        stroke_pieces(canvas, outline, pieces, origin, opacity);
    }
}

fn stroke_pieces(
    canvas: &sk::Canvas,
    outline: &SceneStroke,
    pieces: &[Piece<'_>],
    origin: sk::Point,
    opacity: f32,
) {
    if outline.position == StrokePosition::Center {
        // Centred strokes are drawn straight from the glyphs.
        if let Some(paint) = stroke::paint(outline, true, opacity) {
            for piece in pieces {
                canvas.draw_glyphs_at(
                    &piece.glyphs,
                    piece.positions.as_slice(),
                    origin,
                    piece.font,
                    &paint,
                );
            }
        }
        return;
    }
    // Inside and outside strokes need the letters' outlines to clip to.
    let mut builder = sk::PathBuilder::new();
    for piece in pieces {
        for (glyph, position) in piece.glyphs.iter().zip(&piece.positions) {
            if let Some(path) = piece.font.get_path(*glyph) {
                let at = *position + origin;
                builder.add_path_with_offset(&path, at, None);
            }
        }
    }
    let shape = Outline::new(builder.detach(), true);
    stroke::draw(canvas, &shape, outline, opacity);
}

#[cfg(test)]
mod tests {
    use moshionly_core::model::{
        BlendMode, Font, GradientStop, StrokeCap, StrokeJoin, TextAlign, TextSplit,
    };
    use moshionly_core::scene::{Content, Scene, SceneNode, ScenePaint, TextStyle};
    use moshionly_core::text::{TextMeasurer, TextRequest, split};
    use moshionly_core::{Affine, Color, NodeId, Point};

    use super::*;
    use crate::{FontLibrary, Frame, RenderOptions, RenderWarning, Renderer};

    const TEXT: &str = "Hello, world";

    fn style() -> TextStyle {
        TextStyle {
            font: Font::default(),
            font_size: 24.0,
            line_height: 30.0,
            letter_spacing: 0.0,
            align: TextAlign::Left,
        }
    }

    fn whole() -> Vec<TextUnit> {
        vec![TextUnit {
            start: 0,
            end: TEXT.len(),
            transform: Affine::IDENTITY,
            opacity: 1.0,
            blur: 0.0,
        }]
    }

    /// One unit per letter, or per word, at rest.
    fn units(library: &FontLibrary, by: TextSplit) -> Vec<TextUnit> {
        let style = style();
        let layout = library.measure(&TextRequest {
            text: TEXT,
            style: &style,
            wrap_width: None,
        });
        split(TEXT, &layout, by)
            .iter()
            .map(|piece| TextUnit {
                start: piece.start,
                end: piece.end,
                ..whole()[0]
            })
            .collect()
    }

    fn content(units: Vec<TextUnit>) -> TextContent {
        TextContent {
            text: TEXT.into(),
            style: style(),
            wrap_width: None,
            offset_y: 0.0,
            fill: Some(ScenePaint::solid(Color::BLACK)),
            stroke: None,
            units,
        }
    }

    fn render(content: TextContent) -> Frame {
        let node = SceneNode {
            id: NodeId::from("text"),
            transform: Affine::translate(10.0, 10.0),
            width: 180.0,
            height: 40.0,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            blur: 0.0,
            shadow: None,
            glow: None,
            noise: None,
            directional_blur: None,
            clip: None,
            mask: None,
            content: Content::Text(content),
        };
        let scene = Scene {
            artboard: NodeId::from("ab"),
            time_ms: 0.0,
            width: 200.0,
            height: 60.0,
            background: None,
            nodes: vec![node],
        };
        Renderer::new(FontLibrary::bundled(), crate::AssetLoader::default())
            .render(&scene, &RenderOptions::new())
            .unwrap()
    }

    /// The box around every pixel with some alpha, as left, top, right,
    /// bottom (inclusive), and the summed alpha.
    fn ink(frame: &Frame, columns: std::ops::Range<u32>) -> Option<(u32, u32, u32, u32, u64)> {
        let mut found: Option<(u32, u32, u32, u32, u64)> = None;
        for y in 0..frame.height() {
            for x in columns.clone() {
                let alpha = frame.premultiplied_pixel(x, y).unwrap()[3];
                if alpha > 0 {
                    let entry = found.get_or_insert((x, y, x, y, 0));
                    *entry = (
                        entry.0.min(x),
                        entry.1.min(y),
                        entry.2.max(x),
                        entry.3.max(y),
                        entry.4 + u64::from(alpha),
                    );
                }
            }
        }
        found
    }

    #[test]
    fn whole_and_unit_by_unit_land_on_the_same_pixels() {
        let library = FontLibrary::bundled();
        let letters = units(&library, TextSplit::Letters);
        assert_eq!(
            letters.len(),
            11,
            "letters and the comma, without the space"
        );
        assert_eq!(render(content(whole())), render(content(letters)));

        // With a gradient and an outside stroke, unit by word, the glyphs
        // land on the same pixels too. The stroke is a path clipped to the
        // outside of the letters, and Skia's anti-aliasing adds up the
        // coverage of every outline along a pixel row, so a stroke drawn
        // word by word can round its edge pixels a level or few apart from
        // one drawn whole. Nothing anyone can see.
        let words = units(&library, TextSplit::Words);
        assert_eq!(words.len(), 2);
        let dressed = |units| TextContent {
            fill: Some(ScenePaint::LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(180.0, 0.0),
                stops: vec![
                    GradientStop::new(0.0, Color::rgb(255, 0, 0)),
                    GradientStop::new(1.0, Color::rgb(0, 0, 255)),
                ],
            }),
            stroke: Some(SceneStroke {
                paint: ScenePaint::solid(Color::rgb(0, 160, 0)),
                width: 1.5,
                position: StrokePosition::Outside,
                cap: StrokeCap::Butt,
                join: StrokeJoin::Round,
            }),
            ..content(units)
        };
        let (whole, by_word) = (render(dressed(whole())), render(dressed(words)));
        let apart = whole
            .premultiplied()
            .iter()
            .zip(by_word.premultiplied())
            .map(|(a, b)| a.abs_diff(*b))
            .max();
        assert!(apart <= Some(4), "{apart:?}");
    }

    #[test]
    fn the_text_sits_in_its_box() {
        let frame = render(content(whole()));
        let (left, top, right, bottom, _) = ink(&frame, 0..200).unwrap();
        // The box is at (10, 10); a 24 pixel font on 30 pixel lines.
        assert!((10..13).contains(&left), "{left}");
        assert!((13..20).contains(&top), "{top}");
        assert!(right > 120 && right < 190, "{right}");
        assert!(bottom < 40, "{bottom}");

        let lowered = render(TextContent {
            offset_y: 12.0,
            ..content(whole())
        });
        let (_, low_top, _, _, _) = ink(&lowered, 0..200).unwrap();
        assert_eq!(low_top, top + 12);
    }

    #[test]
    fn units_move_fade_and_blur_on_their_own() {
        let library = FontLibrary::bundled();
        let mut units = units(&library, TextSplit::Letters);
        let at_rest = render(content(units.clone()));
        let h = 10..26;
        let (_, top, _, _, ink_at_rest) = ink(&at_rest, h.clone()).unwrap();

        units[0].transform = Affine::translate(0.0, 15.0);
        let moved = render(content(units.clone()));
        let (_, moved_top, _, _, moved_ink) = ink(&moved, h.clone()).unwrap();
        assert_eq!(moved_top, top + 15);
        assert_eq!(moved_ink, ink_at_rest, "moved whole, not cut");
        // The other letters stay.
        assert_eq!(ink(&moved, 40..200), ink(&at_rest, 40..200));

        units[0].transform = Affine::IDENTITY;
        units[0].opacity = 0.5;
        let faded = render(content(units.clone()));
        let (_, _, _, _, faded_ink) = ink(&faded, h.clone()).unwrap();
        assert!(faded_ink.abs_diff(ink_at_rest / 2) < ink_at_rest / 50);

        units[0].opacity = 1.0;
        units[0].blur = 6.0;
        let blurred = render(content(units));
        let (left, blurred_top, _, _, _) = ink(&blurred, 0..26).unwrap();
        assert!(blurred_top < top && left < 10, "the blur spreads");
    }

    #[test]
    fn text_outside_every_unit_is_not_drawn() {
        let hello = TextUnit {
            end: 5,
            ..whole()[0]
        };
        let frame = render(content(vec![hello]));
        let full = render(content(whole()));
        let (_, _, right, _, _) = ink(&frame, 0..200).unwrap();
        let (_, _, full_right, _, _) = ink(&full, 0..200).unwrap();
        assert!(right < full_right / 2 + 20, "{right} of {full_right}");
        assert!(ink(&render(content(Vec::new())), 0..200).is_none());
    }

    #[test]
    fn gradients_span_the_box() {
        let frame = render(TextContent {
            fill: Some(ScenePaint::LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(180.0, 0.0),
                stops: vec![
                    GradientStop::new(0.0, Color::rgb(255, 0, 0)),
                    GradientStop::new(1.0, Color::rgb(0, 0, 255)),
                ],
            }),
            ..content(whole())
        });
        let colour = |x: u32| {
            (0..frame.height())
                .filter_map(|y| frame.unpremultiplied_pixel(x, y))
                .find(|pixel| pixel[3] == 255)
                .unwrap()
        };
        let (left, _, right, _, _) = ink(&frame, 0..200).unwrap();
        let (first, last) = (colour(left + 1), colour(right - 1));
        assert!(
            first[0] > 200 && first[2] < 60,
            "red on the left: {first:?}"
        );
        assert!(last[2] > last[0], "bluer on the right: {last:?}");
    }

    #[test]
    fn strokes_on_text_sit_where_they_are_asked() {
        let stroked = |position| {
            let frame = render(TextContent {
                fill: None,
                stroke: Some(SceneStroke {
                    paint: ScenePaint::solid(Color::BLACK),
                    width: 2.0,
                    position,
                    cap: StrokeCap::Butt,
                    join: StrokeJoin::Miter,
                }),
                ..content(whole())
            });
            ink(&frame, 0..200).unwrap()
        };
        let fill = ink(&render(content(whole())), 0..200).unwrap();
        let (inside, outside) = (
            stroked(StrokePosition::Inside),
            stroked(StrokePosition::Outside),
        );
        let centre = stroked(StrokePosition::Center);
        assert!(
            inside.0 >= fill.0 && inside.2 <= fill.2,
            "{inside:?} {fill:?}"
        );
        assert!(
            outside.0 < fill.0 && outside.2 > fill.2,
            "{outside:?} {fill:?}"
        );
        assert!(centre.0 <= fill.0 && centre.0 >= outside.0, "{centre:?}");
    }

    #[test]
    fn missing_fonts_are_reported_and_drawn_in_inter() {
        let mut missing = content(whole());
        missing.style.font.family = "No Such Font".into();
        let frame = render(missing);
        assert_eq!(
            frame.warnings(),
            [RenderWarning::MissingFont {
                node: NodeId::from("text"),
                family: "No Such Font".into(),
            }]
        );
        let inter = render(content(whole()));
        assert!(inter.warnings().is_empty());
        assert_eq!(frame.premultiplied(), inter.premultiplied());
    }
}
