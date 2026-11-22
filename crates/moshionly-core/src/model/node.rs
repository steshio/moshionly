//! Nodes: the flat building blocks of a document.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::layer::{
    Artboard, EllipseLayer, Group, ImageLayer, LayerProps, MaskGroup, PathLayer, RectLayer,
    StarLayer, TextLayer,
};
use super::paint::{CornerRadius, Paint, Stroke};
use crate::defaults;
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

/// Every node type, without its properties.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// See [`Artboard`].
    Artboard,
    /// See [`Group`].
    Group,
    /// See [`MaskGroup`].
    MaskGroup,
    /// See [`RectLayer`].
    Rect,
    /// See [`EllipseLayer`].
    Ellipse,
    /// See [`StarLayer`].
    Star,
    /// See [`PathLayer`].
    Path,
    /// See [`TextLayer`].
    Text,
    /// See [`ImageLayer`].
    Image,
}

impl NodeType {
    /// Every node type: layers first, then animations.
    pub const ALL: &'static [NodeType] = &[
        NodeType::Artboard,
        NodeType::Group,
        NodeType::MaskGroup,
        NodeType::Rect,
        NodeType::Ellipse,
        NodeType::Star,
        NodeType::Path,
        NodeType::Text,
        NodeType::Image,
    ];

    /// The name used in the `type` field, for example `"mask_group"`.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeType::Artboard => "artboard",
            NodeType::Group => "group",
            NodeType::MaskGroup => "mask_group",
            NodeType::Rect => "rect",
            NodeType::Ellipse => "ellipse",
            NodeType::Star => "star",
            NodeType::Path => "path",
            NodeType::Text => "text",
            NodeType::Image => "image",
        }
    }

    /// Looks a type up by the name used in the `type` field.
    pub fn parse(name: &str) -> Option<NodeType> {
        Self::ALL.iter().copied().find(|ty| ty.as_str() == name)
    }

    /// Whether nodes of this type are layers or animations.
    pub fn family(self) -> Family {
        match self {
            NodeType::Artboard
            | NodeType::Group
            | NodeType::MaskGroup
            | NodeType::Rect
            | NodeType::Ellipse
            | NodeType::Star
            | NodeType::Path
            | NodeType::Text
            | NodeType::Image => Family::Layer,
        }
    }

    /// Whether layers can be placed inside a node of this type.
    pub fn holds_layers(self) -> bool {
        matches!(
            self,
            NodeType::Artboard | NodeType::Group | NodeType::MaskGroup
        )
    }

    /// One line saying what the type is, for pickers and AI prompts.
    pub fn description(self) -> &'static str {
        match self {
            NodeType::Artboard => {
                "A root frame with a size, a background and its own timeline. Everything else lives inside one."
            }
            NodeType::Group => {
                "A box that holds other layers and moves them together; can clip them and have its own fill, stroke and rounded corners."
            }
            NodeType::MaskGroup => {
                "A group whose first child (lowest index) is an alpha mask for the other children."
            }
            NodeType::Rect => "A rectangle with optional rounded corners.",
            NodeType::Ellipse => "An ellipse; can be cut down to an arc, a pie slice or a ring.",
            NodeType::Star => {
                "A star with a chosen number of points; an inner radius of 1 makes a polygon."
            }
            NodeType::Path => "A pen-drawn shape made of straight and curved segments.",
            NodeType::Text => "A block of text in one font and style.",
            NodeType::Image => "A bitmap image from a file or a data URI.",
        }
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
            NodeData::MaskGroup(_) | NodeData::Image(_) => None,
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
            NodeData::Artboard(_) | NodeData::MaskGroup(_) => None,
        }
    }

    /// The corner radius at rest, for layer types that have one.
    pub fn corner_radius(&self) -> Option<CornerRadius> {
        match self {
            NodeData::Group(n) => Some(n.corner_radius),
            NodeData::Rect(n) => Some(n.corner_radius),
            NodeData::Image(n) => Some(n.corner_radius),
            _ => None,
        }
    }

    /// The name given to the node, which may be empty.
    pub fn name(&self) -> &str {
        match self.layer() {
            Some(layer) => &layer.name,
            None => "",
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
        assert!(!NodeType::Rect.holds_layers());
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
}
