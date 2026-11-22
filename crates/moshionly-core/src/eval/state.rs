//! The state of one layer at one time: every animatable property resolved.

use super::preset::{Expansion, Pose, WHOLE_BOX, expand};
use super::textfx::TextStep;
use super::tween::{
    BoxState, NO_SHADOW, ResizeStep, ShadowStep, Span, StrokeState, StrokeStep, Tween, additive,
    replacing,
};
use crate::geometry::{Affine, Point, Rect};
use crate::model::{LayerProps, Node, NodeData, Paint, Shadow, Stroke};
use crate::text::find_number;

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
    /// What presets lay on top of the layer's own values. See
    /// [`super::preset`].
    pose_opacity: Pose<f64>,
    pose_scale: Pose<f64>,
    pose_offset: Pose<Point>,
    pose_rotation: Pose<f64>,
    pose_blur: Pose<f64>,
    pose_reveal: Pose<[f64; 4]>,
    /// `text_in` and `text_out`, for text layers.
    text: Vec<TextStep>,
    /// `counter`, for text layers.
    counters: Vec<Tween<f64>>,
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
            (NodeData::TextIn(a), Some(span)) => self.text.push(TextStep {
                span,
                entering: true,
                animation: a.clone(),
            }),
            (NodeData::TextOut(a), Some(span)) => self.text.push(TextStep {
                span,
                entering: false,
                animation: a.clone(),
            }),
            (NodeData::Counter(a), Some(span)) => {
                self.counters.push(Tween::new(span, a.from, a.to))
            }
            (preset, Some(span)) => {
                if let Some(expansion) = expand(preset) {
                    self.add_preset(span, expansion);
                }
            }
            _ => {}
        }
    }

    /// Lays a preset onto the pose channels.
    fn add_preset(&mut self, span: Span, expansion: Expansion) {
        let entrance = expansion.entrance;
        self.pose_opacity
            .lay(span, expansion.opacity, entrance, 1.0);
        self.pose_scale.lay(span, expansion.scale, entrance, 1.0);
        self.pose_offset
            .lay(span, expansion.offset, entrance, Point::ZERO);
        self.pose_rotation
            .lay(span, expansion.rotation, entrance, 0.0);
        self.pose_blur.lay(span, expansion.blur, entrance, 0.0);
        self.pose_reveal
            .lay(span, expansion.reveal, entrance, WHOLE_BOX);
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

    /// The content of a text layer at `time`: what is typed, with its first
    /// number replaced by the value its counters give it.
    ///
    /// A counter follows the replacing rule, with the number written in the
    /// text as the resting value. The number keeps its notation: decimals,
    /// thousands separators, sign and whatever stands around it.
    fn content(&self, node: &Node, time: f64) -> Option<String> {
        let NodeData::Text(text) = &node.data else {
            return None;
        };
        let counted = find_number(&text.content)
            .filter(|_| !self.counters.is_empty())
            .map(|number| {
                let value = replacing(&number.value, &self.counters, time);
                format!(
                    "{}{}{}",
                    &text.content[..number.start],
                    number.format.format(value),
                    &text.content[number.end..]
                )
            });
        Some(counted.unwrap_or_else(|| text.content.clone()))
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

        let pose_opacity = replacing(&1.0, &self.pose_opacity.steps, time).clamp(0.0, 1.0);
        let pose_scale = replacing(&1.0, &self.pose_scale.steps, time).max(0.0);
        let pose_offset = replacing(&Point::ZERO, &self.pose_offset.steps, time);
        let pose_rotation = replacing(&0.0, &self.pose_rotation.steps, time);
        let pose_blur = replacing(&0.0, &self.pose_blur.steps, time).max(0.0);
        let reveal =
            replacing(&WHOLE_BOX, &self.pose_reveal.steps, time).map(|edge| edge.clamp(0.0, 1.0));

        Some(LayerState {
            visible: self.visible(layer.hidden, time),
            rest: RestFrame::of(layer),
            offset: Point::new(offset.x + pose_offset.x, offset.y + pose_offset.y),
            width: size.width.max(0.0),
            height: size.height.max(0.0),
            shift: size.shift,
            rotation: layer.rotation + turn + pose_rotation,
            scale: layer.scale * multiplier * pose_scale,
            opacity: opacity / 100.0 * pose_opacity,
            blur: blur + pose_blur,
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
            reveal,
            content: self.content(node, time),
            text: self.text.clone(),
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
    /// Movement in the parent's coordinates, from `move` animations and
    /// sliding presets.
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
    /// The part of the box that reveal presets leave showing, as
    /// `[left, top, right, bottom]` fractions of it.
    pub reveal: [f64; 4],
    /// For a text layer: its content, with counters applied.
    pub content: Option<String>,
    /// For a text layer: its text animations, in the order they start.
    pub text: Vec<TextStep>,
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

    /// The region that reveal presets leave showing, in the layer's own
    /// coordinates, or `None` when they leave everything showing.
    ///
    /// Only the edge that is wiping cuts. The other sides reach far past
    /// the box, so that a stroke or a shadow outside it is not cut off.
    pub(crate) fn reveal_rect(&self) -> Option<Rect> {
        /// How far the sides that are not wiping reach past the box.
        const REACH: f64 = 100_000.0;
        if self.reveal == WHOLE_BOX {
            return None;
        }
        let [left, top, right, bottom] = self.reveal;
        let x0 = if left <= 0.0 {
            -REACH
        } else {
            left * self.width
        };
        let y0 = if top <= 0.0 {
            -REACH
        } else {
            top * self.height
        };
        let x1 = if right >= 1.0 {
            self.width + REACH
        } else {
            right * self.width
        };
        let y1 = if bottom >= 1.0 {
            self.height + REACH
        } else {
            bottom * self.height
        };
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    /// Whether anything of the layer can be seen.
    pub(crate) fn is_drawn(&self) -> bool {
        let [left, top, right, bottom] = self.reveal;
        self.visible && self.opacity > 0.0 && self.scale > 0.0 && right > left && bottom > top
    }
}
