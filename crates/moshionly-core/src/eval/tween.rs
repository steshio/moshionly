//! The two rules for combining animations on one property, and how values
//! of each kind blend.
//!
//! Everything here is a pure function of a time. Nothing remembers the
//! previous frame.

use crate::color::Color;
use crate::defaults;
use crate::easing::{Easing, ease};
use crate::geometry::Point;
use crate::model::{
    DirectionalBlur, DirectionalBlurChange, Glow, GlowChange, Gradient, GradientStop,
    LinearGradient, Noise, NoiseChange, Paint, RadialGradient, Shadow, ShadowChange, Timing,
};

/// When a step runs and how it is paced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Span {
    pub start: f64,
    pub end: f64,
    pub easing: Easing,
}

impl Span {
    pub(crate) fn new(start: f64, end: f64, easing: Easing) -> Self {
        Self { start, end, easing }
    }

    /// Eased progress at `time`, for a time at or after the start: 0 at the
    /// start, 1 from the end on. A span without length is at 1 at once. The
    /// result is not limited to 0..1 in between: an overshooting easing
    /// carries the value past its target.
    pub(crate) fn progress(&self, time: f64) -> f64 {
        if self.end <= self.start {
            return 1.0;
        }
        ease(&self.easing, (time - self.start) / (self.end - self.start))
    }
}

impl From<&Timing> for Span {
    fn from(timing: &Timing) -> Self {
        Self::new(timing.start, timing.end, timing.easing)
    }
}

/// A value that can be mixed with another of its kind.
pub(crate) trait Blend: Clone {
    /// The value `t` of the way from `self` to `other`. `t` may leave 0..1.
    fn blend(&self, other: &Self, t: f64) -> Self;
}

/// One animation's say over a property of type `V`.
pub(crate) trait Step<V> {
    /// When it runs.
    fn span(&self) -> &Span;
    /// The value it starts from, given the value the property has at that
    /// moment.
    fn from(&self, incoming: &V) -> V;
    /// The value it ends on, given the resting value and where it started.
    fn to(&self, rest: &V, from: &V) -> V;
}

/// A step with an optional start and end value of the property's own type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tween<V> {
    pub span: Span,
    pub from: Option<V>,
    pub to: Option<V>,
}

impl<V> Tween<V> {
    pub(crate) fn new(span: Span, from: Option<V>, to: Option<V>) -> Self {
        Self { span, from, to }
    }
}

impl<V: Clone> Step<V> for Tween<V> {
    fn span(&self) -> &Span {
        &self.span
    }

    fn from(&self, incoming: &V) -> V {
        self.from.clone().unwrap_or_else(|| incoming.clone())
    }

    fn to(&self, rest: &V, _from: &V) -> V {
        self.to.clone().unwrap_or_else(|| rest.clone())
    }
}

/// The replacing rule.
///
/// `steps` are sorted by start. The value is the resting value until the
/// first step, and from then on the most recently started step decides. A
/// step without a start value picks up whatever the value is at the moment
/// it starts, cutting off a step that is still running. A step without an
/// end value returns to rest. If the first step has a start value, that
/// value also holds before it starts.
pub(crate) fn replacing<V: Blend, S: Step<V>>(rest: &V, steps: &[S], time: f64) -> V {
    let Some(first) = steps.first() else {
        return rest.clone();
    };
    if time < first.span().start {
        return first.from(rest);
    }
    // The step in charge, with its resolved start and end values.
    let mut current: Option<(V, V, &Span)> = None;
    for step in steps {
        let span = step.span();
        if span.start > time {
            break;
        }
        let incoming = match &current {
            None => rest.clone(),
            Some((from, to, running)) => from.blend(to, running.progress(span.start)),
        };
        let from = step.from(&incoming);
        let to = step.to(rest, &from);
        current = Some((from, to, span));
    }
    match current {
        Some((from, to, span)) => from.blend(&to, span.progress(time)),
        None => rest.clone(),
    }
}

/// The additive rule.
///
/// `steps` are sorted by start. Every step contributes a delta, and the
/// deltas add up. A missing start or end value is no delta. Before it
/// starts a step contributes nothing, except the first one, whose start
/// value holds from time 0: a layer that will move in from the side waits
/// there.
pub(crate) fn additive<V: Blend, S: Step<V>>(
    zero: &V,
    steps: &[S],
    time: f64,
    add: impl Fn(&V, &V) -> V,
) -> V {
    let mut sum = zero.clone();
    for (position, step) in steps.iter().enumerate() {
        let span = step.span();
        let from = step.from(zero);
        let delta = if time >= span.start {
            from.blend(&step.to(zero, &from), span.progress(time))
        } else if position == 0 {
            from
        } else {
            continue;
        };
        sum = add(&sum, &delta);
    }
    sum
}

