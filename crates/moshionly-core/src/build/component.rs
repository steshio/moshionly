//! Components: making one, placing instances of it, keeping them the size
//! of a resized component, and detaching an instance into a plain group.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde_json::{Map, Value, json};

use super::{
    BuildError, GroupKind, Place, check_holds, existing, group, index_at, indices_at, layer_of,
    tidy, top_level,
};
use crate::document::Document;
use crate::edit::{Edit, patched};
use crate::geometry::Point;
use crate::id::NodeId;
use crate::model::{
    AutoSize, CornerRadius, Family, Group, Instance, LayerProps, Node, NodeData, NodeType, Offset,
    SizeChange,
};

/// Edits that make a component.
///
/// A single `group` becomes a component in place: it keeps its id, its
/// children and the animations that target it. Other layers are wrapped in
/// a new component the way [`group`](super::group) wraps them in a group.
/// Artboards cannot be components, and a component cannot be inside
/// another one or hold one.
///
/// Returns the id of the component with the edits.
pub fn make_component(
    document: &Document,
    ids: &[NodeId],
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let top = top_level(document, ids);
    if top.is_empty() {
        return Err(BuildError::Empty);
    }
    for id in &top {
        let node = layer_of(document, id)?;
        match node.node_type() {
            NodeType::Artboard => {
                return Err(BuildError::Artboard(node.label(), "made into a component"));
            }
            NodeType::Component if top.len() == 1 => {
                return Err(BuildError::AlreadyAComponent(node.label()));
            }
            _ => {}
        }
        if let Some(outer) = document.enclosing_component(id) {
            return Err(BuildError::NestedComponent(format!(
                "{} is inside {}",
                node.label(),
                outer.label()
            )));
        }
        let held = std::iter::once(node)
            .chain(document.walk(id, Family::Layer).map(|(_, inner)| inner))
            .find(|inner| inner.node_type() == NodeType::Component);
        if let Some(held) = held {
            return Err(BuildError::NestedComponent(format!(
                "{} is already a component",
                held.label()
            )));
        }
    }
    if let [only] = top[..]
        && document.get(only).map(Node::node_type) == Some(NodeType::Group)
    {
        return Ok((only.clone(), vec![Edit::set(only, "type", "component")]));
    }
    group(document, ids, GroupKind::Component)
}

/// Edits that add an instance of `component` under `parent` at `place`,
/// with the top-left corner of its box at `position` in the parent's
/// coordinates.
///
/// The instance is named after the component and has its size, opacity,
/// blend mode, blur and shadow, so it looks like the component where it is
/// put. From then on those are the instance's own.
///
/// Returns the id of the instance with the edits.
pub fn add_instance(
    document: &Document,
    component: &NodeId,
    parent: &NodeId,
    place: &Place,
    position: Point,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let main = existing(document, component)?;
    let NodeData::Component(look) = &main.data else {
        return Err(BuildError::NotAComponent(main.label()));
    };
    let holder = existing(document, parent)?;
    check_holds(holder, NodeType::Instance, "an instance".into())?;
    let around = match holder.node_type() {
        NodeType::Component => Some(holder),
        _ => document.enclosing_component(parent),
    };
    if let Some(around) = around
        && (around.id == *component || document.component_shows(component, &around.id))
    {
        return Err(BuildError::ComponentLoop {
            component: main.label(),
            place: holder.label(),
        });
    }
    let instance = Instance {
        layer: LayerProps {
            name: look.layer.name.clone(),
            x: position.x,
            y: position.y,
            width: look.layer.width,
            height: look.layer.height,
            opacity: look.layer.opacity,
            blend_mode: look.layer.blend_mode,
            blur: look.layer.blur,
            shadow: look.layer.shadow,
            ..LayerProps::default()
        },
        component: component.clone(),
        time_offset: 0.0,
    };
    let node = Node::new(instance).with_parent(parent).with_index(index_at(
        document,
        Some(parent),
        Family::Layer,
        place,
        None,
    )?);
    Ok((node.id.clone(), vec![Edit::add(node)]))
}

/// How near two sizes are to count as the same: a hundredth of a pixel,
/// the precision the editor keeps them to.
const SAME_SIZE: f64 = 0.01;

