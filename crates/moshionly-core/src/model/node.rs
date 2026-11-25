//! Nodes: the flat building blocks of a document.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::animation::{
    AnimationGroup, AnimationKind, BlurAnimation, BlurPreset, CornerRadiusAnimation,
    CounterAnimation, FadePreset, FillAnimation, GrowIn, GrowOut, InstantAnimation, MoveAnimation,
    OpacityAnimation, ResizeAnimation, RevealPreset, RotateAnimation, ScaleAnimation,
    ShadowAnimation, ShrinkIn, ShrinkOut, SlidePreset, SpinPreset, StrokeAnimation, TextAnimation,
    Timing,
};
use super::layer::{
    Artboard, Component, EllipseLayer, Group, ImageLayer, Instance, LayerProps, MaskGroup,
    PathLayer, RectLayer, StarLayer, TextLayer,
};
use super::paint::{CornerRadius, Paint, Stroke};
use crate::defaults;
use crate::easing::Easing;
use crate::id::NodeId;
use crate::json;

/// One node of a document: a layer or an animation.
///
/// A document is a flat list of nodes. The tree is assembled from `parent`,
/// and siblings are ordered by `index`.
///
/// ```json
/// { "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card",
///   "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#0D99FF", "corner_radius": 32 }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Node {
    /// Unique id of the node within the document.
    pub id: NodeId,
    /// Id of the parent node, or `null` for an artboard.
    #[serde(default)]
    pub parent: Option<NodeId>,
    /// Fractional index: siblings of the same family sort by it using plain
    /// string comparison. For layers a larger index is drawn later, in front.
    pub index: String,
    /// The type of the node and its properties.
    #[serde(flatten)]
    pub data: NodeData,
}

/// The type of a node together with that type's properties.
///
/// In files the variant is chosen by the `type` field, in snake case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeData {
    /// A root frame with its own timeline.
    Artboard(Artboard),
    /// A box that holds other layers.
    Group(Group),
    /// A group whose first child masks the rest.
    MaskGroup(MaskGroup),
    /// A rectangle.
    Rect(RectLayer),
    /// An ellipse, arc or ring.
    Ellipse(EllipseLayer),
    /// A star or polygon.
    Star(StarLayer),
    /// A pen-drawn shape.
    Path(PathLayer),
    /// A block of text.
    Text(TextLayer),
    /// A bitmap image.
    Image(ImageLayer),
    /// A main component, whose look and content instances show.
    Component(Component),
    /// A copy of a main component.
    Instance(Instance),

    /// Shifts a layer from its resting position.
    Move(MoveAnimation),
    /// Turns a layer.
    Rotate(RotateAnimation),
    /// Scales a layer.
    Scale(ScaleAnimation),
    /// Fades a layer.
    Opacity(OpacityAnimation),
    /// Changes the size of a layer's box.
    Resize(ResizeAnimation),
    /// Changes a layer's fill.
    Fill(FillAnimation),
    /// Changes a layer's stroke.
    Stroke(StrokeAnimation),
    /// Changes a layer's corner rounding.
    CornerRadius(CornerRadiusAnimation),
    /// Changes a layer's blur.
    Blur(BlurAnimation),
    /// Changes a layer's drop shadow.
    Shadow(ShadowAnimation),

    /// Makes a layer visible at one moment.
    Show(InstantAnimation),
    /// Makes a layer invisible at one moment.
    Hide(InstantAnimation),

    /// Fades a layer in.
    FadeIn(FadePreset),
    /// Fades a layer out.
    FadeOut(FadePreset),
    /// Slides a layer into place.
    SlideIn(SlidePreset),
    /// Slides a layer away.
    SlideOut(SlidePreset),
    /// Grows a layer into place.
    GrowIn(GrowIn),
    /// Grows a layer away.
    GrowOut(GrowOut),
    /// Shrinks a layer into place.
    ShrinkIn(ShrinkIn),
    /// Shrinks a layer away.
    ShrinkOut(ShrinkOut),
    /// Spins a layer into place.
    SpinIn(SpinPreset),
    /// Spins a layer away.
    SpinOut(SpinPreset),
    /// Brings a layer into focus.
    BlurIn(BlurPreset),
    /// Takes a layer out of focus.
    BlurOut(BlurPreset),
    /// Wipes a layer into view.
    RevealIn(RevealPreset),
    /// Wipes a layer out of view.
    RevealOut(RevealPreset),

    /// Brings text in piece by piece.
    TextIn(TextAnimation),
    /// Takes text out piece by piece.
    TextOut(TextAnimation),
    /// Counts a number in a text layer.
    Counter(CounterAnimation),

    /// A folder of animations on the timeline.
    AnimationGroup(AnimationGroup),
}

/// The two families of nodes. Siblings are ordered within their family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Things that are drawn.
    Layer,
    /// Things that change layers over time.
    Animation,
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Family::Layer => "layer",
            Family::Animation => "animation",
        })
    }
}

