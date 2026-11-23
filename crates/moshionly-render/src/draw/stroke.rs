//! Strokes: an outline drawn along the edge of a shape, centred on it,
//! inside it or outside it.
//!
//! Skia only strokes centred on a path. A stroke inside the edge is drawn
//! twice as wide and clipped to the inside of the shape, so only its inner
//! half is left; a stroke outside is clipped to the outside instead. A path
//! that is not closed has no inside, so its stroke is always centred.

use moshionly_core::model::{StrokeCap, StrokeJoin, StrokePosition};
use moshionly_core::scene::SceneStroke;
use skia_safe as sk;

use super::geometry::Outline;
use super::paint;

/// How far a mitred corner may reach, as a multiple of half the stroke
/// width, before it is cut off square. 4 keeps corners sharp down to about
/// 29 degrees and stops thin spikes on sharper ones.
pub(crate) const MITER_LIMIT: f32 = 4.0;

/// A Skia paint for `stroke` at the width Skia should draw it with, or
/// `None` when there is nothing to see.
///
/// `opacity` (0 to 1) fades the stroke. The width is doubled for strokes
/// that will be clipped to one side of their edge.
pub(crate) fn paint(stroke: &SceneStroke, closed: bool, opacity: f32) -> Option<sk::Paint> {
    if !(stroke.width.is_finite() && stroke.width > 0.0) {
        return None;
    }
    let mut paint = paint::fill(&stroke.paint, opacity)?;
    let width = match side(stroke, closed) {
        Some(_) => stroke.width * 2.0,
        None => stroke.width,
    };
    paint.set_style(sk::PaintStyle::Stroke);
    paint.set_stroke_width(width as f32);
    paint.set_stroke_miter(MITER_LIMIT);
    paint.set_stroke_cap(match stroke.cap {
        StrokeCap::Butt => sk::PaintCap::Butt,
        StrokeCap::Round => sk::PaintCap::Round,
        StrokeCap::Square => sk::PaintCap::Square,
    });
    paint.set_stroke_join(match stroke.join {
        StrokeJoin::Miter => sk::PaintJoin::Miter,
        StrokeJoin::Round => sk::PaintJoin::Round,
        StrokeJoin::Bevel => sk::PaintJoin::Bevel,
    });
    Some(paint)
}

/// The clip that keeps one side of a doubled stroke, or `None` for a
/// stroke drawn centred: `Intersect` keeps the inside, `Difference` the
/// outside.
pub(crate) fn side(stroke: &SceneStroke, closed: bool) -> Option<sk::ClipOp> {
    match stroke.position {
        _ if !closed => None,
        StrokePosition::Center => None,
        StrokePosition::Inside => Some(sk::ClipOp::Intersect),
        StrokePosition::Outside => Some(sk::ClipOp::Difference),
    }
}

/// Strokes `outline` on `canvas`, faded by `opacity`.
pub(crate) fn draw(canvas: &sk::Canvas, outline: &Outline, stroke: &SceneStroke, opacity: f32) {
    let Some(paint) = paint(stroke, outline.closed, opacity) else {
        return;
    };
    match side(stroke, outline.closed) {
        None => {
            canvas.draw_path(&outline.path, &paint);
        }
        Some(op) => {
            canvas.save();
            canvas.clip_path(&outline.path, op, true);
            canvas.draw_path(&outline.path, &paint);
            canvas.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::geometry;
    use moshionly_core::Color;
    use moshionly_core::model::{Anchor, FillRule, PathData, Subpath};
    use moshionly_core::scene::ScenePaint;

    fn stroke(width: f64, position: StrokePosition) -> SceneStroke {
        SceneStroke {
            paint: ScenePaint::solid(Color::BLACK),
            width,
            position,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
        }
    }

    fn square() -> Outline {
        Outline {
            path: geometry::rect(40.0, 40.0, &[0.0; 4]),
            closed: true,
        }
    }

    /// Strokes the outline, offset by 10 so outside strokes fit, and reads
    /// back the alpha along the middle row.
    fn alpha_row(outline: &Outline, stroke: &SceneStroke) -> Vec<u8> {
        let mut surface = sk::surfaces::raster_n32_premul((60, 60)).unwrap();
        let canvas = surface.canvas();
        canvas.translate((10.0, 10.0));
        draw(canvas, outline, stroke, 1.0);
        let image = surface.image_snapshot();
        let pixmap = image.peek_pixels().unwrap();
        (0..60).map(|x| pixmap.get_color((x, 30)).a()).collect()
    }

    #[test]
    fn strokes_sit_on_the_side_of_the_edge_they_ask_for() {
        // The square's left edge is at x = 10 on the surface; the stroke is
        // 4 wide.
        let centre = alpha_row(&square(), &stroke(4.0, StrokePosition::Center));
        assert_eq!(&centre[7..14], [0, 255, 255, 255, 255, 0, 0]);

        let inside = alpha_row(&square(), &stroke(4.0, StrokePosition::Inside));
        assert_eq!(&inside[7..16], [0, 0, 0, 255, 255, 255, 255, 0, 0]);
        assert_eq!(&inside[44..51], [0, 0, 255, 255, 255, 255, 0]);

        let outside = alpha_row(&square(), &stroke(4.0, StrokePosition::Outside));
        assert_eq!(&outside[5..12], [0, 255, 255, 255, 255, 0, 0]);
        assert_eq!(outside[30], 0, "the middle stays empty");
    }

    /// A horizontal line at y = 20 from `from` to `to`, not closed.
    fn open_line(from: f64, to: f64) -> Outline {
        geometry::pen(&PathData {
            subpaths: vec![Subpath {
                anchors: vec![Anchor::corner(from, 20.0), Anchor::corner(to, 20.0)],
                closed: false,
            }],
            fill_rule: FillRule::NonZero,
        })
    }

    #[test]
    fn open_paths_are_stroked_on_their_line() {
        let line = open_line(0.0, 40.0);
        assert!(!line.closed);
        // Asked to be inside, a line that has no inside is centred.
        let mut surface = sk::surfaces::raster_n32_premul((60, 60)).unwrap();
        draw(
            surface.canvas(),
            &line,
            &stroke(4.0, StrokePosition::Inside),
            1.0,
        );
        let image = surface.image_snapshot();
        let pixmap = image.peek_pixels().unwrap();
        let column: Vec<u8> = (16..24).map(|y| pixmap.get_color((20, y)).a()).collect();
        assert_eq!(column, [0, 0, 255, 255, 255, 255, 0, 0]);
    }
}