/// Edits that keep instances the size of their component when `edits`
/// resize it, so that what they show is not squeezed or stretched: an
/// instance as wide as the component was takes the width `edits` give it,
/// and the same for the height, each side on its own. An instance resized
/// by hand keeps its size, and so does a side of an instance that `edits`
/// set themselves. Instances that `edits` remove, or point at another
/// component, are left alone.
///
/// `document` is the document before `edits`. The edits returned go after
/// them, in the same batch, so that one undo step resizes the component and
/// its instances. A side is set even when `edits` leave the component that
/// size: a drag that applies edits made from the document it began with
/// at every step puts the instances back when it puts the component back.
pub fn instances_follow_mains(document: &Document, edits: &[Edit]) -> Vec<Edit> {
    const SIDES: [&str; 2] = ["width", "height"];
    // Each component that `edits` update, as they leave it, in the order
    // they first touch it; the sides they set on each node; and the nodes
    // they remove or point at another component.
    let mut mains: Vec<(&NodeId, Node)> = Vec::new();
    let mut set: HashMap<&NodeId, [bool; 2]> = HashMap::new();
    let mut left_alone: HashSet<&NodeId> = HashSet::new();
    for edit in edits {
        match edit {
            Edit::Update { id, patch } => {
                let sides = set.entry(id).or_default();
                for (side, field) in SIDES.iter().enumerate() {
                    sides[side] |= patch.contains_key(*field);
                }
                if patch.contains_key("component") {
                    left_alone.insert(id);
                }
                let at = match mains.iter().position(|(main, _)| *main == id) {
                    Some(at) => at,
                    None => match document.get(id) {
                        Some(node) if node.node_type() == NodeType::Component => {
                            mains.push((id, node.clone()));
                            mains.len() - 1
                        }
                        _ => continue,
                    },
                };
                // A patch that does not apply fails the batch anyway.
                if let Ok(after) = patched(&mains[at].1, patch) {
                    mains[at].1 = after;
                }
            }
            Edit::Remove { id } => {
                left_alone.insert(id);
            }
            Edit::Add { .. } => {}
        }
    }
    let same = |a: f64, b: f64| (a - b).abs() <= SAME_SIZE;
    let mut follow = Vec::new();
    for (id, after) in &mains {
        let (Some(was), Some(now)) = (document.get(id).and_then(Node::layer), after.layer()) else {
            continue;
        };
        let (was, now) = ([was.width, was.height], [now.width, now.height]);
        let resized = set.get(id).copied().unwrap_or_default();
        for instance in document.instances_of(id) {
            let gone = left_alone.contains(&instance.id)
                || document
                    .ancestors(&instance.id)
                    .any(|ancestor| left_alone.contains(&ancestor.id));
            let Some(own) = instance.layer().filter(|_| !gone) else {
                continue;
            };
            let own_set = set.get(&instance.id).copied().unwrap_or_default();
            let mut patch = Map::new();
            for (side, size) in [own.width, own.height].into_iter().enumerate() {
                if resized[side] && !own_set[side] && same(size, was[side]) {
                    patch.insert(SIDES[side].to_owned(), Value::from(now[side]));
                }
            }
            if !patch.is_empty() {
                follow.push(Edit::update(instance.id.clone(), patch));
            }
        }
    }
    follow
}

/// Edits that replace an instance with a plain group that looks the same.
///
/// The group takes the instance's place, name, box and effects, and the
/// animations that targeted the instance. It gets the component's look
/// (fill, stroke, corner radius, clip) and copies of the component's
/// layers. The animations on those layers are copied too, onto the end of
/// the instance's artboard's timeline, later by the instance's
/// `time_offset`. Instances inside the component stay instances, and their
/// `time_offset` grows by the instance's, so they play later by it too.
///
/// A copy resized in proportion is scaled to look exactly like the
/// instance. One resized out of proportion is stretched the way a resized
/// group is: positions and sizes stretch, stroke widths and font sizes
/// stay, so it can differ a little from the instance, whose content is
/// stretched as a picture. With a negative `time_offset`, an animation that
/// would start before the timeline does is squeezed into what is left of
/// it. [`detach_losses`] says beforehand whether either happens.
///
/// Returns the id of the new group with the edits.
pub fn detach(document: &Document, instance: &NodeId) -> Result<(NodeId, Vec<Edit>), BuildError> {
    detach_with(document, instance, &mut |_| NodeId::generate())
}

