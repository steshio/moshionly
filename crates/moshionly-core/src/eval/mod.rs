//! Evaluation: a document, an artboard and a time go in, a [`Scene`] comes
//! out.
//!
//! A frame is a pure function. Nothing depends on the previous frame, so
//! scrubbing, playback, export and screenshots all give the same result for
//! the same time.
//!
//! # How animations combine
//!
//! For one property of one layer, the animations are put in the order they
//! start (for equal starts, in timeline row order) and combined by one of
//! two rules.
//!
//! **Additive** (`move`, `rotate`): every animation contributes a delta that
//! is added to the resting value, so two moves add up. A missing `from` or
//! `to` is no delta. Before it starts an animation contributes nothing,
//! except the first one, whose `from` holds from time 0.
//!
//! **Replacing** (everything else): the value is the resting value until the
//! first animation starts, and from then on the most recently started
//! animation decides. A missing `from` is the value at the moment the
//! animation starts, so it picks up smoothly from an animation it
//! interrupts. A missing `to` is the resting value. If the first animation
//! has a `from`, that value also holds before it starts.
//!
//! `scale` values multiply the layer's resting scale. `opacity`, sizes,
//! colours, radii, blur and shadows are absolute.
//!
//! # Presets
//!
//! A preset (`fade_in`, `slide_out`, ...) is expanded while evaluating into
//! what it does to the layer, and that is laid on top of the layer's own
//! values instead of replacing them: its fade multiplies the opacity, its
//! scale multiplies the scale, its slide adds to the position, its turn adds
//! to the rotation, its blur adds to the blur, and a reveal clips. So a
//! preset and a property animation on the same layer both take effect.
//! Among themselves presets follow the replacing rule, and an entrance
//! clears what earlier presets left behind.
//!
//! # Text
//!
//! `text_in` and `text_out` cut the text into letters, words or lines and
//! play their effect on each piece, one starting `stagger` milliseconds
//! after the other, all together filling the time from `start` to `end`.
//! The scene lists the pieces as text units with their own transform,
//! opacity and blur. A `counter` replaces the first number in the text,
//! following the replacing rule with the written number as resting value.
//!
//! # Components
//!
//! An `instance` shows its main `component` as the component looks at the
//! instance's time minus its `time_offset`: what the component draws itself
//! (fill, stroke, corner radius, clip) and its layers, with every animation
//! that targets a layer inside the component. Those animations live in the
//! component's own artboard, which may be another one. Animations on the
//! component itself belong to where it is placed and are left out.
//! Before the offset has passed, the instance shows the component as it is
//! before its animations start. When the instance's box differs from the
//! component's, the content is scaled to fill it. A resize animation on the
//! instance changes its box the way it changes a group's: the component's
//! background, outline and clip follow the box, and the layers keep the
//! scale that fits the instance's resting box. Every scene node drawn for
//! an instance carries the instance's id, so the instance is one layer to
//! hit testing and selection.

mod preset;
mod state;
mod textfx;
mod tween;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use state::{Channels, LayerState};

use crate::document::Document;
use crate::geometry::{Affine, Point, Rect};
use crate::id::NodeId;
use crate::model::{
    AutoSize, CornerRadius, Family, Gradient, Instance, Node, NodeData, NodeType, Paint, Shadow,
    Stroke, TextLayer,
};
use crate::scene::{
    Clip, Content, Scene, SceneNode, ScenePaint, SceneStroke, TextContent, TextStyle,
};
use crate::text::{TextMeasurer, TextRequest};

/// Why a scene could not be evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvalError {
    /// No node has the id.
    #[error("there is no artboard with the id \"{0}\"")]
    UnknownArtboard(NodeId),
    /// The id belongs to something that is not an artboard.
    #[error(
        "{label} is not an artboard; a scene is evaluated for a whole artboard, here \"{artboard}\""
    )]
    NotAnArtboard {
        /// The node that was passed in.
        label: String,
        /// The artboard that node is in.
        artboard: NodeId,
    },
}

/// A property of a layer whose value over time [`value_at`] can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Property {
    /// Left edge of the box in the parent's coordinates, before the box is
    /// turned and scaled about its centre.
    X,
    /// Top edge of the box, likewise.
    Y,
    /// Width of the box.
    Width,
    /// Height of the box.
    Height,
    /// Rotation in degrees.
    Rotation,
    /// Scale, resting scale included.
    Scale,
    /// Opacity from 0 to 100.
    Opacity,
    /// Whether `show` and `hide` leave the layer visible.
    Visible,
    /// The fill.
    Fill,
    /// The stroke.
    Stroke,
    /// The corner radius.
    CornerRadius,
    /// The blur.
    Blur,
    /// The drop shadow.
    Shadow,
    /// The text of a text layer, with counters applied.
    Content,
}