/// Declares [`NodeType`] with, for every type, its name in files, its kind
/// (`layer`, or the kind of animation) and a one-line description.
macro_rules! node_types {
    ($($variant:ident, $name:literal, $kind:tt, $description:literal;)*) => {
        /// Every node type, without its properties.
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
            JsonSchema,
        )]
        #[serde(rename_all = "snake_case")]
        pub enum NodeType {
            $(#[doc = $description] $variant,)*
        }

        impl NodeType {
            /// Every node type: layers first, then animations.
            pub const ALL: &'static [NodeType] = &[$(NodeType::$variant,)*];

            /// The name used in the `type` field, for example `"mask_group"`.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(NodeType::$variant => $name,)*
                }
            }

            /// One line saying what the type is, for pickers and AI prompts.
            pub fn description(self) -> &'static str {
                match self {
                    $(NodeType::$variant => $description,)*
                }
            }

            /// The kind of animation, or `None` for a layer type.
            pub fn animation_kind(self) -> Option<AnimationKind> {
                match self {
                    $(NodeType::$variant => node_types!(@kind $kind),)*
                }
            }
        }
    };
    (@kind layer) => { None };
    (@kind property) => { Some(AnimationKind::Property) };
    (@kind instant) => { Some(AnimationKind::Instant) };
    (@kind preset) => { Some(AnimationKind::Preset) };
    (@kind text) => { Some(AnimationKind::Text) };
    (@kind structure) => { Some(AnimationKind::Structure) };
}

node_types! {
    Artboard, "artboard", layer, "A root frame with a size, a background and its own timeline. Everything else lives inside one.";
    Group, "group", layer, "A box that holds other layers and moves them together; can clip them and have its own fill, stroke and rounded corners.";
    MaskGroup, "mask_group", layer, "A group whose first child (lowest index) is an alpha mask for the other children.";
    Rect, "rect", layer, "A rectangle with optional rounded corners.";
    Ellipse, "ellipse", layer, "An ellipse; can be cut down to an arc, a pie slice or a ring.";
    Star, "star", layer, "A star with a chosen number of points; an inner radius of 1 makes a polygon.";
    Path, "path", layer, "A pen-drawn shape made of straight and curved segments.";
    Text, "text", layer, "A block of text in one font and style.";
    Image, "image", layer, "A bitmap image from a file or a data URI.";
    Component, "component", layer, "A main component: a group whose look, layers and their animations every instance of it shows; edit it and every instance changes.";
    Instance, "instance", layer, "A copy of a main component: shows its look, layers and animations, time_offset milliseconds later; holds no layers of its own.";

    Move, "move", property, "Shifts a layer by an offset from its resting position. Additive: several moves add up.";
    Rotate, "rotate", property, "Turns a layer by an angle added to its resting rotation. Additive: several rotations add up.";
    Scale, "scale", property, "Scales a layer by a multiplier of its resting scale.";
    Opacity, "opacity", property, "Fades a layer to an opacity from 0 to 100.";
    Resize, "resize", property, "Changes the width and height of a layer's box while one anchor point stays fixed.";
    Fill, "fill", property, "Changes a layer's fill, or an artboard's background, to another colour or gradient.";
    Stroke, "stroke", property, "Changes the paint and width of a layer's stroke.";
    CornerRadius, "corner_radius", property, "Changes the corner rounding of a rect, group or image.";
    Blur, "blur", property, "Changes how blurred a layer is.";
    Shadow, "shadow", property, "Changes a layer's drop shadow: colour, offset and blur.";

    Show, "show", instant, "Makes a layer visible at one moment; the layer is hidden before the first show.";
    Hide, "hide", instant, "Makes a layer invisible at one moment.";

    FadeIn, "fade_in", preset, "The layer fades in from invisible; it stays invisible until this starts.";
    FadeOut, "fade_out", preset, "The layer fades out to invisible.";
    SlideIn, "slide_in", preset, "The layer travels into its resting position from a distance, fading in.";
    SlideOut, "slide_out", preset, "The layer travels away from its resting position, fading out.";
    GrowIn, "grow_in", preset, "The layer appears while growing from a smaller size.";
    GrowOut, "grow_out", preset, "The layer disappears while growing to a larger size.";
    ShrinkIn, "shrink_in", preset, "The layer appears while shrinking from a larger size.";
    ShrinkOut, "shrink_out", preset, "The layer disappears while shrinking to a smaller size.";
    SpinIn, "spin_in", preset, "The layer turns into place while growing from nothing.";
    SpinOut, "spin_out", preset, "The layer turns away while shrinking to nothing.";
    BlurIn, "blur_in", preset, "The layer fades in while coming into focus.";
    BlurOut, "blur_out", preset, "The layer fades out while going out of focus.";
    RevealIn, "reveal_in", preset, "A straight edge wipes across the layer and uncovers it.";
    RevealOut, "reveal_out", preset, "A straight edge wipes across the layer and covers it.";

    TextIn, "text_in", text, "Text appears letter by letter, word by word or line by line, each with a fade, slide, scale or blur.";
    TextOut, "text_out", text, "Text disappears letter by letter, word by word or line by line.";
    Counter, "counter", text, "Counts the first number in a text layer up or down, keeping the text around it.";

    AnimationGroup, "animation_group", structure, "A folder that organises animations on the timeline; it does not change playback.";
}

impl NodeType {
    /// Looks a type up by the name used in the `type` field.
    pub fn parse(name: &str) -> Option<NodeType> {
        Self::ALL.iter().copied().find(|ty| ty.as_str() == name)
    }

    /// Whether nodes of this type are layers or animations.
    pub fn family(self) -> Family {
        match self.animation_kind() {
            None => Family::Layer,
            Some(_) => Family::Animation,
        }
    }

    /// Whether layers can be placed inside a node of this type.
    pub fn holds_layers(self) -> bool {
        matches!(
            self,
            NodeType::Artboard | NodeType::Group | NodeType::MaskGroup | NodeType::Component
        )
    }

    /// Whether animations can be placed inside a node of this type.
    pub fn holds_animations(self) -> bool {
        matches!(self, NodeType::Artboard | NodeType::AnimationGroup)
    }

    /// Whether this is a preset whose name ends in `_in`: an entrance.
    pub fn is_entrance(self) -> bool {
        self.animation_kind() == Some(AnimationKind::Preset) && self.as_str().ends_with("_in")
    }

