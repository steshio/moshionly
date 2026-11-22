//! Text animations: a text layer coming in or going out piece by piece.
//!
//! A text animation cuts the text into pieces (letters, words or lines) and
//! plays the same effect on each, one starting a little after the other.
//! What comes out is the list of [`TextUnit`]s the renderer draws: runs of
//! text that share a transform, an opacity and a blur.

use super::preset::Pose;
use super::tween::{Span, replacing};
use crate::geometry::{Affine, Point};
use crate::model::{TextAnimation, TextEffect, TextOrder, TextSplit};
use crate::scene::TextUnit;
use crate::text::{TextLayout, TextPiece, split};

/// One `text_in` or `text_out`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextStep {
    pub span: Span,
    /// `text_in` rather than `text_out`.
    pub entering: bool,
    pub animation: TextAnimation,
}

/// How fine a split is: letters are the finest.
fn fineness(split: TextSplit) -> u8 {
    match split {
        TextSplit::Letters => 0,
        TextSplit::Words => 1,
        TextSplit::Lines => 2,
    }
}

/// When the piece in position `position` of `count` plays.
///
/// Pieces start `stagger` apart and all last equally long, the last one
/// ending with the animation. When the staggers alone would not fit, they
/// shrink so that the pieces play one after another.
fn slot(span: &Span, stagger: f64, position: usize, count: usize) -> Span {
    let length = (span.end - span.start).max(0.0);
    let stagger = stagger.min(length / count.max(1) as f64);
    let each = length - stagger * count.saturating_sub(1) as f64;
    let start = span.start + stagger * position as f64;
    Span::new(start, start + each, span.easing)
}

/// What the effects do to one piece of text, on top of where layout put it.
#[derive(Default)]
struct PiecePose {
    opacity: Pose<f64>,
    /// Vertical travel in pixels, positive downward.
    travel: Pose<f64>,
    scale: Pose<f64>,
    /// The point each step of `scale` scales about.
    pivots: Vec<Point>,
    blur: Pose<f64>,
}

impl PiecePose {
    fn lay(&mut self, step: &TextStep, slot: Span, pivot: Point, line_height: f64) {
        let animation = &step.animation;
        let entering = step.entering;
        // The value the effect holds while the piece is away, and neutral.
        let between = |away: f64, neutral: f64| {
            Some(if entering {
                (away, neutral)
            } else {
                (neutral, away)
            })
        };
        let distance = line_height * animation.distance / 100.0;
        let (travel, scale, blur) = match animation.effect {
            TextEffect::Fade => (None, None, None),
            // Rising: in from below, out toward the top.
            TextEffect::SlideUp => (
                between(if entering { distance } else { -distance }, 0.0),
                None,
                None,
            ),
            // Dropping: in from above, out toward the bottom.
            TextEffect::SlideDown => (
                between(if entering { -distance } else { distance }, 0.0),
                None,
                None,
            ),
            TextEffect::Scale => (None, between(animation.scale, 1.0), None),
            TextEffect::Blur => (None, None, between(animation.blur, 0.0)),
        };
        // Every effect fades, so pieces are unseen while they are away.
        self.opacity.lay(slot, between(0.0, 1.0), entering, 1.0);
        self.travel.lay(slot, travel, entering, 0.0);
        if self.scale.lay(slot, scale, entering, 1.0) {
            self.pivots.push(pivot);
        }
        self.blur.lay(slot, blur, entering, 0.0);
    }

    /// The unit for this piece at `time`, or `None` when it cannot be seen.
    fn unit(&self, piece: &TextPiece, time: f64) -> Option<TextUnit> {
        let opacity = replacing(&1.0, &self.opacity.steps, time).clamp(0.0, 1.0);
        let scale = replacing(&1.0, &self.scale.steps, time).max(0.0);
        if opacity <= 0.0 || scale <= 0.0 {
            return None;
        }
        let travel = replacing(&0.0, &self.travel.steps, time);
        // The step in charge of the scale also says what to scale about.
        let in_charge = self
            .scale
            .steps
            .iter()
            .rposition(|step| step.span.start <= time)
            .unwrap_or(0);
        let transform = match self.pivots.get(in_charge) {
            Some(pivot) if scale != 1.0 => {
                Affine::translate(pivot.x, pivot.y + travel)
                    * Affine::scale(scale, scale)
                    * Affine::translate(-pivot.x, -pivot.y)
            }
            _ if travel != 0.0 => Affine::translate(0.0, travel),
            _ => Affine::IDENTITY,
        };
        Some(TextUnit {
            start: piece.start,
            end: piece.end,
            transform,
            opacity,
            blur: replacing(&0.0, &self.blur.steps, time).max(0.0),
        })
    }
}

