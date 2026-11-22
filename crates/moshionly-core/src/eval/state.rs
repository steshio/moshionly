//! The state of one layer at one time: every animatable property resolved.

use super::tween::{
    BoxState, NO_SHADOW, ResizeStep, ShadowStep, Span, StrokeState, StrokeStep, Tween, additive,
    replacing,
};
use crate::geometry::{Affine, Point};
use crate::model::{LayerProps, Node, NodeData, Paint, Shadow, Stroke};

/// The animations of one layer, sorted into the properties they act on.
/// Each list is in the order the animations start.
#[derive(Debug, Default)]
pub(crate) struct Channels {
    moves: Vec<Tween<Point>>,
    rotations: Vec<Tween<f64>>,
    scales: Vec<Tween<f64>>,
    opacities: Vec<Tween<f64>>,
    sizes: Vec<ResizeStep>,
    fills: Vec<Tween<Option<Paint>>>,
    strokes: Vec<StrokeStep>,
    radii: Vec<Tween<[f64; 4]>>,
    blurs: Vec<Tween<f64>>,
    shadows: Vec<ShadowStep>,
    /// `(time, becomes visible)` for every `show` and `hide`.
    instants: Vec<(f64, bool)>,
}

impl Channels {
    /// Sorts a layer's animations, given in the order they start, into
    /// channels.
    pub(crate) fn of<'a>(animations: impl IntoIterator<Item = &'a Node>) -> Self {
        let mut channels = Channels::default();
        for animation in animations {
            channels.add(&animation.data);
        }
        channels
    }

    fn add(&mut self, animation: &NodeData) {
        let span = animation.timing().map(Span::from);
        match (animation, span) {
            (NodeData::Move(a), Some(span)) => {
                let offset = |offset: crate::model::Offset| Point::new(offset.x, offset.y);
                self.moves
                    .push(Tween::new(span, a.from.map(offset), a.to.map(offset)));
            }
            (NodeData::Rotate(a), Some(span)) => {
                self.rotations.push(Tween::new(span, a.from, a.to))
            }
            (NodeData::Scale(a), Some(span)) => self.scales.push(Tween::new(span, a.from, a.to)),
            (NodeData::Opacity(a), Some(span)) => {
                self.opacities.push(Tween::new(span, a.from, a.to))
            }
            (NodeData::Resize(a), Some(span)) => self.sizes.push(ResizeStep {
                span,
                from: a.from,
                to: a.to,
                anchor: a.anchor.factors(),
            }),
            (NodeData::Fill(a), Some(span)) => self.fills.push(Tween::new(
                span,
                a.from.clone().map(Some),
                a.to.clone().map(Some),
            )),
            (NodeData::Stroke(a), Some(span)) => self.strokes.push(StrokeStep {
                span,
                from: a.from.clone(),
                to: a.to.clone(),
            }),
            (NodeData::CornerRadius(a), Some(span)) => self.radii.push(Tween::new(
                span,
                a.from.map(|radius| radius.radii()),
                a.to.map(|radius| radius.radii()),
            )),
            (NodeData::Blur(a), Some(span)) => self.blurs.push(Tween::new(span, a.from, a.to)),
            (NodeData::Shadow(a), Some(span)) => self.shadows.push(ShadowStep {
                span,
                from: a.from,
                to: a.to,
            }),
            (NodeData::Show(a), _) => self.instants.push((a.start, true)),
            (NodeData::Hide(a), _) => self.instants.push((a.start, false)),
            _ => {}
        }
    }

    /// Whether the layer is visible at `time`.
    ///
    /// Without `show` or `hide` that is the layer's own `hidden`. With them,
    /// the most recent one decides, and before the first one the layer is in
    /// the opposite state: hidden until its first `show`, visible until its
    /// first `hide`.
    fn visible(&self, hidden: bool, time: f64) -> bool {
        let Some((_, first)) = self.instants.first() else {
            return !hidden;
        };
        self.instants
            .iter()
            .take_while(|(at, _)| *at <= time)
            .last()
            .map_or(!first, |(_, visible)| *visible)
    }

    /// Resolves every property of `node` at `time`.
    pub(crate) fn resolve(&self, node: &Node, time: f64) -> Option<LayerState> {
        let layer = node.layer()?;
        let offset = additive(&Point::ZERO, &self.moves, time, |a, b| {
            Point::new(a.x + b.x, a.y + b.y)
        });
        let turn = additive(&0.0, &self.rotations, time, |a, b| a + b);
        let multiplier = replacing(&1.0, &self.scales, time).max(0.0);
        let opacity = replacing(&layer.opacity, &self.opacities, time).clamp(0.0, 100.0);
        let size = replacing(
            &BoxState::at_rest(layer.width, layer.height),
            &self.sizes,
            time,
        );
        let fill = replacing(&node.data.fill().cloned(), &self.fills, time);
        let rest_stroke = node.data.stroke();
        let stroke = replacing(
            &StrokeState {
                paint: rest_stroke.map(|stroke| stroke.paint.clone()),
                width: rest_stroke.map_or(0.0, |stroke| stroke.width),
            },
            &self.strokes,
            time,
        );
        let radii = node
            .data
            .corner_radius()
            .map(|rest| replacing(&rest.radii(), &self.radii, time).map(|radius| radius.max(0.0)));
        let blur = replacing(&layer.blur, &self.blurs, time).max(0.0);
        let shadow = replacing(&layer.shadow.unwrap_or(NO_SHADOW), &self.shadows, time);

        Some(LayerState {
            visible: self.visible(layer.hidden, time),
            rest: RestFrame::of(layer),
            offset,
            width: size.width.max(0.0),
            height: size.height.max(0.0),
            shift: size.shift,
            rotation: layer.rotation + turn,
            scale: layer.scale * multiplier,
            opacity: opacity / 100.0,
            blur,
            fill,
            stroke: stroke
                .paint
                .filter(|_| stroke.width > 0.0)
                .map(|paint| Stroke {
                    paint,
                    width: stroke.width,
                    ..rest_stroke.cloned().unwrap_or_default()
                }),
            radii: radii.unwrap_or_default(),
            shadow: (shadow.color.a > 0).then_some(shadow),
        })
    }
}