/// [`detach`], with the ids of the new nodes made by `fresh` from the ids
/// of the nodes they copy (or, for the group, of the instance).
fn detach_with(
    document: &Document,
    instance_id: &NodeId,
    fresh: &mut dyn FnMut(&NodeId) -> NodeId,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let node = existing(document, instance_id)?;
    let NodeData::Instance(instance) = &node.data else {
        return Err(BuildError::NotAnInstance(node.label()));
    };
    let main = existing(document, &instance.component)?;
    let NodeData::Component(component) = &main.data else {
        return Err(BuildError::NotAComponent(main.label()));
    };
    let parent = node
        .parent
        .clone()
        .ok_or_else(|| BuildError::UnknownNode(instance_id.clone()))?;
    let artboard_of = |id: &NodeId| {
        document
            .artboard_of(id)
            .map(|artboard| artboard.id.clone())
            .ok_or_else(|| BuildError::UnknownNode(id.clone()))
    };
    let artboard = artboard_of(instance_id)?;
    let main_artboard = artboard_of(&main.id)?;
    let fit = Fit::between(&component.layer, &instance.layer);

    // The group, in the instance's place, with the component's look.
    let group_id = fresh(instance_id);
    let mut look = Group::from(component.clone());
    look.layer = instance.layer.clone();
    fit.scale_look(&mut look);
    let mut edits = vec![Edit::add(Node {
        id: group_id.clone(),
        parent: Some(parent),
        index: node.index.clone(),
        data: look.into(),
    })];

    // Copies of the component's layers.
    let layers: Vec<&Node> = document
        .walk(&main.id, Family::Layer)
        .map(|(_, layer)| layer)
        .collect();
    let mut ids: HashMap<&NodeId, NodeId> = HashMap::from([(&main.id, group_id.clone())]);
    for layer in &layers {
        ids.insert(&layer.id, fresh(&layer.id));
    }
    let mut factors = HashMap::new();
    fit.factors(document, &main.id, &mut factors);
    for layer in &layers {
        let mut copy = (*layer).clone().with_id(ids[&layer.id].clone());
        copy.parent = layer
            .parent
            .as_ref()
            .and_then(|parent| ids.get(parent).cloned());
        if let Some(factors) = factors.get(&layer.id) {
            factors.apply_to_layer(&mut copy.data);
        }
        // An instance inside the component plays later by this one's
        // offset too, like the copied animations do.
        if let NodeData::Instance(inner) = &mut copy.data {
            inner.time_offset = tidy(inner.time_offset + instance.time_offset);
        }
        edits.push(Edit::add(copy));
    }

    // Copies of the animations on those layers, in the order they start so
    // that animations moved to the very start keep their order.
    let mut followers = inner_animations(document, &main.id, &main_artboard);
    let start = |node: &Node| node.data.span().map_or(0.0, |(start, _)| start);
    followers.sort_by(|a, b| start(a).total_cmp(&start(b)));
    let indices = indices_at(
        document,
        Some(&artboard),
        Family::Animation,
        &Place::Last,
        followers.len(),
        &[],
    )?;
    for (follower, index) in followers.into_iter().zip(indices) {
        let Some(target) = follower.target() else {
            continue;
        };
        let mut copy = follower.clone().with_id(fresh(&follower.id));
        copy.parent = Some(artboard.clone());
        copy.index = index;
        copy.data.set_target(ids[target].clone());
        shift(&mut copy.data, instance.time_offset);
        if let Some(factors) = factors.get(target) {
            factors.apply_to_animation(&mut copy.data);
        }
        edits.push(Edit::add(copy));
    }

    // The instance's own animations go over to the group, then it goes.
    for animation in document.animations_targeting(instance_id) {
        edits.push(Edit::set(&animation.id, "target", json!(group_id)));
    }
    edits.push(Edit::remove(instance_id));
    Ok((group_id, edits))
}

/// A way in which [`detach`] cannot keep exactly how an instance looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetachLoss {
    /// The instance is resized out of proportion. It stretches its content
    /// like a picture, while the group stretches its layers like a resized
    /// group, which keeps stroke widths and font sizes.
    Stretched,
    /// The instance's `time_offset` is negative, and an animation inside
    /// its component is under way when the instance starts. The group
    /// cannot start that animation before 0 ms, so it squeezes it into
    /// what is left of it.
    Squeezed,
}

impl fmt::Display for DetachLoss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DetachLoss::Stretched => {
                "it is resized out of proportion, so its layers are stretched like a resized \
                 group's, keeping stroke widths and font sizes"
            }
            DetachLoss::Squeezed => {
                "its time offset is negative, so the motion before 0 ms is squeezed into what \
                 is left of it"
            }
        })
    }
}

