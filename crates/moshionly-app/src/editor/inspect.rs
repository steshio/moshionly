//! What the inspector shows for a selection, and the edits it makes.
//!
//! Every property is read from each selected layer that has it. When they
//! all agree the inspector shows the value; when they differ it shows
//! "Mixed", and setting a value sets it on every one of them.

use moshionly_core::model::{AutoSize, NodeData};
use moshionly_core::{Document, Edit, Node, NodeId, NodeType};
use serde_json::Value;

use super::settle::Frame;

/// A property's value across the selection: the same everywhere, or not.
/// Either way it carries the value of the first layer, for a starting
/// point.
#[derive(Clone, Debug, PartialEq)]
pub enum Shared<T> {
    /// Every layer has this value.
    Same(T),
    /// The layers differ; this is the first one's.
    Mixed(T),
}

impl<T> Shared<T> {
    /// The value, or the first layer's value when they differ.
    pub fn value(&self) -> &T {
        match self {
            Shared::Same(value) | Shared::Mixed(value) => value,
        }
    }

    /// Whether the layers differ.
    pub fn is_mixed(&self) -> bool {
        matches!(self, Shared::Mixed(_))
    }
}

/// The values' agreement, or `None` when there are none.
pub fn shared<T: PartialEq>(values: impl IntoIterator<Item = T>) -> Option<Shared<T>> {
    let mut values = values.into_iter();
    let first = values.next()?;
    if values.all(|value| value == first) {
        Some(Shared::Same(first))
    } else {
        Some(Shared::Mixed(first))
    }
}

/// The selected nodes, for reading properties.
pub struct Inspected<'a> {
    nodes: Vec<&'a Node>,
}

impl<'a> Inspected<'a> {
    /// The nodes of `ids` that are in `document`.
    pub fn new(document: &'a Document, ids: &[NodeId]) -> Self {
        Self {
            nodes: ids.iter().filter_map(|id| document.get(id)).collect(),
        }
    }

