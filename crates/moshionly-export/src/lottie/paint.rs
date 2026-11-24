//! Paints: fills and strokes in solid colours and gradients.
//!
//! A paint can change over time: a colour can blend into another, a fill
//! can turn into a gradient, a gradient's stops can slide. Lottie's fill
//! and stroke items cannot change kind, so each paint is looked at over the
//! whole export first. One that is only ever a colour (or nothing) becomes
//! a solid fill whose colour and opacity are keyframed. One that is ever a
//! gradient becomes a gradient fill throughout: a colour is a gradient with
//! every stop in that colour, which is also how Moshionly blends a colour
//! into a gradient, and no paint at all is the gradient made see-through.

use moshionly_core::Color;
use moshionly_core::model::{FillRule, Gradient, GradientStop, Paint, StrokeCap, StrokeJoin};
use serde_json::{Map, Value};

use super::fit::{Animated, Hints};
use super::json::{num, object, places};
use super::props::{fixed_property, property};
use super::timeline::{Timeline, places_for, tolerance};

/// The stroke miter limit the renderer uses.
pub(crate) const MITER_LIMIT: f64 = 4.0;

/// What a paint is applied to, and how.
pub(crate) enum Role<'a> {
    /// The inside of the shapes before it.
    Fill(FillRule),
    /// The outline of the shapes before it.
    Stroke {
        /// Width in pixels over time.
        width: &'a Animated,
        /// Ends of open paths.
        cap: StrokeCap,
        /// Corners.
        join: StrokeJoin,
    },
}

/// Where a paint's values come from.
pub(crate) struct Source<'a> {
    /// Fits the keyframes.
    pub timeline: Timeline<'a>,
    /// Where the paint may change.
    pub hints: &'a Hints,
    /// The paint at a frame.
    pub paint: &'a dyn Fn(f64) -> Option<Paint>,
    /// The size of the box gradients span, at a frame.
    pub size: &'a dyn Fn(f64) -> (f64, f64),
    /// The frames the paint is looked at to choose its kind.
    pub survey: &'a [f64],
}

/// The kind of gradient a paint is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Linear,
    Radial,
}

impl Kind {
    fn of(gradient: &Gradient) -> Self {
        match gradient {
            Gradient::LinearGradient(_) => Kind::Linear,
            Gradient::RadialGradient(_) => Kind::Radial,
        }
    }
}

/// What a paint is over the whole export.
struct Survey {
    /// `None` when it is only ever a colour or nothing.
    gradient: Option<Gradient>,
    /// The most stops any of its gradients has.
    stops: usize,
    /// Whether any colour of it is ever see-through, or it is ever missing.
    translucent: bool,
    /// Whether it is both linear and radial at different times.
    mixed: bool,
    /// Whether a radial gradient of it is ever stretched into an ellipse.
    stretched: bool,
    /// Whether it is ever there at all.
    seen: bool,
}

fn survey(source: &Source<'_>) -> Survey {
    let mut found = Survey {
        gradient: None,
        stops: 0,
        translucent: false,
        mixed: false,
        stretched: false,
        seen: false,
    };
    let (mut linear, mut radial) = (0, 0);
    for frame in source.survey {
        let Some(paint) = (source.paint)(*frame) else {
            found.translucent = true;
            continue;
        };
        found.seen = true;
        match &paint {
            Paint::Solid(color) => found.translucent |= color.a < 255,
            Paint::Gradient(gradient) => {
                found.translucent |= gradient.stops().iter().any(|stop| stop.color.a < 255);
                found.stops = found.stops.max(gradient.stops().len());
                match gradient {
                    Gradient::LinearGradient(_) => linear += 1,
                    Gradient::RadialGradient(radial_gradient) => {
                        radial += 1;
                        let (width, height) = (source.size)(*frame);
                        let rx = radial_gradient.radius[0] * width;
                        let ry = radial_gradient.radius[1] * height;
                        found.stretched |= (rx - ry).abs() > tolerance::LENGTH * 5.0;
                    }
                }
                if found.gradient.is_none() {
                    found.gradient = Some(gradient.clone());
                }
            }
        }
    }
    found.mixed = linear > 0 && radial > 0;
    // The kind shown longest is the one drawn.
    if found.mixed {
        let wanted = if radial > linear {
            Kind::Radial
        } else {
            Kind::Linear
        };
        found.gradient = source
            .survey
            .iter()
            .find_map(|frame| match (source.paint)(*frame) {
                Some(Paint::Gradient(gradient)) if Kind::of(&gradient) == wanted => Some(gradient),
                _ => None,
            });
    }
    found
}

/// What came out of painting, besides the item.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Notes {
    /// The paint is both linear and radial over time; one kind is drawn.
    pub mixed: bool,
    /// A radial gradient is stretched into an ellipse; it is drawn round.
    pub stretched: bool,
    /// A stroke inside or outside the edge is drawn centred on it.
    pub centred_stroke: bool,
}

