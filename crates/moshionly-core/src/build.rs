//! Factories and helpers that turn an intention into edits.
//!
//! Nothing here changes a document. Every helper reads the document and
//! returns the [`Edit`]s that would do the job; the caller applies them
//! through [`History::apply`](crate::History::apply) (or
//! [`Document::apply`]), so that the change is validated and can be undone
//! like any other.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::color::Color;
use crate::document::Document;
use crate::easing::Easing;
use crate::edit::Edit;
use crate::geometry::{Affine, Point, Rect};
use crate::id::NodeId;
use crate::index::{self, IndexError};
use crate::model::{
    AnimationGroup, Artboard, BlurAnimation, BlurPreset, CornerRadius, CornerRadiusAnimation,
    CounterAnimation, Direction, EllipseLayer, FadePreset, Family, FillAnimation, Group, GrowIn,
    GrowOut, ImageFit, ImageLayer, InstantAnimation, LayerProps, MaskGroup, MoveAnimation, Node,
    NodeData, NodeType, Offset, OpacityAnimation, Paint, PathData, PathLayer, RectLayer,
    ResizeAnchor, ResizeAnimation, RevealPreset, RotateAnimation, ScaleAnimation, ShadowAnimation,
    ShadowChange, ShrinkIn, ShrinkOut, SizeChange, SlidePreset, SpinPreset, StarLayer,
    StrokeAnimation, StrokeChange, TextAnimation, TextLayer, Timing,
};
use crate::validate::animation_applies_to;

/// Why a helper could not produce edits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// An id that was passed in does not exist.
    #[error("there is no node with the id \"{0}\"")]
    UnknownNode(NodeId),
    /// The operation needs a layer and got something else.
    #[error("{0} is not a layer, and only layers can be used here")]
    NotALayer(String),
    /// The operation needs an animation and got something else.
    #[error("{0} is not an animation, and only animations can be used here")]
    NotAnAnimation(String),
    /// The operation does not work on artboards.
    #[error("{0} is an artboard, and an artboard cannot be {1}")]
    Artboard(String, &'static str),
    /// The operation needs a group and got something else.
    #[error(
        "{0} is not a group, a mask_group or an animation_group, so there is nothing to ungroup"
    )]
    NotAGroup(String),
    /// No ids were passed in.
    #[error("no nodes were given, so there is nothing to do")]
    Empty,
    /// The nodes passed in are spread over several artboards.
    #[error("the nodes are in different artboards; pick nodes from one artboard")]
    DifferentArtboards,
    /// The nodes passed in are a mix of layers and animations.
    #[error("the nodes are a mix of layers and animations; pick one kind")]
    MixedFamilies,
    /// The parent cannot hold the node.
    #[error("{child} cannot be placed in {parent}: {rule}")]
    CannotHold {
        /// The node that was to be placed.
        child: String,
        /// The node it was to be placed in.
        parent: String,
        /// What the parent may hold.
        rule: &'static str,
    },
    /// A node would be moved into itself or into one of its descendants.
    #[error("{0} cannot be moved inside itself")]
    IntoItself(String),
    /// The place refers to a node that is not among the siblings.
    #[error("\"{0}\" is not a child of the chosen parent, so nothing can be placed next to it")]
    NotASibling(NodeId),
    /// The animation type cannot target the layer.
    #[error("a {animation} animation does not work on {target}")]
    WrongTarget {
        /// The animation type.
        animation: NodeType,
        /// The layer it was aimed at.
        target: String,
    },
    /// The type is not one the helper can make.
    #[error("{0} is not a {1} type")]
    WrongType(NodeType, &'static str),
    /// No fractional index could be made.
    #[error(transparent)]
    Index(#[from] IndexError),
}

/// Where a node goes among the children of its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// Before every sibling: at the back for layers, the top row for animations.
    First,
    /// After every sibling: in front for layers, the bottom row for animations.
    Last,
    /// Just before this sibling.
    Before(NodeId),
    /// Just after this sibling.
    After(NodeId),
}

/// A step in the order of siblings.
///
/// "Front" is the end of the sibling list: on top for layers, the bottom
/// row for animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reorder {
    /// After every sibling.
    ToFront,
    /// One place toward the front.
    Forward,
    /// One place toward the back.
    Backward,
    /// Before every sibling.
    ToBack,
}

/// Which kind of group [`group`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// A plain `group`.
    Group,
    /// A `mask_group`: the backmost of the grouped layers masks the others.
    Mask,
}

/// Space left between artboards that are laid out next to each other.
pub const ARTBOARD_GAP: f64 = 100.0;

/// A transparent 1 by 1 pixel PNG, used as the source of a new image layer
/// until a real image is chosen.
pub const PLACEHOLDER_IMAGE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNgAAIAAAUAAen63NgAAAAASUVORK5CYII=";

/// Fill of a new shape: a neutral grey.
const SHAPE_FILL: Color = Color::rgb(0xD9, 0xD9, 0xD9);

/// A layer of the given type with sensible starting properties: shapes are
/// 100 by 100 and grey, text says "Text" in black, an artboard is 1080 by
/// 1080 and white, a path is a triangle, an image shows a transparent pixel.
///
/// `None` when `ty` is not a layer type.
pub fn default_layer(ty: NodeType) -> Option<NodeData> {
    let fill = Some(Paint::Solid(SHAPE_FILL));
    Some(match ty {
        NodeType::Artboard => Artboard {
            layer: LayerProps::new("Artboard", 0.0, 0.0, 1080.0, 1080.0),
            duration: 4000.0,
            fill: Some(Paint::Solid(Color::WHITE)),
        }
        .into(),
        NodeType::Group => Group::default().into(),
        NodeType::MaskGroup => MaskGroup::default().into(),
        NodeType::Rect => RectLayer {
            fill,
            ..RectLayer::default()
        }
        .into(),
        NodeType::Ellipse => EllipseLayer {
            fill,
            ..EllipseLayer::default()
        }
        .into(),
        NodeType::Star => StarLayer {
            fill,
            ..StarLayer::default()
        }
        .into(),
        NodeType::Path => PathLayer {
            fill,
            path: PathData::polygon(&[[0.5, 0.0], [1.0, 1.0], [0.0, 1.0]]),
            ..PathLayer::default()
        }
        .into(),
        NodeType::Text => TextLayer {
            layer: LayerProps::new("", 0.0, 0.0, 100.0, 58.0),
            fill: Some(Paint::Solid(Color::BLACK)),
            content: "Text".to_owned(),
            ..TextLayer::default()
        }
        .into(),
        NodeType::Image => ImageLayer {
            layer: LayerProps::default(),
            source: PLACEHOLDER_IMAGE.to_owned(),
            fit: ImageFit::default(),
            stroke: None,
            corner_radius: CornerRadius::default(),
        }
        .into(),
        _ => return None,
    })
}

/// How long a new animation of the given type lasts, in milliseconds.
/// 0 for `show`, `hide` and types that are not animations.
pub fn default_duration(ty: NodeType) -> f64 {
    use NodeType as T;
    match ty {
        T::Show | T::Hide | T::AnimationGroup => 0.0,
        T::FadeIn | T::FadeOut => 500.0,
        T::GrowIn | T::GrowOut | T::ShrinkIn | T::ShrinkOut => 600.0,
        T::SlideIn | T::SlideOut | T::BlurIn | T::BlurOut | T::RevealIn | T::RevealOut => 700.0,
        T::SpinIn | T::SpinOut => 900.0,
        T::TextIn | T::TextOut => 1000.0,
        T::Counter => 1500.0,
        _ if ty.family() == Family::Animation => 800.0,
        _ => 0.0,
    }
}

/// The easing a new animation of the given type starts with: things that
/// arrive brake into place, things that leave accelerate away, and changes
/// in between ease both ways.
pub fn default_easing(ty: NodeType) -> Easing {
    if ty.is_entrance() || matches!(ty, NodeType::TextIn | NodeType::Counter) {
        Easing::slow_down()
    } else if ty.is_exit() || ty == NodeType::TextOut {
        Easing::accelerate()
    } else {
        Easing::smooth()
    }
}