impl Property {
    /// Every property.
    pub const ALL: [Property; 14] = [
        Property::X,
        Property::Y,
        Property::Width,
        Property::Height,
        Property::Rotation,
        Property::Scale,
        Property::Opacity,
        Property::Visible,
        Property::Fill,
        Property::Stroke,
        Property::CornerRadius,
        Property::Blur,
        Property::Shadow,
        Property::Content,
    ];

    /// The name of the property, which is the name of the layer field it
    /// starts from (`visible` being the opposite of `hidden`).
    pub fn as_str(self) -> &'static str {
        match self {
            Property::X => "x",
            Property::Y => "y",
            Property::Width => "width",
            Property::Height => "height",
            Property::Rotation => "rotation",
            Property::Scale => "scale",
            Property::Opacity => "opacity",
            Property::Visible => "visible",
            Property::Fill => "fill",
            Property::Stroke => "stroke",
            Property::CornerRadius => "corner_radius",
            Property::Blur => "blur",
            Property::Shadow => "shadow",
            Property::Content => "content",
        }
    }

    /// Looks a property up by name.
    pub fn parse(name: &str) -> Option<Property> {
        Self::ALL
            .into_iter()
            .find(|property| property.as_str() == name)
    }
}

/// The value of a [`Property`] at one time, in the units of the document.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// A number: pixels, degrees, a multiplier, or a percentage.
    Number(f64),
    /// Yes or no.
    Flag(bool),
    /// A fill, or none.
    Paint(Option<Paint>),
    /// A stroke, or none.
    Stroke(Option<Stroke>),
    /// A corner radius.
    CornerRadius(CornerRadius),
    /// A shadow, or none.
    Shadow(Option<Shadow>),
    /// Text.
    Text(String),
}

/// The length of an artboard's timeline in milliseconds. `None` when `id` is
/// not an artboard.
pub fn duration_of(document: &Document, artboard: &NodeId) -> Option<f64> {
    match &document.get(artboard)?.data {
        NodeData::Artboard(artboard) => Some(artboard.duration),
        _ => None,
    }
}

/// The animations of an artboard that are running at `time_ms`: those whose
/// span from `start` to `end` contains it, in timeline row order.
///
/// `show` and `hide` take no time, so they are only listed at exactly their
/// moment. Animation groups are folders and are never listed.
pub fn animations_active_at<'a>(
    document: &'a Document,
    artboard: &NodeId,
    time_ms: f64,
) -> Vec<&'a Node> {
    document
        .walk(artboard, Family::Animation)
        .map(|(_, node)| node)
        .filter(|node| {
            node.data
                .span()
                .is_some_and(|(start, end)| start <= time_ms && time_ms <= end)
        })
        .collect()
}

/// The value a property of a layer has at `time_ms`, with every animation
/// that targets the layer applied.
///
/// `None` when `layer` is not a layer, or the layer's type does not have the
/// property (a `corner_radius` on an ellipse, say).
///
/// For text layers whose box follows the text, `width`, `height`, `x` and
/// `y` are those of the stored box; the box in the scene is measured from
/// the text with the renderer's fonts.
pub fn value_at(
    document: &Document,
    layer: &NodeId,
    property: Property,
    time_ms: f64,
) -> Option<PropertyValue> {
    let node = document.get(layer)?;
    let animations = document.animations_targeting(layer);
    let state = Channels::of(animations).resolve(node, clean_time(time_ms))?;
    let ty = node.node_type();
    let has = |field: &str| {
        crate::schema::field_names(ty)
            .iter()
            .any(|name| name == field)
    };
    Some(match property {
        Property::X => PropertyValue::Number(state.origin().x),
        Property::Y => PropertyValue::Number(state.origin().y),
        Property::Width => PropertyValue::Number(state.width),
        Property::Height => PropertyValue::Number(state.height),
        Property::Rotation => PropertyValue::Number(state.rotation),
        Property::Scale => PropertyValue::Number(state.scale),
        Property::Opacity => PropertyValue::Number(state.opacity * 100.0),
        Property::Visible => PropertyValue::Flag(state.visible),
        Property::Blur => PropertyValue::Number(state.blur),
        Property::Shadow => PropertyValue::Shadow(state.shadow),
        Property::Content => PropertyValue::Text(state.content?),
        Property::Fill if has("fill") => PropertyValue::Paint(state.fill),
        Property::Stroke if has("stroke") => PropertyValue::Stroke(state.stroke),
        Property::CornerRadius if has("corner_radius") => {
            PropertyValue::CornerRadius(CornerRadius::from_radii(state.radii))
        }
        _ => return None,
    })
}