    /// Whether this is a preset whose name ends in `_out`: an exit.
    pub fn is_exit(self) -> bool {
        self.animation_kind() == Some(AnimationKind::Preset) && self.as_str().ends_with("_out")
    }
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl NodeData {
    /// The type of the node.
    pub fn node_type(&self) -> NodeType {
        match self {
            NodeData::Artboard(_) => NodeType::Artboard,
            NodeData::Group(_) => NodeType::Group,
            NodeData::MaskGroup(_) => NodeType::MaskGroup,
            NodeData::Rect(_) => NodeType::Rect,
            NodeData::Ellipse(_) => NodeType::Ellipse,
            NodeData::Star(_) => NodeType::Star,
            NodeData::Path(_) => NodeType::Path,
            NodeData::Text(_) => NodeType::Text,
            NodeData::Image(_) => NodeType::Image,
            NodeData::Component(_) => NodeType::Component,
            NodeData::Instance(_) => NodeType::Instance,
            NodeData::Move(_) => NodeType::Move,
            NodeData::Rotate(_) => NodeType::Rotate,
            NodeData::Scale(_) => NodeType::Scale,
            NodeData::Opacity(_) => NodeType::Opacity,
            NodeData::Resize(_) => NodeType::Resize,
            NodeData::Fill(_) => NodeType::Fill,
            NodeData::Stroke(_) => NodeType::Stroke,
            NodeData::CornerRadius(_) => NodeType::CornerRadius,
            NodeData::Blur(_) => NodeType::Blur,
            NodeData::Shadow(_) => NodeType::Shadow,
            NodeData::Show(_) => NodeType::Show,
            NodeData::Hide(_) => NodeType::Hide,
            NodeData::FadeIn(_) => NodeType::FadeIn,
            NodeData::FadeOut(_) => NodeType::FadeOut,
            NodeData::SlideIn(_) => NodeType::SlideIn,
            NodeData::SlideOut(_) => NodeType::SlideOut,
            NodeData::GrowIn(_) => NodeType::GrowIn,
            NodeData::GrowOut(_) => NodeType::GrowOut,
            NodeData::ShrinkIn(_) => NodeType::ShrinkIn,
            NodeData::ShrinkOut(_) => NodeType::ShrinkOut,
            NodeData::SpinIn(_) => NodeType::SpinIn,
            NodeData::SpinOut(_) => NodeType::SpinOut,
            NodeData::BlurIn(_) => NodeType::BlurIn,
            NodeData::BlurOut(_) => NodeType::BlurOut,
            NodeData::RevealIn(_) => NodeType::RevealIn,
            NodeData::RevealOut(_) => NodeType::RevealOut,
            NodeData::TextIn(_) => NodeType::TextIn,
            NodeData::TextOut(_) => NodeType::TextOut,
            NodeData::Counter(_) => NodeType::Counter,
            NodeData::AnimationGroup(_) => NodeType::AnimationGroup,
        }
    }

    /// Whether the node is a layer or an animation.
    pub fn family(&self) -> Family {
        self.node_type().family()
    }

    /// The shared layer properties, when the node is a layer.
    pub fn layer(&self) -> Option<&LayerProps> {
        match self {
            NodeData::Artboard(n) => Some(&n.layer),
            NodeData::Group(n) => Some(&n.layer),
            NodeData::MaskGroup(n) => Some(&n.layer),
            NodeData::Rect(n) => Some(&n.layer),
            NodeData::Ellipse(n) => Some(&n.layer),
            NodeData::Star(n) => Some(&n.layer),
            NodeData::Path(n) => Some(&n.layer),
            NodeData::Text(n) => Some(&n.layer),
            NodeData::Image(n) => Some(&n.layer),
            NodeData::Component(n) => Some(&n.layer),
            NodeData::Instance(n) => Some(&n.layer),
            _ => None,
        }
    }

    /// Mutable access to the shared layer properties, when the node is a layer.
    pub fn layer_mut(&mut self) -> Option<&mut LayerProps> {
        match self {
            NodeData::Artboard(n) => Some(&mut n.layer),
            NodeData::Group(n) => Some(&mut n.layer),
            NodeData::MaskGroup(n) => Some(&mut n.layer),
            NodeData::Rect(n) => Some(&mut n.layer),
            NodeData::Ellipse(n) => Some(&mut n.layer),
            NodeData::Star(n) => Some(&mut n.layer),
            NodeData::Path(n) => Some(&mut n.layer),
            NodeData::Text(n) => Some(&mut n.layer),
            NodeData::Image(n) => Some(&mut n.layer),
            NodeData::Component(n) => Some(&mut n.layer),
            NodeData::Instance(n) => Some(&mut n.layer),
            _ => None,
        }
    }

    /// The fill at rest, for layer types that have one and have it set.
    pub fn fill(&self) -> Option<&Paint> {
        match self {
            NodeData::Artboard(n) => n.fill.as_ref(),
            NodeData::Group(n) => n.fill.as_ref(),
            NodeData::Rect(n) => n.fill.as_ref(),
            NodeData::Ellipse(n) => n.fill.as_ref(),
            NodeData::Star(n) => n.fill.as_ref(),
            NodeData::Path(n) => n.fill.as_ref(),
            NodeData::Text(n) => n.fill.as_ref(),
            NodeData::Component(n) => n.fill.as_ref(),
            _ => None,
        }
    }

    /// The stroke at rest, for layer types that have one and have it set.
    pub fn stroke(&self) -> Option<&Stroke> {
        match self {
            NodeData::Group(n) => n.stroke.as_ref(),
            NodeData::Rect(n) => n.stroke.as_ref(),
            NodeData::Ellipse(n) => n.stroke.as_ref(),
            NodeData::Star(n) => n.stroke.as_ref(),
            NodeData::Path(n) => n.stroke.as_ref(),
            NodeData::Text(n) => n.stroke.as_ref(),
            NodeData::Image(n) => n.stroke.as_ref(),
            NodeData::Component(n) => n.stroke.as_ref(),
            _ => None,
        }
    }

    /// The corner radius at rest, for layer types that have one.
    pub fn corner_radius(&self) -> Option<CornerRadius> {
        match self {
            NodeData::Group(n) => Some(n.corner_radius),
            NodeData::Rect(n) => Some(n.corner_radius),
            NodeData::Image(n) => Some(n.corner_radius),
            NodeData::Component(n) => Some(n.corner_radius),
            _ => None,
        }
    }

    /// Target, span and easing, for every animation that runs over time.
    ///
    /// `show`, `hide` and `animation_group` have none; use
    /// [`NodeData::target`] and [`NodeData::span`] to cover instants too.
    pub fn timing(&self) -> Option<&Timing> {
        match self {
            NodeData::Move(n) => Some(&n.timing),
            NodeData::Rotate(n) => Some(&n.timing),
            NodeData::Scale(n) => Some(&n.timing),
            NodeData::Opacity(n) => Some(&n.timing),
            NodeData::Resize(n) => Some(&n.timing),
            NodeData::Fill(n) => Some(&n.timing),
            NodeData::Stroke(n) => Some(&n.timing),
            NodeData::CornerRadius(n) => Some(&n.timing),
            NodeData::Blur(n) => Some(&n.timing),
            NodeData::Shadow(n) => Some(&n.timing),
            NodeData::FadeIn(n) | NodeData::FadeOut(n) => Some(&n.timing),
            NodeData::SlideIn(n) | NodeData::SlideOut(n) => Some(&n.timing),
            NodeData::GrowIn(n) => Some(&n.timing),
            NodeData::GrowOut(n) => Some(&n.timing),
            NodeData::ShrinkIn(n) => Some(&n.timing),
            NodeData::ShrinkOut(n) => Some(&n.timing),
            NodeData::SpinIn(n) | NodeData::SpinOut(n) => Some(&n.timing),
            NodeData::BlurIn(n) | NodeData::BlurOut(n) => Some(&n.timing),
            NodeData::RevealIn(n) | NodeData::RevealOut(n) => Some(&n.timing),
            NodeData::TextIn(n) | NodeData::TextOut(n) => Some(&n.timing),
            NodeData::Counter(n) => Some(&n.timing),
            _ => None,
        }
    }

    /// Mutable access to [`NodeData::timing`].
    pub fn timing_mut(&mut self) -> Option<&mut Timing> {
        match self {
            NodeData::Move(n) => Some(&mut n.timing),
            NodeData::Rotate(n) => Some(&mut n.timing),
            NodeData::Scale(n) => Some(&mut n.timing),
            NodeData::Opacity(n) => Some(&mut n.timing),
            NodeData::Resize(n) => Some(&mut n.timing),
            NodeData::Fill(n) => Some(&mut n.timing),
            NodeData::Stroke(n) => Some(&mut n.timing),
            NodeData::CornerRadius(n) => Some(&mut n.timing),
            NodeData::Blur(n) => Some(&mut n.timing),
            NodeData::Shadow(n) => Some(&mut n.timing),
            NodeData::FadeIn(n) | NodeData::FadeOut(n) => Some(&mut n.timing),
            NodeData::SlideIn(n) | NodeData::SlideOut(n) => Some(&mut n.timing),
            NodeData::GrowIn(n) => Some(&mut n.timing),
            NodeData::GrowOut(n) => Some(&mut n.timing),
            NodeData::ShrinkIn(n) => Some(&mut n.timing),
            NodeData::ShrinkOut(n) => Some(&mut n.timing),
            NodeData::SpinIn(n) | NodeData::SpinOut(n) => Some(&mut n.timing),
            NodeData::BlurIn(n) | NodeData::BlurOut(n) => Some(&mut n.timing),
            NodeData::RevealIn(n) | NodeData::RevealOut(n) => Some(&mut n.timing),
            NodeData::TextIn(n) | NodeData::TextOut(n) => Some(&mut n.timing),
            NodeData::Counter(n) => Some(&mut n.timing),
            _ => None,
        }
    }

    /// The layer an animation changes. `None` for layers and animation groups.
    pub fn target(&self) -> Option<&NodeId> {
        match self {
            NodeData::Show(n) | NodeData::Hide(n) => Some(&n.target),
            other => other.timing().map(|timing| &timing.target),
        }
    }

    /// Points an animation at another layer. Does nothing for other nodes.
    pub fn set_target(&mut self, target: NodeId) {
        match self {
            NodeData::Show(n) | NodeData::Hide(n) => n.target = target,
            other => {
                if let Some(timing) = other.timing_mut() {
                    timing.target = target;
                }
            }
        }
    }

    /// The main component an instance shows. `None` for every other node.
    ///
    /// Next to `parent` and `target`, this is the third way a node refers
    /// to another.
    pub fn component(&self) -> Option<&NodeId> {
        match self {
            NodeData::Instance(instance) => Some(&instance.component),
            _ => None,
        }
    }

    /// Points an instance at another component. Does nothing for other
    /// nodes.
    pub fn set_component(&mut self, component: NodeId) {
        if let NodeData::Instance(instance) = self {
            instance.component = component;
        }
    }

    /// Start and end of an animation in milliseconds. Instants start and end
    /// at the same moment. `None` for layers and animation groups.
    pub fn span(&self) -> Option<(f64, f64)> {
        match self {
            NodeData::Show(n) | NodeData::Hide(n) => Some((n.start, n.start)),
            other => other.timing().map(|timing| (timing.start, timing.end)),
        }
    }

    /// The easing of an animation that has one.
    pub fn easing(&self) -> Option<&Easing> {
        self.timing().map(|timing| &timing.easing)
    }

    /// The name given to the node, which may be empty.
    pub fn name(&self) -> &str {
        match self {
            NodeData::Show(n) | NodeData::Hide(n) => &n.name,
            NodeData::AnimationGroup(n) => &n.name,
            other => match (other.layer(), other.timing()) {
                (Some(layer), _) => &layer.name,
                (None, Some(timing)) => &timing.name,
                (None, None) => "",
            },
        }
    }
}

macro_rules! node_data_from {
    ($($ty:ty => $variant:ident),* $(,)?) => {
        $(impl From<$ty> for NodeData {
            fn from(value: $ty) -> Self {
                NodeData::$variant(value)
            }
        })*
    };
}

// Types used by exactly one variant convert on their own. Types shared by an
// "in" and an "out" variant (`FadePreset`, `InstantAnimation`, ...) have to be
// wrapped by hand, because the type alone does not say which one is meant.
node_data_from! {
    Artboard => Artboard,
    Group => Group,
    MaskGroup => MaskGroup,
    RectLayer => Rect,
    EllipseLayer => Ellipse,
    StarLayer => Star,
    PathLayer => Path,
    TextLayer => Text,
    ImageLayer => Image,
    Component => Component,
    Instance => Instance,
    MoveAnimation => Move,
    RotateAnimation => Rotate,
    ScaleAnimation => Scale,
    OpacityAnimation => Opacity,
    ResizeAnimation => Resize,
    FillAnimation => Fill,
    StrokeAnimation => Stroke,
    CornerRadiusAnimation => CornerRadius,
    BlurAnimation => Blur,
    ShadowAnimation => Shadow,
    GrowIn => GrowIn,
    GrowOut => GrowOut,
    ShrinkIn => ShrinkIn,
    ShrinkOut => ShrinkOut,
    CounterAnimation => Counter,
    AnimationGroup => AnimationGroup,
}

impl Node {
    /// Index given to a node that has not been placed among siblings yet.
    pub const FIRST_INDEX: &'static str = "a0";