/// Linear interpolation that lands exactly on `to` at the end, so a value
/// that has finished animating is the value that was asked for.
fn mix(from: f64, to: f64, t: f64) -> f64 {
    if t == 1.0 { to } else { from + (to - from) * t }
}

impl Blend for f64 {
    fn blend(&self, other: &Self, t: f64) -> Self {
        mix(*self, *other, t)
    }
}

impl Blend for Point {
    fn blend(&self, other: &Self, t: f64) -> Self {
        Point::new(mix(self.x, other.x, t), mix(self.y, other.y, t))
    }
}

impl Blend for [f64; 4] {
    fn blend(&self, other: &Self, t: f64) -> Self {
        std::array::from_fn(|i| mix(self[i], other[i], t))
    }
}

impl Blend for [f64; 2] {
    fn blend(&self, other: &Self, t: f64) -> Self {
        [mix(self[0], other[0], t), mix(self[1], other[1], t)]
    }
}

impl Blend for Color {
    fn blend(&self, other: &Self, t: f64) -> Self {
        self.lerp(*other, t)
    }
}

/// A fill: a paint, or nothing. Nothing blends as the other paint at zero
/// alpha, so a fill can fade in from, or out to, no fill at all.
impl Blend for Option<Paint> {
    fn blend(&self, other: &Self, t: f64) -> Self {
        match (self, other) {
            (None, None) => None,
            (Some(paint), None) => Some(faded(paint, 1.0 - t)),
            (None, Some(paint)) => Some(faded(paint, t)),
            (Some(from), Some(to)) => Some(blend_paints(from, to, t)),
        }
    }
}

/// The paint with every alpha multiplied by `amount` (limited to 0..1).
fn faded(paint: &Paint, amount: f64) -> Paint {
    let amount = amount.clamp(0.0, 1.0);
    let fade = |color: Color| color.with_alpha((f64::from(color.a) * amount).round() as u8);
    match paint {
        Paint::Solid(color) => Paint::Solid(fade(*color)),
        Paint::Gradient(gradient) => {
            let mut gradient = gradient.clone();
            let stops = match &mut gradient {
                Gradient::LinearGradient(linear) => &mut linear.stops,
                Gradient::RadialGradient(radial) => &mut radial.stops,
            };
            for stop in stops {
                stop.color = fade(stop.color);
            }
            Paint::Gradient(gradient)
        }
    }
}

/// The same gradient with every stop in one colour.
fn in_one_color(gradient: &Gradient, color: Color) -> Gradient {
    let mut flat = gradient.clone();
    let stops = match &mut flat {
        Gradient::LinearGradient(linear) => &mut linear.stops,
        Gradient::RadialGradient(radial) => &mut radial.stops,
    };
    for stop in stops {
        stop.color = color;
    }
    flat
}

/// Blends two paints.
///
/// - Two colours blend channel by channel.
/// - A colour and a gradient blend as if the colour were that gradient with
///   every stop in the one colour.
/// - Two gradients of the same kind blend their geometry and their stops
///   (see [`blend_stops`]).
/// - A linear and a radial gradient have no in-between: the first shows for
///   the first half of the way, the second for the rest.
pub(crate) fn blend_paints(from: &Paint, to: &Paint, t: f64) -> Paint {
    match (from, to) {
        (Paint::Solid(from), Paint::Solid(to)) => Paint::Solid(from.lerp(*to, t)),
        (Paint::Solid(color), Paint::Gradient(gradient)) => Paint::Gradient(blend_gradients(
            &in_one_color(gradient, *color),
            gradient,
            t,
        )),
        (Paint::Gradient(gradient), Paint::Solid(color)) => Paint::Gradient(blend_gradients(
            gradient,
            &in_one_color(gradient, *color),
            t,
        )),
        (Paint::Gradient(from), Paint::Gradient(to)) => {
            Paint::Gradient(blend_gradients(from, to, t))
        }
    }
}

fn blend_gradients(from: &Gradient, to: &Gradient, t: f64) -> Gradient {
    match (from, to) {
        (Gradient::LinearGradient(from), Gradient::LinearGradient(to)) => {
            Gradient::LinearGradient(LinearGradient {
                start: from.start.blend(&to.start, t),
                end: from.end.blend(&to.end, t),
                stops: blend_stops(&from.stops, &to.stops, t),
            })
        }
        (Gradient::RadialGradient(from), Gradient::RadialGradient(to)) => {
            let radius = from.radius.blend(&to.radius, t);
            Gradient::RadialGradient(RadialGradient {
                center: from.center.blend(&to.center, t),
                radius: [radius[0].max(0.0), radius[1].max(0.0)],
                rotation: mix(from.rotation, to.rotation, t),
                stops: blend_stops(&from.stops, &to.stops, t),
            })
        }
        _ if t < 0.5 => from.clone(),
        _ => to.clone(),
    }
}

