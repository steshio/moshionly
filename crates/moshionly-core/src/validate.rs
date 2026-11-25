//! The rules every document satisfies, and the messages for breaking them.
//!
//! There are two kinds of rule. *Value* rules look at one node alone: every
//! number is finite and inside its documented range. *Placement* rules look
//! at a node in its document: its parent may hold it, its index is a distinct
//! fractional index, its target is a layer of the right type in the same
//! artboard.
//!
//! Messages are written to be read by people and by AI agents: each one names
//! the node, the field and what would be accepted instead.

use serde::{Deserialize, Serialize};

mod components;

pub(crate) use components::check_components;

use crate::document::Document;
use crate::easing::Easing;
use crate::id::NodeId;
use crate::index;
use crate::model::{
    BlendMode, CornerRadius, Family, Gradient, LayerProps, Node, NodeData, NodeType, Paint,
    PathData, Shadow, ShadowChange, SizeChange, Stroke, StrokeChange, Timing,
};

/// What kind of rule a [`Problem`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// An id is empty, too long or uses characters outside `A-Z a-z 0-9 _ -`.
    InvalidId,
    /// A node is added with an id that is already in use.
    DuplicateId,
    /// An edit names a node that does not exist.
    UnknownNode,
    /// An update tries to change `id` or `type`, which are fixed (a group
    /// and a component can only turn into each other).
    Immutable,
    /// The JSON is not a node of its type, or a patch does not leave one: an
    /// unknown type or field, a value of the wrong shape, a required field
    /// missing.
    Malformed,
    /// A value is outside its documented range.
    OutOfRange,
    /// `parent` is missing, unknown, or of a type that cannot hold the node.
    Parent,
    /// A node would end up inside itself.
    Cycle,
    /// `index` is not a fractional index, or a sibling already uses it.
    Index,
    /// `target` is unknown, not a layer, of a type the animation does not
    /// work on, or in another artboard.
    Target,
    /// An instance's `component` is unknown or names a layer that is not a
    /// component.
    Component,
    /// A component would show itself: it contains an instance of itself,
    /// directly or through other components.
    ComponentLoop,
    /// A component is inside another component.
    NestedComponent,
}

/// A rule that a node breaks, with a message that says how to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Problem {
    /// Which rule was broken.
    pub kind: ProblemKind,
    /// The node the problem is about, when there is one.
    pub node: Option<NodeId>,
    /// The field to change, as a path such as `opacity` or `stroke.width`.
    pub field: Option<String>,
    /// The full explanation, naming the node, the field and the fix.
    pub message: String,
}

impl Problem {
    pub(crate) fn new(
        kind: ProblemKind,
        node: &NodeId,
        field: Option<&str>,
        message: String,
    ) -> Self {
        Self {
            kind,
            node: Some(node.clone()),
            field: field.map(str::to_owned),
            message,
        }
    }
}

/// Whether an animation of type `animation` can target a layer of type `layer`.
///
/// - An artboard takes only `fill`, which animates its background.
/// - `fill` needs a layer with a fill: everything but `mask_group`, `image`
///   and `instance`.
/// - `stroke` needs a layer with a stroke: everything but `mask_group`,
///   `instance` and `artboard`.
/// - `corner_radius` works on `rect`, `group`, `image` and `component`.
/// - `text_in`, `text_out` and `counter` work on `text`.
/// - Everything else works on every layer except an artboard.
///
/// `false` when `animation` is not an animation type that has a target, or
/// `layer` is not a layer type.
pub fn animation_applies_to(animation: NodeType, layer: NodeType) -> bool {
    use NodeType as T;
    if layer.family() != Family::Layer || animation.family() != Family::Animation {
        return false;
    }
    match animation {
        T::AnimationGroup => false,
        T::Fill => !matches!(layer, T::MaskGroup | T::Image | T::Instance),
        _ if layer == T::Artboard => false,
        T::Stroke => !matches!(layer, T::MaskGroup | T::Instance),
        T::CornerRadius => matches!(layer, T::Rect | T::Group | T::Image | T::Component),
        T::TextIn | T::TextOut | T::Counter => layer == T::Text,
        _ => true,
    }
}

/// The layer types an animation type can target, for messages.
fn targets_of(animation: NodeType) -> String {
    let names: Vec<&str> = NodeType::ALL
        .iter()
        .filter(|layer| animation_applies_to(animation, **layer))
        .map(|layer| layer.as_str())
        .collect();
    join_or(&names)
}