/// Evaluates an artboard at a time into a scene.
///
/// `time_ms` is in milliseconds from the start of the artboard. Times past
/// the artboard's duration are evaluated like any other; negative times
/// count as 0. `measurer` lays out text; use the renderer's so that the
/// scene matches what is drawn, or [`MonospaceMeasurer`](crate::text::MonospaceMeasurer)
/// when no fonts are at hand.
pub fn evaluate(
    document: &Document,
    artboard: &NodeId,
    time_ms: f64,
    measurer: &dyn TextMeasurer,
) -> Result<Scene, EvalError> {
    Evaluator::new(document, artboard, Some(clean_time(time_ms)), measurer)?.scene()
}

/// Evaluates an artboard as it is designed, with no animation applied: what
/// the editor shows while the design is being worked on.
pub fn evaluate_at_rest(
    document: &Document,
    artboard: &NodeId,
    measurer: &dyn TextMeasurer,
) -> Result<Scene, EvalError> {
    Evaluator::new(document, artboard, None, measurer)?.scene()
}

/// Negative and undefined times count as the start.
fn clean_time(time_ms: f64) -> f64 {
    if time_ms.is_nan() {
        0.0
    } else {
        time_ms.max(0.0)
    }
}

/// How many instances deep an instance may sit inside other components'
/// instances before its content is left out. The rules forbid a component
/// that shows itself, so this only guards against a broken document.
const MAX_INSTANCE_DEPTH: usize = 32;

/// The animations of each layer, by target, in the order they start.
type Targets<'a> = HashMap<&'a NodeId, Vec<&'a Node>>;

/// The animations of each artboard, gathered the first time an artboard is
/// needed: the evaluated one, and those that hold components shown by
/// instances.
struct Timelines<'a> {
    document: &'a Document,
    /// `false` when evaluating at rest: no animation applies.
    animate: bool,
    built: RefCell<HashMap<&'a NodeId, Rc<Targets<'a>>>>,
}

impl<'a> Timelines<'a> {
    fn of(&self, artboard: &'a NodeId) -> Rc<Targets<'a>> {
        if let Some(targets) = self.built.borrow().get(artboard) {
            return Rc::clone(targets);
        }
        let mut targets: Targets<'a> = HashMap::new();
        if self.animate {
            for (_, animation) in self.document.walk(artboard, Family::Animation) {
                if let Some(target) = animation.target() {
                    targets.entry(target).or_default().push(animation);
                }
            }
            // The walk is in row order and the sort is stable, so equal
            // starts keep their rows.
            let start = |node: &Node| node.data.span().map_or(0.0, |(start, _)| start);
            for list in targets.values_mut() {
                list.sort_by(|a, b| start(a).total_cmp(&start(b)));
            }
        }
        let targets = Rc::new(targets);
        self.built
            .borrow_mut()
            .insert(artboard, Rc::clone(&targets));
        targets
    }
}

struct Evaluator<'a> {
    document: &'a Document,
    artboard: &'a Node,
    time: f64,
    measurer: &'a dyn TextMeasurer,
    /// The animations of every layer of the artboard, in the order they
    /// start. Empty when evaluating at rest.
    animations: Rc<Targets<'a>>,
    /// Every artboard's animations, shared with the evaluators of the
    /// instances inside this one.
    timelines: Rc<Timelines<'a>>,
    /// How many instances deep this evaluator draws: 0 for the artboard.
    depth: usize,
}

impl<'a> Evaluator<'a> {
    fn new(
        document: &'a Document,
        artboard: &NodeId,
        time: Option<f64>,
        measurer: &'a dyn TextMeasurer,
    ) -> Result<Self, EvalError> {
        let node = document
            .get(artboard)
            .ok_or_else(|| EvalError::UnknownArtboard(artboard.clone()))?;
        if node.node_type() != NodeType::Artboard {
            return Err(match document.artboard_of(artboard) {
                Some(owner) => EvalError::NotAnArtboard {
                    label: node.label(),
                    artboard: owner.id.clone(),
                },
                None => EvalError::UnknownArtboard(artboard.clone()),
            });
        }
        let timelines = Rc::new(Timelines {
            document,
            animate: time.is_some(),
            built: RefCell::default(),
        });
        Ok(Self {
            document,
            artboard: node,
            time: time.unwrap_or(0.0),
            measurer,
            animations: timelines.of(&node.id),
            timelines,
            depth: 0,
        })
    }