/// What [`detach`] cannot keep of how `instance` looks: nothing when the
/// group it makes looks the same at every time, or when `instance` is not
/// an instance of a component.
pub fn detach_losses(document: &Document, instance: &NodeId) -> Vec<DetachLoss> {
    let Some(NodeData::Instance(instance)) = document.get(instance).map(|node| &node.data) else {
        return Vec::new();
    };
    let Some(main) = document.get(&instance.component) else {
        return Vec::new();
    };
    let (NodeData::Component(component), Some(artboard)) =
        (&main.data, document.artboard_of(&main.id))
    else {
        return Vec::new();
    };
    let mut losses = Vec::new();
    if let Fit::Stretch(..) = Fit::between(&component.layer, &instance.layer) {
        losses.push(DetachLoss::Stretched);
    }
    let early = -instance.time_offset;
    let squeezed = early > 0.0
        && inner_animations(document, &main.id, &artboard.id)
            .iter()
            .filter_map(|animation| animation.data.span())
            .any(|(start, end)| start < early && end > early);
    if squeezed {
        losses.push(DetachLoss::Squeezed);
    }
    losses
}

/// The animations on the layers inside the component `main`, which live in
/// its artboard `artboard`. Those on the component itself are not part of
/// it.
fn inner_animations<'a>(document: &'a Document, main: &NodeId, artboard: &NodeId) -> Vec<&'a Node> {
    let inside: HashSet<&NodeId> = document
        .walk(main, Family::Layer)
        .map(|(_, layer)| &layer.id)
        .collect();
    document
        .walk(artboard, Family::Animation)
        .map(|(_, animation)| animation)
        .filter(|animation| {
            animation
                .target()
                .is_some_and(|target| inside.contains(target))
        })
        .collect()
}

/// Edits that detach every instance that would lose its component when
/// the nodes `removed` go, so that their look is kept as plain groups. A
/// component inside a removed node goes too. Instances that go themselves
/// are left alone. With the edits comes what detaching cannot keep of
/// each instance (see [`detach_with_losses`]).
pub(super) fn detach_orphans(
    document: &Document,
    removed: &[&NodeId],
) -> (Vec<Edit>, Vec<(NodeId, Vec<DetachLoss>)>) {
    let mut gone: HashSet<NodeId> = removed.iter().map(|id| (*id).clone()).collect();
    for id in removed {
        gone.extend(
            document
                .walk(id, Family::Layer)
                .map(|(_, node)| node.id.clone()),
        );
    }
    let doomed: HashSet<NodeId> = gone
        .iter()
        .filter(|id| document.get(id).map(Node::node_type) == Some(NodeType::Component))
        .cloned()
        .collect();
    detach_with_losses(document, &doomed, &gone)
}

/// Edits that detach every instance of the components `doomed`, except
/// those in `gone` or inside a node in it, one after the other so that
/// each sees the ones before. Detaching an instance can copy instances of
/// other doomed components out of its component; those are detached too.
pub(crate) fn detach_instances_of(
    document: &Document,
    doomed: &HashSet<NodeId>,
    gone: &HashSet<NodeId>,
) -> Vec<Edit> {
    detach_with_losses(document, doomed, gone).0
}

/// [`detach_instances_of`], with what detaching cannot keep of each
/// instance of `document` it detaches, in the order they are detached
/// (see [`detach_losses`]). An instance copied out of one while detaching
/// it is part of the group that takes its place, so what detaching the
/// copy cannot keep counts toward that one.
fn detach_with_losses(
    document: &Document,
    doomed: &HashSet<NodeId>,
    gone: &HashSet<NodeId>,
) -> (Vec<Edit>, Vec<(NodeId, Vec<DetachLoss>)>) {
    let mut losses: Vec<(NodeId, Vec<DetachLoss>)> = Vec::new();
    if doomed.is_empty() {
        return (Vec::new(), losses);
    }
    // The instance of `document` that each new node was copied for.
    let mut origin: HashMap<NodeId, NodeId> = HashMap::new();
    let mut scratch = document.clone();
    let mut edits = Vec::new();
    for _ in 0..document.len() * 4 + 16 {
        let next = scratch
            .nodes()
            .find(|node| {
                node.component().is_some_and(|shown| doomed.contains(shown))
                    && !gone.contains(&node.id)
                    && !scratch
                        .ancestors(&node.id)
                        .any(|ancestor| gone.contains(&ancestor.id))
            })
            .map(|node| node.id.clone());
        let Some(next) = next else {
            break;
        };
        let lost = detach_losses(&scratch, &next);
        let Ok((_, step)) = detach(&scratch, &next) else {
            break;
        };
        if scratch.apply(&step).is_err() {
            break;
        }
        let root = origin.get(&next).cloned().unwrap_or_else(|| next.clone());
        for edit in &step {
            if let Edit::Add { node } = edit {
                origin.insert(node.id.clone(), root.clone());
            }
        }
        match losses.iter_mut().find(|(id, _)| *id == root) {
            Some((_, known)) => {
                for loss in lost {
                    if !known.contains(&loss) {
                        known.push(loss);
                    }
                }
            }
            None => losses.push((root, lost)),
        }
        edits.extend(step);
    }
    (edits, losses)
}