/// An animation of the given type on `target`, starting at `start`, with a
/// duration, an easing and settings that give a visible result straight away.
///
/// Property animations get a `to` value to move toward (a `move` goes 100
/// pixels right, an `opacity` fades to 0, a `scale` grows to 1.5 and so on)
/// and no `from`, so they start from wherever the layer is.
///
/// `None` when `ty` is not an animation type, or is `animation_group`.
pub fn default_animation(ty: NodeType, target: &NodeId, start: f64) -> Option<NodeData> {
    use NodeType as T;
    let start = start.max(0.0);
    let timing =
        Timing::new(target, start, start + default_duration(ty)).with_easing(default_easing(ty));
    let instant = || InstantAnimation::new(target, start);
    Some(match ty {
        T::Move => MoveAnimation {
            timing,
            from: None,
            to: Some(Offset::new(100.0, 0.0)),
        }
        .into(),
        T::Rotate => RotateAnimation {
            timing,
            from: None,
            to: Some(90.0),
        }
        .into(),
        T::Scale => ScaleAnimation {
            timing,
            from: None,
            to: Some(1.5),
        }
        .into(),
        T::Opacity => OpacityAnimation {
            timing,
            from: None,
            to: Some(0.0),
        }
        .into(),
        T::Resize => ResizeAnimation {
            timing,
            from: None,
            to: Some(SizeChange {
                width: Some(200.0),
                height: None,
            }),
            anchor: ResizeAnchor::default(),
        }
        .into(),
        T::Fill => FillAnimation {
            timing,
            from: None,
            to: Some(Paint::Solid(Color::rgb(0x0D, 0x99, 0xFF))),
        }
        .into(),
        T::Stroke => StrokeAnimation {
            timing,
            from: None,
            to: Some(StrokeChange {
                paint: Some(Paint::Solid(Color::BLACK)),
                width: Some(4.0),
            }),
        }
        .into(),
        T::CornerRadius => CornerRadiusAnimation {
            timing,
            from: None,
            to: Some(CornerRadius::Uniform(40.0)),
        }
        .into(),
        T::Blur => BlurAnimation {
            timing,
            from: None,
            to: Some(20.0),
        }
        .into(),
        T::Shadow => ShadowAnimation {
            timing,
            from: None,
            to: Some(ShadowChange {
                color: Some(Color::rgba(0, 0, 0, 0x66)),
                x: Some(0.0),
                y: Some(16.0),
                blur: Some(32.0),
            }),
        }
        .into(),
        T::Show => NodeData::Show(instant()),
        T::Hide => NodeData::Hide(instant()),
        T::FadeIn => NodeData::FadeIn(FadePreset { timing }),
        T::FadeOut => NodeData::FadeOut(FadePreset { timing }),
        T::SlideIn | T::SlideOut => {
            let preset = SlidePreset {
                timing,
                direction: Direction::default(),
                distance: 100.0,
                fade: true,
            };
            if ty == T::SlideIn {
                NodeData::SlideIn(preset)
            } else {
                NodeData::SlideOut(preset)
            }
        }
        T::GrowIn => GrowIn {
            timing,
            scale: 0.5,
            fade: true,
        }
        .into(),
        T::GrowOut => GrowOut {
            timing,
            scale: 1.5,
            fade: true,
        }
        .into(),
        T::ShrinkIn => ShrinkIn {
            timing,
            scale: 1.5,
            fade: true,
        }
        .into(),
        T::ShrinkOut => ShrinkOut {
            timing,
            scale: 0.5,
            fade: true,
        }
        .into(),
        T::SpinIn | T::SpinOut => {
            let preset = SpinPreset {
                timing,
                angle: 180.0,
                fade: false,
            };
            if ty == T::SpinIn {
                NodeData::SpinIn(preset)
            } else {
                NodeData::SpinOut(preset)
            }
        }
        T::BlurIn => NodeData::BlurIn(BlurPreset { timing, blur: 20.0 }),
        T::BlurOut => NodeData::BlurOut(BlurPreset { timing, blur: 20.0 }),
        T::RevealIn | T::RevealOut => {
            let preset = RevealPreset {
                timing,
                direction: Direction::Right,
            };
            if ty == T::RevealIn {
                NodeData::RevealIn(preset)
            } else {
                NodeData::RevealOut(preset)
            }
        }
        T::TextIn => NodeData::TextIn(TextAnimation::new(timing)),
        T::TextOut => NodeData::TextOut(TextAnimation::new(timing)),
        T::Counter => CounterAnimation {
            timing,
            from: Some(0.0),
            to: None,
        }
        .into(),
        _ => return None,
    })
}

/// The siblings a new child of `parent` would join, without `skip`.
fn siblings_without<'a>(
    document: &'a Document,
    parent: Option<&NodeId>,
    family: Family,
    skip: &[&NodeId],
) -> Vec<&'a Node> {
    document
        .child_list(parent, family)
        .iter()
        .filter(|id| !skip.contains(id))
        .filter_map(|id| document.get(id))
        .collect()
}

/// `count` fresh indices, in order, for nodes that go to `place` among the
/// children of `parent` (`None` for artboards) in one family.
///
/// When the nodes being placed are already children of that parent, list
/// them in `moving` so their current indices do not count as neighbours.
pub fn indices_at(
    document: &Document,
    parent: Option<&NodeId>,
    family: Family,
    place: &Place,
    count: usize,
    moving: &[&NodeId],
) -> Result<Vec<String>, BuildError> {
    let siblings = siblings_without(document, parent, family, moving);
    let position = |id: &NodeId| {
        siblings
            .iter()
            .position(|sibling| &sibling.id == id)
            .ok_or_else(|| BuildError::NotASibling(id.clone()))
    };
    let (before, after): (Option<&Node>, Option<&Node>) = match place {
        Place::First => (None, siblings.first().copied()),
        Place::Last => (siblings.last().copied(), None),
        Place::Before(id) => {
            let at = position(id)?;
            (
                at.checked_sub(1).and_then(|i| siblings.get(i).copied()),
                siblings.get(at).copied(),
            )
        }
        Place::After(id) => {
            let at = position(id)?;
            (siblings.get(at).copied(), siblings.get(at + 1).copied())
        }
    };
    Ok(index::keys_between(
        before.map(|node| node.index.as_str()),
        after.map(|node| node.index.as_str()),
        count,
    )?)
}

/// A fresh index for one node that goes to `place`. See [`indices_at`].
pub fn index_at(
    document: &Document,
    parent: Option<&NodeId>,
    family: Family,
    place: &Place,
    moving: Option<&NodeId>,
) -> Result<String, BuildError> {
    let moving: Vec<&NodeId> = moving.into_iter().collect();
    let mut indices = indices_at(document, parent, family, place, 1, &moving)?;
    Ok(indices.pop().unwrap_or_else(|| index::FIRST.to_owned()))
}

/// The index that makes a new node the last child of `parent` in its
/// family: the front layer, or the bottom timeline row. Pass `None` as the
/// parent for a new artboard.
pub fn last_index(
    document: &Document,
    parent: Option<&NodeId>,
    family: Family,
) -> Result<String, BuildError> {
    index_at(document, parent, family, &Place::Last, None)
}

fn existing<'a>(document: &'a Document, id: &NodeId) -> Result<&'a Node, BuildError> {
    document
        .get(id)
        .ok_or_else(|| BuildError::UnknownNode(id.clone()))
}

fn layer_of<'a>(document: &'a Document, id: &NodeId) -> Result<&'a Node, BuildError> {
    let node = existing(document, id)?;
    if node.is_layer() {
        Ok(node)
    } else {
        Err(BuildError::NotALayer(node.label()))
    }
}