/// The Lottie item that paints with `source` in `role`, or `None` when the
/// paint is never there.
pub(crate) fn item(source: &Source<'_>, role: &Role<'_>) -> (Option<Value>, Notes) {
    let found = survey(source);
    if !found.seen {
        return (None, Notes::default());
    }
    let notes = Notes {
        mixed: found.mixed,
        stretched: found.stretched,
        centred_stroke: false,
    };
    let mut item = match &found.gradient {
        None => solid(source, role),
        Some(template) => gradient(source, role, template, found.stops, found.translucent),
    };
    match role {
        Role::Fill(rule) => {
            if *rule == FillRule::EvenOdd {
                item.insert("r".into(), Value::from(2));
            }
        }
        Role::Stroke { width, cap, join } => {
            item.insert("w".into(), property(width, places::LENGTH));
            item.insert("lc".into(), Value::from(cap_code(*cap)));
            item.insert("lj".into(), Value::from(join_code(*join)));
            if *join == StrokeJoin::Miter {
                item.insert("ml".into(), num(MITER_LIMIT, 1));
            }
        }
    }
    (Some(Value::Object(item)), notes)
}

/// The colour to use where a paint is missing: the first colour it has.
fn fallback_color(source: &Source<'_>) -> Color {
    source
        .survey
        .iter()
        .find_map(|frame| (source.paint)(*frame).and_then(|paint| paint.as_solid()))
        .unwrap_or(Color::BLACK)
}

fn solid(source: &Source<'_>, role: &Role<'_>) -> Map<String, Value> {
    let fallback = fallback_color(source);
    let color_at = |frame: f64| match (source.paint)(frame) {
        Some(Paint::Solid(color)) => (color, f64::from(color.a) / 255.0),
        _ => (fallback, 0.0),
    };
    let rgb = |frame: f64| {
        let (color, _) = color_at(frame);
        channels(color)[..3].to_vec()
    };
    let opacity = |frame: f64| vec![color_at(frame).1 * 100.0];
    let color_places = places_for(tolerance::COLOR);
    let c = source
        .timeline
        .fit(&rgb, source.hints, tolerance::COLOR, color_places);
    let o = source.timeline.fit(
        &opacity,
        source.hints,
        tolerance::ALPHA,
        places_for(tolerance::ALPHA),
    );
    let ty = match role {
        Role::Fill(_) => "fl",
        Role::Stroke { .. } => "st",
    };
    let mut item = Map::new();
    item.insert("ty".into(), Value::from(ty));
    item.insert("c".into(), property(&c, color_places));
    item.insert("o".into(), property(&o, places::PERCENT));
    item
}

/// A colour's channels from 0 to 1, alpha last.
fn channels(color: Color) -> [f64; 4] {
    [color.r, color.g, color.b, color.a].map(|channel| f64::from(channel) / 255.0)
}

/// `gradient`'s stops, padded to `count` by repeating the last one.
fn padded(stops: &[GradientStop], count: usize) -> Vec<GradientStop> {
    let mut stops = stops.to_vec();
    let last = stops
        .last()
        .copied()
        .unwrap_or(GradientStop::new(1.0, Color::TRANSPARENT));
    stops.resize(count.max(1), last);
    stops
}

/// The gradient a paint is at a moment, in the template's kind: colours
/// become the template in one colour, and a missing paint becomes it
/// see-through.
fn as_gradient(paint: Option<Paint>, template: &Gradient, fallback: Color) -> Gradient {
    let recolor = |color: Color| {
        let mut flat = template.clone();
        let stops = match &mut flat {
            Gradient::LinearGradient(linear) => &mut linear.stops,
            Gradient::RadialGradient(radial) => &mut radial.stops,
        };
        for stop in stops {
            stop.color = color;
        }
        flat
    };
    match paint {
        Some(Paint::Gradient(gradient)) if Kind::of(&gradient) == Kind::of(template) => gradient,
        Some(Paint::Gradient(gradient)) => {
            // Drawn in the other kind: its stops on the template's geometry.
            let mut shaped = template.clone();
            let stops = gradient.stops().to_vec();
            match &mut shaped {
                Gradient::LinearGradient(linear) => linear.stops = stops,
                Gradient::RadialGradient(radial) => radial.stops = stops,
            }
            shaped
        }
        Some(Paint::Solid(color)) => recolor(color),
        None => recolor(fallback.with_alpha(0)),
    }
}