/// What the placement of an animated layer is measured from: the layer at
/// rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RestFrame {
    /// Centre of the resting box in the parent's coordinates.
    center: Point,
    rotation: f64,
    scale: f64,
}

impl RestFrame {
    fn of(layer: &LayerProps) -> Self {
        Self {
            center: layer.center(),
            rotation: layer.rotation,
            scale: layer.scale,
        }
    }
}

/// Every animatable property of a layer at one time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LayerState {
    pub visible: bool,
    rest: RestFrame,
    /// Movement in the parent's coordinates, from `move` animations.
    pub offset: Point,
    pub width: f64,
    pub height: f64,
    /// How far the centre of the box has moved, in the layer's own axes, to
    /// keep the anchors of resizes (and the alignment of auto-sized text) in
    /// place.
    pub shift: Point,
    /// In degrees, resting rotation included.
    pub rotation: f64,
    /// Resting scale included.
    pub scale: f64,
    /// From 0 to 1.
    pub opacity: f64,
    pub blur: f64,
    pub fill: Option<Paint>,
    /// `None` when there is nothing to draw: no paint, or no width.
    pub stroke: Option<Stroke>,
    /// Top-left, top-right, bottom-right, bottom-left; zeros for layer types
    /// without corners.
    pub radii: [f64; 4],
    pub shadow: Option<Shadow>,
}

impl LayerState {
    /// Changes the size of the box while the point at `anchor` (as fractions
    /// of the box) stays where it is.
    pub(crate) fn resize(&mut self, width: f64, height: f64, anchor: (f64, f64)) {
        let resized = BoxState {
            width: self.width,
            height: self.height,
            shift: self.shift,
        }
        .resized(width, height, anchor);
        (self.width, self.height, self.shift) = (resized.width, resized.height, resized.shift);
    }

    /// The centre of the box in the parent's coordinates.
    ///
    /// The shift that keeps resize anchors in place is measured along the
    /// layer's resting axes, so the anchored point of a turned or scaled
    /// layer stays put too.
    pub(crate) fn center(&self) -> Point {
        let along_rest_axes = (Affine::rotate(self.rest.rotation)
            * Affine::scale(self.rest.scale, self.rest.scale))
        .apply_vector(self.shift);
        Point::new(
            self.rest.center.x + along_rest_axes.x + self.offset.x,
            self.rest.center.y + along_rest_axes.y + self.offset.y,
        )
    }

    /// The top-left corner the box has before it is turned and scaled about
    /// its centre: the `x` and `y` the layer would need at rest to sit here.
    pub(crate) fn origin(&self) -> Point {
        let center = self.center();
        Point::new(center.x - self.width / 2.0, center.y - self.height / 2.0)
    }

    /// The transform from the layer's own coordinates to its parent's.
    pub(crate) fn transform(&self) -> Affine {
        let origin = self.origin();
        Affine::layer(
            origin.x,
            origin.y,
            self.width,
            self.height,
            self.rotation,
            self.scale,
        )
    }

    /// Whether anything of the layer can be seen.
    pub(crate) fn is_drawn(&self) -> bool {
        self.visible && self.opacity > 0.0 && self.scale > 0.0
    }
}