/// The units to draw for a text at `time`.
///
/// `steps` are the layer's text animations in the order they start.
/// `offset_y` is how far the text block sits below the top of the layer's
/// box, which is where piece boxes are measured from.
///
/// Without animations the whole text is one unit. With them, the text is
/// cut as finely as the finest animation asks, every piece gets the effect
/// of the piece it belongs to in each animation, pieces that cannot be seen
/// are dropped, and neighbours that ended up looking the same are joined.
pub(crate) fn units(
    text: &str,
    layout: &TextLayout,
    steps: &[TextStep],
    time: f64,
    line_height: f64,
    offset_y: f64,
) -> Vec<TextUnit> {
    let whole = |start: usize, end: usize| TextUnit {
        start,
        end,
        transform: Affine::IDENTITY,
        opacity: 1.0,
        blur: 0.0,
    };
    if text.is_empty() {
        return Vec::new();
    }
    let Some(finest) = steps
        .iter()
        .map(|step| step.animation.split)
        .min_by_key(|split| fineness(*split))
    else {
        return vec![whole(0, text.len())];
    };
    let atoms = split(text, layout, finest);
    let mut poses: Vec<PiecePose> = atoms.iter().map(|_| PiecePose::default()).collect();

    // Lay every animation onto every atom. Per atom the slots can start in
    // another order than the animations do (a backward stagger, say), so
    // gather first and lay in slot order.
    let mut slots: Vec<Vec<(Span, &TextStep, Point)>> = vec![Vec::new(); atoms.len()];
    for step in steps {
        let pieces = split(text, layout, step.animation.split);
        let mut piece = 0;
        for (atom, slots) in atoms.iter().zip(&mut slots) {
            // Atoms and pieces are both in reading order, and every atom
            // lies inside one piece of any coarser or equal split.
            while piece + 1 < pieces.len() && pieces[piece].end <= atom.start {
                piece += 1;
            }
            let Some(owner) = pieces.get(piece) else {
                continue;
            };
            let position = match step.animation.order {
                TextOrder::Forward => piece,
                TextOrder::Backward => pieces.len() - 1 - piece,
            };
            let center = owner.rect.center();
            slots.push((
                slot(&step.span, step.animation.stagger, position, pieces.len()),
                step,
                Point::new(center.x, center.y + offset_y),
            ));
        }
    }
    for (pose, mut slots) in poses.iter_mut().zip(slots) {
        slots.sort_by(|a, b| a.0.start.total_cmp(&b.0.start));
        for (slot, step, pivot) in slots {
            pose.lay(step, slot, pivot, line_height);
        }
    }

    let mut units: Vec<TextUnit> = Vec::new();
    // Whether the unit at the end of `units` came from the atom just before.
    let mut adjoining = false;
    for (atom, pose) in atoms.iter().zip(&poses) {
        let Some(unit) = pose.unit(atom, time) else {
            adjoining = false;
            continue;
        };
        match units.last_mut() {
            Some(last)
                if adjoining
                    && (last.transform, last.opacity, last.blur)
                        == (unit.transform, unit.opacity, unit.blur) =>
            {
                last.end = unit.end;
            }
            _ => units.push(unit),
        }
        adjoining = true;
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::easing::Easing;

    #[test]
    fn slots_fill_the_animation_from_start_to_end() {
        let span = Span::new(1000.0, 2000.0, Easing::Linear);
        let slots: Vec<(f64, f64)> = (0..5)
            .map(|position| slot(&span, 100.0, position, 5))
            .map(|slot| (slot.start, slot.end))
            .collect();
        assert_eq!(
            slots,
            [
                (1000.0, 1600.0),
                (1100.0, 1700.0),
                (1200.0, 1800.0),
                (1300.0, 1900.0),
                (1400.0, 2000.0)
            ]
        );
        // One piece takes the whole time.
        let only = slot(&span, 100.0, 0, 1);
        assert_eq!((only.start, only.end), (1000.0, 2000.0));
    }

    #[test]
    fn staggers_that_do_not_fit_shrink_until_pieces_play_in_turn() {
        let span = Span::new(0.0, 1000.0, Easing::Linear);
        let slots: Vec<(f64, f64)> = (0..4)
            .map(|position| slot(&span, 900.0, position, 4))
            .map(|slot| (slot.start, slot.end))
            .collect();
        assert_eq!(
            slots,
            [
                (0.0, 250.0),
                (250.0, 500.0),
                (500.0, 750.0),
                (750.0, 1000.0)
            ]
        );
        // No stagger: everything at once.
        let together = slot(&span, 0.0, 3, 4);
        assert_eq!((together.start, together.end), (0.0, 1000.0));
        // An animation without length switches every piece at once.
        let instant = slot(&Span::new(500.0, 500.0, Easing::Linear), 40.0, 2, 4);
        assert_eq!((instant.start, instant.end), (500.0, 500.0));
    }
}