    /// Creates a node with a fresh id, no parent and the first index.
    ///
    /// Use [`Node::with_parent`] and [`Node::with_index`] to place it.
    pub fn new(data: impl Into<NodeData>) -> Self {
        Self {
            id: NodeId::generate(),
            parent: None,
            index: Self::FIRST_INDEX.to_owned(),
            data: data.into(),
        }
    }

    /// The same node with another id.
    pub fn with_id(mut self, id: impl Into<NodeId>) -> Self {
        self.id = id.into();
        self
    }

    /// The same node under another parent.
    pub fn with_parent(mut self, parent: impl Into<NodeId>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    /// The same node with another index.
    pub fn with_index(mut self, index: impl Into<String>) -> Self {
        self.index = index.into();
        self
    }

    /// The type of the node.
    pub fn node_type(&self) -> NodeType {
        self.data.node_type()
    }

    /// Whether the node is a layer or an animation.
    pub fn family(&self) -> Family {
        self.data.family()
    }

    /// Whether the node is a layer.
    pub fn is_layer(&self) -> bool {
        self.family() == Family::Layer
    }

    /// Whether the node is an animation (or an animation group).
    pub fn is_animation(&self) -> bool {
        self.family() == Family::Animation
    }

    /// The shared layer properties, when the node is a layer.
    pub fn layer(&self) -> Option<&LayerProps> {
        self.data.layer()
    }

    /// The layer an animation changes. `None` for layers and animation groups.
    pub fn target(&self) -> Option<&NodeId> {
        self.data.target()
    }

    /// The main component an instance shows. `None` for every other node.
    pub fn component(&self) -> Option<&NodeId> {
        self.data.component()
    }

    /// The name given to the node, which may be empty.
    pub fn name(&self) -> &str {
        self.data.name()
    }

    /// How messages refer to the node: its type, id and name, for example
    /// `rect "cd34" (Card)`.
    pub fn label(&self) -> String {
        match self.name() {
            "" => format!("{} \"{}\"", self.node_type(), self.id),
            name => format!("{} \"{}\" ({name})", self.node_type(), self.id),
        }
    }

    /// The node as JSON, as it is written to files: properties at their
    /// default are left out and whole numbers are written without a fraction.
    pub fn to_value(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("a node always serialises");
        json::tidy_numbers(&mut value);
        value
    }

    /// The node as JSON with every property written out, defaults included.
    ///
    /// This is what an inspector or an AI agent wants to read; files use
    /// [`Node::to_value`].
    pub fn to_full_value(&self) -> Value {
        defaults::with_defaults(|| self.to_value())
    }

    /// Reads a node from JSON. Unknown types and unknown fields are errors.
    pub fn from_value(value: Value) -> Result<Node, serde_json::Error> {
        serde_json::from_value(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::model::{AutoSize, TextAlign};
    use serde_json::json;

    #[test]
    fn reads_the_example_from_the_architecture_document() {
        let node = Node::from_value(json!({
            "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card",
            "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#0D99FF", "corner_radius": 32
        }))
        .unwrap();
        assert_eq!(node.id, NodeId::from("cd34"));
        assert_eq!(node.parent, Some(NodeId::from("ab12")));
        assert_eq!(node.node_type(), NodeType::Rect);
        assert_eq!(node.family(), Family::Layer);
        let NodeData::Rect(rect) = &node.data else {
            panic!("expected a rect")
        };
        assert_eq!(rect.layer.name, "Card");
        assert_eq!(
            rect.layer.frame(),
            crate::Rect::new(240.0, 340.0, 600.0, 400.0)
        );
        assert_eq!(rect.fill, Some(Paint::Solid(Color::rgb(0x0D, 0x99, 0xFF))));
        assert_eq!(rect.corner_radius, CornerRadius::Uniform(32.0));
        // Everything not mentioned takes its default.
        assert_eq!(
            (rect.layer.rotation, rect.layer.scale, rect.layer.opacity),
            (0.0, 1.0, 100.0)
        );
        assert_eq!(node.label(), "rect \"cd34\" (Card)");
    }

    #[test]
    fn writes_fields_in_a_stable_order_without_defaults() {
        let source = json!({
            "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card",
            "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#0D99FF", "corner_radius": 32
        });
        let node = Node::from_value(source.clone()).unwrap();
        assert_eq!(
            node.to_value().to_string(),
            r##"{"id":"cd34","parent":"ab12","index":"a0","type":"rect","name":"Card","x":240,"y":340,"width":600,"height":400,"fill":"#0D99FF","corner_radius":32}"##
        );
        assert_eq!(Node::from_value(node.to_value()).unwrap(), node);
    }

    #[test]
    fn artboards_write_an_explicit_null_parent() {
        let node = Node::new(Artboard {
            layer: LayerProps::new("Scene 1", 0.0, 0.0, 1080.0, 1080.0),
            duration: 4000.0,
            fill: Some(Paint::Solid(Color::WHITE)),
        })
        .with_id("ab12");
        assert_eq!(
            node.to_value(),
            json!({
                "id": "ab12", "parent": null, "index": "a0", "type": "artboard", "name": "Scene 1",
                "width": 1080, "height": 1080, "fill": "#FFFFFF"
            })
        );
        // `parent` may also be left out when reading.
        let read =
            Node::from_value(json!({ "id": "ab12", "index": "a0", "type": "artboard" })).unwrap();
        assert_eq!(read.parent, None);
    }

    #[test]
    fn full_value_spells_out_every_default() {
        let node = Node::new(TextLayer::default())
            .with_id("t1")
            .with_parent("ab12");
        let full = node.to_full_value();
        assert_eq!(full["opacity"], json!(100));
        assert_eq!(full["scale"], json!(1));
        assert_eq!(
            full["font"],
            json!({ "family": "Inter", "weight": 400, "italic": false })
        );
        assert_eq!(full["font_size"], json!(48));
        assert_eq!(full["line_height"], json!(120));
        assert_eq!(full["auto_size"], json!("width_and_height"));
        assert_eq!(full["align"], json!("left"));
        // The compact form of the same node has none of them.
        assert_eq!(
            node.to_value(),
            json!({ "id": "t1", "parent": "ab12", "index": "a0", "type": "text" })
        );
        // Reading the full form gives the same node back.
        assert_eq!(Node::from_value(full).unwrap(), node);
    }

    #[test]
    fn every_layer_type_round_trips_with_non_default_values() {
        let layers = [
            json!({ "type": "artboard", "name": "A", "width": 640, "height": 360, "duration": 2500, "fill": "#101010" }),
            json!({ "type": "group", "x": 5, "clip": true, "corner_radius": [1, 2, 3, 4], "fill": "#FFFFFF",
                    "stroke": { "width": 2 } }),
            json!({ "type": "mask_group", "rotation": 45, "opacity": 50 }),
            json!({ "type": "rect", "hidden": true, "locked": true, "blend_mode": "multiply", "blur": 4,
                    "shadow": { "y": 8 }, "source_id": "figma:abc:1:2" }),
            json!({ "type": "ellipse", "start_angle": 90, "sweep": 180, "inner_radius": 0.5 }),
            json!({ "type": "star", "points": 7, "inner_radius": 0.6, "scale": 2 }),
            json!({ "type": "path", "path": { "subpaths": [{ "anchors": [{ "point": [0, 0] }, { "point": [1, 1] }] }] } }),
            json!({ "type": "text", "content": "Hello", "font": { "family": "Lora", "weight": 700 }, "font_size": 64,
                    "line_height": 100, "letter_spacing": -2, "align": "center", "vertical_align": "bottom",
                    "auto_size": "fixed", "case": "upper", "fill": "#000000" }),
            json!({ "type": "image", "source": "assets/photo.png", "fit": "contain", "corner_radius": 12 }),
            json!({ "type": "component", "name": "Button", "clip": true, "corner_radius": 8, "fill": "#0D99FF",
                    "stroke": { "width": 1 } }),
            json!({ "type": "instance", "component": "m1", "time_offset": -150, "width": 240 }),
        ];
        for mut layer in layers {
            let fields = layer.as_object_mut().unwrap();
            fields.insert("id".into(), json!("n1"));
            fields.insert("parent".into(), json!(null));
            fields.insert("index".into(), json!("a0"));
            let node = Node::from_value(layer.clone()).unwrap_or_else(|e| panic!("{layer}: {e}"));
            let written = node.to_value();
            assert_eq!(
                written.as_object().unwrap().len(),
                layer.as_object().unwrap().len(),
                "nothing dropped or added: {written}"
            );
            assert_eq!(Node::from_value(written).unwrap(), node);
        }
    }

    #[test]
    fn unknown_fields_types_and_bad_values_are_rejected() {
        let base = |extra: Value| {
            let mut value = json!({ "id": "n1", "parent": "p", "index": "a0" });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            Node::from_value(value).unwrap_err().to_string()
        };
        assert!(
            base(json!({ "type": "rect", "colour": "#FFFFFF" })).contains("unknown field `colour`")
        );
        assert!(base(json!({ "type": "ellipse", "corner_radius": 4 })).contains("corner_radius"));
        assert!(base(json!({ "type": "mask_group", "fill": "#FFFFFF" })).contains("fill"));
        assert!(base(json!({ "type": "hexagon" })).contains("unknown variant `hexagon`"));
        assert!(base(json!({ "type": "rect", "x": "left" })).contains("invalid type"));
        assert!(base(json!({ "type": "rect", "fill": "blue" })).contains("not a colour"));
        assert!(base(json!({ "type": "image" })).contains("missing field `source`"));
        assert!(base(json!({ "type": "instance" })).contains("missing field `component`"));
        assert!(
            base(json!({ "type": "instance", "component": "m1", "fill": "#FFFFFF" }))
                .contains("fill")
        );
        assert!(base(json!({})).contains("missing field `type`"));
    }

    #[test]
    fn type_names_round_trip() {
        for ty in NodeType::ALL {
            assert_eq!(NodeType::parse(ty.as_str()), Some(*ty));
            assert_eq!(serde_json::to_value(ty).unwrap(), json!(ty.as_str()));
            assert!(!ty.description().is_empty());
        }
        assert_eq!(NodeType::parse("nope"), None);
        assert!(NodeType::Group.holds_layers());
        assert!(NodeType::Component.holds_layers());
        assert!(!NodeType::Rect.holds_layers());
        assert!(
            !NodeType::Instance.holds_layers(),
            "an instance shows its main's layers"
        );
    }

    #[test]
    fn accessors_reach_shared_properties() {
        let mut data = NodeData::from(TextLayer {
            layer: LayerProps::new("Title", 10.0, 20.0, 300.0, 60.0),
            fill: Some(Paint::Solid(Color::BLACK)),
            align: TextAlign::Center,
            auto_size: AutoSize::Height,
            ..TextLayer::default()
        });
        assert_eq!(data.name(), "Title");
        assert_eq!(data.fill(), Some(&Paint::Solid(Color::BLACK)));
        assert_eq!(data.stroke(), None);
        assert_eq!(data.corner_radius(), None);
        data.layer_mut().unwrap().x = 99.0;
        assert_eq!(data.layer().unwrap().x, 99.0);
    }

    #[test]
    fn reads_the_animation_example_from_the_architecture_document() {
        let node = Node::from_value(json!({
            "id": "ef56", "parent": "ab12", "index": "a0", "type": "fade_in",
            "target": "cd34", "start": 0, "end": 600, "easing": { "type": "smooth", "intensity": 50 }
        }))
        .unwrap();
        assert_eq!(node.node_type(), NodeType::FadeIn);
        assert_eq!(node.family(), Family::Animation);
        assert_eq!(node.target(), Some(&NodeId::from("cd34")));
        assert_eq!(node.data.span(), Some((0.0, 600.0)));
        assert_eq!(node.data.easing(), Some(&Easing::smooth()));
        // The default easing is not written back.
        assert_eq!(
            node.to_value(),
            json!({ "id": "ef56", "parent": "ab12", "index": "a0", "type": "fade_in",
                    "target": "cd34", "start": 0, "end": 600 })
        );
    }

    /// One example of every animation type with non-default settings.
    fn animation_examples() -> Vec<Value> {
        let gradient = json!({ "type": "linear_gradient", "stops": [
            { "offset": 0, "color": "#FF0000" }, { "offset": 1, "color": "#0000FF" } ] });
        vec![
            json!({ "type": "move", "from": { "x": -200 }, "to": { "y": 40 } }),
            json!({ "type": "rotate", "from": -90, "to": 180 }),
            json!({ "type": "scale", "from": 0, "to": 1.2 }),
            json!({ "type": "opacity", "from": 0, "to": 80 }),
            json!({ "type": "resize", "from": { "width": 10 }, "to": { "width": 300, "height": 50 },
                    "anchor": "top_left" }),
            json!({ "type": "fill", "from": "#FF0000", "to": gradient }),
            json!({ "type": "stroke", "from": { "width": 0 }, "to": { "paint": "#00FF00", "width": 6 } }),
            json!({ "type": "corner_radius", "from": 0, "to": [8, 8, 0, 0] }),
            json!({ "type": "blur", "from": 24, "to": 0 }),
            json!({ "type": "shadow", "from": { "blur": 0 }, "to": { "color": "#00000080", "x": 2, "y": 12, "blur": 30 } }),
            json!({ "type": "show" }),
            json!({ "type": "hide", "name": "Cut" }),
            json!({ "type": "fade_in" }),
            json!({ "type": "fade_out", "easing": { "type": "linear" } }),
            json!({ "type": "slide_in", "direction": "left", "distance": 240, "fade": false }),
            json!({ "type": "slide_out", "direction": "down" }),
            json!({ "type": "grow_in", "scale": 0.2 }),
            json!({ "type": "grow_out", "scale": 3, "fade": false }),
            json!({ "type": "shrink_in", "scale": 2 }),
            json!({ "type": "shrink_out", "scale": 0 }),
            json!({ "type": "spin_in", "angle": -360, "fade": true }),
            json!({ "type": "spin_out", "angle": 90 }),
            json!({ "type": "blur_in", "blur": 40 }),
            json!({ "type": "blur_out" }),
            json!({ "type": "reveal_in", "direction": "down" }),
            json!({ "type": "reveal_out" }),
            json!({ "type": "text_in", "split": "words", "effect": "slide_up", "stagger": 80, "order": "backward",
                    "distance": 50 }),
            json!({ "type": "text_out", "split": "lines", "effect": "scale", "scale": 1.5 }),
            json!({ "type": "counter", "from": 0, "to": 1250.5, "easing": { "type": "slow_down", "intensity": 70 } }),
        ]
    }

    #[test]
    fn every_animation_type_round_trips_with_non_default_values() {
        for mut animation in animation_examples() {
            let fields = animation.as_object_mut().unwrap();
            fields.insert("id".into(), json!("a1"));
            fields.insert("parent".into(), json!("ab12"));
            fields.insert("index".into(), json!("a0"));
            fields.insert("target".into(), json!("cd34"));
            fields.insert("start".into(), json!(250));
            if !matches!(fields["type"].as_str(), Some("show" | "hide")) {
                fields.insert("end".into(), json!(900));
            }
            let node =
                Node::from_value(animation.clone()).unwrap_or_else(|e| panic!("{animation}: {e}"));
            assert_eq!(node.family(), Family::Animation);
            assert_eq!(node.target(), Some(&NodeId::from("cd34")));
            let written = node.to_value();
            assert_eq!(
                written.as_object().unwrap().len(),
                animation.as_object().unwrap().len(),
                "nothing dropped or added: {written}"
            );
            assert_eq!(Node::from_value(written).unwrap(), node);
            assert_eq!(Node::from_value(node.to_full_value()).unwrap(), node);
        }
    }

    #[test]
    fn the_examples_cover_every_animation_type() {
        let covered: Vec<String> = animation_examples()
            .iter()
            .map(|example| example["type"].as_str().unwrap().to_owned())
            .collect();
        for ty in NodeType::ALL {
            if ty.family() == Family::Animation && *ty != NodeType::AnimationGroup {
                assert!(
                    covered.iter().any(|name| name == ty.as_str()),
                    "no example for {ty}"
                );
            }
        }
    }

    #[test]
    fn animation_groups_are_plain_folders() {
        let group = Node::from_value(json!({
            "id": "g1", "parent": "ab12", "index": "a1", "type": "animation_group", "name": "Intro"
        }))
        .unwrap();
        assert_eq!(group.name(), "Intro");
        assert_eq!(group.target(), None);
        assert_eq!(group.data.span(), None);
        assert!(NodeType::AnimationGroup.holds_animations());
        assert!(!NodeType::AnimationGroup.holds_layers());
        assert!(
            Node::from_value(json!({
                "id": "g1", "parent": "ab12", "index": "a1", "type": "animation_group", "target": "x"
            }))
            .is_err()
        );
    }

    #[test]
    fn instants_span_a_single_moment_and_ignore_end() {
        let show = Node::from_value(json!({
            "id": "s1", "parent": "ab12", "index": "a0", "type": "show", "target": "cd34", "start": 400, "end": 900
        }))
        .unwrap();
        assert_eq!(show.data.span(), Some((400.0, 400.0)));
        assert_eq!(show.data.timing(), None);
        assert_eq!(show.data.easing(), None);
        assert!(
            Node::from_value(json!({
                "id": "s1", "parent": "ab12", "index": "a0", "type": "show", "target": "cd34", "start": 400,
                "easing": { "type": "linear" }
            }))
            .is_err(),
            "an instant has no easing"
        );
    }

    #[test]
    fn animations_reject_fields_of_other_types() {
        let attempt = |extra: Value| {
            let mut value = json!({ "id": "a1", "parent": "ab12", "index": "a0", "target": "cd34", "start": 0, "end": 1 });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            Node::from_value(value).unwrap_err().to_string()
        };
        assert!(attempt(json!({ "type": "fade_in", "from": 0 })).contains("from"));
        assert!(attempt(json!({ "type": "move", "to": { "z": 1 } })).contains("z"));
        assert!(attempt(json!({ "type": "resize", "anchor": "middle" })).contains("middle"));
        assert!(attempt(json!({ "type": "opacity", "to": "half" })).contains("invalid type"));
        assert!(attempt(json!({ "type": "slide_in", "direction": "north" })).contains("north"));
    }

    #[test]
    fn retargeting_reaches_every_animation() {
        let mut fade = NodeData::FadeIn(FadePreset {
            timing: Timing::new("old", 0.0, 500.0),
        });
        fade.set_target(NodeId::from("new"));
        assert_eq!(fade.target(), Some(&NodeId::from("new")));

        let mut hide = NodeData::Hide(InstantAnimation::new("old", 100.0));
        hide.set_target(NodeId::from("new"));
        assert_eq!(hide.target(), Some(&NodeId::from("new")));

        let mut rect = NodeData::from(RectLayer::default());
        rect.set_target(NodeId::from("new"));
        assert_eq!(rect.target(), None);
    }

    #[test]
    fn kinds_and_entrances() {
        assert_eq!(
            NodeType::Move.animation_kind(),
            Some(AnimationKind::Property)
        );
        assert_eq!(
            NodeType::Show.animation_kind(),
            Some(AnimationKind::Instant)
        );
        assert_eq!(
            NodeType::Counter.animation_kind(),
            Some(AnimationKind::Text)
        );
        assert_eq!(NodeType::Rect.animation_kind(), None);
        assert!(NodeType::SlideIn.is_entrance() && !NodeType::SlideIn.is_exit());
        assert!(NodeType::RevealOut.is_exit());
        assert!(
            !NodeType::TextIn.is_entrance(),
            "text animations are their own kind"
        );
        assert_eq!(NodeType::ALL.len(), 41);
    }
}