/// Checks that `parent` may hold a node of type `child`.
fn check_holds(parent: &Node, child: NodeType, child_label: String) -> Result<(), BuildError> {
    let (holds, rule) = match child.family() {
        _ if child == NodeType::Artboard => (false, "an artboard is a root and has no parent"),
        Family::Layer => (
            parent.node_type().holds_layers(),
            "layers go in an artboard, a group or a mask_group",
        ),
        Family::Animation => (
            parent.node_type().holds_animations(),
            "animations go in an artboard or an animation_group",
        ),
    };
    if holds {
        Ok(())
    } else {
        Err(BuildError::CannotHold {
            child: child_label,
            parent: parent.label(),
            rule,
        })
    }
}

/// Edits that add a new artboard to the right of the existing ones.
///
/// Returns the id of the artboard with the edits.
pub fn add_artboard(
    document: &Document,
    name: &str,
    width: f64,
    height: f64,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let mut artboard = Artboard {
        layer: LayerProps::new(name, 0.0, 0.0, width, height),
        duration: 4000.0,
        fill: Some(Paint::Solid(Color::WHITE)),
    };
    (artboard.layer.x, artboard.layer.y) = free_canvas_spot(document);
    let node = Node::new(artboard).with_index(last_index(document, None, Family::Layer)?);
    Ok((node.id.clone(), vec![Edit::add(node)]))
}

/// Where the next artboard goes on the canvas: right of the rightmost one,
/// level with the first.
fn free_canvas_spot(document: &Document) -> (f64, f64) {
    let frames: Vec<Rect> = document
        .artboards()
        .iter()
        .filter_map(|id| document.get(id)?.layer())
        .map(LayerProps::frame)
        .collect();
    match frames.first() {
        None => (0.0, 0.0),
        Some(first) => {
            let right = frames.iter().map(Rect::right).fold(f64::MIN, f64::max);
            (right + ARTBOARD_GAP, first.y)
        }
    }
}

/// Edits that add a layer as the front child of `parent`.
///
/// Returns the id of the new layer with the edits. Use [`default_layer`] to
/// get `data` for a type, or build it by hand.
pub fn add_layer(
    document: &Document,
    parent: &NodeId,
    data: impl Into<NodeData>,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let data = data.into();
    let parent_node = existing(document, parent)?;
    let node = Node::new(data).with_parent(parent);
    if !node.is_layer() {
        return Err(BuildError::NotALayer(node.label()));
    }
    check_holds(parent_node, node.node_type(), node.label())?;
    let node = node.with_index(last_index(document, Some(parent), Family::Layer)?);
    Ok((node.id.clone(), vec![Edit::add(node)]))
}

/// Edits that add an animation as the bottom row of its target's artboard,
/// or of the animation group `folder` when one is given.
///
/// Returns the id of the new animation with the edits. Use
/// [`default_animation`] to get `data` for a type, or build it by hand.
pub fn add_animation(
    document: &Document,
    data: impl Into<NodeData>,
    folder: Option<&NodeId>,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let node = Node::new(data);
    let Some(target_id) = node.target() else {
        return Err(BuildError::NotAnAnimation(node.label()));
    };
    let target = layer_of(document, target_id)?;
    if !animation_applies_to(node.node_type(), target.node_type()) {
        return Err(BuildError::WrongTarget {
            animation: node.node_type(),
            target: target.label(),
        });
    }
    let parent = match folder {
        Some(folder) => {
            check_holds(existing(document, folder)?, node.node_type(), node.label())?;
            folder.clone()
        }
        None => document
            .artboard_of(target_id)
            .map(|artboard| artboard.id.clone())
            .ok_or_else(|| BuildError::UnknownNode(target_id.clone()))?,
    };
    let index = last_index(document, Some(&parent), Family::Animation)?;
    let node = node.with_parent(parent).with_index(index);
    Ok((node.id.clone(), vec![Edit::add(node)]))
}

/// Rounds away the noise that trigonometry leaves behind, so that files do
/// not fill up with values like `89.99999999999999`.
fn tidy(value: f64) -> f64 {
    let rounded = (value * 1e6).round() / 1e6;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// The `x`, `y`, `rotation` and `scale` that give a `width` by `height`
/// layer the transform `local` in its parent's coordinates.
///
/// Layer transforms only ever turn, scale evenly and move, and so do their
/// products, so this is exact.
fn placement(local: Affine, width: f64, height: f64) -> Map<String, Value> {
    let [a, b, ..] = local.0;
    let center = local.apply(Point::new(width / 2.0, height / 2.0));
    let mut fields = Map::new();
    fields.insert("x".into(), json!(tidy(center.x - width / 2.0)));
    fields.insert("y".into(), json!(tidy(center.y - height / 2.0)));
    fields.insert("rotation".into(), json!(tidy(b.atan2(a).to_degrees())));
    fields.insert("scale".into(), json!(tidy(a.hypot(b))));
    fields
}

/// The transform that takes artboard coordinates into the coordinates of
/// `parent`'s children. Identity when the parent is flattened to nothing.
fn into_parent(document: &Document, parent: &NodeId) -> Affine {
    document
        .world_transform(parent)
        .and_then(|world| world.invert())
        .unwrap_or(Affine::IDENTITY)
}

/// Drops ids that are inside another listed node, and repeats.
fn top_level<'a>(document: &Document, ids: &'a [NodeId]) -> Vec<&'a NodeId> {
    let listed: HashSet<&NodeId> = ids.iter().collect();
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(*id))
        .filter(|id| {
            !document
                .ancestors(id)
                .any(|ancestor| listed.contains(&ancestor.id))
        })
        .collect()
}

/// Sorts ids into tree order: back to front for layers.
fn in_tree_order<'a>(document: &Document, ids: Vec<&'a NodeId>) -> Vec<&'a NodeId> {
    let position: HashMap<&NodeId, usize> = document
        .nodes()
        .enumerate()
        .map(|(position, node)| (&node.id, position))
        .collect();
    let mut ids = ids;
    ids.sort_by_key(|id| position.get(id).copied().unwrap_or(usize::MAX));
    ids
}

/// Edits that change the place of a node among its siblings.
///
/// Returns no edits when the node is already there.
pub fn reorder(document: &Document, id: &NodeId, step: Reorder) -> Result<Vec<Edit>, BuildError> {
    let node = existing(document, id)?;
    let siblings = document.siblings(id);
    let Some(at) = siblings.iter().position(|sibling| sibling == id) else {
        return Ok(Vec::new());
    };
    let place = match step {
        Reorder::ToFront if at + 1 < siblings.len() => Place::Last,
        Reorder::ToBack if at > 0 => Place::First,
        Reorder::Forward if at + 1 < siblings.len() => Place::After(siblings[at + 1].clone()),
        Reorder::Backward if at > 0 => Place::Before(siblings[at - 1].clone()),
        _ => return Ok(Vec::new()),
    };
    let index = index_at(
        document,
        node.parent.as_ref(),
        node.family(),
        &place,
        Some(id),
    )?;
    Ok(vec![Edit::set(id, "index", index)])
}