/// `a`, `a or b`, `a, b or c`.
fn join_or(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Prints a number the way it is written in a file: `240`, not `240.0`.
fn number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Checks the values of one node against their documented ranges.
pub(crate) fn check_values(node: &Node) -> Result<(), Problem> {
    let check = Values { node };
    if !node.id.is_valid() {
        return Err(Problem::new(
            ProblemKind::InvalidId,
            &node.id,
            Some("id"),
            format!(
                "\"{}\" cannot be used as an id. An id is 1 to 64 characters from A-Z, a-z, \
                 0-9, \"_\" and \"-\"; 12 random characters is the usual choice.",
                node.id
            ),
        ));
    }
    if let Some(layer) = node.data.layer() {
        check.layer(layer)?;
    }
    if let Some(timing) = node.data.timing() {
        check.timing(timing)?;
    }
    match &node.data {
        NodeData::Artboard(artboard) => {
            check.at_least("width", artboard.layer.width, 1.0)?;
            check.at_least("height", artboard.layer.height, 1.0)?;
            check.at_least("duration", artboard.duration, 1.0)?;
            check.artboard_defaults(&artboard.layer)?;
            check.paint("fill", artboard.fill.as_ref())?;
        }
        NodeData::Group(group) => {
            check.paint("fill", group.fill.as_ref())?;
            check.stroke(group.stroke.as_ref())?;
            check.radius("corner_radius", Some(group.corner_radius))?;
        }
        NodeData::MaskGroup(_) => {}
        NodeData::Rect(rect) => {
            check.paint("fill", rect.fill.as_ref())?;
            check.stroke(rect.stroke.as_ref())?;
            check.radius("corner_radius", Some(rect.corner_radius))?;
        }
        NodeData::Ellipse(ellipse) => {
            check.paint("fill", ellipse.fill.as_ref())?;
            check.stroke(ellipse.stroke.as_ref())?;
            check.finite("start_angle", ellipse.start_angle)?;
            check.between("sweep", ellipse.sweep, -360.0, 360.0)?;
            check.between("inner_radius", ellipse.inner_radius, 0.0, 1.0)?;
        }
        NodeData::Star(star) => {
            check.paint("fill", star.fill.as_ref())?;
            check.stroke(star.stroke.as_ref())?;
            check.between("points", f64::from(star.points), 3.0, 64.0)?;
            check.between("inner_radius", star.inner_radius, 0.0, 1.0)?;
        }
        NodeData::Path(path) => {
            check.paint("fill", path.fill.as_ref())?;
            check.stroke(path.stroke.as_ref())?;
            check.path(&path.path)?;
        }
        NodeData::Text(text) => {
            check.paint("fill", text.fill.as_ref())?;
            check.stroke(text.stroke.as_ref())?;
            check.more_than_zero("font_size", text.font_size)?;
            check.more_than_zero("line_height", text.line_height)?;
            check.finite("letter_spacing", text.letter_spacing)?;
            check.between("font.weight", f64::from(text.font.weight), 1.0, 1000.0)?;
            if text.font.family.trim().is_empty() {
                return Err(check.fail(
                    "font.family",
                    "is empty, but it must name a font family, for example \"Inter\"".into(),
                ));
            }
        }
        NodeData::Image(image) => {
            if image.source.trim().is_empty() {
                return Err(check.fail(
                    "source",
                    "is empty, but it must be a file path or a \"data:\" URI".into(),
                ));
            }
            check.stroke(image.stroke.as_ref())?;
            check.radius("corner_radius", Some(image.corner_radius))?;
        }
        NodeData::Component(component) => {
            check.paint("fill", component.fill.as_ref())?;
            check.stroke(component.stroke.as_ref())?;
            check.radius("corner_radius", Some(component.corner_radius))?;
        }
        NodeData::Instance(instance) => check.finite("time_offset", instance.time_offset)?,
        NodeData::Move(animation) => {
            for (field, offset) in [("from", animation.from), ("to", animation.to)] {
                if let Some(offset) = offset {
                    check.finite(&format!("{field}.x"), offset.x)?;
                    check.finite(&format!("{field}.y"), offset.y)?;
                }
            }
        }
        NodeData::Rotate(animation) => {
            check.optional("from", animation.from, Values::finite)?;
            check.optional("to", animation.to, Values::finite)?;
        }
        NodeData::Scale(animation) => {
            check.optional("from", animation.from, Values::not_negative)?;
            check.optional("to", animation.to, Values::not_negative)?;
        }
        NodeData::Opacity(animation) => {
            check.optional("from", animation.from, Values::percentage)?;
            check.optional("to", animation.to, Values::percentage)?;
        }
        NodeData::Resize(animation) => {
            check.size("from", animation.from)?;
            check.size("to", animation.to)?;
        }
        NodeData::Fill(animation) => {
            check.paint("from", animation.from.as_ref())?;
            check.paint("to", animation.to.as_ref())?;
        }
        NodeData::Stroke(animation) => {
            check.stroke_change("from", animation.from.as_ref())?;
            check.stroke_change("to", animation.to.as_ref())?;
        }
        NodeData::CornerRadius(animation) => {
            check.radius("from", animation.from)?;
            check.radius("to", animation.to)?;
        }
        NodeData::Blur(animation) => {
            check.optional("from", animation.from, Values::not_negative)?;
            check.optional("to", animation.to, Values::not_negative)?;
        }
        NodeData::Shadow(animation) => {
            check.shadow_change("from", animation.from)?;
            check.shadow_change("to", animation.to)?;
        }
        NodeData::Show(instant) | NodeData::Hide(instant) => {
            check.not_negative("start", instant.start)?;
            check.optional("end", instant.end, Values::finite)?;
        }
        NodeData::FadeIn(_) | NodeData::FadeOut(_) => {}
        NodeData::SlideIn(preset) | NodeData::SlideOut(preset) => {
            check.not_negative("distance", preset.distance)?;
        }
        NodeData::GrowIn(preset) => check.not_negative("scale", preset.scale)?,
        NodeData::GrowOut(preset) => check.not_negative("scale", preset.scale)?,
        NodeData::ShrinkIn(preset) => check.not_negative("scale", preset.scale)?,
        NodeData::ShrinkOut(preset) => check.not_negative("scale", preset.scale)?,
        NodeData::SpinIn(preset) | NodeData::SpinOut(preset) => {
            check.finite("angle", preset.angle)?;
        }
        NodeData::BlurIn(preset) | NodeData::BlurOut(preset) => {
            check.not_negative("blur", preset.blur)?;
        }
        NodeData::RevealIn(_) | NodeData::RevealOut(_) => {}
        NodeData::TextIn(animation) | NodeData::TextOut(animation) => {
            check.not_negative("stagger", animation.stagger)?;
            check.not_negative("distance", animation.distance)?;
            check.not_negative("scale", animation.scale)?;
            check.not_negative("blur", animation.blur)?;
        }
        NodeData::Counter(animation) => {
            check.optional("from", animation.from, Values::finite)?;
            check.optional("to", animation.to, Values::finite)?;
        }
        NodeData::AnimationGroup(_) => {}
    }
    Ok(())
}

/// Value checks for one node. Every method names the field it looked at.
struct Values<'a> {
    node: &'a Node,
}

