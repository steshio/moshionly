//! Lottie properties: a fixed value or keyframes, and the transform every
//! layer and shape group has.

use serde_json::{Map, Value};

use super::fit::{Animated, Ease, Keyframe};
use super::json::{num, object, places};

/// A property as Lottie writes it: `{"k": value}` when fixed, or
/// `{"a": 1, "k": [keyframes]}`. One-component values are written as a
/// plain number when fixed, as Lottie expects.
pub(crate) fn property(animated: &Animated, places: i32) -> Value {
    match animated {
        Animated::Static(value) => object([("k", fixed(value, places))]),
        Animated::Keyframes(keys) => {
            let frames = keys
                .iter()
                .enumerate()
                .map(|(index, key)| keyframe(key, index + 1 == keys.len(), places))
                .collect();
            object([("a", Value::from(1)), ("k", Value::Array(frames))])
        }
    }
}

/// A fixed value: a number for one component, else an array.
fn fixed(value: &[f64], places: i32) -> Value {
    match value {
        [only] => num(*only, places),
        _ => array(value, places),
    }
}

/// Numbers as an array.
pub(crate) fn array(values: &[f64], places: i32) -> Value {
    Value::Array(values.iter().map(|value| num(*value, places)).collect())
}

fn keyframe(key: &Keyframe, last: bool, value_places: i32) -> Value {
    let mut frame = Map::new();
    frame.insert("t".into(), num(key.time, places::TIME));
    frame.insert("s".into(), array(&key.value, value_places));
    if !last {
        match key.ease {
            Ease::Hold => {
                frame.insert("h".into(), Value::from(1));
            }
            Ease::Curve([x1, y1, x2, y2]) => {
                let handle = |x: f64, y: f64| {
                    object([("x", num(x, places::EASE)), ("y", num(y, places::EASE))])
                };
                frame.insert("o".into(), handle(x1, y1));
                frame.insert("i".into(), handle(x2, y2));
            }
        }
    }
    Value::Object(frame)
}

/// The same property with each value changed by `f`, keyframes kept.
pub(crate) fn map(animated: &Animated, f: impl Fn(&[f64]) -> Vec<f64>) -> Animated {
    match animated {
        Animated::Static(value) => Animated::Static(f(value)),
        Animated::Keyframes(keys) => Animated::Keyframes(
            keys.iter()
                .map(|key| Keyframe {
                    time: key.time,
                    value: f(&key.value),
                    ease: key.ease,
                })
                .collect(),
        ),
    }
}

/// Where a layer or a group sits: its position as one property, or as two
/// when its coordinates need keyframes of their own.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Position {
    /// `[x, y]` together.
    Joined(Animated),
    /// `x` and `y` apart.
    Split(Animated, Animated),
}

impl Position {
    /// One position from the fitted `x` and `y`: joined when they can
    /// share keyframes, which is the usual case and plays everywhere.
    pub fn from_parts(x: Animated, y: Animated) -> Self {
        match join(&x, &y) {
            Some(joined) => Position::Joined(joined),
            None => Position::Split(x, y),
        }
    }

    /// Whether the position is `[0, 0]` the whole time.
    fn is_origin(&self) -> bool {
        match self {
            Position::Joined(Animated::Static(value)) => value.iter().all(|v| *v == 0.0),
            Position::Split(Animated::Static(x), Animated::Static(y)) => x[0] == 0.0 && y[0] == 0.0,
            _ => false,
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Position::Joined(position) => property(position, places::LENGTH),
            Position::Split(x, y) => object([
                ("s", Value::Bool(true)),
                ("x", property(x, places::LENGTH)),
                ("y", property(y, places::LENGTH)),
            ]),
        }
    }
}

/// `x` and `y` as one property, when their keyframes are at the same
/// moments with the same easings, or when one of them never changes.
fn join(x: &Animated, y: &Animated) -> Option<Animated> {
    let pair = |x: &[f64], y: &[f64]| vec![x[0], y[0]];
    match (x, y) {
        (Animated::Static(x), Animated::Static(y)) => Some(Animated::Static(pair(x, y))),
        (Animated::Static(fixed), moving) => Some(map(moving, |v| pair(fixed, v))),
        (moving, Animated::Static(fixed)) => Some(map(moving, |v| pair(v, fixed))),
        (Animated::Keyframes(xs), Animated::Keyframes(ys)) => {
            let same = xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys)
                    .all(|(x, y)| x.time == y.time && same_ease(x.ease, y.ease));
            same.then(|| {
                Animated::Keyframes(
                    xs.iter()
                        .zip(ys)
                        .map(|(x, y)| Keyframe {
                            time: x.time,
                            value: pair(&x.value, &y.value),
                            ease: x.ease,
                        })
                        .collect(),
                )
            })
        }
    }
}

/// Whether two keyframes move on the same way. A curve between equal
/// values does not move, so any easing will do for it; that case is not
/// looked into, and such keyframes are simply not joined.
fn same_ease(a: Ease, b: Ease) -> bool {
    a == b
}