/// Edits that move a node under `parent` at `place`.
///
/// With `keep_position`, a layer keeps its place, turn and size on the
/// artboard: its `x`, `y`, `rotation` and `scale` are rewritten for the new
/// parent. Without it those fields are left alone and the layer jumps.
///
/// When a layer moves to another artboard, the animations that target it or
/// its descendants move to the end of that artboard's timeline with it.
pub fn move_node(
    document: &Document,
    id: &NodeId,
    parent: &NodeId,
    place: &Place,
    keep_position: bool,
) -> Result<Vec<Edit>, BuildError> {
    let node = existing(document, id)?;
    let parent_node = existing(document, parent)?;
    if node.node_type() == NodeType::Artboard {
        return Err(BuildError::Artboard(
            node.label(),
            "moved into another node",
        ));
    }
    check_holds(parent_node, node.node_type(), node.label())?;
    if id == parent || document.is_ancestor(id, parent) {
        return Err(BuildError::IntoItself(node.label()));
    }
    let mut patch = Map::new();
    let index = index_at(document, Some(parent), node.family(), place, Some(id))?;
    if node.parent.as_ref() != Some(parent) {
        patch.insert("parent".into(), json!(parent));
    }
    patch.insert("index".into(), json!(index));
    if keep_position
        && node.parent.as_ref() != Some(parent)
        && let (Some(layer), Some(world)) = (node.layer(), document.world_transform(id))
    {
        let local = into_parent(document, parent) * world;
        patch.extend(placement(local, layer.width, layer.height));
    }
    let mut edits = vec![Edit::update(id, patch)];

    // Animations cannot reach into another artboard, so they come along.
    let from = document.artboard_of(id).map(|artboard| &artboard.id);
    let to = document.artboard_of(parent).map(|artboard| &artboard.id);
    if let (true, Some(from), Some(to)) = (node.is_layer(), from, to)
        && from != to
    {
        let mut moved: HashSet<&NodeId> = HashSet::from([id]);
        moved.extend(document.walk(id, Family::Layer).map(|(_, node)| &node.id));
        let followers: Vec<&Node> = document
            .walk(from, Family::Animation)
            .map(|(_, node)| node)
            .filter(|node| node.target().is_some_and(|target| moved.contains(target)))
            .collect();
        let indices = indices_at(
            document,
            Some(to),
            Family::Animation,
            &Place::Last,
            followers.len(),
            &[],
        )?;
        for (follower, index) in followers.into_iter().zip(indices) {
            edits.push(Edit::update(
                &follower.id,
                [("parent", json!(to)), ("index", json!(index))],
            ));
        }
    }
    Ok(edits)
}

/// Edits that wrap layers in a new group.
///
/// The group takes the place of the frontmost of the layers, and its box is
/// the smallest one around all of them. The layers keep their place on the
/// artboard and their order. Layers inside another listed layer come along
/// with it. Returns the id of the group with the edits.
pub fn group(
    document: &Document,
    ids: &[NodeId],
    kind: GroupKind,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let ids = in_tree_order(document, top_level(document, ids));
    let front = *ids.last().ok_or(BuildError::Empty)?;
    let mut artboards = HashSet::new();
    for id in &ids {
        let node = layer_of(document, id)?;
        if node.node_type() == NodeType::Artboard {
            return Err(BuildError::Artboard(node.label(), "put in a group"));
        }
        artboards.insert(document.artboard_of(id).map(|artboard| &artboard.id));
    }
    if artboards.len() > 1 {
        return Err(BuildError::DifferentArtboards);
    }
    let front_node = existing(document, front)?;
    let parent = front_node
        .parent
        .clone()
        .ok_or_else(|| BuildError::UnknownNode(front.clone()))?;

    // The box around every layer, measured in the new parent's coordinates.
    let to_parent = into_parent(document, &parent);
    let corners = ids.iter().flat_map(|id| {
        let world = document.world_transform(id).unwrap_or(Affine::IDENTITY);
        let size = document
            .get(id)
            .and_then(Node::layer)
            .map_or(Rect::default(), |layer| {
                Rect::new(0.0, 0.0, layer.width, layer.height)
            });
        size.corners()
            .map(|corner| (to_parent * world).apply(corner))
    });
    let frame = Rect::bounding(corners).unwrap_or_default();
    let frame = Rect::new(
        tidy(frame.x),
        tidy(frame.y),
        tidy(frame.width),
        tidy(frame.height),
    );

    let (name, data): (&str, NodeData) = match kind {
        GroupKind::Group => ("Group", Group::default().into()),
        GroupKind::Mask => ("Mask group", MaskGroup::default().into()),
    };
    let mut node = Node::new(data)
        .with_parent(parent)
        .with_index(front_node.index.clone());
    if let Some(layer) = node.data.layer_mut() {
        *layer = LayerProps::new(name, frame.x, frame.y, frame.width, frame.height);
    }
    let group_id = node.id.clone();
    let mut edits = vec![Edit::add(node)];

    let into_group = Affine::translate(-frame.x, -frame.y) * to_parent;
    let indices = index::keys_between(None, None, ids.len())?;
    for (id, index) in ids.into_iter().zip(indices) {
        let mut patch = Map::new();
        patch.insert("parent".into(), json!(group_id));
        patch.insert("index".into(), json!(index));
        if let (Some(layer), Some(world)) = (
            document.get(id).and_then(Node::layer),
            document.world_transform(id),
        ) {
            patch.extend(placement(into_group * world, layer.width, layer.height));
        }
        edits.push(Edit::update(id, patch));
    }
    Ok((group_id, edits))
}

/// Edits that put animations in a new timeline folder.
///
/// The folder takes the row of the first of the animations. Returns the id
/// of the folder with the edits.
pub fn group_animations(
    document: &Document,
    ids: &[NodeId],
    name: &str,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let ids = in_tree_order(document, top_level(document, ids));
    let first = *ids.first().ok_or(BuildError::Empty)?;
    let mut artboards = HashSet::new();
    for id in &ids {
        let node = existing(document, id)?;
        if !node.is_animation() {
            return Err(BuildError::NotAnAnimation(node.label()));
        }
        artboards.insert(document.artboard_of(id).map(|artboard| &artboard.id));
    }
    if artboards.len() > 1 {
        return Err(BuildError::DifferentArtboards);
    }
    let first_node = existing(document, first)?;
    let parent = first_node
        .parent
        .clone()
        .ok_or_else(|| BuildError::UnknownNode(first.clone()))?;
    let folder = Node::new(AnimationGroup {
        name: name.to_owned(),
    })
    .with_parent(parent)
    .with_index(first_node.index.clone());
    let folder_id = folder.id.clone();
    let mut edits = vec![Edit::add(folder)];
    let indices = index::keys_between(None, None, ids.len())?;
    for (id, index) in ids.into_iter().zip(indices) {
        edits.push(Edit::update(
            id,
            [("parent", json!(folder_id)), ("index", json!(index))],
        ));
    }
    Ok((folder_id, edits))
}

/// Edits that dissolve a group, a mask group or an animation folder.
///
/// The children take the group's place among its siblings, in their order.
/// Layers keep their place, turn and size on the artboard. What the group
/// itself added is lost with it: its opacity, blur, shadow, clip and mask,
/// and the animations that targeted it.
pub fn ungroup(document: &Document, id: &NodeId) -> Result<Vec<Edit>, BuildError> {
    let node = existing(document, id)?;
    let family = match node.node_type() {
        NodeType::Group | NodeType::MaskGroup => Family::Layer,
        NodeType::AnimationGroup => Family::Animation,
        _ => return Err(BuildError::NotAGroup(node.label())),
    };
    let parent = node
        .parent
        .clone()
        .ok_or_else(|| BuildError::UnknownNode(id.clone()))?;
    let children: Vec<&Node> = document.child_nodes(id, family).collect();
    let indices = indices_at(
        document,
        Some(&parent),
        family,
        &Place::After(id.clone()),
        children.len(),
        &[],
    )?;
    let group_transform = node.layer().map(LayerProps::transform);
    let mut edits = Vec::with_capacity(children.len() + 1);
    for (child, index) in children.into_iter().zip(indices) {
        let mut patch = Map::new();
        patch.insert("parent".into(), json!(parent));
        patch.insert("index".into(), json!(index));
        if let (Some(layer), Some(outer)) = (child.layer(), group_transform) {
            patch.extend(placement(
                outer * layer.transform(),
                layer.width,
                layer.height,
            ));
        }
        edits.push(Edit::update(&child.id, patch));
    }
    edits.push(Edit::remove(id));
    Ok(edits)
}

/// Edits that remove nodes, leaving out the ones that go anyway because a
/// listed node contains them or is their target.
pub fn remove(document: &Document, ids: &[NodeId]) -> Vec<Edit> {
    let listed: HashSet<&NodeId> = ids.iter().collect();
    top_level(document, ids)
        .into_iter()
        .filter(|id| document.contains(id))
        .filter(|id| {
            // An animation goes with its target, or with a listed layer
            // that contains its target.
            document
                .get(id)
                .and_then(Node::target)
                .is_none_or(|target| {
                    !listed.contains(target)
                        && !document
                            .ancestors(target)
                            .any(|ancestor| listed.contains(&ancestor.id))
                })
        })
        .map(Edit::remove)
        .collect()
}