impl Values<'_> {
    fn fail(&self, field: &str, complaint: String) -> Problem {
        Problem::new(
            ProblemKind::OutOfRange,
            &self.node.id,
            Some(field),
            format!("{}: \"{field}\" {complaint}.", self.node.label()),
        )
    }

    fn finite(&self, field: &str, value: f64) -> Result<(), Problem> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(self.fail(field, format!("is {value}, but it must be a finite number")))
        }
    }

    fn at_least(&self, field: &str, value: f64, min: f64) -> Result<(), Problem> {
        self.finite(field, value)?;
        if value >= min {
            Ok(())
        } else {
            Err(self.fail(
                field,
                format!(
                    "is {}, but it must be {} or more",
                    number(value),
                    number(min)
                ),
            ))
        }
    }

    fn not_negative(&self, field: &str, value: f64) -> Result<(), Problem> {
        self.at_least(field, value, 0.0)
    }

    fn more_than_zero(&self, field: &str, value: f64) -> Result<(), Problem> {
        self.finite(field, value)?;
        if value > 0.0 {
            Ok(())
        } else {
            Err(self.fail(
                field,
                format!("is {}, but it must be more than 0", number(value)),
            ))
        }
    }

    fn between(&self, field: &str, value: f64, min: f64, max: f64) -> Result<(), Problem> {
        self.finite(field, value)?;
        if (min..=max).contains(&value) {
            Ok(())
        } else {
            Err(self.fail(
                field,
                format!(
                    "is {}, but it must be between {} and {}",
                    number(value),
                    number(min),
                    number(max)
                ),
            ))
        }
    }

    fn percentage(&self, field: &str, value: f64) -> Result<(), Problem> {
        self.between(field, value, 0.0, 100.0)
    }

    fn optional(
        &self,
        field: &str,
        value: Option<f64>,
        rule: fn(&Self, &str, f64) -> Result<(), Problem>,
    ) -> Result<(), Problem> {
        value.map_or(Ok(()), |value| rule(self, field, value))
    }

    fn layer(&self, layer: &LayerProps) -> Result<(), Problem> {
        self.finite("x", layer.x)?;
        self.finite("y", layer.y)?;
        self.not_negative("width", layer.width)?;
        self.not_negative("height", layer.height)?;
        self.finite("rotation", layer.rotation)?;
        self.not_negative("scale", layer.scale)?;
        self.percentage("opacity", layer.opacity)?;
        self.not_negative("blur", layer.blur)?;
        if let Some(Shadow { x, y, blur, .. }) = layer.shadow {
            self.finite("shadow.x", x)?;
            self.finite("shadow.y", y)?;
            self.not_negative("shadow.blur", blur)?;
        }
        Ok(())
    }

    /// An artboard is only a frame: the properties that would transform or
    /// composite it have to stay at their defaults.
    fn artboard_defaults(&self, layer: &LayerProps) -> Result<(), Problem> {
        let fixed = [
            ("rotation", layer.rotation != 0.0, "0"),
            ("scale", layer.scale != 1.0, "1"),
            ("opacity", layer.opacity != 100.0, "100"),
            ("blur", layer.blur != 0.0, "0"),
            (
                "blend_mode",
                layer.blend_mode != BlendMode::Normal,
                "\"normal\"",
            ),
        ];
        for (field, changed, default) in fixed {
            if changed {
                return Err(self.fail(
                    field,
                    format!(
                        "cannot be set on an artboard and must stay {default}; put the layers \
                         in a group and set it there"
                    ),
                ));
            }
        }
        if layer.shadow.is_some() {
            return Err(self.fail(
                "shadow",
                "cannot be set on an artboard; remove it, or put the layers in a group and \
                 give that a shadow"
                    .into(),
            ));
        }
        Ok(())
    }

    fn timing(&self, timing: &Timing) -> Result<(), Problem> {
        self.not_negative("start", timing.start)?;
        self.finite("end", timing.end)?;
        if timing.end < timing.start {
            return Err(self.fail(
                "end",
                format!(
                    "is {}, which is before \"start\" ({}); an animation cannot end before it \
                     starts, so make \"end\" at least {}",
                    number(timing.end),
                    number(timing.start),
                    number(timing.start)
                ),
            ));
        }
        self.easing(&timing.easing)
    }

    fn easing(&self, easing: &Easing) -> Result<(), Problem> {
        if let Some(intensity) = easing.intensity() {
            self.percentage("easing.intensity", intensity)?;
        }
        if let Easing::Bezier { x1, y1, x2, y2 } = *easing {
            self.between("easing.x1", x1, 0.0, 1.0)?;
            self.finite("easing.y1", y1)?;
            self.between("easing.x2", x2, 0.0, 1.0)?;
            self.finite("easing.y2", y2)?;
        }
        Ok(())
    }

    fn paint(&self, field: &str, paint: Option<&Paint>) -> Result<(), Problem> {
        let Some(Paint::Gradient(gradient)) = paint else {
            return Ok(());
        };
        let points = |pairs: &[(&str, [f64; 2])]| {
            pairs.iter().try_for_each(|(name, point)| {
                self.finite(&format!("{field}.{name}"), point[0])?;
                self.finite(&format!("{field}.{name}"), point[1])
            })
        };
        match gradient {
            Gradient::LinearGradient(linear) => {
                points(&[("start", linear.start), ("end", linear.end)])?
            }
            Gradient::RadialGradient(radial) => {
                points(&[("center", radial.center)])?;
                self.not_negative(&format!("{field}.radius"), radial.radius[0])?;
                self.not_negative(&format!("{field}.radius"), radial.radius[1])?;
                self.finite(&format!("{field}.rotation"), radial.rotation)?;
            }
        }
        let stops = gradient.stops();
        if stops.len() < 2 {
            return Err(self.fail(
                &format!("{field}.stops"),
                format!(
                    "has {} stop{}, but a gradient needs at least 2; for one colour use a \
                     colour string such as \"#0D99FF\"",
                    stops.len(),
                    if stops.len() == 1 { "" } else { "s" }
                ),
            ));
        }
        let mut previous = 0.0;
        for (i, stop) in stops.iter().enumerate() {
            let name = format!("{field}.stops[{i}].offset");
            self.between(&name, stop.offset, 0.0, 1.0)?;
            if stop.offset < previous {
                return Err(self.fail(
                    &name,
                    format!(
                        "is {}, which is less than the stop before it ({}); list the stops \
                         from offset 0 up to 1",
                        number(stop.offset),
                        number(previous)
                    ),
                ));
            }
            previous = stop.offset;
        }
        Ok(())
    }

    fn stroke(&self, stroke: Option<&Stroke>) -> Result<(), Problem> {
        let Some(stroke) = stroke else {
            return Ok(());
        };
        self.not_negative("stroke.width", stroke.width)?;
        self.paint("stroke.paint", Some(&stroke.paint))
    }

    fn stroke_change(&self, field: &str, change: Option<&StrokeChange>) -> Result<(), Problem> {
        let Some(change) = change else {
            return Ok(());
        };
        self.optional(
            &format!("{field}.width"),
            change.width,
            Values::not_negative,
        )?;
        self.paint(&format!("{field}.paint"), change.paint.as_ref())
    }

    fn shadow_change(&self, field: &str, change: Option<ShadowChange>) -> Result<(), Problem> {
        let Some(change) = change else {
            return Ok(());
        };
        self.optional(&format!("{field}.x"), change.x, Values::finite)?;
        self.optional(&format!("{field}.y"), change.y, Values::finite)?;
        self.optional(&format!("{field}.blur"), change.blur, Values::not_negative)
    }

    fn size(&self, field: &str, size: Option<SizeChange>) -> Result<(), Problem> {
        let Some(size) = size else {
            return Ok(());
        };
        self.optional(&format!("{field}.width"), size.width, Values::not_negative)?;
        self.optional(
            &format!("{field}.height"),
            size.height,
            Values::not_negative,
        )
    }

    fn radius(&self, field: &str, radius: Option<CornerRadius>) -> Result<(), Problem> {
        let Some(radius) = radius else {
            return Ok(());
        };
        radius
            .radii()
            .iter()
            .try_for_each(|value| self.not_negative(field, *value))
    }

    fn path(&self, path: &PathData) -> Result<(), Problem> {
        for (s, subpath) in path.subpaths.iter().enumerate() {
            for (a, anchor) in subpath.anchors.iter().enumerate() {
                let points = [Some(anchor.point), anchor.handle_in, anchor.handle_out];
                for value in points.into_iter().flatten().flatten() {
                    self.finite(&format!("path.subpaths[{s}].anchors[{a}]"), value)?;
                }
            }
        }
        Ok(())
    }
}

