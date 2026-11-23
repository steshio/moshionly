//! Effects: a layer's drop shadow and blur, as Lottie layer effects.
//!
//! Moshionly draws a layer's shadow behind it and then blurs the two
//! together, in the layer's own space, so both turn and scale with it.
//! Lottie layer effects work the same way and in the same order: a Drop
//! Shadow effect, then a Gaussian Blur effect.
//!
//! The numbers follow lottie-web, the most used player: its shadow
//! softness is four times the Gaussian's standard deviation, and its
//! blurriness 1 / 0.3 times it. Moshionly's blur amounts are twice the
//! standard deviation. Skottie and the Android player read softness as a
//! blur radius, which draws shadows somewhat softer.

use moshionly_core::Node;
use moshionly_core::eval::{Property, PropertyValue};
use moshionly_core::model::Shadow;
use serde_json::{Map, Value};

use super::fit::Hints;
use super::json::{object, places};
use super::layers::Converter;
use super::props::property;
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
    /// Adds a layer's drop shadow and blur to its Lottie layer, when it
    /// ever has them.
    pub fn add_effects(&mut self, node: &Node, layer: &mut Map<String, Value>) {
        let id = &node.id;
        let timeline = self.timeline;
        let shadow_at =
            |frame: f64| match timeline.value(id, Property::Shadow, timeline.clock.ms(frame)) {
                Some(PropertyValue::Shadow(shadow)) => shadow,
                _ => None,
            };
        let shadowed = self.survey.iter().any(|frame| shadow_at(*frame).is_some());
        let blurred = self
            .survey
            .iter()
            .any(|frame| timeline.number(id, Property::Blur, *frame) > 0.0);
        if !shadowed && !blurred {
            return;
        }
        let hints: Hints = timeline.hints(id);
        let mut effects = Vec::new();
        if shadowed {
            let part = |range: std::ops::Range<usize>| {
                move |frame: f64| shadow_values(shadow_at(frame))[range.clone()].to_vec()
            };
            let color = timeline.fit(
                &part(0..4),
                &hints,
                tolerance::COLOR,
                places_for(tolerance::COLOR),
            );
            let opacity = timeline.fit(&part(4..5), &hints, 0.2, 2);
            let direction = timeline.fit(&part(5..6), &hints, tolerance::ANGLE * 10.0, 3);
            let length = places_for(tolerance::LENGTH);
            let distance = timeline.fit(&part(6..7), &hints, tolerance::LENGTH, length);
            let softness = timeline.fit(&part(7..8), &hints, tolerance::LENGTH, length);
            effects.push(effect(
                DROP_SHADOW,
                "Drop Shadow",
                vec![
                    value(COLOR, "Shadow Color", 0, property(&color, places::COLOR)),
                    value(SLIDER, "Opacity", 1, property(&opacity, 2)),
                    value(ANGLE, "Direction", 2, property(&direction, places::ANGLE)),
                    value(SLIDER, "Distance", 3, property(&distance, places::LENGTH)),
                    value(SLIDER, "Softness", 4, property(&softness, places::LENGTH)),
                ],
            ));
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
            let blurriness =
                |frame: f64| vec![timeline.number(id, Property::Blur, frame).max(0.0) * 5.0 / 3.0];
            let blurriness = timeline.fit(
                &blurriness,
                &hints,
                tolerance::LENGTH,
                places_for(tolerance::LENGTH),
            );
            let fixed = |number: i64| object([("k", Value::from(number))]);
            effects.push(effect(
                GAUSSIAN_BLUR,
                "Gaussian Blur",
                vec![
                    value(
                        SLIDER,
                        "Blurriness",
                        0,
                        property(&blurriness, places::LENGTH),
                    ),
                    value(DROPDOWN, "Blur Dimensions", 1, fixed(1)),
                    value(CHECKBOX, "Repeat Edge Pixels", 2, fixed(0)),
                ],
            ));
        }
        // lottie-web draws effects only inside the layer's own box unless
        // told how far they reach, as parts of the box.
        let reach = self
            .survey
            .iter()
            .map(|frame| {
                let shadow = shadow_values(shadow_at(*frame));
                let blur = timeline.number(id, Property::Blur, *frame).max(0.0);
                // Three standard deviations of each blur, and the offset.
                shadow[6] + 0.75 * shadow[7] + 1.5 * blur
            })
            .fold(0.0, f64::max);
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
}