/// A copy of `document` in which every instance is detached, for code that
/// cannot draw instances, such as exporters that convert the document
/// rather than its frames. It looks the same as `document`, within what
/// [`detach`] can keep; with it comes what that is not, by the id of the
/// instance, in the order the instances were detached.
///
/// Instances inside components are detached first, so every component's
/// content is copied once per instance and, in a valid document, every
/// instance detached is one of `document`. The new nodes get ids made from
/// the ids they copy, so the same document always gives the same copy.
pub fn without_instances(document: &Document) -> (Document, Vec<(NodeId, DetachLoss)>) {
    let mut copy = document.clone();
    let mut losses = Vec::new();
    for _ in 0..document.len() * 4 + 16 {
        let holds_instances = |component: &NodeId| {
            copy.walk(component, Family::Layer)
                .any(|(_, node)| node.node_type() == NodeType::Instance)
        };
        let instances: Vec<&Node> = copy
            .nodes()
            .filter(|node| node.node_type() == NodeType::Instance)
            .collect();
        let Some(next) = instances
            .iter()
            .find(|node| node.component().is_some_and(|main| !holds_instances(main)))
            .or(instances.first())
            .map(|node| node.id.clone())
        else {
            break;
        };
        let lost = detach_losses(&copy, &next);
        let mut fresh = |original: &NodeId| derived_id(&copy, &next, original);
        let done = detach_with(&copy, &next, &mut fresh)
            .ok()
            .is_some_and(|(_, edits)| copy.apply(&edits).is_ok());
        if done {
            losses.extend(lost.into_iter().map(|loss| (next.clone(), loss)));
        } else {
            // Only a broken document gets here; leave the instance out.
            let _ = copy.apply(&[Edit::remove(&next)]);
        }
    }
    (copy, losses)
}

/// An id for the copy of `original` made while detaching `instance`: the
/// same every time, and not in use in `document`.
fn derived_id(document: &Document, instance: &NodeId, original: &NodeId) -> NodeId {
    /// FNV-1a, which is short and the same on every platform and release.
    fn hash(parts: &[&str], salt: u64) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ salt;
        for part in parts {
            for byte in part.bytes().chain([0]) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        hash
    }
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    for salt in 0.. {
        let mut value = hash(&[instance.as_str(), original.as_str()], salt);
        let id: String = (0..12)
            .map(|_| {
                let digit = DIGITS[(value % 62) as usize];
                value /= 62;
                char::from(digit)
            })
            .collect();
        let id = NodeId::from(id);
        if !document.contains(&id) {
            return id;
        }
    }
    unreachable!("some salt gives a free id")
}

/// Moves an animation `offset` milliseconds later (earlier when negative),
/// not before the start of the timeline.
fn shift(data: &mut NodeData, offset: f64) {
    if offset == 0.0 {
        return;
    }
    let moved = |time: f64| tidy((time + offset).max(0.0));
    match data {
        NodeData::Show(instant) | NodeData::Hide(instant) => {
            instant.start = moved(instant.start);
            instant.end = instant.end.map(moved);
        }
        other => {
            if let Some(timing) = other.timing_mut() {
                timing.start = moved(timing.start);
                timing.end = moved(timing.end);
            }
        }
    }
}

/// How a component's content fits an instance's box.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fit {
    /// The boxes are the same size.
    Same,
    /// The instance is the component scaled evenly by this much.
    Even(f64),
    /// The instance is wider by the first and taller by the second.
    Stretch(f64, f64),
}