/// Checks where one node sits in its document: parent, index, target and,
/// for an instance, its component.
///
/// The rules that span several nodes at once (no component inside another,
/// no component that shows itself) are checked by [`check_components`].
pub(crate) fn check_placement(document: &Document, node: &Node) -> Result<(), Problem> {
    check_parent(document, node)?;
    check_index(document, node)?;
    check_target(document, node)?;
    components::check_reference(document, node)
}

fn check_parent(document: &Document, node: &Node) -> Result<(), Problem> {
    let label = node.label();
    let problem = |kind: ProblemKind, message: String| {
        Err(Problem::new(kind, &node.id, Some("parent"), message))
    };
    let family = node.family();
    let holders = match family {
        Family::Layer => "an artboard, a group, a mask_group or a component",
        Family::Animation => "an artboard or an animation_group",
    };
    let is_artboard = node.node_type() == NodeType::Artboard;
    let parent_id = match (&node.parent, is_artboard) {
        (None, true) => return Ok(()),
        (Some(parent), true) => {
            return problem(
                ProblemKind::Parent,
                format!(
                    "{label}: \"parent\" is \"{parent}\", but an artboard is a root and has no \
                     parent. Set \"parent\" to null, or use a group if this should sit inside \
                     another artboard."
                ),
            );
        }
        (None, false) => {
            return problem(
                ProblemKind::Parent,
                format!(
                    "{label}: \"parent\" is null, but only artboards are roots. Set \"parent\" \
                     to the id of {holders}."
                ),
            );
        }
        (Some(parent), false) => parent,
    };
    let Some(parent) = document.get(parent_id) else {
        return problem(
            ProblemKind::Parent,
            format!(
                "{label}: \"parent\" is \"{parent_id}\", but no node has that id. Set \"parent\" \
                 to the id of {holders}, and add the parent in the same batch if it is new."
            ),
        );
    };
    let holds = match family {
        Family::Layer => parent.node_type().holds_layers(),
        Family::Animation => parent.node_type().holds_animations(),
    };
    if !holds {
        let hint = match (family, document.artboard_of(parent_id)) {
            (Family::Animation, Some(artboard)) => format!(
                " Animations live in the timeline of their artboard, not inside the layer \
                 they change: set \"parent\" to \"{}\" and keep the layer in \"target\".",
                artboard.id
            ),
            _ => String::new(),
        };
        return problem(
            ProblemKind::Parent,
            format!(
                "{label}: \"parent\" is {}, but {} can only be placed in {holders}.{hint}",
                parent.label(),
                match family {
                    Family::Layer => "a layer",
                    Family::Animation => "an animation",
                },
            ),
        );
    }
    // Every step up has to reach a root without coming back to this node.
    let mut steps = 0;
    for ancestor in document.ancestors(&node.id) {
        steps += 1;
        if ancestor.id == node.id || steps >= document.len() {
            return problem(
                ProblemKind::Cycle,
                format!(
                    "{label}: \"parent\" is \"{parent_id}\", which is this node itself or \
                     inside it. A node cannot be placed inside its own descendants; choose a \
                     parent outside this node."
                ),
            );
        }
    }
    Ok(())
}

