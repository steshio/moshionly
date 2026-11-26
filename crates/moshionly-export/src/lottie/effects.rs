//! Effects: a layer's glow, drop shadow, blur and directional blur, as
//! Lottie layer effects.
//!
//! Moshionly draws a layer's shadow and glow behind it and then blurs it
//! all, in the layer's own space, so they turn and scale with it. Lottie
//! layer effects work the same way, one after the other:
//!
//! - the glow is a Drop Shadow effect with no distance, before the shadow
//!   so that it does not glow around the shadow (the shadow then falls
//!   from the glow too, which is said in a warning). Lottie has no spread,
//!   so a glow's spread is left out with a warning;
//! - the shadow is a Drop Shadow effect;
//! - the blur is a Gaussian Blur effect, and the directional blur another
//!   one set to blur across or down only, whichever is nearer its angle
//!   (with a warning when the angle is between).
//!
//! Film grain has no Lottie effect and is left out, with a warning.
//!
//! The numbers follow lottie-web, the most used player: its shadow
//! softness is four times the Gaussian's standard deviation, and its
//! blurriness 1 / 0.3 times it. Moshionly's blur amounts are twice the
//! standard deviation. Skottie and the Android player read softness as a
//! blur radius, which draws shadows somewhat softer.

use moshionly_core::eval::{Property, PropertyValue};
use moshionly_core::model::{DirectionalBlur, Glow, Shadow};
use moshionly_core::{Node, NodeId};
use serde_json::{Map, Value};

use super::fit::Hints;
use super::json::{object, places};
use super::layers::Converter;
use super::props::{fixed_property, property};
use super::timeline::{places_for, tolerance};

/// Lottie's number for a Drop Shadow effect.
const DROP_SHADOW: u8 = 25;
/// Lottie's number for a Gaussian Blur effect.
const GAUSSIAN_BLUR: u8 = 29;

/// Lottie's numbers for the kinds of effect values.
const SLIDER: u8 = 0;
const ANGLE: u8 = 1;
const COLOR: u8 = 2;
const CHECKBOX: u8 = 4;
const DROPDOWN: u8 = 7;

/// A Gaussian Blur effect's "Blur Dimensions": both ways, across only or
/// down only.
const BOTH_WAYS: i64 = 1;
const ACROSS: i64 = 2;
const DOWN: i64 = 3;

/// A drop shadow as Lottie's effect values: colour, opacity from 0 to 255,
/// direction in degrees clockwise from up, distance and softness.
fn shadow_values(shadow: Option<Shadow>) -> [f64; 8] {
    let Some(shadow) = shadow else {
        return [0.0, 0.0, 0.0, 1.0, 0.0, 180.0, 0.0, 0.0];
    };
    let channel = |value: u8| f64::from(value) / 255.0;
    let distance = shadow.x.hypot(shadow.y);
    let direction = if distance > 0.0 {
        shadow.y.atan2(shadow.x).to_degrees() + 90.0
    } else {
        180.0
    };
    [
        channel(shadow.color.r),
        channel(shadow.color.g),
        channel(shadow.color.b),
        1.0,
        f64::from(shadow.color.a),
        direction,
        distance,
        2.0 * shadow.blur.max(0.0),
    ]
}

/// A glow as Drop Shadow values: no distance, and softness from its radius
/// the way a shadow's comes from its blur. The spread has no equivalent.
fn glow_values(glow: Option<Glow>) -> [f64; 8] {
    shadow_values(glow.map(|glow| Shadow {
        color: glow.color,
        x: 0.0,
        y: 0.0,
        blur: glow.radius,
    }))
}

/// How much of a directional blur at `angle` degrees goes across and how
/// much down.
fn ways(angle: f64) -> (f64, f64) {
    let angle = angle.to_radians();
    (angle.cos().abs(), angle.sin().abs())
}

/// One value of an effect.
fn value(ty: u8, name: &str, index: usize, property: Value) -> Value {
    object([
        ("ty", Value::from(ty)),
        ("nm", Value::from(name)),
        ("ix", Value::from(index + 1)),
        ("v", property),
    ])
}