    /// The nodes.
    pub fn nodes(&self) -> &[&'a Node] {
        &self.nodes
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The ids of the nodes for which `has` is true.
    pub fn ids_where(&self, has: impl Fn(&Node) -> bool) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|node| has(node))
            .map(|node| node.id.clone())
            .collect()
    }

    /// The ids of every node.
    pub fn ids(&self) -> Vec<NodeId> {
        self.ids_where(|_| true)
    }

    /// Whether every node is of type `ty`.
    pub fn all_of(&self, ty: NodeType) -> bool {
        !self.nodes.is_empty() && self.nodes.iter().all(|node| node.node_type() == ty)
    }

    /// Whether any node is of type `ty`.
    pub fn any_of(&self, ty: NodeType) -> bool {
        self.nodes.iter().any(|node| node.node_type() == ty)
    }

    /// A property read with `read` from every node that has it.
    pub fn read<T: PartialEq>(&self, read: impl Fn(&'a Node) -> Option<T>) -> Option<Shared<T>> {
        shared(self.nodes.iter().filter_map(|node| read(node)))
    }

    /// A number read with `read` from every node that has it.
    pub fn number(&self, read: impl Fn(&'a Node) -> Option<f64>) -> Option<Shared<f64>> {
        self.read(read)
    }
}

/// Edits that set `field` to `value` on each of `ids`.
pub fn set_all(ids: &[NodeId], field: &str, value: impl Into<Value>) -> Vec<Edit> {
    let value = value.into();
    ids.iter()
        .map(|id| Edit::set(id.clone(), field, value.clone()))
        .collect()
}

/// Edits that merge `patch` into each of `ids`: `{"font": {"weight": 700}}`
/// changes the weight and keeps the family.
pub fn patch_all(ids: &[NodeId], field: &str, patch: Value) -> Vec<Edit> {
    set_all(ids, field, patch)
}

/// Edits that set the width (or height, with `height`) of every layer in
/// `nodes` to `value`, changing the other side in proportion when
/// `keep_proportions` is on. Each layer comes with its box as drawn at rest
/// (see [`settle`](super::settle)), which the proportions are taken from;
/// for text that hugs its words, settle it in the same step first.
pub fn resize_all(
    nodes: &[(&Node, Frame)],
    value: f64,
    height: bool,
    keep_proportions: bool,
) -> Vec<Edit> {
    nodes
        .iter()
        .filter_map(|(node, frame)| {
            node.layer()?;
            let (field, other_field, old, other) = if height {
                ("height", "width", frame.height, frame.width)
            } else {
                ("width", "height", frame.width, frame.height)
            };
            let mut patch = serde_json::Map::new();
            patch.insert(field.to_owned(), tidy(value).into());
            if keep_proportions && old > 0.0 {
                patch.insert(other_field.to_owned(), tidy(other * value / old).into());
            }
            // Text that hugs its words stops hugging on the side set.
            if let Some(text) = text_of(node) {
                let sizing = match (text.auto_size, height || keep_proportions) {
                    (AutoSize::WidthAndHeight, false) => Some("height"),
                    (AutoSize::WidthAndHeight | AutoSize::Height, true) => Some("fixed"),
                    _ => None,
                };
                if let Some(sizing) = sizing {
                    patch.insert("auto_size".to_owned(), sizing.into());
                }
            }
            Some(Edit::update(node.id.clone(), patch))
        })
        .collect()
}

/// A number with the noise of arithmetic rounded away, at a hundredth.
pub fn tidy(value: f64) -> f64 {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// The text properties of a node, when it is a text layer.
pub fn text_of(node: &Node) -> Option<&moshionly_core::model::TextLayer> {
    match &node.data {
        NodeData::Text(text) => Some(text),
        _ => None,
    }
}

/// The image properties of a node, when it is an image layer.
pub fn image_of(node: &Node) -> Option<&moshionly_core::model::ImageLayer> {
    match &node.data {
        NodeData::Image(image) => Some(image),
        _ => None,
    }
}

/// The artboard properties of a node, when it is an artboard.
pub fn artboard_of(node: &Node) -> Option<&moshionly_core::model::Artboard> {
    match &node.data {
        NodeData::Artboard(artboard) => Some(artboard),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"a","parent":"ab","index":"a0","type":"rect","x":10,"width":200,"height":100,"opacity":50},
            {"id":"b","parent":"ab","index":"a1","type":"rect","x":10,"width":100,"height":100},
            {"id":"t","parent":"ab","index":"a2","type":"text","x":10,"content":"Hi","font_size":32}
            ]}"##,
        )
        .unwrap()
    }

    fn ids(texts: &[&str]) -> Vec<NodeId> {
        texts.iter().map(|text| NodeId::from(*text)).collect()
    }

    /// Nodes with the boxes their properties describe.
    fn framed(nodes: &[Node]) -> Vec<(&Node, Frame)> {
        nodes
            .iter()
            .map(|node| (node, Frame::of(node.layer().unwrap())))
            .collect()
    }

    #[test]
    fn proportions_come_from_the_box_that_is_drawn() {
        let doc = document();
        let text = doc.get(&NodeId::from("t")).unwrap();
        let drawn = Frame {
            x: 10.0,
            y: 0.0,
            width: 300.0,
            height: 50.0,
        };
        let mut moved = doc.clone();
        moved
            .apply(&resize_all(&[(text, drawn)], 100.0, true, true))
            .unwrap();
        let sized = text_of(moved.get(&NodeId::from("t")).unwrap()).unwrap();
        assert_eq!((sized.layer.width, sized.layer.height), (600.0, 100.0));
    }

    #[test]
    fn equal_values_are_shown_and_different_ones_are_mixed() {
        let doc = document();
        let both = Inspected::new(&doc, &ids(&["a", "b"]));
        let x = both.number(|node| Some(node.layer()?.x)).unwrap();
        assert_eq!(x, Shared::Same(10.0));
        let width = both.number(|node| Some(node.layer()?.width)).unwrap();
        assert!(width.is_mixed());
        assert_eq!(*width.value(), 200.0, "the first layer's value");
        let opacity = both.number(|node| Some(node.layer()?.opacity)).unwrap();
        assert!(opacity.is_mixed());
        // Only the layers that have a property count.
        let all = Inspected::new(&doc, &ids(&["a", "t"]));
        let size = all.number(|node| Some(text_of(node)?.font_size)).unwrap();
        assert_eq!(size, Shared::Same(32.0));
        assert!(all.any_of(NodeType::Text) && !all.all_of(NodeType::Text));
        assert_eq!(all.ids_where(|node| text_of(node).is_some()), ids(&["t"]));
        // Nothing selected, nothing to show.
        assert!(
            Inspected::new(&doc, &[])
                .number(|n| Some(n.layer()?.x))
                .is_none()
        );
    }

    #[test]
    fn setting_a_value_sets_it_everywhere() {
        let mut doc = document();
        doc.apply(&set_all(&ids(&["a", "b"]), "opacity", 25.0))
            .unwrap();
        let both = Inspected::new(&doc, &ids(&["a", "b"]));
        assert_eq!(
            both.number(|node| Some(node.layer()?.opacity)),
            Some(Shared::Same(25.0))
        );
        doc.apply(&patch_all(
            &ids(&["t"]),
            "font",
            serde_json::json!({"weight": 700}),
        ))
        .unwrap();
        let text = text_of(doc.get(&NodeId::from("t")).unwrap()).unwrap();
        assert_eq!(
            (text.font.weight, text.font.family.as_str()),
            (700, "Inter")
        );
    }

    #[test]
    fn resizing_can_keep_proportions() {
        let mut doc = document();
        let nodes: Vec<Node> = ["a", "b"]
            .iter()
            .map(|id| doc.get(&NodeId::from(*id)).unwrap().clone())
            .collect();
        let refs = framed(&nodes);
        doc.apply(&resize_all(&refs, 400.0, false, true)).unwrap();
        let size = |doc: &Document, id: &str| {
            let layer = doc.get(&NodeId::from(id)).unwrap().layer().unwrap().clone();
            (layer.width, layer.height)
        };
        assert_eq!(size(&doc, "a"), (400.0, 200.0));
        assert_eq!(size(&doc, "b"), (400.0, 400.0));
        let nodes: Vec<Node> = ["a"]
            .iter()
            .map(|id| doc.get(&NodeId::from(*id)).unwrap().clone())
            .collect();
        let refs = framed(&nodes);
        doc.apply(&resize_all(&refs, 50.0, true, false)).unwrap();
        assert_eq!(size(&doc, "a"), (400.0, 50.0));
    }

    #[test]
    fn sizing_text_stops_it_hugging_its_words() {
        let mut doc = document();
        let text = doc.get(&NodeId::from("t")).unwrap().clone();
        doc.apply(&resize_all(
            &framed(std::slice::from_ref(&text)),
            300.0,
            false,
            false,
        ))
        .unwrap();
        let sized = text_of(doc.get(&NodeId::from("t")).unwrap())
            .unwrap()
            .clone();
        assert_eq!(
            (sized.layer.width, sized.auto_size),
            (300.0, AutoSize::Height)
        );
        let text = doc.get(&NodeId::from("t")).unwrap().clone();
        doc.apply(&resize_all(
            &framed(std::slice::from_ref(&text)),
            80.0,
            true,
            false,
        ))
        .unwrap();
        let sized = text_of(doc.get(&NodeId::from("t")).unwrap()).unwrap();
        assert_eq!(sized.auto_size, AutoSize::Fixed);
    }

    #[test]
    fn tidy_rounds_to_hundredths() {
        assert_eq!(tidy(0.1 + 0.2), 0.3);
        assert_eq!(tidy(12.345_678), 12.35);
        assert_eq!(tidy(-0.001), 0.0);
        assert!(tidy(-0.001).is_sign_positive());
    }
}