fn check_index(document: &Document, node: &Node) -> Result<(), Problem> {
    let label = node.label();
    if let Err(error) = index::validate(&node.index) {
        return Err(Problem::new(
            ProblemKind::Index,
            &node.id,
            Some("index"),
            format!("{label}: {error}. {}", suggest_index(document, node)),
        ));
    }
    // Siblings are kept sorted by index and then id, so one with the same
    // index sits right next to this node.
    let siblings = document.child_list(node.parent.as_ref(), node.family());
    let position = siblings.partition_point(|sibling| {
        document.get(sibling).is_some_and(|sibling| {
            (sibling.index.as_str(), &sibling.id) < (node.index.as_str(), &node.id)
        })
    });
    let clash = [position.checked_sub(1), Some(position + 1)]
        .into_iter()
        .flatten()
        .filter_map(|neighbour| document.get(siblings.get(neighbour)?))
        .find(|sibling| sibling.id != node.id && sibling.index == node.index);
    match clash {
        None => Ok(()),
        Some(sibling) => Err(Problem::new(
            ProblemKind::Index,
            &node.id,
            Some("index"),
            format!(
                "{label}: \"index\" is \"{}\", which its sibling {} already uses. Siblings \
                 need different indices because the index decides their order. {}",
                node.index,
                sibling.label(),
                suggest_index(document, node)
            ),
        )),
    }
}