    fn scene(&self) -> Result<Scene, EvalError> {
        let (width, height, background) = match self.state(self.artboard) {
            Some(state) => (
                state.width,
                state.height,
                paint(state.fill.as_ref(), state.width, state.height),
            ),
            None => (0.0, 0.0, None),
        };
        Ok(Scene {
            artboard: self.artboard.id.clone(),
            time_ms: self.time,
            width,
            height,
            background,
            nodes: self.children(&self.artboard.id),
        })
    }

    fn state(&self, node: &Node) -> Option<LayerState> {
        let animations = self.animations.get(&node.id).into_iter().flatten().copied();
        Channels::of(animations).resolve(node, self.time)
    }

    fn children(&self, parent: &NodeId) -> Vec<SceneNode> {
        self.document
            .child_nodes(parent, Family::Layer)
            .filter_map(|child| self.layer(child))
            .collect()
    }

    /// The scene node of one layer, or `None` when nothing of it shows.
    fn layer(&self, node: &Node) -> Option<SceneNode> {
        let mut state = self.state(node)?;
        if !state.is_drawn() {
            return None;
        }
        let mut mask = None;
        let content = match &node.data {
            NodeData::Group(group) => self.frame(node, group.clip, &state),
            NodeData::Component(component) => self.frame(node, component.clip, &state),
            NodeData::Instance(instance) => self.instance(node, instance, &state),
            NodeData::MaskGroup(_) => {
                let mut layers = self.document.child_nodes(&node.id, Family::Layer);
                // Without a mask, or with one that cannot be seen, nothing
                // of the group shows.
                mask = Some(Box::new(self.layer(layers.next()?)?));
                Content::Group {
                    children: layers.filter_map(|child| self.layer(child)).collect(),
                }
            }
            NodeData::Rect(_) => Content::Rect {
                radii: fitted(state.radii, state.width, state.height),
                fill: paint(state.fill.as_ref(), state.width, state.height),
                stroke: stroke(state.stroke.as_ref(), state.width, state.height),
            },
            NodeData::Ellipse(ellipse) => Content::Ellipse {
                start_angle: ellipse.start_angle,
                sweep: ellipse.sweep,
                inner_radius: ellipse.inner_radius,
                fill: paint(state.fill.as_ref(), state.width, state.height),
                stroke: stroke(state.stroke.as_ref(), state.width, state.height),
            },
            NodeData::Star(star) => Content::Star {
                points: star.points,
                inner_radius: star.inner_radius,
                fill: paint(state.fill.as_ref(), state.width, state.height),
                stroke: stroke(state.stroke.as_ref(), state.width, state.height),
            },
            NodeData::Path(path) => Content::Path {
                path: path.path.scaled(state.width, state.height),
                fill: paint(state.fill.as_ref(), state.width, state.height),
                stroke: stroke(state.stroke.as_ref(), state.width, state.height),
            },
            NodeData::Text(text) => self.text(text, &mut state),
            NodeData::Image(image) => Content::Image {
                source: image.source.clone(),
                fit: image.fit,
                radii: fitted(state.radii, state.width, state.height),
                stroke: stroke(state.stroke.as_ref(), state.width, state.height),
            },
            // Artboards are roots and never nested; animations are not drawn.
            _ => return None,
        };
        let layer = node.layer()?;
        Some(SceneNode {
            id: node.id.clone(),
            transform: state.transform(),
            width: state.width,
            height: state.height,
            opacity: state.opacity,
            blend_mode: layer.blend_mode,
            blur: state.blur,
            shadow: state.shadow,
            clip: state.reveal_rect().map(|rect| Clip {
                rect,
                radii: [0.0; 4],
            }),
            mask,
            content,
        })
    }

    /// What a group or a component draws: its background, its children
    /// (clipped to the box when `clip` is set) and its outline.
    fn frame(&self, node: &Node, clip: bool, state: &LayerState) -> Content {
        self.frame_around(node, clip, state, self.children(&node.id))
    }