/// The colour of a gradient at `offset`.
fn color_at(stops: &[GradientStop], offset: f64) -> Color {
    let Some(first) = stops.first() else {
        return Color::TRANSPARENT;
    };
    let mut before = first;
    for stop in stops {
        if stop.offset >= offset {
            let length = stop.offset - before.offset;
            if length <= 0.0 {
                return stop.color;
            }
            return before.color.lerp(
                stop.color,
                ((offset - before.offset) / length).clamp(0.0, 1.0),
            );
        }
        before = stop;
    }
    before.color
}

/// Blends the stops of two gradients.
///
/// With the same number of stops, the stops pair up in order and each pair
/// blends its offset and its colour, so stops slide. With different numbers
/// there is no pairing: both gradients are read at every offset that either
/// of them has a stop at, and the colours blend there.
pub(crate) fn blend_stops(from: &[GradientStop], to: &[GradientStop], t: f64) -> Vec<GradientStop> {
    let mut stops: Vec<GradientStop> = if from.len() == to.len() {
        from.iter()
            .zip(to)
            .map(|(from, to)| {
                GradientStop::new(
                    mix(from.offset, to.offset, t).clamp(0.0, 1.0),
                    from.color.lerp(to.color, t),
                )
            })
            .collect()
    } else {
        let mut offsets: Vec<f64> = from.iter().chain(to).map(|stop| stop.offset).collect();
        offsets.sort_by(f64::total_cmp);
        offsets.dedup();
        offsets
            .into_iter()
            .map(|offset| {
                GradientStop::new(offset, color_at(from, offset).lerp(color_at(to, offset), t))
            })
            .collect()
    };
    // An overshooting blend can carry a stop past its neighbour.
    let mut floor: f64 = 0.0;
    for stop in &mut stops {
        floor = floor.max(stop.offset);
        stop.offset = floor;
    }
    stops
}

/// The animatable part of a stroke. A layer without a stroke has no paint
/// and a width of 0.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StrokeState {
    pub paint: Option<Paint>,
    pub width: f64,
}

impl Blend for StrokeState {
    /// The width does the appearing and disappearing, so a missing paint
    /// simply takes the other side's paint instead of fading.
    fn blend(&self, other: &Self, t: f64) -> Self {
        let paint = match (&self.paint, &other.paint) {
            (Some(from), Some(to)) => Some(blend_paints(from, to, t)),
            (paint, None) | (None, paint) => paint.clone(),
        };
        StrokeState {
            paint,
            width: mix(self.width, other.width, t).max(0.0),
        }
    }
}

/// A step on a stroke, from a `stroke` animation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StrokeStep {
    pub span: Span,
    pub from: Option<crate::model::StrokeChange>,
    pub to: Option<crate::model::StrokeChange>,
}

fn stroke_over(change: Option<&crate::model::StrokeChange>, base: &StrokeState) -> StrokeState {
    match change {
        None => base.clone(),
        Some(change) => StrokeState {
            paint: change.paint.clone().or_else(|| base.paint.clone()),
            width: change.width.unwrap_or(base.width),
        },
    }
}

impl Step<StrokeState> for StrokeStep {
    fn span(&self) -> &Span {
        &self.span
    }

    fn from(&self, incoming: &StrokeState) -> StrokeState {
        stroke_over(self.from.as_ref(), incoming)
    }

    fn to(&self, rest: &StrokeState, _from: &StrokeState) -> StrokeState {
        stroke_over(self.to.as_ref(), rest)
    }
}

/// A shadow that is not there: fully transparent, no offset, no blur.
pub(crate) const NO_SHADOW: Shadow = Shadow {
    color: Color::TRANSPARENT,
    x: 0.0,
    y: 0.0,
    blur: 0.0,
};

impl Blend for Shadow {
    fn blend(&self, other: &Self, t: f64) -> Self {
        Shadow {
            color: self.color.lerp(other.color, t),
            x: mix(self.x, other.x, t),
            y: mix(self.y, other.y, t),
            blur: mix(self.blur, other.blur, t).max(0.0),
        }
    }
}