/// The start and end points of a gradient in pixels: for a radial one,
/// its centre and a point on its circle.
fn ends(gradient: &Gradient, (width, height): (f64, f64)) -> [f64; 4] {
    match gradient {
        Gradient::LinearGradient(linear) => [
            linear.start[0] * width,
            linear.start[1] * height,
            linear.end[0] * width,
            linear.end[1] * height,
        ],
        Gradient::RadialGradient(radial) => {
            let (cx, cy) = (radial.center[0] * width, radial.center[1] * height);
            // A stretched gradient is drawn as the circle of the same area.
            let radius = (radial.radius[0] * width * radial.radius[1] * height)
                .max(0.0)
                .sqrt();
            let angle = radial.rotation.to_radians();
            [cx, cy, cx + radius * angle.cos(), cy + radius * angle.sin()]
        }
    }
}

fn gradient(
    source: &Source<'_>,
    role: &Role<'_>,
    template: &Gradient,
    count: usize,
    translucent: bool,
) -> Map<String, Value> {
    let fallback = template
        .stops()
        .first()
        .map_or(Color::BLACK, |stop| stop.color);
    let at = |frame: f64| as_gradient((source.paint)(frame), template, fallback);
    let stops = |frame: f64| {
        let stops = padded(at(frame).stops(), count);
        let mut values = Vec::with_capacity(count * 6);
        for stop in &stops {
            let [r, g, b, _] = channels(stop.color);
            values.extend([stop.offset, r, g, b]);
        }
        if translucent {
            for stop in &stops {
                values.extend([stop.offset, channels(stop.color)[3]]);
            }
        }
        values
    };
    let start = |frame: f64| ends(&at(frame), (source.size)(frame))[..2].to_vec();
    let end = |frame: f64| ends(&at(frame), (source.size)(frame))[2..].to_vec();
    let (timeline, hints) = (source.timeline, source.hints);
    let color_places = places_for(tolerance::COLOR);
    let length_places = places_for(tolerance::LENGTH);
    let g = timeline.fit(&stops, hints, tolerance::COLOR, color_places);
    let s = timeline.fit(&start, hints, tolerance::LENGTH, length_places);
    let e = timeline.fit(&end, hints, tolerance::LENGTH, length_places);
    let ty = match role {
        Role::Fill(_) => "gf",
        Role::Stroke { .. } => "gs",
    };
    let mut item = Map::new();
    item.insert("ty".into(), Value::from(ty));
    item.insert(
        "t".into(),
        Value::from(match Kind::of(template) {
            Kind::Linear => 1,
            Kind::Radial => 2,
        }),
    );
    item.insert("s".into(), property(&s, places::LENGTH));
    item.insert("e".into(), property(&e, places::LENGTH));
    item.insert(
        "g".into(),
        object([("p", Value::from(count)), ("k", property(&g, color_places))]),
    );
    item.insert("o".into(), fixed_property(Value::from(100)));
    item
}

/// Lottie's number for a line cap.
fn cap_code(cap: StrokeCap) -> u8 {
    match cap {
        StrokeCap::Butt => 1,
        StrokeCap::Round => 2,
        StrokeCap::Square => 3,
    }
}

/// Lottie's number for a line join.
fn join_code(join: StrokeJoin) -> u8 {
    match join {
        StrokeJoin::Miter => 1,
        StrokeJoin::Round => 2,
        StrokeJoin::Bevel => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::model::{LinearGradient, RadialGradient};

    fn linear() -> Gradient {
        Gradient::LinearGradient(LinearGradient {
            start: [0.0, 0.5],
            end: [1.0, 0.5],
            stops: vec![
                GradientStop::new(0.0, Color::rgb(255, 0, 0)),
                GradientStop::new(1.0, Color::rgb(0, 0, 255)),
            ],
        })
    }

    #[test]
    fn colours_become_the_gradient_in_one_colour() {
        let template = linear();
        let flat = as_gradient(Some(Paint::Solid(Color::WHITE)), &template, Color::BLACK);
        assert!(flat.stops().iter().all(|stop| stop.color == Color::WHITE));
        let clear = as_gradient(None, &template, Color::BLACK);
        assert!(clear.stops().iter().all(|stop| stop.color.a == 0));
        assert_eq!(padded(template.stops(), 3)[2], template.stops()[1]);
    }

    #[test]
    fn gradient_ends_are_in_pixels() {
        assert_eq!(ends(&linear(), (200.0, 100.0)), [0.0, 50.0, 200.0, 50.0]);
        let round = Gradient::RadialGradient(RadialGradient {
            center: [0.5, 0.5],
            radius: [0.5, 0.5],
            rotation: 90.0,
            stops: Vec::new(),
        });
        let [cx, cy, ex, ey] = ends(&round, (100.0, 100.0));
        assert_eq!((cx, cy), (50.0, 50.0));
        assert!((ex - 50.0).abs() < 1e-9 && (ey - 100.0).abs() < 1e-9);
    }
}