/// A sentence offering an index that would put the node last among its siblings.
fn suggest_index(document: &Document, node: &Node) -> String {
    let last = document
        .child_list(node.parent.as_ref(), node.family())
        .iter()
        .filter(|sibling| **sibling != node.id)
        .filter_map(|sibling| document.get(sibling))
        .map(|sibling| sibling.index.as_str())
        .rfind(|index| index::is_valid(index));
    let place = match node.family() {
        Family::Layer => "in front of its siblings",
        Family::Animation => "on the last timeline row",
    };
    match index::key_between(last, None) {
        Ok(free) => format!("For example, \"{free}\" would put it {place}."),
        Err(_) => String::new(),
    }
}

fn check_target(document: &Document, node: &Node) -> Result<(), Problem> {
    let Some(target_id) = node.target() else {
        return Ok(());
    };
    let label = node.label();
    let problem = |message: String| {
        Err(Problem::new(
            ProblemKind::Target,
            &node.id,
            Some("target"),
            message,
        ))
    };
    let kind = node.node_type();
    let Some(target) = document.get(target_id) else {
        return problem(format!(
            "{label}: \"target\" is \"{target_id}\", but no node has that id. Set \"target\" to \
             the id of the layer to animate, and add that layer in the same batch if it is new."
        ));
    };
    if target.family() != Family::Layer {
        return problem(format!(
            "{label}: \"target\" is {}, which is an animation. An animation changes a layer: \
             set \"target\" to the id of a layer ({}).",
            target.label(),
            targets_of(kind)
        ));
    }
    if !animation_applies_to(kind, target.node_type()) {
        let hint = if target.node_type() == NodeType::Artboard {
            " To animate everything in an artboard, put the layers in a group and target the group."
        } else {
            ""
        };
        return problem(format!(
            "{label}: \"target\" is {}, but {kind} only works on these layer types: {}.{hint}",
            target.label(),
            targets_of(kind)
        ));
    }
    let own = document.artboard_of(&node.id).map(|artboard| &artboard.id);
    let theirs = document.artboard_of(target_id).map(|artboard| &artboard.id);
    if let (Some(own), Some(theirs)) = (own, theirs)
        && own != theirs
    {
        return problem(format!(
            "{label}: \"target\" is {}, which is in artboard \"{theirs}\", but this animation \
             is in artboard \"{own}\". An animation and its target must be in the same \
             artboard: move the animation under \"{theirs}\", or pick a target in \"{own}\".",
            target.label()
        ));
    }
    Ok(())
}