/// A partial value, such as a [`ShadowChange`], that fills in the parts it
/// leaves alone from a whole value.
pub(crate) trait Change<V> {
    /// The whole value with this change's parts laid over `base`.
    fn over(&self, base: &V) -> V;
}

/// A step whose start and end values are partial changes of the property's
/// value: from a `shadow`, `glow`, `noise` or `directional_blur` animation.
/// A part a change leaves out is the value at that moment (for `from`) or
/// at rest (for `to`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChangeStep<C> {
    pub span: Span,
    pub from: Option<C>,
    pub to: Option<C>,
}

impl<V: Clone, C: Change<V>> Step<V> for ChangeStep<C> {
    fn span(&self) -> &Span {
        &self.span
    }

    fn from(&self, incoming: &V) -> V {
        self.from
            .as_ref()
            .map_or_else(|| incoming.clone(), |change| change.over(incoming))
    }

    fn to(&self, rest: &V, _from: &V) -> V {
        self.to
            .as_ref()
            .map_or_else(|| rest.clone(), |change| change.over(rest))
    }
}

impl Change<Shadow> for ShadowChange {
    fn over(&self, base: &Shadow) -> Shadow {
        Shadow {
            color: self.color.unwrap_or(base.color),
            x: self.x.unwrap_or(base.x),
            y: self.y.unwrap_or(base.y),
            blur: self.blur.unwrap_or(base.blur),
        }
    }
}

/// A glow that is not there: the default glow, fully transparent, so that
/// a glow fading in keeps its size.
pub(crate) fn no_glow() -> Glow {
    Glow {
        color: defaults::glow_color().with_alpha(0),
        ..Glow::default()
    }
}

impl Blend for Glow {
    fn blend(&self, other: &Self, t: f64) -> Self {
        Glow {
            color: self.color.lerp(other.color, t),
            radius: mix(self.radius, other.radius, t).max(0.0),
            spread: mix(self.spread, other.spread, t).max(0.0),
        }
    }
}

impl Change<Glow> for GlowChange {
    fn over(&self, base: &Glow) -> Glow {
        Glow {
            color: self.color.unwrap_or(base.color),
            radius: self.radius.unwrap_or(base.radius),
            spread: self.spread.unwrap_or(base.spread),
        }
    }
}

/// Noise that is not there: the default grain with no strength.
pub(crate) fn no_noise() -> Noise {
    Noise {
        amount: 0.0,
        ..Noise::default()
    }
}

/// The smallest grain noise blends to, so an overshooting easing never
/// makes a grain of size 0.
const MIN_GRAIN: f64 = 0.01;

impl Blend for Noise {
    fn blend(&self, other: &Self, t: f64) -> Self {
        Noise {
            amount: mix(self.amount, other.amount, t).clamp(0.0, 1.0),
            size: mix(self.size, other.size, t).max(MIN_GRAIN),
            // Whether the grain moves is the layer's own and does not blend.
            animated: self.animated,
        }
    }
}

impl Change<Noise> for NoiseChange {
    fn over(&self, base: &Noise) -> Noise {
        Noise {
            amount: self.amount.unwrap_or(base.amount),
            size: self.size.unwrap_or(base.size),
            animated: base.animated,
        }
    }
}

/// A directional blur that is not there: no amount, at angle 0.
pub(crate) const NO_DIRECTIONAL_BLUR: DirectionalBlur = DirectionalBlur {
    amount: 0.0,
    angle: 0.0,
};

impl Blend for DirectionalBlur {
    fn blend(&self, other: &Self, t: f64) -> Self {
        DirectionalBlur {
            amount: mix(self.amount, other.amount, t).max(0.0),
            angle: mix(self.angle, other.angle, t),
        }
    }
}

impl Change<DirectionalBlur> for DirectionalBlurChange {
    fn over(&self, base: &DirectionalBlur) -> DirectionalBlur {
        DirectionalBlur {
            amount: self.amount.unwrap_or(base.amount),
            angle: self.angle.unwrap_or(base.angle),
        }
    }
}

/// The size of a layer's box, and how far its centre has moved (in the
/// box's own axes) to keep the anchors of its resizes in place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoxState {
    pub width: f64,
    pub height: f64,
    pub shift: Point,
}

impl BoxState {
    pub(crate) fn at_rest(width: f64, height: f64) -> Self {
        Self {
            width,
            height,
            shift: Point::ZERO,
        }
    }

    /// The box at another size, with the point at `anchor` (as fractions of
    /// the box) staying where it is.
    pub(crate) fn resized(&self, width: f64, height: f64, anchor: (f64, f64)) -> Self {
        Self {
            width,
            height,
            shift: Point::new(
                self.shift.x - (anchor.0 - 0.5) * (width - self.width),
                self.shift.y - (anchor.1 - 0.5) * (height - self.height),
            ),
        }
    }
}