    /// [`frame`](Self::frame) around `inside` in place of the node's
    /// children.
    fn frame_around(
        &self,
        node: &Node,
        clip: bool,
        state: &LayerState,
        inside: Vec<SceneNode>,
    ) -> Content {
        let shape = |fill: Option<ScenePaint>, stroke: Option<SceneStroke>| {
            let content = Content::Rect {
                radii: fitted(state.radii, state.width, state.height),
                fill,
                stroke,
            };
            plain(&node.id, state, content)
        };
        let mut children = Vec::new();
        if let Some(fill) = paint(state.fill.as_ref(), state.width, state.height) {
            children.push(shape(Some(fill), None));
        }
        if clip {
            // The clip sits on a node of its own so that it cuts the
            // children but not the group's own shadow.
            let mut holder = plain(&node.id, state, Content::Group { children: inside });
            holder.clip = Some(Clip {
                rect: Rect::new(0.0, 0.0, state.width, state.height),
                radii: fitted(state.radii, state.width, state.height),
            });
            children.push(holder);
        } else {
            children.extend(inside);
        }
        if let Some(stroke) = stroke(state.stroke.as_ref(), state.width, state.height) {
            children.push(shape(None, Some(stroke)));
        }
        Content::Group { children }
    }

    /// What an instance shows: its component's look over the instance's
    /// current box and its layers scaled to the instance's resting box, at
    /// the instance's time minus its offset, with every scene node carrying
    /// the instance's id. Nothing when the component is missing or
    /// instances nest too deep.
    fn instance(&self, node: &Node, instance: &Instance, state: &LayerState) -> Content {
        let nothing = Content::Group {
            children: Vec::new(),
        };
        let Some(main) = self.document.get(&instance.component) else {
            return nothing;
        };
        let (NodeData::Component(component), Some(artboard)) =
            (&main.data, self.document.artboard_of(&main.id))
        else {
            return nothing;
        };
        if self.depth >= MAX_INSTANCE_DEPTH {
            return nothing;
        }
        let inner = Evaluator {
            document: self.document,
            artboard,
            // Not cleaned up to 0: before the offset has passed, the
            // component shows as it is before its animations start.
            time: self.time - instance.time_offset,
            measurer: self.measurer,
            animations: self.timelines.of(&artboard.id),
            timelines: Rc::clone(&self.timelines),
            depth: self.depth + 1,
        };
        // Animations on the component itself belong to where it is placed,
        // so its own look is the one at rest.
        let Some(mut look) = Channels::default().resolve(main, inner.time) else {
            return nothing;
        };
        let ratio = |size: f64, main: f64| {
            if main > 0.0 && (size / main).is_finite() {
                size / main
            } else {
                1.0
            }
        };
        // The content fits the instance's resting box. A resize animation
        // on the instance changes its box the way it changes a group's:
        // the background, outline and clip follow it, the layers do not.
        let (sx, sy) = (
            ratio(instance.layer.width, look.width),
            ratio(instance.layer.height, look.height),
        );
        if sx == 0.0 || sy == 0.0 {
            // A resting box with no width or no height squeezes the layers
            // flat, as in the group detaching the instance makes, and the
            // rest cannot be scaled back up from it: the background,
            // outline and clip take the instance's current box as it is.
            // With no size at all, the layers, outline and corners shrink
            // to nothing.
            let mut layers = Vec::new();
            if sx == 0.0 && sy == 0.0 {
                if let Some(stroke) = &mut look.stroke {
                    stroke.width = 0.0;
                }
                look.radii = [0.0; 4];
            } else {
                let children = inner.children(&main.id);
                let mut squeezed = plain(&node.id, &look, Content::Group { children });
                squeezed.transform = Affine::scale(sx, sy);
                layers.push(squeezed);
            }
            look.width = state.width;
            look.height = state.height;
            let mut content = inner.frame_around(main, component.clip, &look, layers);
            relabel(&mut content, &node.id);
            return content;
        }
        look.width = state.width / sx;
        look.height = state.height / sy;
        let mut content = inner.frame(main, component.clip, &look);
        relabel(&mut content, &node.id);
        if sx == 1.0 && sy == 1.0 {
            return content;
        }
        let mut holder = plain(&node.id, &look, content);
        holder.transform = Affine::scale(sx, sy);
        Content::Group {
            children: vec![holder],
        }
    }