/// A layer's or group's transform: anchor, position, scale, rotation and
/// opacity, each fixed or keyframed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Transform {
    /// The point of the content that sits at `position`, and that rotation
    /// and scale turn about: `[x, y]`.
    pub anchor: Animated,
    /// Where the anchor goes in the parent.
    pub position: Position,
    /// Scale in percent: one component for the same both ways, or two.
    pub scale: Animated,
    /// Clockwise rotation in degrees.
    pub rotation: Animated,
    /// Opacity from 0 to 100.
    pub opacity: Animated,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            anchor: Animated::Static(vec![0.0, 0.0]),
            position: Position::Joined(Animated::Static(vec![0.0, 0.0])),
            scale: Animated::Static(vec![100.0]),
            rotation: Animated::Static(vec![0.0]),
            opacity: Animated::Static(vec![100.0]),
        }
    }
}

impl Transform {
    /// The transform of a layer as Lottie writes it, values at their
    /// default left out.
    pub fn to_json(&self) -> Value {
        let is = |animated: &Animated, value: &[f64]| animated == &Animated::Static(value.to_vec());
        self.write(|key, animated| match key {
            "a" => !is(animated, &[0.0, 0.0]),
            "s" => !is(animated, &[100.0]),
            "r" => !is(animated, &[0.0]),
            "o" => !is(animated, &[100.0]),
            _ => true,
        })
    }

    /// The transform as the last item of a shape group, which Lottie
    /// requires even when it changes nothing. Every value is written:
    /// players read a group's transform without defaults. Its kind `ty`
    /// comes first, as in every shape item: rlottie reads an item's keys in
    /// order and skips those before `ty`.
    pub fn to_group_item(&self) -> Value {
        let mut map = Map::new();
        map.insert("ty".into(), Value::from("tr"));
        self.write_into(&mut map, |_, _| true);
        if !map.contains_key("p") {
            map.insert("p".into(), object([("k", array(&[0.0, 0.0], 0))]));
        }
        Value::Object(map)
    }

    /// The values `keep` says yes to.
    fn write(&self, keep: impl Fn(&str, &Animated) -> bool) -> Value {
        let mut map = Map::new();
        self.write_into(&mut map, keep);
        Value::Object(map)
    }

    /// Adds the values `keep` says yes to to `map`.
    fn write_into(&self, map: &mut Map<String, Value>, keep: impl Fn(&str, &Animated) -> bool) {
        if keep("a", &self.anchor) {
            map.insert("a".into(), property(&self.anchor, places::LENGTH));
        }
        if !self.position.is_origin() {
            map.insert("p".into(), self.position.to_json());
        }
        if keep("s", &self.scale) {
            let both = map_scale(&self.scale);
            map.insert("s".into(), property(&both, places::PERCENT));
        }
        if keep("r", &self.rotation) {
            map.insert("r".into(), property(&self.rotation, places::ANGLE));
        }
        if keep("o", &self.opacity) {
            map.insert("o".into(), property(&self.opacity, places::PERCENT));
        }
    }
}

/// A scale as Lottie's `[x, y]`: a uniform one is the same both ways.
fn map_scale(scale: &Animated) -> Animated {
    map(scale, |value| match value {
        [both] => vec![*both, *both],
        _ => value.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lottie::fit::LINEAR;

    fn keys(values: &[(f64, f64)]) -> Animated {
        Animated::Keyframes(
            values
                .iter()
                .map(|(time, value)| Keyframe {
                    time: *time,
                    value: vec![*value],
                    ease: Ease::Curve(LINEAR),
                })
                .collect(),
        )
    }

    #[test]
    fn fixed_and_keyframed_values() {
        assert_eq!(
            property(&Animated::Static(vec![12.5]), 3).to_string(),
            r#"{"k":12.5}"#
        );
        assert_eq!(
            property(&Animated::Static(vec![1.0, 2.0]), 3).to_string(),
            r#"{"k":[1,2]}"#
        );
        let mut animated = keys(&[(0.0, 0.0), (10.0, 100.0)]);
        if let Animated::Keyframes(keys) = &mut animated {
            keys[0].ease = Ease::Curve([0.25, 0.1, 0.25, 1.0]);
        }
        assert_eq!(
            property(&animated, 3).to_string(),
            r#"{"a":1,"k":[{"t":0,"s":[0],"o":{"x":0.25,"y":0.1},"i":{"x":0.25,"y":1}},{"t":10,"s":[100]}]}"#
        );
    }

    #[test]
    fn positions_join_when_they_can() {
        let moving = keys(&[(0.0, 0.0), (10.0, 100.0)]);
        let fixed = Animated::Static(vec![5.0]);
        let joined = Position::from_parts(moving.clone(), fixed);
        assert!(matches!(joined, Position::Joined(_)));
        let other = keys(&[(0.0, 0.0), (12.0, 100.0)]);
        assert!(matches!(
            Position::from_parts(moving, other),
            Position::Split(..)
        ));
    }

    #[test]
    fn defaults_are_left_out() {
        assert_eq!(Transform::default().to_json().to_string(), "{}");
        let transform = Transform {
            scale: Animated::Static(vec![50.0]),
            ..Transform::default()
        };
        assert_eq!(transform.to_json().to_string(), r#"{"s":{"k":[50,50]}}"#);
        assert_eq!(
            Transform::default().to_group_item().to_string(),
            r#"{"ty":"tr","a":{"k":[0,0]},"s":{"k":[100,100]},"r":{"k":0},"o":{"k":100},"p":{"k":[0,0]}}"#
        );
    }
}