/// A copy of `node` under a fresh id, recorded in `ids`.
fn copy_of(node: &Node, ids: &mut HashMap<NodeId, NodeId>) -> Node {
    let copy = node.clone().with_id(NodeId::generate());
    ids.insert(node.id.clone(), copy.id.clone());
    copy
}

/// Edits that copy a layer with everything in it and every animation that
/// targets the layer or something in it.
///
/// The copy of `root` goes under `parent` at `place` (for an artboard both
/// are ignored: the copy is placed to the right of the existing artboards).
/// Copied animations go to the end of the timeline of the copy's artboard;
/// when a whole artboard is copied its timeline is copied as it is, folders
/// included.
///
/// Returns the edits with a map from every copied node's id to the id of
/// its copy.
pub fn clone_subtree(
    document: &Document,
    root: &NodeId,
    parent: &NodeId,
    place: &Place,
) -> Result<(Vec<Edit>, HashMap<NodeId, NodeId>), BuildError> {
    let source = layer_of(document, root)?;
    let mut ids = HashMap::new();
    let mut copies = Vec::new();

    let mut top = copy_of(source, &mut ids);
    if source.node_type() == NodeType::Artboard {
        top.index = last_index(document, None, Family::Layer)?;
        if let Some(layer) = top.data.layer_mut() {
            (layer.x, layer.y) = free_canvas_spot(document);
        }
    } else {
        check_holds(existing(document, parent)?, top.node_type(), source.label())?;
        top.parent = Some(parent.clone());
        top.index = index_at(document, Some(parent), Family::Layer, place, None)?;
    }
    copies.push(top);
    for (_, layer) in document.walk(root, Family::Layer) {
        copies.push(copy_of(layer, &mut ids));
    }

    if source.node_type() == NodeType::Artboard {
        for (_, animation) in document.walk(root, Family::Animation) {
            copies.push(copy_of(animation, &mut ids));
        }
    } else {
        let artboard = |id: &NodeId| document.artboard_of(id).map(|artboard| artboard.id.clone());
        let (Some(from), Some(to)) = (artboard(root), artboard(parent)) else {
            return Err(BuildError::UnknownNode(parent.clone()));
        };
        let followers: Vec<&Node> = document
            .walk(&from, Family::Animation)
            .map(|(_, node)| node)
            .filter(|node| node.target().is_some_and(|target| ids.contains_key(target)))
            .collect();
        let indices = indices_at(
            document,
            Some(&to),
            Family::Animation,
            &Place::Last,
            followers.len(),
            &[],
        )?;
        for (follower, index) in followers.into_iter().zip(indices) {
            let mut copy = copy_of(follower, &mut ids);
            copy.parent = Some(to.clone());
            copy.index = index;
            copies.push(copy);
        }
    }

    // Point every copy at the copies of its parent and target. The top copy
    // has its parent set already, and that parent is not a copy.
    for copy in copies.iter_mut().skip(1) {
        if let Some(parent) = copy.parent.as_ref().and_then(|parent| ids.get(parent)) {
            copy.parent = Some(parent.clone());
        }
    }
    for copy in &mut copies {
        if let Some(target) = copy.target().and_then(|target| ids.get(target)) {
            copy.data.set_target(target.clone());
        }
    }
    Ok((copies.into_iter().map(Edit::add).collect(), ids))
}

/// Edits that duplicate nodes in place.
///
/// A layer is copied with everything in it and with the animations that
/// target it, and the copy sits just in front of the original. An artboard
/// is copied whole, to the right of the others. An animation is copied onto
/// the next timeline row, aimed at the same layer; a folder is copied with
/// its animations. Nodes inside another listed node are copied once, with it.
///
/// Returns the edits with a map from every copied node's id to the id of
/// its copy.
pub fn duplicate(
    document: &Document,
    ids: &[NodeId],
) -> Result<(Vec<Edit>, HashMap<NodeId, NodeId>), BuildError> {
    let mut edits = Vec::new();
    let mut map = HashMap::new();
    // Work on a scratch copy so that each duplicate sees the ones before it,
    // which keeps their indices and canvas spots from colliding.
    let mut scratch = document.clone();
    for id in top_level(document, ids) {
        let node = existing(&scratch, id)?;
        let (step, step_map) = match (node.family(), node.parent.clone()) {
            (Family::Layer, None) => clone_subtree(&scratch, id, id, &Place::Last)?,
            (Family::Layer, Some(parent)) => {
                clone_subtree(&scratch, id, &parent, &Place::After(id.clone()))?
            }
            (Family::Animation, parent) => duplicate_animation(&scratch, node, parent.as_ref())?,
        };
        // The helpers above only ever produce valid batches.
        if scratch.apply(&step).is_ok() {
            edits.extend(step);
            map.extend(step_map);
        }
    }
    if edits.is_empty() && ids.is_empty() {
        return Err(BuildError::Empty);
    }
    Ok((edits, map))
}