    /// Lays out a text layer. This also settles the size of its box, which
    /// for auto-sized text comes from the text.
    fn text(&self, text: &TextLayer, state: &mut LayerState) -> Content {
        let typed = state.content.take().unwrap_or_else(|| text.content.clone());
        let content = text.case.apply(&typed);
        let style = TextStyle {
            font: text.font.clone(),
            font_size: text.font_size,
            line_height: text.font_size * text.line_height / 100.0,
            letter_spacing: text.font_size * text.letter_spacing / 100.0,
            align: text.align,
        };
        let wrap_width = match text.auto_size {
            AutoSize::WidthAndHeight => None,
            AutoSize::Height | AutoSize::Fixed => Some(state.width),
        };
        let layout = self.measurer.measure(&TextRequest {
            text: &content,
            style: &style,
            wrap_width,
        });
        let measured = |value: f64, fallback: f64| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                fallback
            }
        };
        let text_width = measured(layout.width, state.width);
        let text_height = measured(layout.height, state.height);
        // An auto-sized box grows from the side the text is aligned to.
        let anchor = (text.align.factor(), text.vertical_align.factor());
        match text.auto_size {
            AutoSize::WidthAndHeight => state.resize(text_width, text_height, anchor),
            AutoSize::Height => state.resize(state.width, text_height, anchor),
            AutoSize::Fixed => {}
        }
        let offset_y = (state.height - text_height) * text.vertical_align.factor();
        let units = textfx::units(
            &content,
            &layout,
            &state.text,
            self.time,
            style.line_height,
            offset_y,
        );
        Content::Text(TextContent {
            offset_y,
            fill: paint(state.fill.as_ref(), state.width, state.height),
            stroke: stroke(state.stroke.as_ref(), state.width, state.height),
            text: content,
            style,
            wrap_width,
            units,
        })
    }
}

/// A node that only draws: no transform or effect of its own. Used for the
/// extra nodes a group needs for its background, clip and outline.
fn plain(id: &NodeId, state: &LayerState, content: Content) -> SceneNode {
    SceneNode {
        id: id.clone(),
        transform: Affine::IDENTITY,
        width: state.width,
        height: state.height,
        opacity: 1.0,
        blend_mode: crate::model::BlendMode::Normal,
        blur: 0.0,
        shadow: None,
        clip: None,
        mask: None,
        content,
    }
}

/// Gives every scene node in `content`, masks included, the id `id`.
fn relabel(content: &mut Content, id: &NodeId) {
    fn relabel_node(node: &mut SceneNode, id: &NodeId) {
        node.id = id.clone();
        if let Some(mask) = node.mask.as_deref_mut() {
            relabel_node(mask, id);
        }
        relabel(&mut node.content, id);
    }
    if let Content::Group { children } = content {
        for child in children {
            relabel_node(child, id);
        }
    }
}

/// Corner radii limited to half the shorter side of the box.
fn fitted(radii: [f64; 4], width: f64, height: f64) -> [f64; 4] {
    let limit = width.min(height) / 2.0;
    radii.map(|radius| radius.clamp(0.0, limit.max(0.0)))
}

/// Resolves a paint for a `width` by `height` box: gradient geometry goes
/// from fractions of the box to pixels. A paint that cannot be seen gives
/// `None`.
fn paint(paint: Option<&Paint>, width: f64, height: f64) -> Option<ScenePaint> {
    let point = |unit: [f64; 2]| Point::new(unit[0] * width, unit[1] * height);
    Some(match paint? {
        Paint::Solid(color) if color.a == 0 => return None,
        Paint::Solid(color) => ScenePaint::Solid { color: *color },
        Paint::Gradient(gradient) if gradient.stops().iter().all(|stop| stop.color.a == 0) => {
            return None;
        }
        Paint::Gradient(Gradient::LinearGradient(linear)) => ScenePaint::LinearGradient {
            start: point(linear.start),
            end: point(linear.end),
            stops: linear.stops.clone(),
        },
        Paint::Gradient(Gradient::RadialGradient(radial)) => ScenePaint::RadialGradient {
            center: point(radial.center),
            radius_x: radial.radius[0] * width,
            radius_y: radial.radius[1] * height,
            rotation: radial.rotation,
            stops: radial.stops.clone(),
        },
    })
}

/// Resolves a stroke for a `width` by `height` box.
fn stroke(stroke: Option<&Stroke>, width: f64, height: f64) -> Option<SceneStroke> {
    let stroke = stroke.filter(|stroke| stroke.width > 0.0)?;
    Some(SceneStroke {
        paint: paint(Some(&stroke.paint), width, height)?,
        width: stroke.width,
        position: stroke.position,
        cap: stroke.cap,
        join: stroke.join,
    })
}

#[cfg(test)]
mod tests;