/// An effect of kind `ty` with its values.
fn effect(ty: u8, name: &str, values: Vec<Value>) -> Value {
    object([
        ("ty", Value::from(ty)),
        ("nm", Value::from(name)),
        ("en", Value::from(1)),
        ("ef", Value::Array(values)),
    ])
}

impl Converter<'_> {
    fn shadow_at(&self, id: &NodeId, frame: f64) -> Option<Shadow> {
        let timeline = self.timeline;
        match timeline.value(id, Property::Shadow, timeline.clock.ms(frame)) {
            Some(PropertyValue::Shadow(shadow)) => shadow,
            _ => None,
        }
    }

    fn glow_at(&self, id: &NodeId, frame: f64) -> Option<Glow> {
        let timeline = self.timeline;
        match timeline.value(id, Property::Glow, timeline.clock.ms(frame)) {
            Some(PropertyValue::Glow(glow)) => glow,
            _ => None,
        }
    }

    fn directional_at(&self, id: &NodeId, frame: f64) -> Option<DirectionalBlur> {
        let timeline = self.timeline;
        match timeline.value(id, Property::DirectionalBlur, timeline.clock.ms(frame)) {
            Some(PropertyValue::DirectionalBlur(blur)) => blur.filter(|blur| blur.amount > 0.0),
            _ => None,
        }
    }

    fn has_noise(&self, id: &NodeId) -> bool {
        let timeline = self.timeline;
        self.survey.iter().any(|frame| {
            matches!(
                timeline.value(id, Property::Noise, timeline.clock.ms(*frame)),
                Some(PropertyValue::Noise(Some(_)))
            )
        })
    }

    /// Says that a layer's film grain is left out, when it ever has some.
    /// Every exported layer comes here, also those that never get effects
    /// of their own, such as a plain group or a picture with nothing but
    /// grain.
    pub fn note_grain(&mut self, node: &Node) {
        if self.has_noise(&node.id) {
            self.warnings.layer(
                node,
                "has film grain (noise), which Lottie cannot show; it is left out.",
            );
        }
    }

    /// How far a layer's effects reach past what it draws, in its own
    /// pixels, at most over the export: the shadow's offset and three
    /// standard deviations of each blur. 0 for a layer without effects.
    pub fn effect_reach(&self, id: &NodeId) -> f64 {
        let timeline = self.timeline;
        self.survey
            .iter()
            .map(|frame| {
                let shadow = shadow_values(self.shadow_at(id, *frame));
                let glow = glow_values(self.glow_at(id, *frame));
                let behind = (shadow[6] + 0.75 * shadow[7]).max(0.75 * glow[7]);
                let blur = timeline.number(id, Property::Blur, *frame).max(0.0);
                let directional = self
                    .directional_at(id, *frame)
                    .map_or(0.0, |blur| blur.amount);
                behind + 1.5 * (blur + directional)
            })
            .fold(0.0, f64::max)
    }

    /// Whether a layer ever has an effect Lottie draws: a blur, a shadow, a
    /// glow or a directional blur.
    pub fn has_effects(&self, id: &NodeId) -> bool {
        self.survey.iter().any(|frame| {
            self.timeline.number(id, Property::Blur, *frame) > 0.0
                || self.shadow_at(id, *frame).is_some()
                || self.glow_at(id, *frame).is_some()
                || self.directional_at(id, *frame).is_some()
        })
    }

    /// A Drop Shadow effect named `name` whose values over time are
    /// `values`, as [`shadow_values`] gives them.
    fn drop_shadow(&self, id: &NodeId, name: &str, values: &dyn Fn(f64) -> [f64; 8]) -> Value {
        let timeline = self.timeline;
        let hints: Hints = timeline.hints(id);
        let part =
            |range: std::ops::Range<usize>| move |frame: f64| values(frame)[range.clone()].to_vec();
        let color = timeline.fit(
            &part(0..4),
            &hints,
            tolerance::COLOR,
            places_for(tolerance::COLOR),
        );
        let opacity = timeline.fit(
            &part(4..5),
            &hints,
            tolerance::ALPHA_255,
            places_for(tolerance::ALPHA_255),
        );
        let direction = timeline.fit(&part(5..6), &hints, tolerance::ANGLE * 10.0, 3);
        let length = places_for(tolerance::LENGTH);
        let distance = timeline.fit(&part(6..7), &hints, tolerance::LENGTH, length);
        let softness = timeline.fit(&part(7..8), &hints, tolerance::LENGTH, length);
        effect(
            DROP_SHADOW,
            name,
            vec![
                value(COLOR, "Shadow Color", 0, property(&color, places::COLOR)),
                value(
                    SLIDER,
                    "Opacity",
                    1,
                    property(&opacity, places_for(tolerance::ALPHA_255)),
                ),
                value(ANGLE, "Direction", 2, property(&direction, places::ANGLE)),
                value(SLIDER, "Distance", 3, property(&distance, places::LENGTH)),
                value(SLIDER, "Softness", 4, property(&softness, places::LENGTH)),
            ],
        )
    }

    /// A Gaussian Blur effect named `name` that blurs `ways` (both ways,
    /// across or down) by `amount`, a Moshionly blur amount over time.
    fn gaussian_blur(
        &self,
        id: &NodeId,
        name: &str,
        amount: &dyn Fn(f64) -> f64,
        ways: i64,
    ) -> Value {
        let timeline = self.timeline;
        let blurriness = |frame: f64| vec![amount(frame).max(0.0) * 5.0 / 3.0];
        let blurriness = timeline.fit(
            &blurriness,
            &timeline.hints(id),
            tolerance::LENGTH,
            places_for(tolerance::LENGTH),
        );
        let fixed = |number: i64| fixed_property(Value::from(number));
        effect(
            GAUSSIAN_BLUR,
            name,
            vec![
                value(
                    SLIDER,
                    "Blurriness",
                    0,
                    property(&blurriness, places::LENGTH),
                ),
                value(DROPDOWN, "Blur Dimensions", 1, fixed(ways)),
                value(CHECKBOX, "Repeat Edge Pixels", 2, fixed(0)),
            ],
        )
    }

    /// Adds a layer's glow, drop shadow, blur and directional blur to its
    /// Lottie layer, when it ever has them, and says what is not drawn
    /// exactly. (Grain is named by [`Self::note_grain`].)
    pub fn add_effects(&mut self, node: &Node, layer: &mut Map<String, Value>) {
        let id = &node.id;
        let frames = self.survey.clone();
        let glowing = frames
            .iter()
            .any(|frame| self.glow_at(id, *frame).is_some());
        let shadowed = frames
            .iter()
            .any(|frame| self.shadow_at(id, *frame).is_some());
        let blurred = frames
            .iter()
            .any(|frame| self.timeline.number(id, Property::Blur, *frame) > 0.0);
        let smeared: Vec<DirectionalBlur> = frames
            .iter()
            .filter_map(|frame| self.directional_at(id, *frame))
            .collect();
        let mut effects = Vec::new();
        if glowing {
            let glow = |frame: f64| glow_values(self.glow_at(id, frame));
            effects.push(self.drop_shadow(id, "Glow", &glow));
            let spreads = frames.iter().any(|frame| {
                self.glow_at(id, *frame)
                    .is_some_and(|glow| glow.spread > 0.0)
            });
            if spreads {
                self.warnings.layer(
                    node,
                    "has a glow with a spread, which Lottie cannot show; the glow is drawn \
                     without it, so it is thinner.",
                );
            }
        }
        if shadowed {
            let shadow = |frame: f64| shadow_values(self.shadow_at(id, frame));
            effects.push(self.drop_shadow(id, "Drop Shadow", &shadow));
            if glowing {
                self.warnings.layer(
                    node,
                    "has a glow and a shadow; in Lottie the shadow also falls from the glow, \
                     so it is a little larger.",
                );
            }
            if self.has_reveal(id) {
                self.warnings.layer(
                    node,
                    "has a shadow and a reveal; Lottie players differ in whether the reveal \
                     also cuts the shadow, as Moshionly does, so some show the shadow past the \
                     edge that wipes.",
                );
            }
        }
        if blurred {
            let timeline = self.timeline;
            let blur = |frame: f64| timeline.number(id, Property::Blur, frame);
            effects.push(self.gaussian_blur(id, "Gaussian Blur", &blur, BOTH_WAYS));
        }
        if !smeared.is_empty() {
            // Lottie blurs across or down only: whichever carries more of
            // the blur over the export.
            let (across, down) = smeared.iter().fold((0.0, 0.0), |(x, y), blur| {
                let (cos, sin) = ways(blur.angle);
                (x + blur.amount * cos, y + blur.amount * sin)
            });
            let way = if across >= down { ACROSS } else { DOWN };
            let amount = |frame: f64| {
                self.directional_at(id, frame)
                    .map_or(0.0, |blur| blur.amount)
            };
            effects.push(self.gaussian_blur(id, "Directional Blur", &amount, way));
            let slanted = smeared.iter().find(|blur| {
                let (cos, sin) = ways(blur.angle);
                (if way == ACROSS { sin } else { cos }) > 1e-6
            });
            if let Some(blur) = slanted {
                let drawn = if way == ACROSS { "across" } else { "down" };
                self.warnings.layer(
                    node,
                    &format!(
                        "has a directional blur at {} degrees, but Lottie blurs only across or \
                         down; it is drawn {drawn}.",
                        super::json::round(blur.angle, 1)
                    ),
                );
            }
        }
        if effects.is_empty() {
            return;
        }
        // lottie-web draws effects only inside the layer's own box unless
        // told how far they reach, as parts of the box.
        let reach = self.effect_reach(id);
        let (width, height) = self
            .survey
            .iter()
            .map(|frame| self.size(id, *frame))
            .fold((f64::MAX, f64::MAX), |(w, h), (fw, fh)| {
                (w.min(fw), h.min(fh))
            });
        let part = |side: f64| (reach + 4.0) / side.max(1.0);
        let (across, down) = (part(width), part(height));
        let percent = |value: f64| format!("{}%", super::json::round(value * 100.0, 1));
        let size = object([
            ("x", Value::from(percent(-across))),
            ("y", Value::from(percent(-down))),
            ("width", Value::from(percent(1.0 + 2.0 * across))),
            ("height", Value::from(percent(1.0 + 2.0 * down))),
        ]);
        for effect in &mut effects {
            if let Value::Object(effect) = effect {
                effect.insert("fs".into(), size.clone());
            }
        }
        layer.insert("ef".into(), Value::Array(effects));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::Color;

    #[test]
    fn shadows_become_direction_and_distance() {
        let below = Shadow {
            color: Color::rgba(0, 0, 0, 64),
            x: 0.0,
            y: 24.0,
            blur: 30.0,
        };
        let [_, _, _, _, opacity, direction, distance, softness] = shadow_values(Some(below));
        assert_eq!((opacity, distance, softness), (64.0, 24.0, 60.0));
        assert!((direction - 180.0).abs() < 1e-9, "down is 180 degrees");
        let right = Shadow {
            x: 10.0,
            y: 0.0,
            ..below
        };
        assert!((shadow_values(Some(right))[5] - 90.0).abs() < 1e-9);
        assert_eq!(shadow_values(None)[4], 0.0, "no shadow is a clear one");
    }

    #[test]
    fn glows_are_shadows_that_go_nowhere() {
        let glow = Glow {
            color: Color::rgba(255, 0, 0, 128),
            radius: 12.0,
            spread: 4.0,
        };
        let [red, green, _, _, opacity, _, distance, softness] = glow_values(Some(glow));
        assert_eq!((red, green, opacity), (1.0, 0.0, 128.0));
        assert_eq!((distance, softness), (0.0, 24.0));
        assert_eq!(ways(0.0), (1.0, 0.0));
        let (across, down) = ways(90.0);
        assert!(across < 1e-12 && (down - 1.0).abs() < 1e-12);
    }
}