/// Copies an animation, or a folder with what is in it, onto the next row.
fn duplicate_animation(
    document: &Document,
    node: &Node,
    parent: Option<&NodeId>,
) -> Result<(Vec<Edit>, HashMap<NodeId, NodeId>), BuildError> {
    let mut ids = HashMap::new();
    let mut top = copy_of(node, &mut ids);
    top.index = index_at(
        document,
        parent,
        Family::Animation,
        &Place::After(node.id.clone()),
        None,
    )?;
    let mut copies = vec![top];
    for (_, inner) in document.walk(&node.id, Family::Animation) {
        let mut copy = copy_of(inner, &mut ids);
        copy.parent = copy.parent.and_then(|parent| ids.get(&parent).cloned());
        copies.push(copy);
    }
    Ok((copies.into_iter().map(Edit::add).collect(), ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn add(value: Value) -> Edit {
        Edit::add(Node::from_value(value).unwrap())
    }

    /// One artboard: a turned group "card" with two rects, a loose rect in
    /// front of it, a fade on "r1", a move on "card" in a folder. A second,
    /// empty artboard.
    fn sample() -> Document {
        let mut document = Document::new("Sample");
        document
            .apply(&[
                add(json!({ "id": "ab", "index": "a0", "type": "artboard", "width": 800, "height": 600 })),
                add(json!({ "id": "ab2", "index": "a1", "type": "artboard", "x": 900, "width": 400, "height": 300 })),
                add(json!({ "id": "card", "parent": "ab", "index": "a0", "type": "group",
                            "x": 100, "y": 100, "width": 200, "height": 200, "rotation": 90 })),
                add(json!({ "id": "r1", "parent": "card", "index": "a0", "type": "rect",
                            "x": 0, "y": 0, "width": 100, "height": 50 })),
                add(json!({ "id": "r2", "parent": "card", "index": "a1", "type": "rect",
                            "x": 50, "y": 75, "width": 100, "height": 50, "scale": 2 })),
                add(json!({ "id": "loose", "parent": "ab", "index": "a1", "type": "rect",
                            "x": 500, "y": 400, "width": 60, "height": 40 })),
                add(json!({ "id": "fade", "parent": "ab", "index": "a0", "type": "fade_in",
                            "target": "r1", "start": 0, "end": 500 })),
                add(json!({ "id": "folder", "parent": "ab", "index": "a1", "type": "animation_group", "name": "Intro" })),
                add(json!({ "id": "move", "parent": "folder", "index": "a0", "type": "move",
                            "target": "card", "start": 0, "end": 500, "to": { "x": 100 } })),
            ])
            .unwrap();
        document
    }

    /// Applies edits to a copy and checks that they undo cleanly.
    fn applied(document: &Document, edits: &[Edit]) -> Document {
        let mut result = document.clone();
        let inverse = result.apply(edits).unwrap();
        result.validate().unwrap();
        let mut undone = result.clone();
        undone.apply(&inverse).unwrap();
        assert_eq!(&undone, document);
        result
    }

    fn close(a: Affine, b: Affine) -> bool {
        a.0.iter().zip(b.0).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    fn order(document: &Document, parent: &str, family: Family) -> Vec<String> {
        document
            .children(&id(parent), family)
            .iter()
            .map(NodeId::to_string)
            .collect()
    }

    #[test]
    fn every_layer_type_has_a_valid_default() {
        let mut document = sample();
        for ty in NodeType::ALL
            .iter()
            .filter(|ty| ty.family() == Family::Layer)
        {
            let data = default_layer(*ty).unwrap();
            assert_eq!(data.node_type(), *ty);
            let edits = if *ty == NodeType::Artboard {
                add_artboard(&document, "Next", 640.0, 360.0).unwrap().1
            } else {
                add_layer(&document, &id("ab"), data).unwrap().1
            };
            document = applied(&document, &edits);
        }
        assert!(default_layer(NodeType::FadeIn).is_none());
        // Each new layer went in front of the one before.
        let layers = document.children(&id("ab"), Family::Layer);
        assert_eq!(layers.len(), 2 + 8);
        assert_eq!(layers[..2], [id("card"), id("loose")]);
    }

    #[test]
    fn every_animation_type_has_a_valid_default() {
        let mut document = sample();
        let (text, edits) =
            add_layer(&document, &id("ab"), default_layer(NodeType::Text).unwrap()).unwrap();
        document = applied(&document, &edits);
        for ty in NodeType::ALL
            .iter()
            .filter(|ty| ty.family() == Family::Animation)
        {
            if *ty == NodeType::AnimationGroup {
                assert!(default_animation(*ty, &text, 0.0).is_none());
                continue;
            }
            let target = if animation_applies_to(*ty, NodeType::Text) {
                text.clone()
            } else {
                id("r1")
            };
            let data = default_animation(*ty, &target, 250.0).unwrap();
            assert_eq!(data.node_type(), *ty);
            let (start, end) = data.span().unwrap();
            assert_eq!(start, 250.0);
            assert_eq!(end - start, default_duration(*ty));
            let (new, edits) = add_animation(&document, data, None).unwrap();
            document = applied(&document, &edits);
            assert_eq!(
                document.children(&id("ab"), Family::Animation).last(),
                Some(&new)
            );
        }
        assert!(default_animation(NodeType::Rect, &text, 0.0).is_none());
        assert_eq!(default_easing(NodeType::SlideIn), Easing::slow_down());
        assert_eq!(default_easing(NodeType::SlideOut), Easing::accelerate());
        assert_eq!(default_easing(NodeType::Move), Easing::smooth());
        assert_eq!(default_duration(NodeType::Show), 0.0);
    }

    #[test]
    fn adding_checks_parents_and_targets() {
        let document = sample();
        let rect = || default_layer(NodeType::Rect).unwrap();
        assert!(matches!(
            add_layer(&document, &id("r1"), rect()),
            Err(BuildError::CannotHold { .. })
        ));
        assert!(matches!(
            add_layer(&document, &id("nope"), rect()),
            Err(BuildError::UnknownNode(_))
        ));
        assert!(matches!(
            add_layer(
                &document,
                &id("ab"),
                default_animation(NodeType::FadeIn, &id("r1"), 0.0).unwrap()
            ),
            Err(BuildError::NotALayer(_))
        ));
        assert!(matches!(
            add_animation(&document, rect(), None),
            Err(BuildError::NotAnAnimation(_))
        ));
        let on = |ty: NodeType, target: &str| default_animation(ty, &id(target), 0.0).unwrap();
        assert!(matches!(
            add_animation(&document, on(NodeType::TextIn, "r1"), None),
            Err(BuildError::WrongTarget { .. })
        ));
        assert!(matches!(
            add_animation(&document, on(NodeType::FadeIn, "fade"), None),
            Err(BuildError::NotALayer(_))
        ));
        assert!(matches!(
            add_animation(&document, on(NodeType::FadeIn, "r1"), Some(&id("card"))),
            Err(BuildError::CannotHold { .. })
        ));
        let (_, edits) =
            add_animation(&document, on(NodeType::FadeIn, "r1"), Some(&id("folder"))).unwrap();
        let result = applied(&document, &edits);
        assert_eq!(result.children(&id("folder"), Family::Animation).len(), 2);
    }

    #[test]
    fn new_artboards_line_up_to_the_right() {
        let document = sample();
        let (new, edits) = add_artboard(&document, "Third", 500.0, 500.0).unwrap();
        let result = applied(&document, &edits);
        let layer = result.get(&new).unwrap().layer().unwrap();
        assert_eq!((layer.x, layer.y), (900.0 + 400.0 + ARTBOARD_GAP, 0.0));
        assert_eq!(result.artboards().last(), Some(&new));

        let (_, edits) = add_artboard(&Document::new("Empty"), "First", 500.0, 500.0).unwrap();
        let Edit::Add { node } = &edits[0] else {
            panic!()
        };
        assert_eq!((node.layer().unwrap().x, node.index.as_str()), (0.0, "a0"));
    }

    #[test]
    fn indices_land_at_the_requested_place() {
        let document = sample();
        let at = |place: Place| {
            index_at(&document, Some(&id("card")), Family::Layer, &place, None).unwrap()
        };
        assert_eq!(at(Place::First), "Zz");
        assert_eq!(at(Place::Last), "a2");
        assert_eq!(at(Place::Before(id("r2"))), "a0V");
        assert_eq!(at(Place::After(id("r1"))), "a0V");
        assert_eq!(at(Place::After(id("r2"))), "a2");
        assert_eq!(last_index(&document, None, Family::Layer).unwrap(), "a2");
        assert_eq!(
            last_index(&document, Some(&id("r1")), Family::Layer).unwrap(),
            "a0"
        );
        assert_eq!(
            indices_at(
                &document,
                Some(&id("card")),
                Family::Layer,
                &Place::Last,
                3,
                &[]
            )
            .unwrap(),
            ["a2", "a3", "a4"]
        );
        assert!(matches!(
            index_at(
                &document,
                Some(&id("card")),
                Family::Layer,
                &Place::After(id("loose")),
                None
            ),
            Err(BuildError::NotASibling(_))
        ));
        // A node that is being moved does not count as its own neighbour.
        assert_eq!(
            index_at(
                &document,
                Some(&id("card")),
                Family::Layer,
                &Place::Last,
                Some(&id("r2"))
            )
            .unwrap(),
            "a1"
        );
    }

    #[test]
    fn reorder_steps_through_the_siblings() {
        let mut document = sample();
        let (third, edits) = add_layer(
            &document,
            &id("card"),
            default_layer(NodeType::Star).unwrap(),
        )
        .unwrap();
        document = applied(&document, &edits);
        let third = third.to_string();
        let step = |document: &Document, target: &str, step: Reorder| {
            let edits = reorder(document, &id(target), step).unwrap();
            (applied(document, &edits), edits.len())
        };
        assert_eq!(
            order(&document, "card", Family::Layer),
            ["r1", "r2", third.as_str()]
        );

        let (document, _) = step(&document, "r1", Reorder::Forward);
        assert_eq!(
            order(&document, "card", Family::Layer),
            ["r2", "r1", third.as_str()]
        );
        let (document, _) = step(&document, "r1", Reorder::ToFront);
        assert_eq!(
            order(&document, "card", Family::Layer),
            ["r2", third.as_str(), "r1"]
        );
        let (document, changes) = step(&document, "r1", Reorder::Forward);
        assert_eq!(changes, 0, "already in front");
        let (document, _) = step(&document, "r1", Reorder::Backward);
        assert_eq!(
            order(&document, "card", Family::Layer),
            ["r2", "r1", third.as_str()]
        );
        let (document, _) = step(&document, &third, Reorder::ToBack);
        assert_eq!(
            order(&document, "card", Family::Layer),
            [third.as_str(), "r2", "r1"]
        );
        let (document, changes) = step(&document, &third, Reorder::ToBack);
        assert_eq!(changes, 0);
        // Timeline rows reorder the same way, and so do artboards.
        let (document, _) = step(&document, "fade", Reorder::ToFront);
        assert_eq!(
            order(&document, "ab", Family::Animation),
            ["folder", "fade"]
        );
        let (document, _) = step(&document, "ab", Reorder::Forward);
        assert_eq!(document.artboards(), [id("ab2"), id("ab")]);
        assert!(reorder(&document, &id("nope"), Reorder::ToFront).is_err());
    }

    #[test]
    fn moving_with_keep_position_preserves_the_world_transform() {
        let document = sample();
        let before = document.world_transform(&id("r2")).unwrap();
        let edits = move_node(&document, &id("r2"), &id("ab"), &Place::Last, true).unwrap();
        let moved = applied(&document, &edits);
        assert_eq!(order(&moved, "ab", Family::Layer), ["card", "loose", "r2"]);
        assert!(close(moved.world_transform(&id("r2")).unwrap(), before));
        let layer = moved.get(&id("r2")).unwrap().layer().unwrap();
        assert_eq!((layer.rotation, layer.scale), (90.0, 2.0));

        // And back into the turned group.
        let edits = move_node(&moved, &id("r2"), &id("card"), &Place::First, true).unwrap();
        let back = applied(&moved, &edits);
        assert!(close(back.world_transform(&id("r2")).unwrap(), before));
        assert_eq!(order(&back, "card", Family::Layer), ["r2", "r1"]);

        // Without keep_position the fields are left alone.
        let edits = move_node(&document, &id("r2"), &id("ab"), &Place::Last, false).unwrap();
        let jumped = applied(&document, &edits);
        assert_eq!(jumped.get(&id("r2")).unwrap().layer().unwrap().x, 50.0);
    }

    #[test]
    fn moving_within_a_parent_only_changes_the_index() {
        let document = sample();
        let edits = move_node(&document, &id("r2"), &id("card"), &Place::First, true).unwrap();
        assert_eq!(edits.len(), 1);
        let Edit::Update { patch, .. } = &edits[0] else {
            panic!()
        };
        assert_eq!(patch.keys().collect::<Vec<_>>(), ["index"]);
        let moved = applied(&document, &edits);
        assert_eq!(order(&moved, "card", Family::Layer), ["r2", "r1"]);
    }

    #[test]
    fn moving_to_another_artboard_takes_the_animations_along() {
        let document = sample();
        let edits = move_node(&document, &id("card"), &id("ab2"), &Place::Last, false).unwrap();
        let moved = applied(&document, &edits);
        assert_eq!(moved.artboard_of(&id("r1")).unwrap().id, id("ab2"));
        // Both the fade on "r1" (inside the card) and the move on the card
        // follow; the emptied folder stays behind.
        assert_eq!(order(&moved, "ab2", Family::Animation), ["fade", "move"]);
        assert_eq!(order(&moved, "ab", Family::Animation), ["folder"]);
    }

    #[test]
    fn moves_that_make_no_sense_are_refused() {
        let document = sample();
        let attempt = |node: &str, parent: &str| {
            move_node(&document, &id(node), &id(parent), &Place::Last, true).unwrap_err()
        };
        assert!(matches!(attempt("card", "card"), BuildError::IntoItself(_)));
        assert!(matches!(attempt("ab", "ab2"), BuildError::Artboard(..)));
        assert!(matches!(
            attempt("r1", "loose"),
            BuildError::CannotHold { .. }
        ));
        assert!(matches!(
            attempt("fade", "card"),
            BuildError::CannotHold { .. }
        ));
        assert!(matches!(
            attempt("r1", "folder"),
            BuildError::CannotHold { .. }
        ));
        assert!(matches!(attempt("nope", "ab"), BuildError::UnknownNode(_)));
        let mut nested = document.clone();
        let (inner, edits) = add_layer(
            &document,
            &id("card"),
            default_layer(NodeType::Group).unwrap(),
        )
        .unwrap();
        nested.apply(&edits).unwrap();
        assert!(matches!(
            move_node(&nested, &id("card"), &inner, &Place::Last, true),
            Err(BuildError::IntoItself(_))
        ));
        // An animation can move into a folder.
        let edits = move_node(&document, &id("fade"), &id("folder"), &Place::First, true).unwrap();
        let moved = applied(&document, &edits);
        assert_eq!(order(&moved, "folder", Family::Animation), ["fade", "move"]);
    }

    #[test]
    fn grouping_keeps_layers_where_they_are() {
        let document = sample();
        let world =
            |document: &Document, layer: &str| document.world_transform(&id(layer)).unwrap();
        let (group_id, edits) =
            group(&document, &[id("loose"), id("r2")], GroupKind::Group).unwrap();
        let grouped = applied(&document, &edits);

        // The group replaces the frontmost layer, "loose", in the artboard.
        assert_eq!(
            order(&grouped, "ab", Family::Layer),
            ["card", group_id.as_str()]
        );
        assert_eq!(
            order(&grouped, group_id.as_str(), Family::Layer),
            ["r2", "loose"]
        );
        for layer in ["r2", "loose"] {
            assert!(
                close(world(&grouped, layer), world(&document, layer)),
                "{layer}"
            );
        }
        // Its box wraps both: "r2" covers x 150..250, y 100..300 on the
        // artboard (turned and doubled), "loose" 500..560, 400..440.
        let frame = grouped.get(&group_id).unwrap().layer().unwrap().frame();
        assert_eq!(frame, Rect::new(150.0, 100.0, 410.0, 340.0));
        assert_eq!(grouped.get(&group_id).unwrap().name(), "Group");
    }

    #[test]
    fn grouping_inside_a_turned_parent_measures_in_that_parent() {
        let document = sample();
        let before = document.world_transform(&id("r1")).unwrap();
        let (group_id, edits) = group(&document, &[id("r1")], GroupKind::Mask).unwrap();
        let grouped = applied(&document, &edits);
        assert_eq!(
            grouped.get(&group_id).unwrap().node_type(),
            NodeType::MaskGroup
        );
        assert_eq!(grouped.get(&group_id).unwrap().parent, Some(id("card")));
        assert_eq!(
            grouped.get(&group_id).unwrap().layer().unwrap().frame(),
            Rect::new(0.0, 0.0, 100.0, 50.0)
        );
        assert!(close(grouped.world_transform(&id("r1")).unwrap(), before));
        // The fade still finds its target.
        assert_eq!(grouped.animations_targeting(&id("r1")).len(), 1);
    }

    #[test]
    fn grouping_refuses_odd_selections() {
        let document = sample();
        assert_eq!(
            group(&document, &[], GroupKind::Group).unwrap_err(),
            BuildError::Empty
        );
        assert!(matches!(
            group(&document, &[id("ab")], GroupKind::Group),
            Err(BuildError::Artboard(..))
        ));
        assert!(matches!(
            group(&document, &[id("fade")], GroupKind::Group),
            Err(BuildError::NotALayer(_))
        ));
        let mut two = document.clone();
        let (far, edits) = add_layer(
            &document,
            &id("ab2"),
            default_layer(NodeType::Rect).unwrap(),
        )
        .unwrap();
        two.apply(&edits).unwrap();
        assert_eq!(
            group(&two, &[id("r1"), far], GroupKind::Group).unwrap_err(),
            BuildError::DifferentArtboards
        );
        // A layer listed together with its parent is grouped once, with it.
        let (group_id, edits) = group(
            &document,
            &[id("r1"), id("card"), id("card")],
            GroupKind::Group,
        )
        .unwrap();
        let grouped = applied(&document, &edits);
        assert_eq!(order(&grouped, group_id.as_str(), Family::Layer), ["card"]);
        assert_eq!(grouped.get(&id("r1")).unwrap().parent, Some(id("card")));
    }

    #[test]
    fn ungrouping_puts_children_in_the_groups_place() {
        let document = sample();
        let world =
            |document: &Document, layer: &str| document.world_transform(&id(layer)).unwrap();
        let edits = ungroup(&document, &id("card")).unwrap();
        let flat = applied(&document, &edits);
        assert_eq!(order(&flat, "ab", Family::Layer), ["r1", "r2", "loose"]);
        for layer in ["r1", "r2"] {
            assert!(
                close(world(&flat, layer), world(&document, layer)),
                "{layer}"
            );
        }
        assert_eq!(flat.get(&id("r2")).unwrap().layer().unwrap().rotation, 90.0);
        assert!(!flat.contains(&id("card")));
        assert!(!flat.contains(&id("move")), "the move targeted the group");
        assert!(flat.contains(&id("fade")));

        // Grouping and ungrouping again is a round trip for the layers.
        let (group_id, edits) =
            group(&document, &[id("loose"), id("card")], GroupKind::Group).unwrap();
        let grouped = applied(&document, &edits);
        let again = applied(&grouped, &ungroup(&grouped, &group_id).unwrap());
        assert_eq!(order(&again, "ab", Family::Layer), ["card", "loose"]);
        for layer in ["card", "loose", "r1"] {
            assert!(
                close(world(&again, layer), world(&document, layer)),
                "{layer}"
            );
        }
        assert!(matches!(
            ungroup(&document, &id("r1")),
            Err(BuildError::NotAGroup(_))
        ));
    }

    #[test]
    fn timeline_folders_group_and_ungroup() {
        let document = sample();
        let (folder, edits) =
            group_animations(&document, &[id("move"), id("fade")], "Both").unwrap();
        let grouped = applied(&document, &edits);
        assert_eq!(
            order(&grouped, "ab", Family::Animation),
            [folder.as_str(), "folder"]
        );
        assert_eq!(
            order(&grouped, folder.as_str(), Family::Animation),
            ["fade", "move"]
        );
        assert_eq!(grouped.get(&folder).unwrap().name(), "Both");

        let flat = applied(&document, &ungroup(&document, &id("folder")).unwrap());
        assert_eq!(order(&flat, "ab", Family::Animation), ["fade", "move"]);
        assert!(matches!(
            group_animations(&document, &[id("r1")], "x"),
            Err(BuildError::NotAnAnimation(_))
        ));
        assert_eq!(
            group_animations(&document, &[], "x").unwrap_err(),
            BuildError::Empty
        );
    }

    #[test]
    fn removing_skips_what_goes_anyway() {
        let document = sample();
        let edits = remove(
            &document,
            &[
                id("r1"),
                id("card"),
                id("fade"),
                id("move"),
                id("loose"),
                id("nope"),
            ],
        );
        let removed: Vec<&str> = edits.iter().map(|edit| edit.node_id().as_str()).collect();
        assert_eq!(removed, ["card", "loose"]);
        let result = applied(&document, &edits);
        assert_eq!(result.len(), 3);
        // An animation whose target stays is removed on its own.
        assert_eq!(remove(&document, &[id("fade")]), [Edit::remove("fade")]);
    }

    #[test]
    fn cloning_copies_layers_and_their_animations() {
        let document = sample();
        let (edits, map) = clone_subtree(&document, &id("card"), &id("ab"), &Place::Last).unwrap();
        let result = applied(&document, &edits);
        assert_eq!(map.len(), 5, "card, r1, r2, fade, move");
        let copy = |original: &str| result.get(&map[&id(original)]).unwrap();

        assert_eq!(
            order(&result, "ab", Family::Layer).last().unwrap(),
            map[&id("card")].as_str()
        );
        assert_eq!(copy("r1").parent, Some(map[&id("card")].clone()));
        assert_eq!(copy("r2").index, "a1");
        assert_eq!(copy("fade").target(), Some(&map[&id("r1")]));
        assert_eq!(copy("move").target(), Some(&map[&id("card")]));
        // Copied animations go to the end of the artboard's timeline.
        assert_eq!(
            order(&result, "ab", Family::Animation),
            [
                "fade",
                "folder",
                map[&id("fade")].as_str(),
                map[&id("move")].as_str()
            ]
        );
        // Apart from ids and placement the copies are identical.
        let strip = |node: &Node| {
            let mut value = node.to_value();
            for field in ["id", "parent", "index", "target"] {
                value.as_object_mut().unwrap().remove(field);
            }
            value
        };
        for original in ["card", "r1", "r2", "fade", "move"] {
            assert_eq!(
                strip(copy(original)),
                strip(document.get(&id(original)).unwrap())
            );
        }
        // The originals keep their animations.
        assert_eq!(result.animations_targeting(&id("r1")).len(), 1);
    }

    #[test]
    fn cloning_into_another_artboard_brings_the_animations() {
        let document = sample();
        let (edits, map) =
            clone_subtree(&document, &id("card"), &id("ab2"), &Place::First).unwrap();
        let result = applied(&document, &edits);
        assert_eq!(result.artboard_of(&map[&id("r1")]).unwrap().id, id("ab2"));
        assert_eq!(result.children(&id("ab2"), Family::Animation).len(), 2);
        assert!(matches!(
            clone_subtree(&document, &id("fade"), &id("ab"), &Place::Last),
            Err(BuildError::NotALayer(_))
        ));
        assert!(matches!(
            clone_subtree(&document, &id("card"), &id("r1"), &Place::Last),
            Err(BuildError::CannotHold { .. })
        ));
    }

    #[test]
    fn duplicating_places_copies_next_to_the_originals() {
        let document = sample();
        let (edits, map) = duplicate(&document, &[id("r1"), id("loose")]).unwrap();
        let result = applied(&document, &edits);
        assert_eq!(
            order(&result, "card", Family::Layer),
            ["r1", map[&id("r1")].as_str(), "r2"]
        );
        assert_eq!(
            order(&result, "ab", Family::Layer),
            ["card", "loose", map[&id("loose")].as_str()]
        );
        assert_eq!(result.animations_targeting(&map[&id("r1")]).len(), 1);
        assert!(duplicate(&document, &[]).is_err());
        assert!(duplicate(&document, &[id("nope")]).is_err());
    }

    #[test]
    fn duplicating_an_artboard_copies_its_whole_timeline() {
        let document = sample();
        let (edits, map) = duplicate(&document, &[id("ab"), id("r1")]).unwrap();
        let result = applied(&document, &edits);
        assert_eq!(map.len(), 8, "r1 is inside the artboard and copied with it");
        let copy = &map[&id("ab")];
        assert_eq!(result.artboards(), [id("ab"), id("ab2"), copy.clone()]);
        assert_eq!(
            result.get(copy).unwrap().layer().unwrap().x,
            1300.0 + ARTBOARD_GAP
        );
        // The folder structure is kept.
        assert_eq!(
            order(&result, copy.as_str(), Family::Animation),
            [map[&id("fade")].as_str(), map[&id("folder")].as_str()]
        );
        let moved = result.get(&map[&id("move")]).unwrap();
        assert_eq!(moved.parent, Some(map[&id("folder")].clone()));
        assert_eq!(moved.target(), Some(&map[&id("card")]));
    }

    #[test]
    fn duplicating_animations_and_folders() {
        let document = sample();
        let (edits, map) = duplicate(&document, &[id("fade"), id("folder")]).unwrap();
        let result = applied(&document, &edits);
        assert_eq!(
            order(&result, "ab", Family::Animation),
            [
                "fade",
                map[&id("fade")].as_str(),
                "folder",
                map[&id("folder")].as_str()
            ]
        );
        let copied_move = result.get(&map[&id("move")]).unwrap();
        assert_eq!(copied_move.parent, Some(map[&id("folder")].clone()));
        assert_eq!(
            copied_move.target(),
            Some(&id("card")),
            "same target as the original"
        );
        assert_eq!(result.animations_targeting(&id("r1")).len(), 2);
    }
}