impl Document {
    /// Checks every rule on every node and reports the first one broken, in
    /// tree order.
    ///
    /// A document that was only ever changed through `Document::apply` always
    /// passes. This is for code that wants to be sure, such as tests.
    pub fn validate(&self) -> Result<(), Problem> {
        let mut nodes: Vec<&Node> = self.nodes_unordered().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        for node in nodes {
            check_values(node)?;
            check_placement(self, node)?;
        }
        check_components(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn node(fields: Value) -> Node {
        let mut value = json!({ "id": "n1", "parent": "ab", "index": "a0" });
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        Node::from_value(value).unwrap()
    }

    fn complaint(fields: Value) -> Problem {
        check_values(&node(fields)).unwrap_err()
    }

    #[test]
    fn default_nodes_of_every_type_pass() {
        for ty in NodeType::ALL {
            let mut fields = json!({ "type": ty.as_str() });
            if ty.family() == Family::Animation && *ty != NodeType::AnimationGroup {
                fields["target"] = json!("x");
                fields["start"] = json!(0);
                fields["end"] = json!(500);
            }
            if *ty == NodeType::Image {
                fields["source"] = json!("photo.png");
            }
            if *ty == NodeType::Instance {
                fields["component"] = json!("m1");
            }
            let node = node(fields);
            check_values(&node).unwrap_or_else(|problem| panic!("{ty}: {problem}"));
        }
    }

    #[test]
    fn messages_name_the_node_the_field_and_the_range() {
        let problem = complaint(json!({ "type": "rect", "name": "Card", "opacity": 140 }));
        assert_eq!(problem.kind, ProblemKind::OutOfRange);
        assert_eq!(problem.node, Some(NodeId::from("n1")));
        assert_eq!(problem.field.as_deref(), Some("opacity"));
        assert_eq!(
            problem.to_string(),
            "rect \"n1\" (Card): \"opacity\" is 140, but it must be between 0 and 100."
        );
    }

    #[test]
    fn layer_values_have_ranges() {
        let cases = [
            (json!({ "type": "rect", "width": -1 }), "width"),
            (json!({ "type": "rect", "scale": -0.5 }), "scale"),
            (json!({ "type": "rect", "blur": -2 }), "blur"),
            (
                json!({ "type": "rect", "shadow": { "blur": -1 } }),
                "shadow.blur",
            ),
            (
                json!({ "type": "rect", "stroke": { "width": -1 } }),
                "stroke.width",
            ),
            (
                json!({ "type": "rect", "corner_radius": [1, 2, -3, 4] }),
                "corner_radius",
            ),
            (json!({ "type": "ellipse", "sweep": 400 }), "sweep"),
            (
                json!({ "type": "ellipse", "inner_radius": 1.5 }),
                "inner_radius",
            ),
            (json!({ "type": "star", "points": 2 }), "points"),
            (
                json!({ "type": "star", "inner_radius": -0.1 }),
                "inner_radius",
            ),
            (json!({ "type": "text", "font_size": 0 }), "font_size"),
            (json!({ "type": "text", "line_height": -10 }), "line_height"),
            (
                json!({ "type": "text", "font": { "weight": 0 } }),
                "font.weight",
            ),
            (
                json!({ "type": "text", "font": { "family": " " } }),
                "font.family",
            ),
            (json!({ "type": "image", "source": "" }), "source"),
        ];
        for (fields, field) in cases {
            let problem = complaint(fields.clone());
            assert_eq!(problem.field.as_deref(), Some(field), "{fields}: {problem}");
            assert!(
                problem.message.contains(&format!("\"{field}\"")),
                "{problem}"
            );
        }
    }

    #[test]
    fn artboards_are_plain_frames() {
        let artboard = |extra: Value| {
            let mut fields = json!({ "type": "artboard", "parent": null });
            fields
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            complaint(fields)
        };
        assert_eq!(
            artboard(json!({ "rotation": 10 })).field.as_deref(),
            Some("rotation")
        );
        assert_eq!(
            artboard(json!({ "opacity": 50 })).field.as_deref(),
            Some("opacity")
        );
        assert_eq!(
            artboard(json!({ "shadow": {} })).field.as_deref(),
            Some("shadow")
        );
        assert_eq!(
            artboard(json!({ "blend_mode": "multiply" }))
                .field
                .as_deref(),
            Some("blend_mode")
        );
        assert_eq!(
            artboard(json!({ "width": 0 })).field.as_deref(),
            Some("width")
        );
        assert_eq!(
            artboard(json!({ "duration": 0 })).field.as_deref(),
            Some("duration")
        );
        assert!(artboard(json!({ "scale": 2 })).message.contains("group"));
    }

    #[test]
    fn gradients_need_two_ordered_stops() {
        let gradient = |stops: Value| json!({ "type": "rect", "fill": { "type": "linear_gradient", "stops": stops } });
        let too_few = complaint(gradient(json!([{ "offset": 0, "color": "#FF0000" }])));
        assert_eq!(too_few.field.as_deref(), Some("fill.stops"));
        assert!(too_few.message.contains("at least 2"), "{too_few}");

        let unordered = complaint(gradient(json!([
            { "offset": 0.8, "color": "#FF0000" }, { "offset": 0.2, "color": "#0000FF" }
        ])));
        assert_eq!(unordered.field.as_deref(), Some("fill.stops[1].offset"));

        let outside = complaint(gradient(json!([
            { "offset": 0, "color": "#FF0000" }, { "offset": 1.5, "color": "#0000FF" }
        ])));
        assert!(outside.message.contains("between 0 and 1"), "{outside}");

        let in_stroke = complaint(json!({ "type": "rect", "stroke": { "paint":
            { "type": "radial_gradient", "stops": [] } } }));
        assert_eq!(in_stroke.field.as_deref(), Some("stroke.paint.stops"));
    }

    #[test]
    fn animation_values_have_ranges() {
        let animation = |extra: Value| {
            let mut fields = json!({ "target": "x", "start": 100, "end": 600 });
            fields
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            complaint(fields)
        };
        let cases = [
            (json!({ "type": "fade_in", "start": -5 }), "start"),
            (json!({ "type": "fade_in", "end": 50 }), "end"),
            (
                json!({ "type": "fade_in", "easing": { "type": "smooth", "intensity": 120 } }),
                "easing.intensity",
            ),
            (
                json!({ "type": "fade_in", "easing": { "type": "bezier", "x1": 1.5, "y1": 0, "x2": 0, "y2": 1 } }),
                "easing.x1",
            ),
            (json!({ "type": "opacity", "to": 101 }), "to"),
            (json!({ "type": "scale", "from": -1 }), "from"),
            (
                json!({ "type": "resize", "to": { "width": -10 } }),
                "to.width",
            ),
            (
                json!({ "type": "stroke", "to": { "width": -1 } }),
                "to.width",
            ),
            (json!({ "type": "blur", "to": -3 }), "to"),
            (
                json!({ "type": "shadow", "from": { "blur": -1 } }),
                "from.blur",
            ),
            (json!({ "type": "corner_radius", "to": -4 }), "to"),
            (json!({ "type": "slide_in", "distance": -1 }), "distance"),
            (json!({ "type": "grow_in", "scale": -1 }), "scale"),
            (json!({ "type": "blur_out", "blur": -1 }), "blur"),
            (json!({ "type": "text_in", "stagger": -1 }), "stagger"),
        ];
        for (fields, field) in cases {
            let problem = animation(fields.clone());
            assert_eq!(problem.field.as_deref(), Some(field), "{fields}: {problem}");
        }
        let backwards = animation(json!({ "type": "move", "end": 50 }));
        assert!(backwards.message.contains("at least 100"), "{backwards}");
    }

    #[test]
    fn ids_must_be_well_formed() {
        let mut bad = node(json!({ "type": "rect" }));
        bad.id = NodeId::from("has space");
        let problem = check_values(&bad).unwrap_err();
        assert_eq!(problem.kind, ProblemKind::InvalidId);
        assert!(problem.message.contains("has space"));
    }

    #[test]
    fn which_animations_work_on_which_layers() {
        use NodeType as T;
        assert!(animation_applies_to(T::Fill, T::Artboard));
        assert!(!animation_applies_to(T::Move, T::Artboard));
        assert!(!animation_applies_to(T::FadeIn, T::Artboard));
        assert!(animation_applies_to(T::Fill, T::Text));
        assert!(!animation_applies_to(T::Fill, T::Image));
        assert!(!animation_applies_to(T::Fill, T::MaskGroup));
        assert!(animation_applies_to(T::Stroke, T::Image));
        assert!(animation_applies_to(T::CornerRadius, T::Group));
        assert!(animation_applies_to(T::CornerRadius, T::Component));
        assert!(animation_applies_to(T::Fill, T::Component));
        assert!(!animation_applies_to(T::Fill, T::Instance));
        assert!(!animation_applies_to(T::Stroke, T::Instance));
        assert!(!animation_applies_to(T::CornerRadius, T::Instance));
        assert!(animation_applies_to(T::FadeIn, T::Instance));
        assert!(animation_applies_to(T::Resize, T::Instance));
        assert!(!animation_applies_to(T::CornerRadius, T::Ellipse));
        assert!(animation_applies_to(T::TextIn, T::Text));
        assert!(!animation_applies_to(T::Counter, T::Rect));
        assert!(animation_applies_to(T::SlideIn, T::MaskGroup));
        assert!(!animation_applies_to(T::AnimationGroup, T::Rect));
        assert!(!animation_applies_to(T::Rect, T::Rect));
        assert!(!animation_applies_to(T::Move, T::FadeIn));
        assert_eq!(
            targets_of(T::CornerRadius),
            "group, rect, image or component"
        );
        assert_eq!(targets_of(T::Counter), "text");
    }
}