impl Fit {
    /// The fit of a component's box into an instance's, as evaluation
    /// scales it.
    fn between(component: &LayerProps, instance: &LayerProps) -> Self {
        let ratio = |size: f64, main: f64| {
            if main > 0.0 && (size / main).is_finite() {
                size / main
            } else {
                1.0
            }
        };
        let sx = ratio(instance.width, component.width);
        let sy = ratio(instance.height, component.height);
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0);
        if near(sx, 1.0) && near(sy, 1.0) {
            Fit::Same
        } else if near(sx, sy) {
            Fit::Even(sx)
        } else {
            Fit::Stretch(sx, sy)
        }
    }

    /// Scales what the group draws itself, when the content is scaled evenly.
    fn scale_look(self, look: &mut Group) {
        let Fit::Even(scale) = self else {
            return;
        };
        if let Some(stroke) = &mut look.stroke {
            stroke.width = tidy(stroke.width * scale);
        }
        look.corner_radius = match look.corner_radius {
            CornerRadius::Uniform(radius) => CornerRadius::Uniform(tidy(radius * scale)),
            CornerRadius::PerCorner(radii) => {
                CornerRadius::PerCorner(radii.map(|radius| tidy(radius * scale)))
            }
        };
    }

    /// What to multiply on each layer below `parent` (by its id) to fit.
    fn factors<'a>(
        self,
        document: &'a Document,
        parent: &NodeId,
        found: &mut HashMap<&'a NodeId, Factors>,
    ) {
        match self {
            Fit::Same => {}
            // Scaling every child evenly about the component's corner
            // scales everything inside it with them.
            Fit::Even(scale) => {
                for child in document.child_nodes(parent, Family::Layer) {
                    found.insert(
                        &child.id,
                        Factors {
                            outside: (scale, scale),
                            size: (1.0, 1.0),
                            scale,
                        },
                    );
                }
            }
            Fit::Stretch(sx, sy) => stretch(document, parent, (sx, sy), found),
        }
    }
}

/// The factors of a group stretched by `outside` across and down its own
/// axes, for every layer below `parent`, the way the editor stretches a
/// resized group: a turned layer is stretched along its own axes as nearly
/// as a box can be, and text whose box hugs its words keeps its size.
fn stretch<'a>(
    document: &'a Document,
    parent: &NodeId,
    outside: (f64, f64),
    found: &mut HashMap<&'a NodeId, Factors>,
) {
    let (sx, sy) = outside;
    for child in document.child_nodes(parent, Family::Layer) {
        let Some(layer) = child.layer() else {
            continue;
        };
        let (sin, cos) = layer.rotation.to_radians().sin_cos();
        let along = (sx * cos).hypot(sy * sin);
        let across = (sx * sin).hypot(sy * cos);
        let hugs = matches!(&child.data, NodeData::Text(text) if text.auto_size == AutoSize::WidthAndHeight);
        let size = if hugs { (1.0, 1.0) } else { (along, across) };
        found.insert(
            &child.id,
            Factors {
                outside,
                size,
                scale: 1.0,
            },
        );
        stretch(document, &child.id, (along, across), found);
    }
}

/// What fitting a component into an instance multiplies on one layer.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Factors {
    /// Lengths in the parent's coordinates, across and down: where the
    /// layer's centre is, and how far moves and slides take it.
    outside: (f64, f64),
    /// The layer's box, and the sizes resizes go to.
    size: (f64, f64),
    /// The layer's own scale.
    scale: f64,
}

impl Factors {
    fn apply_to_layer(&self, data: &mut NodeData) {
        let Some(layer) = data.layer_mut() else {
            return;
        };
        let center = layer.center();
        let width = layer.width * self.size.0;
        let height = layer.height * self.size.1;
        let center = Point::new(center.x * self.outside.0, center.y * self.outside.1);
        layer.x = tidy(center.x - width / 2.0);
        layer.y = tidy(center.y - height / 2.0);
        layer.width = tidy(width);
        layer.height = tidy(height);
        layer.scale = tidy(layer.scale * self.scale);
    }

    fn apply_to_animation(&self, data: &mut NodeData) {
        let (x, y) = self.outside;
        let offset = |offset: Offset| Offset::new(tidy(offset.x * x), tidy(offset.y * y));
        let size = |size: SizeChange| SizeChange {
            width: size.width.map(|width| tidy(width * self.size.0)),
            height: size.height.map(|height| tidy(height * self.size.1)),
        };
        match data {
            NodeData::Move(animation) => {
                animation.from = animation.from.map(offset);
                animation.to = animation.to.map(offset);
            }
            NodeData::SlideIn(slide) | NodeData::SlideOut(slide) => {
                let (dx, dy) = slide.direction.vector();
                slide.distance = tidy(slide.distance * (dx.abs() * x + dy.abs() * y));
            }
            NodeData::Resize(animation) => {
                animation.from = animation.from.map(size);
                animation.to = animation.to.map(size);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
