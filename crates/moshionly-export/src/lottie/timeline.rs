//! Where values come from: a layer's properties at any moment, as core's
//! evaluation gives them, and the hints that say where they change.

use moshionly_core::eval::{Property, PropertyValue, value_at};
use moshionly_core::model::{CornerRadius, Paint, Stroke};
use moshionly_core::{Document, Node, NodeData, NodeId};

use super::clock::Clock;
use super::fit::{self, Animated, Hints, Track};
use super::scenes::Scenes;

/// Reads layer properties over the exported time.
#[derive(Clone, Copy)]
pub(crate) struct Timeline<'a> {
    /// The document.
    pub document: &'a Document,
    /// Frames and milliseconds.
    pub clock: Clock,
    /// Evaluated scenes, for what only a scene knows.
    pub scenes: &'a Scenes<'a>,
}

/// How close keyframes keep to the true value, per kind of value.
pub(crate) mod tolerance {
    /// Pixels: positions, sizes, radii, stroke widths, blurs.
    pub const LENGTH: f64 = 0.1;
    /// Degrees. At 300 pixels from the centre this is a twentieth of a
    /// pixel.
    pub const ANGLE: f64 = 0.01;
    /// Scale in percent. On a layer 600 pixels wide this is a tenth of a
    /// pixel at the edge.
    pub const SCALE: f64 = 0.03;
    /// Opacity in percent: a fifth of one of 255 steps.
    pub const OPACITY: f64 = 0.08;
    /// Colour channels from 0 to 1: about one of 255 steps, which is how
    /// finely colours blend.
    pub const COLOR: f64 = 0.004;
}

impl<'a> Timeline<'a> {
    /// A property of `layer` at artboard moment `ms`.
    pub fn value(&self, layer: &NodeId, property: Property, ms: f64) -> Option<PropertyValue> {
        value_at(self.document, layer, property, ms)
    }

    /// A number property of `layer` at frame `frame`; 0 when the layer
    /// does not have it.
    pub fn number(&self, layer: &NodeId, property: Property, frame: f64) -> f64 {
        match self.value(layer, property, self.clock.ms(frame)) {
            Some(PropertyValue::Number(value)) => value,
            Some(PropertyValue::Flag(true)) => 1.0,
            _ => 0.0,
        }
    }

    /// Whether `layer` is shown (by `show`, `hide` and `hidden`) at frame
    /// `frame`.
    pub fn visible(&self, layer: &NodeId, frame: f64) -> bool {
        matches!(
            self.value(layer, Property::Visible, self.clock.ms(frame)),
            Some(PropertyValue::Flag(true))
        )
    }

    /// The fill of `layer` at frame `frame`.
    pub fn fill(&self, layer: &NodeId, frame: f64) -> Option<Paint> {
        match self.value(layer, Property::Fill, self.clock.ms(frame)) {
            Some(PropertyValue::Paint(paint)) => paint,
            _ => None,
        }
    }

    /// The stroke of `layer` at frame `frame`.
    pub fn stroke(&self, layer: &NodeId, frame: f64) -> Option<Stroke> {
        match self.value(layer, Property::Stroke, self.clock.ms(frame)) {
            Some(PropertyValue::Stroke(stroke)) => stroke,
            _ => None,
        }
    }

    /// The corner radii of `layer` at frame `frame`: top-left, top-right,
    /// bottom-right, bottom-left.
    pub fn radii(&self, layer: &NodeId, frame: f64) -> [f64; 4] {
        match self.value(layer, Property::CornerRadius, self.clock.ms(frame)) {
            Some(PropertyValue::CornerRadius(radius)) => radius.radii(),
            _ => CornerRadius::default().radii(),
        }
    }

    /// Where the values of `layer` may change and how: the times and
    /// easings of the animations that target it.
    pub fn hints(&self, layer: &NodeId) -> Hints {
        let mut hints = Hints::default();
        for animation in self.document.animations_targeting(layer) {
            self.add_hints(animation, &mut hints);
        }
        hints
    }

    fn add_hints(&self, animation: &Node, hints: &mut Hints) {
        let Some((start, end)) = animation.data.span() else {
            return;
        };
        let (start, end) = (self.clock.frame(start), self.clock.frame(end));
        match (&animation.data, animation.data.easing()) {
            (NodeData::Show(_) | NodeData::Hide(_), _) | (_, None) => hints.cut(start),
            (_, Some(easing)) => hints.add(start, end, *easing),
        }
    }

    /// Fits keyframes to `at`, a function of frames, over the whole export.
    pub fn fit(
        &self,
        at: &dyn Fn(f64) -> Vec<f64>,
        hints: &Hints,
        tolerance: f64,
        places: i32,
    ) -> Animated {
        self.fit_between(at, hints, tolerance, places, 0.0, self.clock.out_point())
    }

    /// Fits keyframes to `at` from frame `start` to frame `end`.
    pub fn fit_between(
        &self,
        at: &dyn Fn(f64) -> Vec<f64>,
        hints: &Hints,
        tolerance: f64,
        places: i32,
        start: f64,
        end: f64,
    ) -> Animated {
        let track = Track {
            at,
            tolerance,
            places,
        };
        fit::fit(&track, hints, start, end)
    }

    /// Fits a number property of `layer`, scaled by `factor`.
    pub fn fit_number(
        &self,
        layer: &NodeId,
        property: Property,
        hints: &Hints,
        factor: f64,
        tolerance: f64,
    ) -> Animated {
        let at = |frame: f64| vec![self.number(layer, property, frame) * factor];
        self.fit(&at, hints, tolerance, places_for(tolerance))
    }
}

/// Decimals that keep rounding well inside `tolerance`.
pub(crate) fn places_for(tolerance: f64) -> i32 {
    // Rounding moves a value by at most half the last decimal; keep that
    // under a tenth of the tolerance.
    let places = (-(tolerance / 5.0).log10()).ceil() as i32;
    places.clamp(0, 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_stays_inside_the_tolerance() {
        for tolerance in [tolerance::LENGTH, tolerance::ANGLE, tolerance::COLOR, 0.5] {
            let places = places_for(tolerance);
            assert!(0.5 * 10f64.powi(-places) <= tolerance / 10.0, "{tolerance}");
        }
        assert_eq!(places_for(tolerance::LENGTH), 2);
        assert_eq!(places_for(tolerance::COLOR), 4);
    }
}