impl Blend for BoxState {
    fn blend(&self, other: &Self, t: f64) -> Self {
        BoxState {
            width: mix(self.width, other.width, t),
            height: mix(self.height, other.height, t),
            shift: self.shift.blend(&other.shift, t),
        }
    }
}

/// A step on the size of a box, from a `resize` animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResizeStep {
    pub span: Span,
    pub from: Option<crate::model::SizeChange>,
    pub to: Option<crate::model::SizeChange>,
    /// The fixed point as fractions of the box.
    pub anchor: (f64, f64),
}

impl Step<BoxState> for ResizeStep {
    fn span(&self) -> &Span {
        &self.span
    }

    fn from(&self, incoming: &BoxState) -> BoxState {
        let size = self.from.unwrap_or_default();
        incoming.resized(
            size.width.unwrap_or(incoming.width),
            size.height.unwrap_or(incoming.height),
            self.anchor,
        )
    }

    fn to(&self, rest: &BoxState, from: &BoxState) -> BoxState {
        let size = self.to.unwrap_or_default();
        from.resized(
            size.width.unwrap_or(rest.width),
            size.height.unwrap_or(rest.height),
            self.anchor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tween(start: f64, end: f64, from: Option<f64>, to: Option<f64>) -> Tween<f64> {
        Tween::new(Span::new(start, end, Easing::Linear), from, to)
    }

    fn sample(rest: f64, steps: &[Tween<f64>], times: &[f64]) -> Vec<f64> {
        times
            .iter()
            .map(|time| replacing(&rest, steps, *time))
            .collect()
    }

    #[test]
    fn replacing_without_steps_is_the_resting_value() {
        assert_eq!(sample(7.0, &[], &[0.0, 1000.0]), [7.0, 7.0]);
    }

    #[test]
    fn a_first_step_with_a_start_value_holds_it_from_time_zero() {
        // Fade in: invisible until its cue.
        let steps = [tween(100.0, 200.0, Some(0.0), Some(100.0))];
        assert_eq!(
            sample(100.0, &steps, &[0.0, 99.0, 100.0, 150.0, 200.0, 5000.0]),
            [0.0, 0.0, 0.0, 50.0, 100.0, 100.0]
        );
    }

    #[test]
    fn a_first_step_without_a_start_value_leaves_rest_alone_until_it_starts() {
        let steps = [tween(100.0, 200.0, None, Some(0.0))];
        assert_eq!(
            sample(80.0, &steps, &[0.0, 100.0, 150.0, 200.0, 300.0]),
            [80.0, 80.0, 40.0, 0.0, 0.0]
        );
    }

    #[test]
    fn a_missing_end_value_returns_to_rest() {
        let steps = [tween(0.0, 100.0, Some(0.0), None)];
        assert_eq!(sample(60.0, &steps, &[0.0, 50.0, 100.0]), [0.0, 30.0, 60.0]);
    }

    #[test]
    fn a_later_step_cuts_off_a_running_one_and_continues_from_there() {
        // A fade in to 100 over 0..100 is interrupted at 50 by a fade out.
        let steps = [
            tween(0.0, 100.0, Some(0.0), Some(100.0)),
            tween(50.0, 150.0, None, Some(0.0)),
        ];
        assert_eq!(
            sample(100.0, &steps, &[0.0, 25.0, 50.0, 100.0, 150.0, 200.0]),
            [0.0, 25.0, 50.0, 25.0, 0.0, 0.0]
        );
    }

    #[test]
    fn a_later_step_with_a_start_value_jumps_to_it() {
        let steps = [
            tween(0.0, 100.0, None, Some(10.0)),
            tween(200.0, 300.0, Some(50.0), Some(60.0)),
        ];
        assert_eq!(
            sample(0.0, &steps, &[100.0, 199.0, 200.0, 250.0, 300.0]),
            [10.0, 10.0, 50.0, 55.0, 60.0]
        );
    }

    #[test]
    fn a_step_after_a_finished_one_starts_from_its_end_value() {
        let steps = [
            tween(0.0, 100.0, None, Some(40.0)),
            tween(200.0, 300.0, None, Some(0.0)),
        ];
        assert_eq!(
            sample(100.0, &steps, &[150.0, 250.0, 300.0]),
            [40.0, 20.0, 0.0]
        );
    }

    #[test]
    fn steps_without_length_switch_at_once() {
        let steps = [tween(100.0, 100.0, Some(5.0), Some(9.0))];
        assert_eq!(
            sample(1.0, &steps, &[0.0, 99.9, 100.0, 101.0]),
            [5.0, 5.0, 9.0, 9.0]
        );
        // Two steps at the same moment: the later one in the list decides.
        let steps = [
            tween(100.0, 200.0, None, Some(50.0)),
            tween(100.0, 200.0, None, Some(80.0)),
        ];
        assert_eq!(sample(0.0, &steps, &[150.0, 200.0]), [40.0, 80.0]);
    }

    #[test]
    fn easing_may_carry_a_value_past_its_target() {
        let steps = [Tween::new(
            Span::new(0.0, 100.0, Easing::overshoot()),
            Some(0.0),
            Some(100.0),
        )];
        let peak = (1..100)
            .map(|time| replacing(&0.0, &steps, f64::from(time)))
            .fold(0.0, f64::max);
        assert!(peak > 100.0, "{peak}");
        assert_eq!(replacing(&0.0, &steps, 100.0), 100.0);
    }

    fn sum(steps: &[Tween<f64>], times: &[f64]) -> Vec<f64> {
        times
            .iter()
            .map(|time| additive(&0.0, steps, *time, |a, b| a + b))
            .collect()
    }

    #[test]
    fn additive_steps_add_up() {
        let steps = [
            tween(0.0, 100.0, None, Some(100.0)),
            tween(50.0, 150.0, None, Some(100.0)),
        ];
        assert_eq!(
            sum(&steps, &[0.0, 50.0, 100.0, 150.0, 999.0]),
            [0.0, 50.0, 150.0, 200.0, 200.0]
        );
        assert_eq!(sum(&[], &[0.0, 10.0]), [0.0, 0.0]);
    }

    #[test]
    fn only_the_first_additive_step_holds_its_start_value_early() {
        let steps = [
            tween(100.0, 200.0, Some(-200.0), None),
            tween(300.0, 400.0, Some(50.0), Some(0.0)),
        ];
        assert_eq!(
            sum(
                &steps,
                &[0.0, 100.0, 150.0, 200.0, 299.0, 300.0, 350.0, 400.0]
            ),
            [-200.0, -200.0, -100.0, 0.0, 0.0, 50.0, 25.0, 0.0]
        );
    }

    #[test]
    fn a_missing_additive_value_is_no_delta() {
        let out_and_back = [tween(0.0, 100.0, Some(30.0), None)];
        assert_eq!(sum(&out_and_back, &[0.0, 50.0, 100.0]), [30.0, 15.0, 0.0]);
    }

    #[test]
    fn colours_and_missing_fills_blend() {
        let red = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        let blue = Some(Paint::Solid(Color::rgb(0, 0, 255)));
        assert_eq!(
            red.blend(&blue, 0.5),
            Some(Paint::Solid(Color::rgb(128, 0, 128)))
        );
        assert_eq!(None::<Paint>.blend(&None, 0.5), None);
        // No fill is the other side at zero alpha.
        assert_eq!(
            None.blend(&red, 0.25),
            Some(Paint::Solid(Color::rgba(255, 0, 0, 64)))
        );
        assert_eq!(
            red.blend(&None, 0.25),
            Some(Paint::Solid(Color::rgba(255, 0, 0, 191)))
        );
        assert_eq!(
            red.blend(&None, 1.0),
            Some(Paint::Solid(Color::rgba(255, 0, 0, 0)))
        );
    }

    fn linear(stops: &[(f64, Color)]) -> Paint {
        Paint::Gradient(Gradient::LinearGradient(LinearGradient {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: stops
                .iter()
                .map(|(offset, color)| GradientStop::new(*offset, *color))
                .collect(),
        }))
    }

    fn stops_of(paint: &Paint) -> Vec<(f64, Color)> {
        let Paint::Gradient(gradient) = paint else {
            panic!("expected a gradient, got {paint:?}");
        };
        gradient
            .stops()
            .iter()
            .map(|stop| (stop.offset, stop.color))
            .collect()
    }

    const BLACK: Color = Color::BLACK;
    const WHITE: Color = Color::WHITE;
    const GREY: Color = Color::rgb(128, 128, 128);

    #[test]
    fn gradients_with_as_many_stops_pair_up_and_slide() {
        let from = linear(&[(0.0, BLACK), (0.25, WHITE), (1.0, BLACK)]);
        let to = linear(&[(0.0, WHITE), (0.75, WHITE), (1.0, WHITE)]);
        let half = blend_paints(&from, &to, 0.5);
        assert_eq!(stops_of(&half), [(0.0, GREY), (0.5, WHITE), (1.0, GREY)]);
        assert_eq!(blend_paints(&from, &to, 0.0), from);
        assert_eq!(blend_paints(&from, &to, 1.0), to);
    }

    #[test]
    fn gradients_with_different_stop_counts_are_read_at_every_offset() {
        let from = linear(&[(0.0, BLACK), (1.0, BLACK)]);
        let to = linear(&[(0.0, WHITE), (0.5, BLACK), (1.0, WHITE)]);
        let half = blend_paints(&from, &to, 0.5);
        assert_eq!(stops_of(&half), [(0.0, GREY), (0.5, BLACK), (1.0, GREY)]);
        let end = blend_paints(&from, &to, 1.0);
        assert_eq!(stops_of(&end), stops_of(&to));
    }

    #[test]
    fn a_colour_blends_with_a_gradient_as_that_gradient_in_one_colour() {
        let gradient = linear(&[(0.0, BLACK), (1.0, WHITE)]);
        let half = blend_paints(&Paint::Solid(WHITE), &gradient, 0.5);
        assert_eq!(stops_of(&half), [(0.0, GREY), (1.0, WHITE)]);
        let back = blend_paints(&gradient, &Paint::Solid(BLACK), 0.5);
        assert_eq!(stops_of(&back), [(0.0, BLACK), (1.0, GREY)]);
        // The geometry is the gradient's own the whole way.
        let Paint::Gradient(Gradient::LinearGradient(geometry)) = half else {
            panic!()
        };
        assert_eq!((geometry.start, geometry.end), ([0.0, 0.0], [1.0, 0.0]));
    }

    #[test]
    fn gradient_geometry_blends_and_kinds_switch_halfway() {
        let radial = |center: [f64; 2], radius: f64, rotation: f64| {
            Paint::Gradient(Gradient::RadialGradient(RadialGradient {
                center,
                radius: [radius, radius],
                rotation,
                stops: vec![GradientStop::new(0.0, BLACK), GradientStop::new(1.0, WHITE)],
            }))
        };
        let half = blend_paints(
            &radial([0.0, 0.0], 0.25, 0.0),
            &radial([1.0, 0.5], 0.75, 90.0),
            0.5,
        );
        assert_eq!(half, radial([0.5, 0.25], 0.5, 45.0));

        let line = linear(&[(0.0, BLACK), (1.0, WHITE)]);
        let round = radial([0.5, 0.5], 0.5, 0.0);
        assert_eq!(blend_paints(&line, &round, 0.49), line);
        assert_eq!(blend_paints(&line, &round, 0.5), round);
    }

    #[test]
    fn an_overshooting_blend_keeps_stops_in_order() {
        let from = linear(&[(0.0, BLACK), (0.5, WHITE), (1.0, BLACK)]);
        let to = linear(&[(0.0, BLACK), (0.9, WHITE), (1.0, BLACK)]);
        let past = blend_paints(&from, &to, 1.5);
        let offsets: Vec<f64> = stops_of(&past).iter().map(|stop| stop.0).collect();
        assert_eq!(offsets, [0.0, 1.0, 1.0]);
    }

    #[test]
    fn strokes_grow_in_with_their_colour_in_place() {
        let none = StrokeState {
            paint: None,
            width: 0.0,
        };
        let green = StrokeState {
            paint: Some(Paint::Solid(Color::rgb(0, 255, 0))),
            width: 6.0,
        };
        let half = none.blend(&green, 0.5);
        assert_eq!(half.width, 3.0);
        assert_eq!(
            half.paint, green.paint,
            "the colour does not fade, the width grows"
        );
        assert_eq!(green.blend(&none, 2.0).width, 0.0, "never negative");

        let step = StrokeStep {
            span: Span::new(0.0, 100.0, Easing::Linear),
            from: Some(crate::model::StrokeChange {
                paint: None,
                width: Some(0.0),
            }),
            to: None,
        };
        let half = replacing(&green, &[step], 50.0);
        assert_eq!((half.width, half.paint), (3.0, green.paint));
    }

    #[test]
    fn shadows_fade_in_from_nothing() {
        let step = ChangeStep {
            span: Span::new(0.0, 100.0, Easing::Linear),
            from: None,
            to: Some(ShadowChange {
                color: Some(Color::rgba(0, 0, 0, 128)),
                x: None,
                y: Some(12.0),
                blur: Some(30.0),
            }),
        };
        let half = replacing(&NO_SHADOW, &[step], 50.0);
        assert_eq!(
            half,
            Shadow {
                color: Color::rgba(0, 0, 0, 64),
                x: 0.0,
                y: 6.0,
                blur: 15.0
            }
        );
    }

    #[test]
    fn glows_fade_in_keeping_their_size() {
        let step = ChangeStep {
            span: Span::new(0.0, 100.0, Easing::Linear),
            from: None,
            to: Some(GlowChange {
                color: Some(Color::rgba(255, 0, 0, 200)),
                radius: None,
                spread: Some(10.0),
            }),
        };
        let half = replacing(&no_glow(), &[step], 50.0);
        assert_eq!(half.color, Color::rgba(255, 0, 0, 100), "no muddy mix");
        assert_eq!((half.radius, half.spread), (20.0, 5.0));
        // A `from` alone starts there and returns to the resting glow.
        let rest = Glow {
            color: Color::WHITE,
            radius: 8.0,
            spread: 0.0,
        };
        let pulse = ChangeStep {
            span: Span::new(100.0, 200.0, Easing::Linear),
            from: Some(GlowChange {
                radius: Some(40.0),
                ..GlowChange::default()
            }),
            to: None,
        };
        assert_eq!(replacing(&rest, &[pulse], 0.0).radius, 40.0, "held before");
        assert_eq!(replacing(&rest, &[pulse], 150.0).radius, 24.0);
        assert_eq!(replacing(&rest, &[pulse], 250.0), rest);
    }

    #[test]
    fn noise_blends_its_strength_and_keeps_whether_it_moves() {
        let rest = Noise {
            amount: 0.4,
            size: 2.0,
            animated: false,
        };
        let step = ChangeStep {
            span: Span::new(0.0, 100.0, Easing::Linear),
            from: Some(NoiseChange {
                amount: Some(0.0),
                size: Some(6.0),
            }),
            to: None,
        };
        let half = replacing(&rest, &[step], 50.0);
        assert_eq!(
            half,
            Noise {
                amount: 0.2,
                size: 4.0,
                animated: false
            }
        );
        let wild = no_noise().blend(&rest, 3.0);
        assert_eq!(wild.amount, 1.0, "clamped to its range");
        assert!(rest.blend(&no_noise(), -5.0).size >= MIN_GRAIN);
    }

    #[test]
    fn directional_blurs_turn_and_grow() {
        let step = ChangeStep {
            span: Span::new(0.0, 100.0, Easing::Linear),
            from: Some(DirectionalBlurChange {
                amount: None,
                angle: Some(90.0),
            }),
            to: Some(DirectionalBlurChange {
                amount: Some(30.0),
                angle: None,
            }),
        };
        let half = replacing(&NO_DIRECTIONAL_BLUR, &[step], 50.0);
        assert_eq!(
            half,
            DirectionalBlur {
                amount: 15.0,
                angle: 45.0
            }
        );
        assert_eq!(
            NO_DIRECTIONAL_BLUR
                .blend(&DirectionalBlur::default(), -1.0)
                .amount,
            0.0,
            "never negative"
        );
    }

    #[test]
    fn resizes_keep_their_anchor_in_place() {
        let rest = BoxState::at_rest(100.0, 40.0);
        let step = |from: Option<f64>, to: Option<f64>, anchor: (f64, f64)| ResizeStep {
            span: Span::new(100.0, 200.0, Easing::Linear),
            from: from.map(|width| crate::model::SizeChange {
                width: Some(width),
                height: None,
            }),
            to: to.map(|width| crate::model::SizeChange {
                width: Some(width),
                height: None,
            }),
            anchor,
        };
        // Growing from nothing, anchored on the left edge: the left edge of
        // the box (centre - width / 2) never moves.
        let grow = [step(Some(0.0), None, (0.0, 0.5))];
        for time in [0.0, 100.0, 130.0, 200.0, 300.0] {
            let state = replacing(&rest, &grow, time);
            let left = state.shift.x - state.width / 2.0;
            assert!((left - -50.0).abs() < 1e-9, "at {time}: {state:?}");
            assert_eq!((state.height, state.shift.y), (40.0, 0.0));
        }
        assert_eq!(replacing(&rest, &grow, 0.0).width, 0.0);
        assert_eq!(replacing(&rest, &grow, 150.0).width, 50.0);
        assert_eq!(replacing(&rest, &grow, 200.0), rest);

        // Anchored in the centre nothing shifts; on the right the right edge holds.
        let centred = replacing(&rest, &[step(None, Some(300.0), (0.5, 0.5))], 200.0);
        assert_eq!((centred.width, centred.shift), (300.0, Point::ZERO));
        let right = replacing(&rest, &[step(None, Some(300.0), (1.0, 0.5))], 200.0);
        assert_eq!(right.shift.x + right.width / 2.0, 50.0);
    }
}
